//! A work's review, where the host is the authority: derived again on every event that can
//! change it (a run opening, a look arriving, done, a cancelled job, a verdict, an accept, a new
//! head), sent to the work's open pages when it changed, and written to GitHub as the app: one
//! check run per pull request head, `Fearlezz review`, and one comment on the pull request.
//!
//! The check run says `queued` until a CI run of the head opens, `in_progress` while one runs,
//! `failure` when the proof failed (what it weakens fails it until an admin accepts it, and the
//! accept runs the proof job again, see `say.rs`), `cancelled` when every run of the head ended
//! without its proof, `failure` too while an open note says a picture needs change (its summary lists the notes),
//! `action_required` while a picture waits for a look,
//! and `success` once nothing waits. `action_required` needs no requested actions: it
//! completes the check run (so a required check blocks the merge), and GitHub sends people to
//! its `details_url`, the work's page, which is where they act.

use fearlezz_proof::events::WorkChange;
use fearlezz_proof::review::{self, Review, State};
use serde::Deserialize;
use serde_json::{Value, json};
use wasm_bindgen::JsValue;
use worker::{D1Database, Env, Method};

use crate::room::Pages;
use crate::work::{self, Event, Statement};
use crate::{Config, github};

/// The check run's name on GitHub.
pub const NAME: &str = "Fearlezz review";

#[derive(Deserialize)]
struct Row {
    repo: String,
    repository: String,
    branch: String,
    pull: Option<f64>,
    head: Option<String>,
    review: Option<String>,
}

/// Derives the review of work `work` again and sends it where it shows: to the work's pages when
/// it changed, and to its pull request on GitHub. `first_run` is said by a CI run opening, which
/// creates the pull request's comment when it has none; everything else only edits it. Only the
/// work's room calls it, while no other write to the work runs (`work::apply`), so it reads the
/// newest state and two never write one comment or check run twice. GitHub failing is logged
/// and never fails the write that changed the work.
pub async fn refresh(
    env: &Env,
    config: &Config,
    pages: &Pages,
    work: &str,
    first_run: bool,
) -> worker::Result<()> {
    let db = env.d1("DB")?;
    let row: Option<Row> = db
        .prepare("SELECT repo, repository, branch, pull, head, review FROM works WHERE id = ?1")
        .bind(&[work.into()])?
        .first(None)
        .await?;
    let Some(row) = row else { return Ok(()) };
    let Some(head) = &row.head else { return Ok(()) };
    let of = work::of_head(&db, work, head).await?;
    let accept = accept(&db, work, head).await?;
    let review = review::review(head, &of.runs, &of.verdicts, &of.notes, accept.as_ref());
    let text = serde_json::to_string(&review)?;
    if row.review.as_deref() != Some(text.as_str()) {
        let change = WorkChange::Review {
            review: Some(review.clone()),
        };
        work::commit(
            env,
            &db,
            pages,
            work,
            vec![Statement::new(
                "UPDATE works SET review = ?2 WHERE id = ?1",
                [work.into(), text.into()],
            )],
            Some(Event::work(change)),
        )
        .await?;
    }
    if let Some(pull) = row.pull
        && let Err(error) = publish(&db, config, &row, pull as u64, &review, first_run).await
    {
        worker::console_error!("the review of {} did not reach GitHub: {error}", row.repo);
    }
    Ok(())
}

/// The accept of a head's weakenings, if an admin gave one.
pub async fn accept(
    db: &D1Database,
    work: &str,
    head: &str,
) -> worker::Result<Option<review::Accept>> {
    #[derive(Deserialize)]
    struct AcceptRow {
        weakened: String,
        login: String,
        at: f64,
    }
    let row: Option<AcceptRow> = db
        .prepare("SELECT weakened, login, at FROM accepts WHERE work = ?1 AND head = ?2")
        .bind(&[work.into(), head.into()])?
        .first(None)
        .await?;
    row.map(|r| {
        Ok(review::Accept {
            head: head.to_owned(),
            weakened: serde_json::from_str(&r.weakened)?,
            by: r.login,
            at: r.at,
        })
    })
    .transpose()
}

/// What the app last wrote on a pull request.
#[derive(Deserialize)]
struct PullRow {
    head: Option<String>,
    check_run: Option<f64>,
    sent: Option<String>,
    comment: Option<f64>,
    comment_body: Option<String>,
    closed: f64,
}

