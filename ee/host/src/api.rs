//! The upload protocol, version 1: CI asks whether its repository is installed, sends the proof
//! with the hashes of its files, sends the blobs the host lacks, and marks the proof done.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use serde_json::{Value, json};
use wasm_bindgen::JsValue;
use worker::{Data, Env, Request, Response};

use crate::oidc::{self, Run};
use crate::{Answer, Config, Refusal, crypto, github, now, origin};

/// The `proof.json` schema the host knows.
const SCHEMA: u64 = 1;

pub async fn installed(req: &Request, config: &Config) -> Answer {
    let run = oidc::check(req, config).await?;
    let answer = match github::installation(config, &run.repo()).await? {
        Some(_) => json!({"installed": true}),
        None => json!({"installed": false, "install": config.install_url}),
    };
    Ok(Response::from_json(&answer)?)
}

#[derive(Deserialize)]
struct Upload {
    proof: Value,
    files: BTreeMap<String, File>,
}

#[derive(Deserialize)]
struct File {
    sha256: String,
    size: u64,
}

/// What the host reads of `proof.json`; the rest is kept as sent.
#[derive(Deserialize)]
struct Proof {
    schema: u64,
    repository: Option<String>,
    pull: Option<Pull>,
}

#[derive(Deserialize)]
struct Pull {
    number: u64,
}

pub async fn create(mut req: Request, env: &Env, config: &Config) -> Answer {
    let run = oidc::check(&req, config).await?;
    let upload: Upload = req
        .json()
        .await
        .map_err(|_| Refusal::new(400, "send {\"proof\": <proof.json>, \"files\": {...}}"))?;
    let proof: Proof = serde_json::from_value(upload.proof.clone())
        .map_err(|_| Refusal::new(400, "proof is not a proof.json"))?;
    if proof.schema != SCHEMA {
        return Err(Refusal::new(
            400,
            format!(
                "proof.json schema {} is unknown here; this host reads {SCHEMA}",
                proof.schema
            ),
        ));
    }
    if let Some(repository) = &proof.repository
        && repository.to_lowercase() != run.repo()
    {
        return Err(Refusal::new(403, "the token is for another repository"));
    }
    let pull = proof.pull.map(|p| p.number);
    refuse_other_pull(&run, pull)?;
    if let Some((path, _)) = upload
        .files
        .iter()
        .find(|(_, file)| !is_sha256(&file.sha256))
    {
        return Err(Refusal::new(
            400,
            format!("{path}: sha256 is not 64 lowercase hex digits"),
        ));
    }
    if github::installation(config, &run.repo()).await?.is_none() {
        return Err(Refusal::new(
            403,
            format!(
                "the Fearlezz app is not installed on {}: {}",
                run.repository, config.install_url
            ),
        ));
    }

    let db = env.d1("DB")?;
    let id = crypto::random_id()?;
    let repo = run.repo();
    let mut statements = vec![
        db.prepare(
            "INSERT INTO proofs (id, repo, pull, proof, created) VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(&[
            id.as_str().into(),
            repo.as_str().into(),
            pull.map_or(JsValue::NULL, |n| (n as f64).into()),
            upload.proof.to_string().into(),
            (now() as f64).into(),
        ])?,
    ];
    for (path, file) in &upload.files {
        statements.push(
            db.prepare("INSERT INTO files (proof, path, sha256, size) VALUES (?1, ?2, ?3, ?4)")
                .bind(&[
                    id.as_str().into(),
                    path.as_str().into(),
                    file.sha256.as_str().into(),
                    (file.size as f64).into(),
                ])?,
        );
    }
    db.batch(statements).await?;

    #[derive(Deserialize)]
    struct Held {
        sha256: String,
    }
    let held: BTreeSet<String> = db
        .prepare(
            "SELECT sha256 FROM blobs WHERE repo = ?1 AND sha256 IN (SELECT sha256 FROM files WHERE proof = ?2)",
        )
        .bind(&[repo.as_str().into(), id.as_str().into()])?
        .all()
        .await?
        .results::<Held>()?
        .into_iter()
        .map(|h| h.sha256)
        .collect();
    let missing: BTreeSet<&str> = upload
        .files
        .values()
        .map(|f| f.sha256.as_str())
        .filter(|sha| !held.contains(*sha))
        .collect();
    let url = page_url(&req, &run, &id)?;
    let answer = json!({"id": id, "url": url, "missing": missing});
    Ok(Response::from_json(&answer)?.with_status(201))
}

