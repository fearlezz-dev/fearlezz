//! The protocol, version 2: CI, or a signed in CLI, asks whether its repository is installed,
//! opens a run before any check runs, says when each check starts and ends with the files it
//! made, sends the blobs the host lacks, and closes the run with `proof.json`. Every write lands
//! in D1 first and then goes to the pages open on the work.
//!
//! CI writes with its OIDC token, which acts for one repository and ref. A person's CLI writes
//! with its host token (`cli.rs`), and only that person's local runs of repositories they may
//! write: a local run is the agent's word, so it never moves the work's head, its review, the
//! GitHub check run or the pull request comment.

use std::collections::BTreeSet;

use fearlezz_proof::events::{
    Change, Closed, Done, Files, Keys, Live, Missing, Open, Opened, Run, RunCheck, RunState,
    Source, Step,
};
use fearlezz_proof::look::Look;
use fearlezz_proof::review as reviews;
use fearlezz_proof::review::Role;
use fearlezz_proof::{Check, SCHEMA};
use serde::Deserialize;
use serde_json::{Value, json};
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, Data, Env, Request, Response};

use crate::cli::{self, Person};
use crate::oidc::{self, Token};
use crate::work::{self, WorkRow};
use crate::{Answer, Config, Refusal, crypto, github, now, review, say};

/// Who writes a run.
enum Writer {
    /// A CI job, by its OIDC token: one repository and ref.
    Ci(Token),
    /// A person's CLI, by its host token: their own local runs.
    Local(Person),
}

impl Writer {
    /// The writer of the request: a CLI token when it carries one, CI's OIDC token otherwise.
    async fn of(req: &Request, env: &Env, config: &Config) -> Result<Writer, Refusal> {
        if cli::carries(req)? {
            return Ok(Writer::Local(cli::person(req, env, config).await?));
        }
        Ok(Writer::Ci(oidc::check(req, env, config).await?))
    }

    /// What a run it opens is kept under (`runs.git_ref`): only a writer with the same writes
    /// it again. A CI token's ref, or the person's login for a local run.
    fn git_ref(&self) -> String {
        match self {
            Writer::Ci(token) => token.git_ref.clone(),
            Writer::Local(person) => format!("local:{}", person.login.to_lowercase()),
        }
    }
}

/// Refuses a person who may not write `repository`, as GitHub says, kept for a few minutes.
async fn may_write(
    env: &Env,
    config: &Config,
    person: &Person,
    repository: &str,
) -> Result<(), Refusal> {
    if say::role(env, config, &person.login, &person.token, repository).await? < Role::Write {
        return Err(Refusal::new(
            403,
            format!(
                "@{} cannot send local runs of {repository}: that needs write access on GitHub",
                person.login
            ),
        ));
    }
    Ok(())
}

pub async fn installed(req: &Request, env: &Env, config: &Config) -> Answer {
    let repo = match Writer::of(req, env, config).await? {
        Writer::Ci(token) => token.repo(),
        // A CLI token speaks for a person, who names the repository they would send.
        Writer::Local(person) => {
            let url = req.url()?;
            let repository = url
                .query_pairs()
                .find(|(name, _)| name == "repository")
                .map(|(_, value)| value.into_owned())
                .filter(|r| r.split('/').count() == 2)
                .ok_or_else(|| Refusal::new(400, "name the repository: ?repository=owner/name"))?;
            may_write(env, config, &person, &repository).await?;
            repository.to_lowercase()
        }
    };
    let answer = match github::installation(config, &repo).await? {
        Some(_) => json!({"installed": true}),
        None => json!({"installed": false, "install": config.install_url}),
    };
    Ok(Response::from_json(&answer)?)
}

