//! GitHub's webhooks to the app, at `POST /api/github/webhook`, each signed with the webhook
//! secret (`X-Hub-Signature-256`); an unsigned or badly signed one is refused before anything
//! in it is read.
//!
//! - `pull_request` (opened, synchronize, reopened): the pull request's head becomes its work's
//!   head, so its check run says it waits for CI; closed: the app writes nothing more on it. A
//!   pull request from a fork is passed over: its CI gets no OIDC token, so no hosted run comes.
//! - `workflow_job`: what `jobs.rs` says, a queued job waiting on the page and a completed one
//!   closing the run that never said done. A local run has no job; `api::close_silent` closes
//!   it once it said nothing for an hour.

use fearlezz_proof::events::{Live, Source, WorkChange};
use serde::Deserialize;
use wasm_bindgen::JsValue;
use worker::{Env, Request, Response};

use crate::work::{self, read};
use crate::{Answer, Config, Refusal, api, crypto, jobs, now, review};

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
        "pull_request" => {
            pull_request(env, config, serde_json::from_slice(&body).map_err(bad)?).await?
        }
        "workflow_job" => {
            workflow_job(env, config, serde_json::from_slice(&body).map_err(bad)?).await?
        }
        _ => {}
    }
    Ok(Response::empty()?.with_status(202))
}

async fn pull_request(env: &Env, config: &Config, event: PullEvent) -> worker::Result<()> {
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
                Some(id) => review::refresh(env, config, &id, false).await,
                None => Ok(()),
            }
        }
        _ => Ok(()),
    }
}

async fn workflow_job(env: &Env, config: &Config, event: JobEvent) -> worker::Result<()> {
    let job = &event.workflow_job;
    let repo = event.repository.full_name.to_lowercase();
    let Some(branch) = &job.head_branch else {
        return Ok(());
    };
    let db = env.d1("DB")?;
    db.prepare(
        "INSERT INTO jobs (repo, id, workflow_run, name, head, branch, state, url, since) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
         ON CONFLICT (repo, id) DO UPDATE SET state = excluded.state, head = excluded.head",
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
    ])?
    .run()
    .await?;
    if event.action == "completed" {
        cancel(env, config, &repo, job).await?;
    }
    let work: Option<String> = db
        .prepare("SELECT id FROM works WHERE repo = ?1 AND branch = ?2")
        .bind(&[repo.as_str().into(), branch.as_str().into()])?
        .first(Some("id"))
        .await?;
    if let Some(work) = work {
        let waiting = jobs::waiting(&db, &work).await?;
        let seq = work::commit(&db, vec![work::bump(&db, &work)?]).await?;
        let change = WorkChange::Waiting { waiting };
        work::notify(
            env,
            &work,
            &Live::Work {
                seq,
                change: Box::new(change),
            },
        )
        .await;
    }
    Ok(())
}

/// Closes, as cancelled, the run of `repo` still running that the completed `job` was.
async fn cancel(env: &Env, config: &Config, repo: &str, job: &WorkflowJob) -> worker::Result<()> {
    #[derive(Deserialize)]
    struct Row {
        id: String,
        work: String,
        source: String,
    }
    let Some(runner) = &job.runner_name else {
        return Ok(());
    };
    let db = env.d1("DB")?;
    let rows = db
        .prepare(
            "SELECT r.id, r.work, r.source FROM runs r JOIN works w ON w.id = r.work \
             WHERE w.repo = ?1 AND r.state = 'running' AND json_extract(r.source, '$.kind') = 'ci' \
             AND json_extract(r.source, '$.run_id') = ?2 AND json_extract(r.source, '$.run_attempt') = ?3 \
             AND json_extract(r.source, '$.runner_name') = ?4",
        )
        .bind(&[
            repo.into(),
            (job.run_id as f64).into(),
            f64::from(job.run_attempt).into(),
            runner.as_str().into(),
        ])?
        .all()
        .await?
        .results::<Row>()?;
    for row in rows {
        let own = match read::<Source>(&row.source)? {
            Source::Ci {
                job: id,
                runner_name,
                ..
            } => jobs::is(&job.name, Some(runner), &id, &runner_name),
            Source::Local { .. } => false,
        };
        if own {
            api::cancel(env, config, &db, &row.id, &row.work, false).await?;
        }
    }
    Ok(())
}