pub async fn blob(req: Request, env: &Env, config: &Config, sha256: &str) -> Answer {
    let run = oidc::check(&req, config).await?;
    if !is_sha256(sha256) {
        return Err(Refusal::new(
            400,
            "a blob is named by its sha256 in lowercase hex",
        ));
    }
    let db = env.d1("DB")?;
    let repo = run.repo();
    // Only a proof of this repository makes room for a blob, so a token from a repository
    // without the app stores nothing.
    let size: Option<f64> = db
        .prepare(
            "SELECT f.size FROM files f JOIN proofs p ON p.id = f.proof WHERE p.repo = ?1 AND f.sha256 = ?2 LIMIT 1",
        )
        .bind(&[repo.as_str().into(), sha256.into()])?
        .first(Some("size"))
        .await?;
    let Some(size) = size else {
        return Err(Refusal::new(
            409,
            "no proof of this repository names this blob",
        ));
    };
    let length = req
        .headers()
        .get("Content-Length")?
        .and_then(|l| l.parse::<f64>().ok());
    if length != Some(size) {
        return Err(Refusal::new(
            400,
            format!("send the blob's {size} bytes with Content-Length"),
        ));
    }
    let body = match req.inner().body() {
        Some(stream) => Data::ReadableStream(stream),
        None => Data::Empty,
    };
    let expected = hex::decode(sha256).map_err(|e| worker::Error::RustError(e.to_string()))?;
    // R2 checks the bytes against the hash and keeps nothing when they differ.
    let key = format!("blobs/{repo}/{sha256}");
    if let Err(error) = env
        .bucket("BLOBS")?
        .put(key, body)
        .sha256(expected)
        .execute()
        .await
    {
        let message = error.to_string();
        if message.contains("checksum") || message.contains("match") {
            return Err(Refusal::new(
                400,
                "the bytes do not hash to the blob's name",
            ));
        }
        return Err(error.into());
    }
    db.prepare("INSERT OR IGNORE INTO blobs (repo, sha256) VALUES (?1, ?2)")
        .bind(&[repo.as_str().into(), sha256.into()])?
        .run()
        .await?;
    Ok(Response::empty()?.with_status(204))
}

pub async fn done(req: &Request, env: &Env, config: &Config, id: &str) -> Answer {
    let run = oidc::check(req, config).await?;
    let db = env.d1("DB")?;
    let repo = run.repo();
    #[derive(Deserialize)]
    struct Row {
        pull: Option<f64>,
    }
    let row: Option<Row> = db
        .prepare("SELECT pull FROM proofs WHERE id = ?1 AND repo = ?2")
        .bind(&[id.into(), repo.as_str().into()])?
        .first(None)
        .await?;
    let Some(row) = row else {
        return Err(Refusal::new(404, "no such proof for this repository"));
    };
    let pull = row.pull.map(|n| n as u64);
    refuse_other_pull(&run, pull)?;
    let missing: f64 = db
        .prepare(
            "SELECT COUNT(*) AS missing FROM files f WHERE f.proof = ?1 AND NOT EXISTS (SELECT 1 FROM blobs b WHERE b.repo = ?2 AND b.sha256 = f.sha256)",
        )
        .bind(&[id.into(), repo.as_str().into()])?
        .first(Some("missing"))
        .await?
        .unwrap_or(0.0);
    if missing > 0.0 {
        return Err(Refusal::new(
            409,
            format!("{missing} files are still missing"),
        ));
    }
    let mut statements = vec![
        db.prepare("UPDATE proofs SET done = 1 WHERE id = ?1")
            .bind(&[id.into()])?,
    ];
    if let Some(pull) = pull {
        statements.push(
            db.prepare(
                "INSERT INTO latest (repo, pull, proof) VALUES (?1, ?2, ?3) ON CONFLICT (repo, pull) DO UPDATE SET proof = excluded.proof",
            )
            .bind(&[repo.as_str().into(), (pull as f64).into(), id.into()])?,
        );
    }
    db.batch(statements).await?;
    Ok(Response::from_json(
        &json!({"url": page_url(req, &run, id)?}),
    )?)
}

/// A pull request's proof comes only from a run of that pull request.
fn refuse_other_pull(run: &Run, pull: Option<u64>) -> Result<(), Refusal> {
    match pull {
        Some(number) if !run.is_for_pull(number) => Err(Refusal::new(
            403,
            format!(
                "the token is for {}, not pull request {number}",
                run.git_ref
            ),
        )),
        _ => Ok(()),
    }
}

fn page_url(req: &Request, run: &Run, id: &str) -> worker::Result<String> {
    Ok(format!("{}/{}/proofs/{id}/", origin(req)?, run.repository))
}

fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}
