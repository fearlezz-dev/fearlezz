//! What a person says on a work's page: a verdict on a picture (write access), an accept of a
//! head's weakenings (admin rights), which runs its failed proof job again, and who the page is
//! shown to, so it offers only what they may do.
//!
//! Only a person in a browser says these. A verdict or an accept takes the session cookie and
//! nothing else: a request that carries an `Authorization` header is refused, whatever token
//! it holds, so no token a program holds (CI's OIDC token, or the CLI's own token to come) can
//! ever give one. It must come from the app origin (`Origin`) as JSON, which a page on another
//! site cannot send with the cookie. Only CI runs are reviewed; a local run never is, though
//! a person may leave notes on its pictures (`notes.rs`).

use fearlezz_proof::events::{Live, Source, WorkChange};
use fearlezz_proof::look::Look;
use fearlezz_proof::review::Role;
use fearlezz_proof::review::{self, Verdict};
use fearlezz_proof::{Proof, events::RunState};
use serde::Deserialize;
use serde_json::json;
use worker::{D1Database, Env, Request, Response};

use crate::session::{self, Session};
use crate::work::{self, read};
use crate::{Answer, Config, Refusal, cache, github, jobs, now, pages, review as hosted};

/// How long the host trusts GitHub's word on what a person may do.
const ROLE_SECONDS: i64 = 300;

#[derive(Deserialize)]
struct Ask {
    check: String,
    picture: String,
}

/// A run, as saying something about it needs it.
#[derive(Deserialize)]
pub struct RunRow {
    pub work: String,
    pub repo: String,
    pub repository: String,
    pub head: String,
    source: String,
    state: String,
    proof: Option<String>,
}

/// The person a browser request speaks for, after the checks every saying passes: no token,
/// from the app origin, JSON, and a session.
pub async fn person(
    req: &Request,
    env: &Env,
    config: &Config,
    what: &str,
) -> Result<Session, Refusal> {
    if req.headers().get("Authorization")?.is_some() {
        return Err(Refusal::new(
            403,
            format!("refused: only a person on the work's page can {what}, never a token"),
        ));
    }
    let origin = req.headers().get("Origin")?;
    if origin.as_deref() != Some(config.app_origin.as_str()) {
        return Err(Refusal::new(
            403,
            "refused: the ask did not come from this site",
        ));
    }
    let json = req
        .headers()
        .get("Content-Type")?
        .is_some_and(|t| t.trim_start().starts_with("application/json"));
    if !json {
        return Err(Refusal::new(415, "send JSON"));
    }
    session::read(req, env)
        .await?
        .ok_or_else(|| Refusal::new(401, format!("sign in to {what}")))
}

/// The run `id`, CI's or local, when the request may see it; a run it may not see is not
/// found.
pub async fn run(
    req: &Request,
    env: &Env,
    config: &Config,
    db: &D1Database,
    id: &str,
) -> Result<RunRow, Refusal> {
    let row: Option<RunRow> = db
        .prepare(
            "SELECT r.work, w.repo, w.repository, r.head, r.source, r.state, r.proof FROM runs r JOIN works w ON w.id = r.work WHERE r.id = ?1",
        )
        .bind(&[id.into()])?
        .first(None)
        .await?;
    let row = row.ok_or_else(|| Refusal::new(404, "not found"))?;
    if !pages::open(req, env, config, &row.repo).await? {
        return Err(Refusal::new(404, "not found"));
    }
    Ok(row)
}

impl RunRow {
    /// Refuses a local run, which is the agent's word and never reviewed: `what` is given on a
    /// CI run.
    fn reviewed(&self, what: &str) -> Result<(), Refusal> {
        if matches!(read::<Source>(&self.source)?, Source::Ci { .. }) {
            return Ok(());
        }
        Err(Refusal::new(
            409,
            format!(
                "this is a local run, which is unproven and never reviewed: {what} on a CI run; notes may be made here"
            ),
        ))
    }
}

