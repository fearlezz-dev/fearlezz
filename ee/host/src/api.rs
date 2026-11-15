//! The protocol, version 2: CI asks whether its repository is installed, opens a run before
//! any check runs, says when each check starts and ends with the files it made, sends the
//! blobs the host lacks, and closes the run with `proof.json`. Every write lands in D1 first
//! and then goes to the pages open on the work.

use std::collections::BTreeSet;

use fearlezz_proof::events::{
    Change, Closed, Done, Files, Live, Missing, Open, Opened, Run, RunCheck, RunState, Step,
};
use fearlezz_proof::look::Look;
use fearlezz_proof::{Check, SCHEMA};
use serde::Deserialize;
use serde_json::{Value, json};
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, Data, Env, Request, Response};

use crate::oidc::{self, Token};
use crate::work::{self, WorkRow};
use crate::{Answer, Config, Refusal, crypto, github, now};

pub async fn installed(req: &Request, env: &Env, config: &Config) -> Answer {
    let token = oidc::check(req, env, config).await?;
    let answer = match github::installation(config, &token.repo()).await? {
        Some(_) => json!({"installed": true}),
        None => json!({"installed": false, "install": config.install_url}),
    };
    Ok(Response::from_json(&answer)?)
}

pub async fn open(mut req: Request, env: &Env, config: &Config) -> Answer {
    let token = oidc::check(&req, env, config).await?;
    let open: Open = req
        .json()
        .await
        .map_err(|e| Refusal::new(400, format!("send the run as protocol 2 opens it: {e}")))?;
    if open.repository.to_lowercase() != token.repo() {
        return Err(Refusal::new(403, "the token is for another repository"));
    }
    let allowed = match &open.pull {
        Some(pull) => token.is_for_pull(pull.number, &open.branch),
        None => token.is_for_branch(&open.branch),
    };
    if !allowed {
        let what = match &open.pull {
            Some(pull) => format!("pull request {} from {}", pull.number, open.branch),
            None => format!("a push of {}", open.branch),
        };
        return Err(Refusal::new(
            403,
            format!("the token is for {}, not {what}", token.git_ref),
        ));
    }
    if github::installation(config, &token.repo()).await?.is_none() {
        return Err(Refusal::new(
            403,
            format!(
                "the Fearlezz app is not installed on {}: {}",
                token.repository, config.install_url
            ),
        ));
    }

    let db = env.d1("DB")?;
    let repo = token.repo();
    let pull = open.pull.as_ref();
    // A work is one branch; the pull request it gets is kept once a run names one.
    let work: WorkRow = db
        .prepare(
            "INSERT INTO works (id, repo, repository, branch, pull, pull_title, pull_url) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
             ON CONFLICT (repo, branch) DO UPDATE SET repository = excluded.repository, \
             pull = COALESCE(excluded.pull, works.pull), pull_title = COALESCE(excluded.pull_title, works.pull_title), \
             pull_url = COALESCE(excluded.pull_url, works.pull_url) \
             RETURNING id, repository, branch, pull, pull_title, pull_url",
        )
        .bind(&[
            crypto::random_id()?.into(),
            repo.as_str().into(),
            token.repository.as_str().into(),
            open.branch.as_str().into(),
            pull.map_or(JsValue::NULL, |p| (p.number as f64).into()),
            pull.map_or(JsValue::NULL, |p| p.title.as_str().into()),
            pull.and_then(|p| p.url.as_deref())
                .map_or(JsValue::NULL, JsValue::from),
        ])?
        .first(None)
        .await?
        .ok_or_else(|| worker::Error::RustError("D1 kept no work".into()))?;

    let id = crypto::random_id()?;
    let opened = now() as f64;
    let source = serde_json::to_string(&open.source)?;
    let mut planned: Vec<&String> = Vec::new();
    for check in &open.checks {
        if !planned.contains(&check) {
            planned.push(check);
        }
    }
    let mut statements = vec![
        db.prepare(
            "INSERT INTO runs (id, work, git_ref, head, base, source, state, opened) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'running', ?7)",
        )
        .bind(&[
            id.as_str().into(),
            work.id.as_str().into(),
            token.git_ref.as_str().into(),
            open.head.as_str().into(),
            open.base.as_str().into(),
            source.as_str().into(),
            opened.into(),
        ])?,
    ];
    for (position, check) in planned.iter().enumerate() {
        statements.push(
            db.prepare("INSERT INTO checks (run, id, position) VALUES (?1, ?2, ?3)")
                .bind(&[
                    id.as_str().into(),
                    check.as_str().into(),
                    (position as f64).into(),
                ])?,
        );
    }
    statements.push(work::bump(&db, &work.id)?);
    let seq = work::commit(&db, statements).await?;

    let run = Run {
        id: id.clone(),
        head: open.head,
        base: open.base,
        source: open.source,
        state: RunState::Running,
        opened,
        checks: planned
            .into_iter()
            .map(|check| RunCheck {
                id: check.clone(),
                started: None,
                entry: None,
                look: None,
            })
            .collect(),
    };
    let change = Change::Opened {
        work: work.work(),
        run,
    };
    notify(env, &work.id, seq, &id, change).await;
    let url = work::page_url(
        config,
        &work.repository,
        &work.branch,
        work.pull.map(|n| n as u64),
    );
    let answer = Opened { run: id, url };
    Ok(Response::from_json(&answer)?.with_status(201))
}

