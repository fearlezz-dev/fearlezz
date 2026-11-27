//! GitHub's webhooks to the app, at `POST /api/github/webhook`, each signed with the webhook
//! secret (`X-Hub-Signature-256`); an unsigned or badly signed one is refused before anything
//! in it is read.
//!
//! - `pull_request` (opened, synchronize, reopened): the pull request's head becomes its work's
//!   head, so its check run says it waits for CI; closed: the app no longer comments on it and
//!   only finishes the head's check run (`review::publish`). A pull request from a fork is
//!   passed over: its CI gets no OIDC token, so no hosted run comes.
//! - `workflow_job`: what `jobs.rs` says, a queued job waiting on the page and a completed one
//!   closing the run that never said done. A local run has no job; `api::close_silent` closes
//!   it once it said nothing for an hour.
//! - `workflow_run`: where an attempt of a proof workflow stands (one a CI run of the
//!   repository came from). While one is going the review waits; once it completed, a leg its
//!   plan made that opened no run never will, and a head no run of reached the host ended
//!   without its proof. The review names what to run again; a person or an agent runs it.

use fearlezz_proof::events::Source;
use serde::Deserialize;
use wasm_bindgen::JsValue;
use worker::{Env, Request, Response};

use crate::work::{Derive, Event, RUNS, RunRow, Write};
use crate::{Answer, Config, Refusal, api, crypto, jobs, now, room};

#[derive(Deserialize)]
struct Repository {
    full_name: String,
}

#[derive(Deserialize)]
struct PullEvent {
    action: String,
    pull_request: Pull,
    repository: Repository,
}

#[derive(Deserialize)]
struct Pull {
    number: u64,
    title: String,
    html_url: Option<String>,
    head: Head,
}

#[derive(Deserialize)]
struct Head {
    sha: String,
    #[serde(rename = "ref")]
    branch: String,
    repo: Option<Repository>,
}

#[derive(Deserialize)]
struct JobEvent {
    action: String,
    workflow_job: WorkflowJob,
    repository: Repository,
}

#[derive(Deserialize)]
struct WorkflowJob {
    id: u64,
    run_id: u64,
    run_attempt: u32,
    /// The runner that took it; none while it is queued.
    runner_name: Option<String>,
    name: String,
    head_sha: String,
    head_branch: Option<String>,
    html_url: Option<String>,
}

#[derive(Deserialize)]
struct RunEvent {
    action: String,
    workflow_run: WorkflowRun,
    repository: Repository,
}

#[derive(Deserialize)]
struct WorkflowRun {
    id: u64,
    run_attempt: u32,
    head_sha: String,
    head_branch: Option<String>,
    /// The workflow file (`.github/workflows/proof.yml`).
    path: String,
}

pub async fn receive(mut req: Request, env: &Env, config: &Config) -> Answer {
    let body = req.bytes().await?;
    let signature = req
        .headers()
        .get("X-Hub-Signature-256")?
        .unwrap_or_default();
    let mac = signature
        .strip_prefix("sha256=")
        .and_then(|h| hex::decode(h).ok());
    let signed = match mac {
        Some(mac) => crypto::hmac_verify(&config.webhook_secret, &body, &mac).await?,
        None => false,
    };
    if !signed {
        return Err(Refusal::new(401, "the webhook's signature does not hold"));
    }
    let event = req.headers().get("X-GitHub-Event")?.unwrap_or_default();
    let bad = |e: serde_json::Error| {
        Refusal::new(400, format!("a {event} event GitHub would not send: {e}"))
    };
    match event.as_str() {
        "pull_request" => pull_request(env, serde_json::from_slice(&body).map_err(bad)?).await?,
        "workflow_job" => workflow_job(env, serde_json::from_slice(&body).map_err(bad)?).await?,
        "workflow_run" => workflow_run(env, serde_json::from_slice(&body).map_err(bad)?).await?,
        _ => {}
    }
    Ok(Response::empty()?.with_status(202))
}