pub async fn open(mut req: Request, env: &Env, config: &Config) -> Answer {
    let writer = Writer::of(&req, env, config).await?;
    let mut open: Open = req
        .json()
        .await
        .map_err(|e| Refusal::new(400, format!("send the run as protocol 2 opens it: {e}")))?;
    let repository = match &writer {
        Writer::Ci(token) => {
            ci_may_open(token, &open)?;
            token.repository.clone()
        }
        Writer::Local(person) => {
            if !matches!(open.source, Source::Local { .. }) {
                return Err(Refusal::new(400, "a CLI token opens only a local run"));
            }
            if open.pull.is_some() {
                return Err(Refusal::new(
                    400,
                    "a local run names no pull request; its work is found by its branch",
                ));
            }
            if open.repository.split('/').count() != 2 {
                return Err(Refusal::new(400, "name the repository as owner/name"));
            }
            may_write(env, config, person, &open.repository).await?;
            // Who ran it is the token's word, not the run's.
            open.source = Source::Local {
                login: person.login.clone(),
            };
            open.repository.clone()
        }
    };
    let repo = repository.to_lowercase();
    if github::installation(config, &repo).await?.is_none() {
        return Err(Refusal::new(
            403,
            format!(
                "the Fearlezz app is not installed on {repository}: {}",
                config.install_url
            ),
        ));
    }

    let db = env.d1("DB")?;
    let local = matches!(writer, Writer::Local(_));
    let pull = open.pull.as_ref();
    // A work is one branch; the pull request it gets is kept once a run names one. A local run
    // finds or makes the work and changes nothing of it: its head is what CI and GitHub say.
    let statement = if local {
        db.prepare(
            "INSERT INTO works (id, repo, repository, branch) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT (repo, branch) DO UPDATE SET seq = works.seq \
             RETURNING id, repository, branch, pull, pull_title, pull_url",
        )
        .bind(&[
            crypto::random_id()?.into(),
            repo.as_str().into(),
            repository.as_str().into(),
            open.branch.as_str().into(),
        ])?
    } else {
        db.prepare(
            "INSERT INTO works (id, repo, repository, branch, pull, pull_title, pull_url, head) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
             ON CONFLICT (repo, branch) DO UPDATE SET repository = excluded.repository, \
             pull = COALESCE(excluded.pull, works.pull), pull_title = COALESCE(excluded.pull_title, works.pull_title), \
             pull_url = COALESCE(excluded.pull_url, works.pull_url), head = excluded.head \
             RETURNING id, repository, branch, pull, pull_title, pull_url",
        )
        .bind(&[
            crypto::random_id()?.into(),
            repo.as_str().into(),
            repository.as_str().into(),
            open.branch.as_str().into(),
            pull.map_or(JsValue::NULL, |p| (p.number as f64).into()),
            pull.map_or(JsValue::NULL, |p| p.title.as_str().into()),
            pull.and_then(|p| p.url.as_deref())
                .map_or(JsValue::NULL, JsValue::from),
            open.head.as_str().into(),
        ])?
    };
    let work: WorkRow = statement
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
            "INSERT INTO runs (id, work, git_ref, head, base, source, state, opened, touched) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'running', ?7, ?7)",
        )
        .bind(&[
            id.as_str().into(),
            work.id.as_str().into(),
            writer.git_ref().as_str().into(),
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
                keys: Keys::new(),
            })
            .collect(),
    };
    let change = Change::Opened {
        work: work.work(),
        run,
    };
    notify(env, &work.id, seq, &id, change).await;
    if !local {
        refresh(env, config, &work.id, true).await;
    }
    let url = work::page_url(
        config,
        &work.repository,
        &work.branch,
        work.pull.map(|n| n as u64),
    );
    let answer = Opened { run: id, url };
    Ok(Response::from_json(&answer)?.with_status(201))
}