/// A run as the calls after opening it need it.
#[derive(Deserialize)]
struct RunRow {
    work: String,
    git_ref: String,
    state: String,
    repository: String,
    branch: String,
    pull: Option<f64>,
}

/// The run `id`, when the token may write it: only a token of the run's own repository finds
/// it, and only one for the same ref (the same pull request, or the same pushed branch) may
/// write it. A closed run takes nothing more.
async fn writable(db: &D1Database, token: &Token, id: &str) -> Result<RunRow, Refusal> {
    let row: Option<RunRow> = db
        .prepare(
            "SELECT r.work, r.git_ref, r.state, w.repository, w.branch, w.pull FROM runs r JOIN works w ON w.id = r.work WHERE r.id = ?1 AND w.repo = ?2",
        )
        .bind(&[id.into(), token.repo().as_str().into()])?
        .first(None)
        .await?;
    let row = row.ok_or_else(|| Refusal::new(404, "no such run for this repository"))?;
    if row.git_ref != token.git_ref {
        return Err(Refusal::new(
            403,
            format!(
                "the token is for {}, not {}, which opened this run",
                token.git_ref, row.git_ref
            ),
        ));
    }
    if work::state(&row.state) != RunState::Running {
        return Err(Refusal::new(409, "the run is closed; open another"));
    }
    Ok(row)
}

pub async fn step(mut req: Request, env: &Env, config: &Config, id: &str) -> Answer {
    let token = oidc::check(&req, env, config).await?;
    let step: Step = req
        .json()
        .await
        .map_err(|e| Refusal::new(400, format!("send a check's start or end: {e}")))?;
    let db = env.d1("DB")?;
    let run = writable(&db, &token, id).await?;
    // A check the run did not plan joins at the end.
    let place = "(SELECT COALESCE(MAX(position), -1) + 1 FROM checks WHERE run = ?1)";
    let (statements, change, files) = match step {
        Step::Started { check } => {
            refuse_empty(&check)?;
            let at = now() as f64;
            let sql = format!(
                "INSERT INTO checks (run, id, position, started) VALUES (?1, ?2, {place}, ?3) \
                 ON CONFLICT (run, id) DO UPDATE SET started = excluded.started"
            );
            let statement =
                db.prepare(&sql)
                    .bind(&[id.into(), check.as_str().into(), at.into()])?;
            (vec![statement], Change::Started { check, at }, Files::new())
        }
        Step::Finished { check, files, look } => {
            refuse_empty(&check.id)?;
            refuse_bad_files(&files)?;
            if let Some(look) = &look
                && look.check != check.id
            {
                return Err(Refusal::new(
                    400,
                    format!("the look is of {}, not of {}", look.check, check.id),
                ));
            }
            let mut statements = vec![entry(&db, id, &check)?];
            if let Some(look) = &look {
                statements.push(record_look(&db, id, look)?);
            }
            statements.extend(record_files(&db, id, &files)?);
            let change = Change::Finished {
                check: *check,
                look,
            };
            (statements, change, files)
        }
    };
    let mut statements = statements;
    statements.push(work::bump(&db, &run.work)?);
    let seq = work::commit(&db, statements).await?;
    // A look whose pictures are not all in yet reaches pages with the upload that completes it.
    let change = match change {
        Change::Finished {
            check,
            look: Some(look),
        } if !work::complete(&look, &work::held(&db, id).await?) => {
            Change::Finished { check, look: None }
        }
        change => change,
    };
    notify(env, &run.work, seq, id, change).await;
    let missing = lacks(&db, &token.repo(), &files).await?;
    Ok(Response::from_json(&Missing { missing })?)
}

/// The statement that records a check's entry, adding the check when the run did not plan it.
fn entry(db: &D1Database, run: &str, check: &Check) -> worker::Result<D1PreparedStatement> {
    let place = "(SELECT COALESCE(MAX(position), -1) + 1 FROM checks WHERE run = ?1)";
    let sql = format!(
        "INSERT INTO checks (run, id, position, entry) VALUES (?1, ?2, {place}, ?3) \
         ON CONFLICT (run, id) DO UPDATE SET entry = excluded.entry"
    );
    db.prepare(&sql).bind(&[
        run.into(),
        check.id.as_str().into(),
        serde_json::to_string(check)?.into(),
    ])
}