/// What a person may do on a repository: GitHub's word, kept for a few minutes.
pub async fn role(
    env: &Env,
    config: &Config,
    login: &str,
    token: &str,
    repo: &str,
) -> Result<Role, Refusal> {
    let key = format!("role:{}:{}", login.to_lowercase(), repo.to_lowercase());
    if let Some(cached) = cache::get(env, &key).await? {
        return Ok(Role::of(&cached));
    }
    let Some(role) = github::permission(config, repo, login, token).await? else {
        return Err(Refusal::new(
            401,
            "your GitHub sign in expired; sign in again",
        ));
    };
    let word = match role {
        Role::Admin => "admin",
        Role::Write => "write",
        Role::Less => "read",
    };
    cache::put(env, &key, word, ROLE_SECONDS).await?;
    Ok(role)
}

/// `POST /api/v2/runs/<run>/verdicts`: a person with write access says a picture of a CI run
/// looks right. It holds for that picture's content, so later runs showing the same keep it.
/// What should change is a note, never a verdict.
pub async fn verdict(mut req: Request, env: &Env, config: &Config, id: &str) -> Answer {
    let person = person(&req, env, config, "give a verdict").await?;
    let ask: Ask = req
        .json()
        .await
        .map_err(|_| Refusal::new(400, "send {\"check\", \"picture\"}"))?;
    let db = env.d1("DB")?;
    let run = run(&req, env, config, &db, id).await?;
    run.reviewed("verdicts are given")?;
    if role(env, config, &person.login, &person.token, &run.repository).await? < Role::Write {
        return Err(Refusal::new(
            403,
            format!(
                "@{} cannot give verdicts here: that needs write access to {} on GitHub",
                person.login, run.repository
            ),
        ));
    }
    let look: Option<String> = db
        .prepare("SELECT look FROM checks WHERE run = ?1 AND id = ?2")
        .bind(&[id.into(), ask.check.as_str().into()])?
        .first(Some("look"))
        .await?;
    let look = look.map(|l| read::<Look>(&l)).transpose()?;
    let item = look
        .as_ref()
        .and_then(|l| l.items.iter().find(|i| i.name == ask.picture))
        .filter(|i| i.to_look_at())
        .ok_or_else(|| {
            Refusal::new(
                409,
                format!(
                    "no picture {} of `{}` waits for a look in this run",
                    ask.picture, ask.check
                ),
            )
        })?;
    let held = work::held(&db, id).await?;
    let key = review::key(item, |path| held.get(path).cloned())
        .ok_or_else(|| Refusal::new(409, "its pictures are not all in yet; wait for them"))?;
    let verdict = Verdict {
        check: ask.check,
        picture: ask.picture,
        key,
        by: person.login,
        at: now() as f64,
        run: id.to_owned(),
        head: run.head,
    };
    let seq = work::commit(
        &db,
        vec![
            db.prepare(
                "INSERT INTO verdicts (work, check_id, picture, key, login, at, run, head) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
                 ON CONFLICT (work, check_id, picture, key) DO UPDATE SET login = excluded.login, at = excluded.at, run = excluded.run, head = excluded.head",
            )
            .bind(&[
                run.work.as_str().into(),
                verdict.check.as_str().into(),
                verdict.picture.as_str().into(),
                verdict.key.as_str().into(),
                verdict.by.as_str().into(),
                verdict.at.into(),
                verdict.run.as_str().into(),
                verdict.head.as_str().into(),
            ])?,
            work::bump(&db, &run.work)?,
        ],
    )
    .await?;
    let change = WorkChange::Verdict {
        verdict: verdict.clone(),
    };
    work::notify(
        env,
        &run.work,
        &Live::Work {
            seq,
            change: Box::new(change),
        },
    )
    .await;
    hosted::refresh(env, config, &run.work, false).await?;
    Ok(Response::from_json(&json!({ "verdict": verdict }))?.with_status(201))
}

