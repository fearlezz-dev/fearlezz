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

use fearlezz_proof::events::{Source, WorkChange};
use fearlezz_proof::look::Look;
use fearlezz_proof::notes::{Note, NoteState};
use fearlezz_proof::review::{self, Verdict};
use fearlezz_proof::{Proof, events::RunState};
use serde::Deserialize;
use serde_json::json;
use worker::{D1Database, Env, Request, Response};

use crate::access::{self, Action, Viewer};
use crate::work::{self, Derive, Event, NOTE_COLUMNS, NoteRow, RunRow, Statement, Write, read};
use crate::{Answer, Config, Refusal, github, jobs, not_found, now, room, session};

#[derive(Deserialize)]
struct Ask {
    check: String,
    picture: String,
}

/// The run `id`, CI's or local, when `viewer` may see it; a run they may not see is not
/// found.
pub(crate) async fn run(
    env: &Env,
    config: &Config,
    db: &D1Database,
    viewer: Viewer<'_>,
    id: &str,
) -> Result<RunRow, Refusal> {
    let row = RunRow::find(db, id).await?.ok_or_else(not_found)?;
    let read = Action::Read {
        work: Some(&row.work),
    };
    access::can(env, config, viewer, read, &row.repo)
        .await
        .map_err(Refusal::hidden)?;
    Ok(row)
}

/// Refuses a local run, which is the agent's word and never reviewed: `what` is given on a CI
/// run.
fn reviewed(run: &RunRow, what: &str) -> Result<(), Refusal> {
    if !run.local()? {
        return Ok(());
    }
    Err(Refusal::new(
        409,
        format!(
            "this is a local run, which is unproven and never reviewed: {what} on a CI run; notes may be made here"
        ),
    ))
}

/// `POST /api/v2/runs/<run>/verdicts`: a person with write access says a picture of a CI run
/// looks right. It holds for that picture's content, so later runs showing the same keep it.
/// What should change is a note, never a verdict.
pub async fn verdict(mut req: Request, env: &Env, config: &Config, id: &str) -> Answer {
    let person = access::speaker(&req, env, config, "give a verdict").await?;
    let ask: Ask = req
        .json()
        .await
        .map_err(|_| Refusal::new(400, "send {\"check\", \"picture\"}"))?;
    let db = env.d1("DB")?;
    let viewer = Viewer::Person(&person);
    let run = run(env, config, &db, viewer, id).await?;
    reviewed(&run, "verdicts are given")?;
    access::can(env, config, viewer, Action::Verdict, &run.repository).await?;
    let key = content(&db, id, &ask).await?;
    let verdict = Verdict {
        check: ask.check,
        picture: ask.picture,
        key,
        by: person.login,
        at: now() as f64,
        run: id.to_owned(),
        head: run.head,
    };
    // Saying it looks right settles what the picture's open notes asked: they are resolved by
    // the same person, in the same write.
    let open: Vec<NoteRow> = db
        .prepare(format!(
            "SELECT {NOTE_COLUMNS} FROM notes WHERE work = ?1 AND check_id = ?2 AND picture = ?3 AND state = 'open' ORDER BY at, rowid"
        ))
        .bind(&[
            run.work.as_str().into(),
            verdict.check.as_str().into(),
            verdict.picture.as_str().into(),
        ])?
        .all()
        .await?
        .results()?;
    let resolved: Vec<Note> = open
        .into_iter()
        .map(|row| {
            let mut note = row.note();
            note.state = NoteState::Resolved;
            note.resolved_by = Some(verdict.by.clone());
            note
        })
        .collect();
    let resolves = resolved.iter().map(|note| {
        Statement::new(
            "UPDATE notes SET state = 'resolved', resolved_by = ?2 WHERE id = ?1 AND state = 'open'",
            [note.id.as_str().into(), verdict.by.as_str().into()],
        )
    });
    let write = Write {
        statements: [Statement::new(
            "INSERT INTO verdicts (work, check_id, picture, key, login, at, run, head) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
             ON CONFLICT (work, check_id, picture, key) DO UPDATE SET login = excluded.login, at = excluded.at, run = excluded.run, head = excluded.head",
            [
                run.work.as_str().into(),
                verdict.check.as_str().into(),
                verdict.picture.as_str().into(),
                verdict.key.as_str().into(),
                verdict.by.as_str().into(),
                verdict.at.into(),
                verdict.run.as_str().into(),
                verdict.head.as_str().into(),
            ],
        )]
        .into_iter()
        .chain(resolves)
        .collect(),
        event: Some(Event::work(WorkChange::Verdict {
            verdict: verdict.clone(),
            resolved: resolved.clone(),
        })),
        derive: Derive::Review,
    };
    room::write(env, &run.work, &write).await?;
    Ok(Response::from_json(&json!({ "verdict": verdict, "resolved": resolved }))?.with_status(201))
}

/// The content key of the picture `ask` names in run `id`, which a verdict is on: refused when
/// the run has no such picture to look at, or not all its files yet.
async fn content(db: &D1Database, id: &str, ask: &Ask) -> Result<String, Refusal> {
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
    let held = work::held(db, id).await?;
    review::key(item, |path| held.get(path).cloned())
        .ok_or_else(|| Refusal::new(409, "its pictures are not all in yet; wait for them"))
}