/// Refuses a run CI's token may not open: one of another repository or ref, or not CI's.
fn ci_may_open(token: &Token, open: &Open) -> Result<(), Refusal> {
    if open.repository.to_lowercase() != token.repo() {
        return Err(Refusal::new(403, "the token is for another repository"));
    }
    if !matches!(open.source, Source::Ci { .. }) {
        return Err(Refusal::new(400, "an OIDC token opens only a CI run"));
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
    Ok(())
}

/// A run as the calls after opening it need it.
#[derive(Deserialize)]
struct RunRow {
    work: String,
    git_ref: String,
    state: String,
    repo: String,
    repository: String,
    branch: String,
    pull: Option<f64>,
}

impl RunRow {
    /// Whether it is a local run, which never moves the work's review.
    fn local(&self) -> bool {
        self.git_ref.starts_with("local:")
    }
}

/// The run `id`, when the writer may write it. Only a CI token of the run's own repository
/// finds a CI run, and only one for the same ref (the same pull request, or the same pushed
/// branch) may write it; a person's CLI finds only their own local runs, while they may write
/// the repository. A closed run takes nothing more.
async fn writable(
    env: &Env,
    config: &Config,
    db: &D1Database,
    writer: &Writer,
    id: &str,
) -> Result<RunRow, Refusal> {
    let select = "SELECT r.work, r.git_ref, r.state, w.repo, w.repository, w.branch, w.pull FROM runs r JOIN works w ON w.id = r.work WHERE r.id = ?1";
    let row: Option<RunRow> = match writer {
        Writer::Ci(token) => {
            db.prepare(format!("{select} AND w.repo = ?2"))
                .bind(&[id.into(), token.repo().as_str().into()])?
                .first(None)
                .await?
        }
        Writer::Local(_) => {
            db.prepare(format!("{select} AND r.git_ref = ?2"))
                .bind(&[id.into(), writer.git_ref().as_str().into()])?
                .first(None)
                .await?
        }
    };
    let row = row.ok_or_else(|| Refusal::new(404, "no such run for this repository"))?;
    match writer {
        Writer::Ci(token) if row.git_ref != token.git_ref => {
            return Err(Refusal::new(
                403,
                format!(
                    "the token is for {}, not {}, which opened this run",
                    token.git_ref, row.git_ref
                ),
            ));
        }
        Writer::Ci(_) => {}
        Writer::Local(person) => may_write(env, config, person, &row.repository).await?,
    }
    if work::state(&row.state) != RunState::Running {
        return Err(Refusal::new(409, "the run is closed; open another"));
    }
    Ok(row)
}

/// The statement that notes a run just said something, for the rule that closes a silent
/// local run: every write for a run counts, a blob upload too (`touch_naming`).
fn touch(db: &D1Database, run: &str) -> worker::Result<D1PreparedStatement> {
    db.prepare("UPDATE runs SET touched = ?2 WHERE id = ?1")
        .bind(&[run.into(), (now() as f64).into()])
}

/// Notes that the writer's open runs of `repo` that name the blob `sha256` just said
/// something: a run sending large films for an hour is busy, not silent.
async fn touch_naming(
    db: &D1Database,
    writer: &Writer,
    repo: &str,
    sha256: &str,
) -> worker::Result<()> {
    db.prepare(
        "UPDATE runs SET touched = ?4 WHERE state = 'running' AND git_ref = ?1 \
         AND work IN (SELECT id FROM works WHERE repo = ?2) \
         AND id IN (SELECT run FROM files WHERE sha256 = ?3)",
    )
    .bind(&[
        writer.git_ref().as_str().into(),
        repo.into(),
        sha256.into(),
        (now() as f64).into(),
    ])?
    .run()
    .await?;
    Ok(())
}

pub async fn step(mut req: Request, env: &Env, config: &Config, id: &str) -> Answer {
    let writer = Writer::of(&req, env, config).await?;
    let step: Step = req
        .json()
        .await
        .map_err(|e| Refusal::new(400, format!("send a check's start or end: {e}")))?;
    let db = env.d1("DB")?;
    let run = writable(env, config, &db, &writer, id).await?;
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
                keys: Keys::new(),
            };
            (statements, change, files)
        }
    };
    let mut statements = statements;
    statements.push(touch(&db, id)?);
    statements.push(work::bump(&db, &run.work)?);
    let seq = work::commit(&db, statements).await?;
    // A look whose pictures are not all in yet reaches pages with the upload that completes it.
    let change = match change {
        Change::Finished {
            check,
            look: Some(look),
            ..
        } => {
            let (look, keys) = work::shown(*look, &work::held(&db, id).await?).unzip();
            Change::Finished {
                check,
                look: look.map(Box::new),
                keys: keys.unwrap_or_default(),
            }
        }
        change => change,
    };
    let shows_look = matches!(&change, Change::Finished { look: Some(_), .. });
    notify(env, &run.work, seq, id, change).await;
    if shows_look && !run.local() {
        refresh(env, config, &run.work, false).await;
    }
    let missing = lacks(&db, &run.repo, &files).await?;
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
    let writer = Writer::of(&req, env, config).await?;
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
    refuse_bad_files(&done.files)?;
    let db = env.d1("DB")?;
    let run = writable(env, config, &db, &writer, id).await?;
    if let Some(repository) = &done.proof.repository
        && repository.to_lowercase() != run.repo
    {
        return Err(Refusal::new(403, "the proof is of another repository"));
    }
    let recorded = record_files(&db, id, &done.files)?;
    if !recorded.is_empty() {
        db.batch(recorded).await?;
    }
    let missing = run_lacks(&db, &run.repo, id).await?;
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
    statements.push(touch(&db, id)?);
    statements.push(work::bump(&db, &run.work)?);
    let seq = work::commit(&db, statements).await?;
    let held = work::held(&db, id).await?;
    let keys = done
        .proof
        .looks
        .iter()
        .map(|look| (look.check.clone(), reviews::keys(&look.items, &held)))
        .collect();
    let change = Change::Done {
        checks: done.proof.checks.clone(),
        looks: done.proof.looks.clone(),
        keys,
    };
    notify(env, &run.work, seq, id, change).await;
    if !run.local() {
        refresh(env, config, &run.work, false).await;
    }
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