/// `POST /api/v2/runs/<run>/accept`: an admin accepts exactly what a finished CI run's change
/// weakens at its head, which failed its proof job. The host keeps it and runs that failed job
/// again as the app, so the job asks for the accept (`api::weakenings`) and passes with it: the
/// job stays the one gate. When GitHub does not run it again, the accept is kept and the answer
/// says to run the job again on GitHub.
pub async fn accept(req: Request, env: &Env, config: &Config, id: &str) -> Answer {
    let person = person(&req, env, config, "accept a weakened proof").await?;
    let db = env.d1("DB")?;
    let run = run(&req, env, config, &db, id).await?;
    run.reviewed("weakenings are accepted")?;
    if role(env, config, &person.login, &person.token, &run.repository).await? < Role::Admin {
        return Err(Refusal::new(
            403,
            format!(
                "@{} cannot accept a weakened proof here: that needs admin rights on {} on GitHub",
                person.login, run.repository
            ),
        ));
    }
    let proof = run.proof.as_deref().map(read::<Proof>).transpose()?;
    let weakened = proof
        .filter(|p| work::state(&run.state) == RunState::Done && !p.weakened.is_empty())
        .map(|p| p.weakened)
        .ok_or_else(|| Refusal::new(409, "this run weakens nothing to accept"))?;
    let accept = review::Accept {
        head: run.head.clone(),
        weakened,
        by: person.login,
        at: now() as f64,
    };
    db.prepare(
        "INSERT INTO accepts (work, head, weakened, login, at) VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT (work, head) DO UPDATE SET weakened = excluded.weakened, login = excluded.login, at = excluded.at",
    )
    .bind(&[
        run.work.as_str().into(),
        run.head.as_str().into(),
        serde_json::to_string(&accept.weakened)?.into(),
        accept.by.as_str().into(),
        accept.at.into(),
    ])?
    .run()
    .await?;
    hosted::refresh(env, config, &run.work, false).await?;
    if let Err(error) = rerun(config, &run).await {
        worker::console_error!(
            "the proof job of {} did not run again: {error}",
            run.repository
        );
        return Err(Refusal::new(
            502,
            format!(
                "accepted, but GitHub did not run the proof job again ({error}); run it again on GitHub"
            ),
        ));
    }
    Ok(Response::from_json(&json!({ "accept": accept }))?.with_status(201))
}

/// Runs the failed proof job of a CI run again, as the app: each failed job of its workflow
/// run whose name is the run's job, alone or in a matrix.
async fn rerun(config: &Config, run: &RunRow) -> worker::Result<()> {
    let Source::Ci {
        job,
        run_url: Some(url),
    } = read::<Source>(&run.source)?
    else {
        return Err(worker::Error::RustError(
            "the run names no workflow run".into(),
        ));
    };
    let workflow_run = url
        .rsplit_once("/actions/runs/")
        .and_then(|(_, rest)| rest.split('/').next()?.parse::<u64>().ok())
        .ok_or_else(|| worker::Error::RustError(format!("{url} names no workflow run")))?;
    let Some(token) = github::installation_token(config, &run.repository).await? else {
        return Err(worker::Error::RustError("the app is not installed".into()));
    };
    let failed = github::failed_jobs(&token, config, &run.repository, workflow_run).await?;
    let mine: Vec<u64> = failed
        .into_iter()
        .filter(|(_, name)| jobs::runs(name, &job))
        .map(|(id, _)| id)
        .collect();
    if mine.is_empty() {
        return Err(worker::Error::RustError(format!(
            "no failed job {job} in workflow run {workflow_run}"
        )));
    }
    for id in mine {
        github::rerun_job(&token, config, &run.repository, id).await?;
    }
    Ok(())
}

/// `GET /api/v2/viewer/<owner>/<repo>`: who the page is shown to and what they may do there,
/// `{"login", "role": "admin" | "write" | "read"}`, or `{"login": null}` without a session.
pub async fn viewer(req: &Request, env: &Env, config: &Config, owner: &str, name: &str) -> Answer {
    let repo = crate::repo_key(owner, name);
    if !pages::open(req, env, config, &repo).await? {
        return Err(Refusal::new(401, "sign in to see this work"));
    }
    let answer = match session::read(req, env).await? {
        Some(person) => {
            let repository = format!("{owner}/{name}");
            let role = match role(env, config, &person.login, &person.token, &repository).await? {
                Role::Admin => "admin",
                Role::Write => "write",
                Role::Less => "read",
            };
            json!({"login": person.login, "role": role})
        }
        None => json!({"login": null}),
    };
    Ok(Response::from_json(&answer)?)
}
