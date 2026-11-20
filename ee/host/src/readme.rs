//! README pictures: a stable address on the files origin for each picture a check takes,
//! `/readme/<owner>/<repo>/<check>/<picture>`, answering with the newest proven one of a push to
//! the repository's default branch, so a README can embed what the proof shows and follows each
//! push without anyone copying files.
//!
//! A README's picture loads for anyone, through GitHub's image proxy, without a sign in, so only
//! a public repository's pictures are served and the address holds no grant: anything else is
//! `404`, as on the rest of this origin. Only a CI run of a push counts (its OIDC token's ref is
//! `refs/heads/<default branch>`), never a pull request, another branch or a local run, and only
//! one whose proof did not fail, as the host's review judges it (`fearlezz_proof::review::failed`): a README
//! shows what the default branch proved, so a newer push that failed leaves the older picture.
//!
//! The picture is the after of the look's item of that name: the commit's own picture. A check id
//! and a picture name may both hold slashes, so the path names the longest check of the run that
//! its leading segments spell, and the picture is the rest.

use fearlezz_proof::Proof;
use fearlezz_proof::look::Look;
use serde::Deserialize;
use worker::{Env, Headers, Request, Response, wasm_bindgen::JsValue};

use crate::work::read;
use crate::{Answer, Config, Refusal, access, files, not_found, repo_key, review};

/// How long a cache keeps a README picture: a push shows within minutes, and GitHub's image
/// proxy does not ask the host on every view of the README.
const CACHE: &str = "public, max-age=300";

/// How many of the newest done pushes that ran the check are looked through for one whose proof
/// did not fail.
const NEWEST: usize = 20;

pub(crate) async fn serve(req: &Request, env: &Env, config: &Config, path: &str) -> Answer {
    let segments = path
        .split('/')
        .map(files::decoded)
        .collect::<Result<Vec<_>, _>>()?;
    let [owner, name, rest @ ..] = segments.as_slice() else {
        return Err(not_found());
    };
    if rest.len() < 2 || segments.iter().any(String::is_empty) {
        return Err(not_found());
    }
    let repo = repo_key(owner, name);
    let branch = access::readme_branch(env, config, &repo)
        .await?
        .ok_or_else(not_found)?;
    let (run, picture) = newest(env, &repo, &branch, rest)
        .await?
        .ok_or_else(not_found)?;
    let stored = files::stored(env, &repo, &run, &picture).await?;
    let tag = format!("\"{}\"", stored.sha256);
    let holds = req.headers().get("If-None-Match")?.is_some_and(|tags| {
        tags.split(',')
            .any(|t| t.trim().trim_start_matches("W/") == tag)
    });
    if holds {
        let headers = Headers::new();
        headers.set("ETag", &tag)?;
        headers.set("Cache-Control", CACHE)?;
        return Ok(Response::empty()?.with_status(304).with_headers(headers));
    }
    let mut response = files::send(req, env, config, &stored, CACHE).await?;
    response.headers_mut().set("ETag", &tag)?;
    Ok(response)
}

/// The run and the path of its file that a README shows for `segments` (the check, then the
/// picture) of `repo` (lowercased): the newest done CI run of a push to `branch` that ran the
/// check, took its pictures and whose proof did not fail. None when there is none, or when that
/// run took no such picture or it is gone.
async fn newest(
    env: &Env,
    repo: &str,
    branch: &str,
    segments: &[String],
) -> Result<Option<(String, String)>, Refusal> {
    #[derive(Deserialize)]
    struct Row {
        run: String,
        work: String,
        head: String,
        proof: Option<String>,
        check_id: String,
        look: String,
    }
    // Every way the segments split into a check and a picture.
    let checks: Vec<String> = (1..segments.len())
        .map(|k| segments.get(..k).unwrap_or_default().join("/"))
        .collect();
    let places: Vec<String> = (0..checks.len()).map(|i| format!("?{}", i + 4)).collect();
    let sql = format!(
        "SELECT r.id AS run, w.id AS work, r.head, r.proof, c.id AS check_id, c.look FROM runs r JOIN works w ON w.id = r.work JOIN checks c ON c.run = r.id WHERE w.repo = ?1 AND w.branch = ?2 AND r.git_ref = ?3 AND r.state = 'done' AND c.look IS NOT NULL AND c.id IN ({}) ORDER BY r.opened DESC, r.rowid DESC, length(c.id) DESC LIMIT {NEWEST}",
        places.join(", ")
    );
    let mut binds: Vec<JsValue> = vec![
        repo.into(),
        branch.into(),
        format!("refs/heads/{branch}").into(),
    ];
    binds.extend(checks.iter().map(|c| JsValue::from(c.as_str())));
    let db = env.d1("DB")?;
    let rows = db
        .prepare(sql)
        .bind(&binds)?
        .all()
        .await?
        .results::<Row>()?;
    for row in rows {
        let Some(proof) = row.proof.as_deref() else {
            continue;
        };
        let proof: Proof = read(proof)?;
        let accept = review::accept(&db, &row.work, &row.head).await?;
        if fearlezz_proof::review::failed(&proof, &row.head, accept.as_ref()) {
            continue;
        }
        let look: Look = read(&row.look)?;
        let Some(at) = checks.iter().position(|c| *c == row.check_id) else {
            continue;
        };
        let picture = segments.get(at + 1..).unwrap_or_default().join("/");
        let after = look
            .items
            .into_iter()
            .find(|item| item.name == picture)
            .and_then(|item| item.after);
        return Ok(after.map(|after| (row.run, after)));
    }
    Ok(None)
}