/// The check run's status and conclusion for a review.
fn status(state: State) -> (&'static str, Option<&'static str>) {
    match state {
        State::Ci => ("queued", None),
        State::Running => ("in_progress", None),
        State::Failed | State::NeedsChange => ("completed", Some("failure")),
        State::Cancelled => ("completed", Some("cancelled")),
        State::Waits => ("completed", Some("action_required")),
        State::Approved => ("completed", Some("success")),
    }
}

/// Writes the review to its pull request as the app: the head's check run, created when the
/// head has none or a completed one would go back to running, updated otherwise; and the
/// comment, created only by a run's first word. Nothing unchanged is written again.
async fn publish(
    db: &D1Database,
    config: &Config,
    row: &Row,
    pull: u64,
    review: &Review,
    first_run: bool,
) -> worker::Result<()> {
    let number = (pull as f64).into();
    let repo: JsValue = row.repo.as_str().into();
    db.prepare("INSERT OR IGNORE INTO pulls (repo, number) VALUES (?1, ?2)")
        .bind(&[repo.clone(), JsValue::clone(&number)])?
        .run()
        .await?;
    let Some(last) = db
        .prepare("SELECT head, check_run, sent, comment, comment_body, closed FROM pulls WHERE repo = ?1 AND number = ?2")
        .bind(&[repo.clone(), JsValue::clone(&number)])?
        .first::<PullRow>(None)
        .await?
    else {
        return Ok(());
    };
    if last.closed != 0.0 {
        return Ok(());
    }
    let page = work::page_url(config, &row.repository, &row.branch, Some(pull));
    let (status, conclusion) = status(review.state);
    let mut check = json!({
        "name": NAME,
        "details_url": page,
        "status": status,
        "output": {"title": review.title(), "summary": review.summary()},
    });
    if let Some(conclusion) = conclusion {
        check["conclusion"] = json!(conclusion);
    }
    let sent = check.to_string();
    let same_head = last.head.as_deref() == Some(review.head.as_str());
    let check_write = match (same_head, last.check_run) {
        (true, Some(_)) if last.sent.as_deref() == Some(sent.as_str()) => None,
        // GitHub keeps a completed check run completed: running again is a new one.
        (true, Some(id)) if !(was_completed(last.sent.as_deref()) && conclusion.is_none()) => {
            Some((
                Method::Patch,
                format!("/repos/{}/check-runs/{}", row.repository, id as u64),
            ))
        }
        _ => Some((
            Method::Post,
            format!("/repos/{}/check-runs", row.repository),
        )),
    };
    let body = format!(
        "**{NAME}**: {}. See [the work's page]({page}).",
        review.title()
    );
    let comment_write = match last.comment {
        Some(_) if last.comment_body.as_deref() == Some(body.as_str()) => None,
        Some(id) => Some((
            Method::Patch,
            format!("/repos/{}/issues/comments/{}", row.repository, id as u64),
        )),
        None if first_run => Some((
            Method::Post,
            format!("/repos/{}/issues/{pull}/comments", row.repository),
        )),
        None => None,
    };
    if check_write.is_none() && comment_write.is_none() {
        return Ok(());
    }
    let Some(token) = github::installation_token(config, &row.repository).await? else {
        return Err(worker::Error::RustError("the app is not installed".into()));
    };
    if let Some((method, path)) = check_write {
        let mut payload = check.clone();
        if method == Method::Post {
            payload["head_sha"] = json!(review.head);
        }
        let id = github::write(&token, config, method, &path, &payload).await?;
        db.prepare(
            "UPDATE pulls SET head = ?3, check_run = ?4, sent = ?5 WHERE repo = ?1 AND number = ?2",
        )
        .bind(&[
            repo.clone(),
            JsValue::clone(&number),
            review.head.as_str().into(),
            (id as f64).into(),
            sent.as_str().into(),
        ])?
        .run()
        .await?;
    }
    if let Some((method, path)) = comment_write {
        let id = github::write(&token, config, method, &path, &json!({"body": body})).await?;
        db.prepare(
            "UPDATE pulls SET comment = ?3, comment_body = ?4 WHERE repo = ?1 AND number = ?2",
        )
        .bind(&[repo, number, (id as f64).into(), body.as_str().into()])?
        .run()
        .await?;
    }
    Ok(())
}

/// Whether the check run body last sent completed it.
fn was_completed(sent: Option<&str>) -> bool {
    sent.and_then(|s| serde_json::from_str::<Value>(s).ok())
        .is_some_and(|v| v["status"] == "completed")
}