/// The statement that records a check's look, adding the check when the run did not plan it.
fn record_look(db: &D1Database, run: &str, look: &Look) -> worker::Result<D1PreparedStatement> {
    let place = "(SELECT COALESCE(MAX(position), -1) + 1 FROM checks WHERE run = ?1)";
    let sql = format!(
        "INSERT INTO checks (run, id, position, look) VALUES (?1, ?2, {place}, ?3) \
         ON CONFLICT (run, id) DO UPDATE SET look = excluded.look"
    );
    db.prepare(&sql).bind(&[
        run.into(),
        look.check.as_str().into(),
        serde_json::to_string(look)?.into(),
    ])
}

/// The statements that record a run's files, a later one replacing an earlier at its path.
fn record_files(
    db: &D1Database,
    run: &str,
    files: &Files,
) -> worker::Result<Vec<D1PreparedStatement>> {
    files
        .iter()
        .map(|(path, file)| {
            db.prepare(
                "INSERT INTO files (run, path, sha256, size) VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT (run, path) DO UPDATE SET sha256 = excluded.sha256, size = excluded.size",
            )
            .bind(&[
                run.into(),
                path.as_str().into(),
                file.sha256.as_str().into(),
                (file.size as f64).into(),
            ])
        })
        .collect()
}

/// The blobs among `files` the repository does not hold yet, sorted, once each.
async fn lacks(db: &D1Database, repo: &str, files: &Files) -> worker::Result<Vec<String>> {
    let wanted: BTreeSet<&str> = files.values().map(|f| f.sha256.as_str()).collect();
    let mut missing = Vec::new();
    for sha in wanted {
        let held: Option<f64> = db
            .prepare("SELECT 1 AS held FROM blobs WHERE repo = ?1 AND sha256 = ?2")
            .bind(&[repo.into(), sha.into()])?
            .first(Some("held"))
            .await?;
        if held.is_none() {
            missing.push(sha.to_owned());
        }
    }
    Ok(missing)
}

/// The version a `proof.json` says it is, read before the rest, which another version may
/// shape otherwise.
#[derive(Deserialize)]
struct Version {
    schema: u32,
}

pub async fn done(mut req: Request, env: &Env, config: &Config, id: &str) -> Answer {
    let token = oidc::check(&req, env, config).await?;
    let body: Value = req
        .json()
        .await
        .map_err(|_| Refusal::new(400, "send {\"proof\": <proof.json>, \"files\": {...}}"))?;
    let version: Version = serde_json::from_value(body["proof"].clone())
        .map_err(|_| Refusal::new(400, "proof is not a proof.json"))?;
    if version.schema != SCHEMA {
        return Err(Refusal::new(
            400,
            format!(
                "proof.json schema {} is unknown here; this host reads {SCHEMA}",
                version.schema
            ),
        ));
    }
    let done: Done = serde_json::from_value(body)
        .map_err(|e| Refusal::new(400, format!("proof is not a proof.json: {e}")))?;
    if let Some(repository) = &done.proof.repository
        && repository.to_lowercase() != token.repo()
    {
        return Err(Refusal::new(403, "the proof is of another repository"));
    }
    refuse_bad_files(&done.files)?;
    let db = env.d1("DB")?;
    let run = writable(&db, &token, id).await?;
    let recorded = record_files(&db, id, &done.files)?;
    if !recorded.is_empty() {
        db.batch(recorded).await?;
    }
    let missing = run_lacks(&db, &token.repo(), id).await?;
    if !missing.is_empty() {
        let answer = json!({
            "error": format!("{} files are still missing; send them, then done again", missing.len()),
            "missing": missing,
        });
        return Ok(Response::from_json(&answer)?.with_status(409));
    }
    let mut statements = vec![
        db.prepare("UPDATE runs SET state = 'done', proof = ?2 WHERE id = ?1")
            .bind(&[id.into(), serde_json::to_string(&done.proof)?.into()])?,
    ];
    // The proof's entries are the final word: red before green and instability come last.
    for check in &done.proof.checks {
        statements.push(entry(&db, id, check)?);
    }
    for look in &done.proof.looks {
        statements.push(record_look(&db, id, look)?);
    }
    statements.push(work::bump(&db, &run.work)?);
    let seq = work::commit(&db, statements).await?;
    let change = Change::Done {
        checks: done.proof.checks.clone(),
        looks: done.proof.looks.clone(),
    };
    notify(env, &run.work, seq, id, change).await;
    let url = work::page_url(
        config,
        &run.repository,
        &run.branch,
        run.pull.map(|n| n as u64),
    );
    Ok(Response::from_json(&Closed { url })?)
}