async fn pull_request(env: &Env, event: PullEvent) -> worker::Result<()> {
    let pull = &event.pull_request;
    let repository = &event.repository.full_name;
    let from = pull.head.repo.as_ref().map(|r| r.full_name.as_str());
    if !from.is_some_and(|f| f.eq_ignore_ascii_case(repository)) {
        return Ok(());
    }
    let repo = repository.to_lowercase();
    let db = env.d1("DB")?;
    let number = (pull.number as f64).into();
    match event.action.as_str() {
        "closed" => {
            db.prepare(
                "INSERT INTO pulls (repo, number, closed) VALUES (?1, ?2, 1) ON CONFLICT (repo, number) DO UPDATE SET closed = 1",
            )
            .bind(&[repo.as_str().into(), number])?
            .run()
            .await?;
            Ok(())
        }
        "opened" | "synchronize" | "reopened" => {
            db.prepare(
                "INSERT INTO pulls (repo, number) VALUES (?1, ?2) ON CONFLICT (repo, number) DO UPDATE SET closed = 0",
            )
            .bind(&[repo.as_str().into(), number])?
            .run()
            .await?;
            let id: Option<String> = db
                .prepare(
                    "INSERT INTO works (id, repo, repository, branch, pull, pull_title, pull_url, head) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
                     ON CONFLICT (repo, branch) DO UPDATE SET repository = excluded.repository, pull = excluded.pull, \
                     pull_title = excluded.pull_title, pull_url = COALESCE(excluded.pull_url, works.pull_url), head = excluded.head \
                     RETURNING id",
                )
                .bind(&[
                    crypto::random_id()?.into(),
                    repo.as_str().into(),
                    repository.as_str().into(),
                    pull.head.branch.as_str().into(),
                    (pull.number as f64).into(),
                    pull.title.as_str().into(),
                    pull.html_url.as_deref().map_or(JsValue::NULL, JsValue::from),
                    pull.head.sha.as_str().into(),
                ])?
                .first(Some("id"))
                .await?;
            match id {
                Some(id) => {
                    let write = Write {
                        statements: Vec::new(),
                        event: None,
                        derive: Derive::Review,
                    };
                    room::write(env, &id, &write).await
                }
                None => Ok(()),
            }
        }
        _ => Ok(()),
    }
}

async fn workflow_job(env: &Env, event: JobEvent) -> worker::Result<()> {
    let job = &event.workflow_job;
    let repo = event.repository.full_name.to_lowercase();
    let Some(branch) = &job.head_branch else {
        return Ok(());
    };
    let db = env.d1("DB")?;
    db.prepare(
        "INSERT INTO jobs (repo, id, workflow_run, run_attempt, runner, name, head, branch, state, url, since) \
         VALUES (?1, ?2, ?3, ?10, ?11, ?4, ?5, ?6, ?7, ?8, ?9) \
         ON CONFLICT (repo, id) DO UPDATE SET state = excluded.state, head = excluded.head, \
         runner = COALESCE(excluded.runner, jobs.runner)",
    )
    .bind(&[
        repo.as_str().into(),
        (job.id as f64).into(),
        (job.run_id as f64).into(),
        job.name.as_str().into(),
        job.head_sha.as_str().into(),
        branch.as_str().into(),
        event.action.as_str().into(),
        job.html_url.as_deref().map_or(JsValue::NULL, JsValue::from),
        (now() as f64).into(),
        f64::from(job.run_attempt).into(),
        job.runner_name.as_deref().map_or(JsValue::NULL, JsValue::from),
    ])?
    .run()
    .await?;
    if event.action == "completed" {
        cancel(env, &repo, job).await?;
    }
    let work: Option<String> = db
        .prepare("SELECT id FROM works WHERE repo = ?1 AND branch = ?2")
        .bind(&[repo.as_str().into(), branch.as_str().into()])?
        .first(Some("id"))
        .await?;
    if let Some(work) = work {
        // A runner names the leg of the run the job is, which can move the review.
        let derive = match job.runner_name {
            Some(_) => Derive::Review,
            None => Derive::No,
        };
        let write = Write {
            statements: Vec::new(),
            event: Some(Event::Waiting),
            derive,
        };
        room::write(env, &work, &write).await?;
    }
    Ok(())
}