/// `DELETE /api/v2/runs/<run>/verdicts`: a person with write access takes back the verdict on
/// a picture's content, which then waits for a look again wherever it shows. Taking back a
/// verdict that is not there changes nothing.
pub(crate) async fn withdraw(mut req: Request, env: &Env, config: &Config, id: &str) -> Answer {
    let person = access::speaker(&req, env, config, "take back a verdict").await?;
    let ask: Ask = req
        .json()
        .await
        .map_err(|_not_json| Refusal::new(400, "send {\"check\", \"picture\"}"))?;
    let db = env.d1("DB")?;
    let viewer = Viewer::Person(&person);
    let run = run(env, config, &db, viewer, id).await?;
    reviewed(&run, "verdicts are taken back")?;
    access::can(env, config, viewer, Action::Verdict, &run.repository).await?;
    let key = content(&db, id, &ask).await?;
    let write = Write {
        statements: vec![Statement::new(
            "DELETE FROM verdicts WHERE work = ?1 AND check_id = ?2 AND picture = ?3 AND key = ?4",
            [
                run.work.as_str().into(),
                ask.check.as_str().into(),
                ask.picture.as_str().into(),
                key.as_str().into(),
            ],
        )],
        event: Some(Event::work(WorkChange::Withdrawn {
            check: ask.check,
            picture: ask.picture,
            key,
        })),
        derive: Derive::Review,
    };
    room::write(env, &run.work, &write).await?;
    Ok(Response::from_json(&json!({}))?)
}

/// `POST /api/v2/runs/<run>/accept`: an admin accepts exactly what a finished CI run's change
/// weakens at its head, which failed its proof job. The host keeps it and runs that failed job
/// again as the app, so the job asks for the accept (`api::weakenings`) and passes with it: the
/// job stays the one gate. When GitHub does not run it again, the accept is kept and the answer
/// says to run the job again on GitHub.
pub async fn accept(req: Request, env: &Env, config: &Config, id: &str) -> Answer {
    let person = access::speaker(&req, env, config, "accept a weakened proof").await?;
    let db = env.d1("DB")?;
    let viewer = Viewer::Person(&person);
    let run = run(env, config, &db, viewer, id).await?;
    reviewed(&run, "weakenings are accepted")?;
    access::can(env, config, viewer, Action::Accept, &run.repository).await?;
    // Read with its state, as the done that wrote it left both.
    #[derive(Deserialize)]
    struct Closed {
        state: String,
        proof: Option<String>,
    }
    let closed: Option<Closed> = db
        .prepare("SELECT state, proof FROM runs WHERE id = ?1")
        .bind(&[id.into()])?
        .first(None)
        .await?;
    let closed = closed.ok_or_else(not_found)?;
    let proof = closed.proof.as_deref().map(read::<Proof>).transpose()?;
    let weakened = proof
        .filter(|p| work::state(&closed.state) == RunState::Done && !p.weakened.is_empty())
        .map(|p| p.weakened)
        .ok_or_else(|| Refusal::new(409, "this run weakens nothing to accept"))?;
    let accept = review::Accept {
        head: run.head.clone(),
        weakened,
        by: person.login,
        at: now() as f64,
    };
    // The accept is no event of its own: the review it moves is.
    let write = Write {
        statements: vec![Statement::new(
            "INSERT INTO accepts (work, head, weakened, login, at) VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT (work, head) DO UPDATE SET weakened = excluded.weakened, login = excluded.login, at = excluded.at",
            [
                run.work.as_str().into(),
                run.head.as_str().into(),
                serde_json::to_string(&accept.weakened)?.into(),
                accept.by.as_str().into(),
                accept.at.into(),
            ],
        )],
        event: None,
        derive: Derive::Review,
    };
    room::write(env, &run.work, &write).await?;
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

/// Runs the failed proof job of a CI run again, as the app: the one job of its workflow run's
/// attempt that is the run's own (`jobs::own`), when it failed. A leg of a split proof passes
/// and leaves what its change weakens to the gate, a job of its own that streams nothing, so
/// for one the workflow run's failed jobs run again, the gate among them.
async fn rerun(config: &Config, run: &RunRow) -> worker::Result<()> {
    let Source::Ci {
        job,
        run_id,
        run_attempt,
        runner_name,
        leg,
        ..
    } = run.source()?
    else {
        return Err(worker::Error::RustError("the run is not a CI run".into()));
    };
    let Some(token) = github::installation_token(config, &run.repository).await? else {
        return Err(worker::Error::RustError("the app is not installed".into()));
    };
    if leg.is_some() {
        return github::rerun_failed_jobs(&token, config, &run.repository, run_id).await;
    }
    let all = github::jobs(&token, config, &run.repository, run_id, run_attempt).await?;
    let on_runner: Vec<&github::ActionsJob> = all
        .iter()
        .filter(|j| j.runner.as_deref() == Some(runner_name.as_str()))
        .collect();
    let own = jobs::own(on_runner.iter().map(|j| j.name.as_str()), &job);
    let mine = on_runner
        .into_iter()
        .find(|j| Some(j.name.as_str()) == own && j.conclusion.as_deref() == Some("failure"))
        .ok_or_else(|| {
            worker::Error::RustError(format!(
                "no failed job {job} on {runner_name} in workflow run {run_id}"
            ))
        })?;
    github::rerun_job(&token, config, &run.repository, mine.id).await
}

/// `GET /api/v2/viewer/<owner>/<repo>`: who the page is shown to and what they may do there,
/// `{"login", "role": "admin" | "write" | "read"}`, or `{"login": null}` without a session.
pub async fn viewer(req: &Request, env: &Env, config: &Config, owner: &str, name: &str) -> Answer {
    let repo = crate::repo_key(owner, name);
    let person = session::read(req, env).await?;
    let read = Action::Read { work: None };
    access::can(env, config, Viewer::of(person.as_ref()), read, &repo).await?;
    let answer = match person {
        Some(person) => {
            let repository = format!("{owner}/{name}");
            let role = access::role(env, config, &person, &repository).await?;
            json!({"login": person.login, "role": role.word()})
        }
        None => json!({"login": null}),
    };
    Ok(Response::from_json(&answer)?)
}