/// Derives the work's review again after a run changed; a failure is logged, and never fails
/// CI's call.
async fn refresh(env: &Env, config: &Config, work: &str, first_run: bool) {
    if let Err(error) = review::refresh(env, config, work, first_run).await {
        worker::console_error!("the review of work {work} was not derived again: {error}");
    }
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
    let writer = Writer::of(&req, env, config).await?;
    if !is_sha256(sha256) {
        return Err(Refusal::new(
            400,
            "a blob is named by its sha256 in lowercase hex",
        ));
    }
    let db = env.d1("DB")?;
    let repo = match &writer {
        Writer::Ci(token) => token.repo(),
        Writer::Local(person) => {
            let (repo, repository) = local_blob_repo(&db, &writer.git_ref(), sha256).await?;
            may_write(env, config, person, &repository).await?;
            repo
        }
    };
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
    // Touched as the bytes start and once they are stored, so a long upload counts all along.
    touch_naming(&db, &writer, &repo, sha256).await?;
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
    touch_naming(&db, &writer, &repo, sha256).await?;
    if held.is_none() {
        show_completed_looks(env, config, &db, &repo, sha256).await?;
    }
    Ok(Response::empty()?.with_status(204))
}

/// The repository a person's blob is for, as a CLI token names none: one whose open local run
/// of theirs names the blob, one that still lacks it first. `409` when no such run names it.
async fn local_blob_repo(
    db: &D1Database,
    git_ref: &str,
    sha256: &str,
) -> Result<(String, String), Refusal> {
    #[derive(Deserialize)]
    struct Row {
        repo: String,
        repository: String,
    }
    let row: Option<Row> = db
        .prepare(
            "SELECT w.repo, w.repository FROM files f JOIN runs r ON r.id = f.run JOIN works w ON w.id = r.work \
             WHERE r.git_ref = ?1 AND r.state = 'running' AND f.sha256 = ?2 \
             ORDER BY EXISTS (SELECT 1 FROM blobs b WHERE b.repo = w.repo AND b.sha256 = f.sha256) LIMIT 1",
        )
        .bind(&[git_ref.into(), sha256.into()])?
        .first(None)
        .await?;
    let row = row.ok_or_else(|| Refusal::new(409, "no run of yours names this blob"))?;
    Ok((row.repo, row.repository))
}