/// Every blob a run's files name that the repository does not hold yet.
async fn run_lacks(db: &D1Database, repo: &str, run: &str) -> worker::Result<Vec<String>> {
    #[derive(Deserialize)]
    struct Row {
        sha256: String,
    }
    Ok(db
        .prepare(
            "SELECT DISTINCT f.sha256 FROM files f WHERE f.run = ?1 AND NOT EXISTS (SELECT 1 FROM blobs b WHERE b.repo = ?2 AND b.sha256 = f.sha256) ORDER BY f.sha256",
        )
        .bind(&[run.into(), repo.into()])?
        .all()
        .await?
        .results::<Row>()?
        .into_iter()
        .map(|row| row.sha256)
        .collect())
}

async fn notify(env: &Env, work: &str, seq: u64, run: &str, change: Change) {
    let live = Live::Event {
        seq,
        run: run.to_owned(),
        change: Box::new(change),
    };
    work::notify(env, work, &live).await;
}

pub async fn blob(req: Request, env: &Env, config: &Config, sha256: &str) -> Answer {
    let token = oidc::check(&req, env, config).await?;
    if !is_sha256(sha256) {
        return Err(Refusal::new(
            400,
            "a blob is named by its sha256 in lowercase hex",
        ));
    }
    let db = env.d1("DB")?;
    let repo = token.repo();
    // Only a run of this repository makes room for a blob, so a token from a repository
    // without the app stores nothing.
    let size: Option<f64> = db
        .prepare(
            "SELECT f.size FROM files f JOIN runs r ON r.id = f.run JOIN works w ON w.id = r.work WHERE w.repo = ?1 AND f.sha256 = ?2 LIMIT 1",
        )
        .bind(&[repo.as_str().into(), sha256.into()])?
        .first(Some("size"))
        .await?;
    let Some(size) = size else {
        return Err(Refusal::new(
            409,
            "no run of this repository names this blob",
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
    let held: Option<f64> = db
        .prepare("SELECT 1 AS held FROM blobs WHERE repo = ?1 AND sha256 = ?2")
        .bind(&[repo.as_str().into(), sha256.into()])?
        .first(Some("held"))
        .await?;
    db.prepare("INSERT OR IGNORE INTO blobs (repo, sha256) VALUES (?1, ?2)")
        .bind(&[repo.as_str().into(), sha256.into()])?
        .run()
        .await?;
    if held.is_none() {
        show_completed_looks(env, &db, &repo, sha256).await?;
    }
    Ok(Response::empty()?.with_status(204))
}

/// Sends pages each look the blob `sha256` may have just completed: every look of a run of
/// `repo` with a file of these bytes that now holds every file it names. Sending one that was
/// complete before changes nothing on a page.
async fn show_completed_looks(
    env: &Env,
    db: &D1Database,
    repo: &str,
    sha256: &str,
) -> worker::Result<()> {
    #[derive(Deserialize)]
    struct Row {
        run: String,
        work: String,
        entry: String,
        look: String,
    }
    let rows = db
        .prepare(
            "SELECT c.run, r.work, c.entry, c.look FROM checks c JOIN runs r ON r.id = c.run JOIN works w ON w.id = r.work WHERE w.repo = ?1 AND c.look IS NOT NULL AND c.entry IS NOT NULL AND EXISTS (SELECT 1 FROM files f WHERE f.run = c.run AND f.sha256 = ?2)",
        )
        .bind(&[repo.into(), sha256.into()])?
        .all()
        .await?
        .results::<Row>()?;
    for row in rows {
        let look: Look = work::read(&row.look)?;
        if !work::complete(&look, &work::held(db, &row.run).await?) {
            continue;
        }
        let check: Check = work::read(&row.entry)?;
        let seq = work::commit(db, vec![work::bump(db, &row.work)?]).await?;
        let change = Change::Finished {
            check,
            look: Some(Box::new(look)),
        };
        notify(env, &row.work, seq, &row.run, change).await;
    }
    Ok(())
}

fn refuse_empty(check: &str) -> Result<(), Refusal> {
    if check.trim().is_empty() {
        return Err(Refusal::new(400, "a check needs its id"));
    }
    Ok(())
}

fn refuse_bad_files(files: &Files) -> Result<(), Refusal> {
    match files.iter().find(|(_, file)| !is_sha256(&file.sha256)) {
        Some((path, _)) => Err(Refusal::new(
            400,
            format!("{path}: sha256 is not 64 lowercase hex digits"),
        )),
        None => Ok(()),
    }
}

fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}