/// Keeps where an attempt of a proof workflow stands (requested, in progress or completed), and
/// derives the review of its branch's work again. Only an attempt of a workflow a CI run of the
/// repository came from, or of a workflow run such a run names, is kept: every other workflow
/// of the repository never moves a review.
async fn workflow_run(env: &Env, event: RunEvent) -> worker::Result<()> {
    let run = &event.workflow_run;
    let repo = event.repository.full_name.to_lowercase();
    let db = env.d1("DB")?;
    let kept: Option<String> = db
        .prepare(
            "INSERT INTO attempts (repo, workflow_run, run_attempt, head, path, status) \
             SELECT ?1, ?2, ?3, ?4, ?5, ?6 WHERE EXISTS ( \
               SELECT 1 FROM runs r JOIN works w ON w.id = r.work \
               WHERE w.repo = ?1 AND json_extract(r.source, '$.kind') = 'ci' \
                 AND (json_extract(r.source, '$.workflow') = ?5 \
                   OR json_extract(r.source, '$.run_id') = ?2)) \
             ON CONFLICT (repo, workflow_run, run_attempt) DO UPDATE SET status = excluded.status \
               WHERE attempts.status != 'completed' \
             RETURNING status",
        )
        .bind(&[
            repo.as_str().into(),
            (run.id as f64).into(),
            f64::from(run.run_attempt).into(),
            run.head_sha.as_str().into(),
            run.path.as_str().into(),
            event.action.as_str().into(),
        ])?
        .first(Some("status"))
        .await?;
    let (Some(_), Some(branch)) = (kept, &run.head_branch) else {
        return Ok(());
    };
    let work: Option<String> = db
        .prepare("SELECT id FROM works WHERE repo = ?1 AND branch = ?2")
        .bind(&[repo.as_str().into(), branch.as_str().into()])?
        .first(Some("id"))
        .await?;
    match work {
        Some(work) => {
            let write = Write {
                statements: Vec::new(),
                event: None,
                derive: Derive::Review,
            };
            room::write(env, &work, &write).await
        }
        None => Ok(()),
    }
}

/// Closes, as cancelled, the run of `repo` still running that the completed `job` was.
async fn cancel(env: &Env, repo: &str, job: &WorkflowJob) -> worker::Result<()> {
    #[derive(Deserialize)]
    struct NameRow {
        name: String,
    }
    let Some(runner) = &job.runner_name else {
        return Ok(());
    };
    let db = env.d1("DB")?;
    let binds = [
        repo.into(),
        (job.run_id as f64).into(),
        f64::from(job.run_attempt).into(),
        runner.as_str().into(),
    ];
    // Every job of the attempt this runner took, this one stored already.
    let names: Vec<String> = db
        .prepare(
            "SELECT name FROM jobs WHERE repo = ?1 AND workflow_run = ?2 AND run_attempt = ?3 AND runner = ?4",
        )
        .bind(&binds)?
        .all()
        .await?
        .results::<NameRow>()?
        .into_iter()
        .map(|r| r.name)
        .collect();
    let rows = db
        .prepare(format!(
            "{RUNS} WHERE w.repo = ?1 AND r.state = 'running' AND json_extract(r.source, '$.kind') = 'ci' \
             AND json_extract(r.source, '$.run_id') = ?2 AND json_extract(r.source, '$.run_attempt') = ?3 \
             AND json_extract(r.source, '$.runner_name') = ?4"
        ))
        .bind(&binds)?
        .all()
        .await?
        .results::<RunRow>()?;
    for row in rows {
        let own = match row.source()? {
            Source::Ci { job: id, .. } => {
                jobs::own(names.iter().map(String::as_str), &id) == Some(job.name.as_str())
            }
            Source::Local { .. } => false,
        };
        if own {
            api::cancel(env, &row.id, &row.work, false).await?;
        }
    }
    Ok(())
}