/// Sends pages each look the blob `sha256` may have just completed: every look of a run of
/// `repo` with a file of these bytes that now holds every file it names. Sending one that was
/// complete before changes nothing on a page.
async fn show_completed_looks(
    env: &Env,
    config: &Config,
    db: &D1Database,
    repo: &str,
    sha256: &str,
) -> worker::Result<()> {
    #[derive(Deserialize)]
    struct Row {
        run: String,
        work: String,
        git_ref: String,
        entry: String,
        look: String,
    }
    let rows = db
        .prepare(
            "SELECT c.run, r.work, r.git_ref, c.entry, c.look FROM checks c JOIN runs r ON r.id = c.run JOIN works w ON w.id = r.work WHERE w.repo = ?1 AND c.look IS NOT NULL AND c.entry IS NOT NULL AND EXISTS (SELECT 1 FROM files f WHERE f.run = c.run AND f.sha256 = ?2)",
        )
        .bind(&[repo.into(), sha256.into()])?
        .all()
        .await?
        .results::<Row>()?;
    for row in rows {
        let look: Look = work::read(&row.look)?;
        let Some((look, keys)) = work::shown(look, &work::held(db, &row.run).await?) else {
            continue;
        };
        let check: Check = work::read(&row.entry)?;
        let seq = work::commit(db, vec![work::bump(db, &row.work)?]).await?;
        let change = Change::Finished {
            check,
            look: Some(Box::new(look)),
            keys,
        };
        notify(env, &row.work, seq, &row.run, change).await;
        if !row.git_ref.starts_with("local:") {
            refresh(env, config, &row.work, false).await;
        }
    }
    Ok(())
}

/// Closes run `run` of work `work` as cancelled, while it still runs: it ended without saying
/// done. Its pages hear it; the review moves only for a CI run.
pub async fn cancel(
    env: &Env,
    config: &Config,
    db: &D1Database,
    run: &str,
    work: &str,
    local: bool,
) -> worker::Result<()> {
    let seq = work::commit(
        db,
        vec![
            db.prepare("UPDATE runs SET state = 'cancelled' WHERE id = ?1 AND state = 'running'")
                .bind(&[run.into()])?,
            work::bump(db, work)?,
        ],
    )
    .await?;
    notify(env, work, seq, run, Change::Cancelled {}).await;
    if !local {
        review::refresh(env, config, work, false).await?;
    }
    Ok(())
}

/// How long a local run may stay silent before it is closed as cancelled.
const SILENT_SECONDS: f64 = 3600.0;

/// The one rule for a local run that never says done, beside CI's (a run is closed when its
/// job ends, `webhook.rs`): it has no job, so once it said nothing for an hour it is closed
/// as cancelled. Run on the host's schedule.
pub async fn close_silent(env: &Env, config: &Config) -> worker::Result<()> {
    #[derive(Deserialize)]
    struct Row {
        id: String,
        work: String,
    }
    let db = env.d1("DB")?;
    let rows = db
        .prepare(
            "SELECT id, work FROM runs WHERE state = 'running' AND git_ref LIKE 'local:%' AND COALESCE(touched, opened) < ?1",
        )
        .bind(&[(now() as f64 - SILENT_SECONDS).into()])?
        .all()
        .await?
        .results::<Row>()?;
    for row in rows {
        cancel(env, config, &db, &row.id, &row.work, true).await?;
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
