//! Notes on a work's page: a person with write access marks what should change on a picture of
//! a run, CI's or a local one (a region of its before, its after or both, a moment of a film,
//! their words), and resolves a note once it is settled. Both are what a person says, so they pass the same
//! checks as a verdict (`say::person`): a browser session from the app origin as JSON, never a
//! token. Every change goes to the work's open pages and moves its review, whose check run
//! summary lists the open notes.

use fearlezz_proof::events::{Live, WorkChange};
use fearlezz_proof::look::Look;
use fearlezz_proof::notes::{Ask, Note, NoteState, Side, WORDS_MAX};
use serde_json::json;
use worker::{D1Database, Env, Request, Response};

use crate::say::{self, Act};
use crate::work::{self, NOTE_COLUMNS, NoteRow, read};
use crate::{Answer, Config, Refusal, crypto, now, pages, review};

/// `POST /api/v2/runs/<run>/notes`: a person with write access notes what should change on a
/// picture of a run. A local run takes notes too, though never a verdict: that is how feedback
/// reaches the agent before a pull request exists. `201 {"note"}`.
pub async fn make(mut req: Request, env: &Env, config: &Config, id: &str) -> Answer {
    let person = say::person(&req, env, config, "make a note").await?;
    let ask: Ask = req.json().await.map_err(|_| {
        Refusal::new(
            400,
            "send {\"check\", \"picture\", \"side\": \"before\" | \"after\" | \"both\", \"region\": {\"x\", \"y\", \"w\", \"h\"}, \"time\"?, \"words\"}",
        )
    })?;
    let words = ask.words.trim();
    if words.is_empty() || words.chars().count() > WORDS_MAX {
        return Err(Refusal::new(
            400,
            format!("say what should change, in at most {WORDS_MAX} characters"),
        ));
    }
    if !ask.region.fits() {
        return Err(Refusal::new(
            400,
            "the region is in fractions of the picture: x, y, w and h from 0 to 1, w and h above 0, inside the picture",
        ));
    }
    let db = env.d1("DB")?;
    let run = say::run(&req, env, config, &db, id).await?;
    say::require(
        env,
        config,
        &person.login,
        &person.token,
        &run.repository,
        Act::Note,
    )
    .await?;
    let look: Option<String> = db
        .prepare("SELECT look FROM checks WHERE run = ?1 AND id = ?2")
        .bind(&[id.into(), ask.check.as_str().into()])?
        .first(Some("look"))
        .await?;
    let look = look.map(|l| read::<Look>(&l)).transpose()?;
    let item = look
        .as_ref()
        .and_then(|l| l.items.iter().find(|i| i.name == ask.picture))
        .ok_or_else(|| {
            Refusal::new(
                409,
                format!("this run has no picture {} of `{}`", ask.picture, ask.check),
            )
        })?;
    let has = match ask.side {
        Side::Before => item.before.is_some(),
        Side::After => item.after.is_some(),
        Side::Both => item.before.is_some() && item.after.is_some(),
    };
    if !has {
        return Err(Refusal::new(
            400,
            format!("{} has no {} side to mark", ask.picture, ask.side.label()),
        ));
    }
    match ask.time {
        Some(_) if item.kind != "film" => {
            return Err(Refusal::new(400, "only a film has a moment to mark"));
        }
        Some(t) if !t.is_finite() || t < 0.0 => {
            return Err(Refusal::new(
                400,
                "a moment is in seconds from the film's start",
            ));
        }
        _ => {}
    }
    let note = Note {
        id: crypto::random_id()?,
        check: ask.check,
        picture: ask.picture,
        run: id.to_owned(),
        head: run.head.clone(),
        side: ask.side,
        region: ask.region,
        time: ask.time,
        words: words.to_owned(),
        by: person.login,
        at: now() as f64,
        state: NoteState::Open,
        resolved_by: None,
    };
    let side = match note.side {
        Side::Before => "before",
        Side::After => "after",
        Side::Both => "both",
    };
    let insert = db
        .prepare(
            "INSERT INTO notes (id, work, check_id, picture, run, head, side, x, y, w, h, time, words, login, at, state) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, 'open')",
        )
        .bind(&[
            note.id.as_str().into(),
            run.work.as_str().into(),
            note.check.as_str().into(),
            note.picture.as_str().into(),
            note.run.as_str().into(),
            note.head.as_str().into(),
            side.into(),
            note.region.x.into(),
            note.region.y.into(),
            note.region.w.into(),
            note.region.h.into(),
            note.time.map_or(wasm_bindgen::JsValue::NULL, Into::into),
            note.words.as_str().into(),
            note.by.as_str().into(),
            note.at.into(),
        ])?;
    changed(env, config, &db, &run.work, insert, &note).await?;
    Ok(Response::from_json(&json!({ "note": note }))?.with_status(201))
}

/// `POST /api/v2/notes/<id>/resolve`: a person with write access resolves a note, which then
/// leaves the pages and the agent's reading. `200 {"note"}`. Once a picture's last note is
/// resolved it no longer needs change: it looks right when a verdict holds for its content, and
/// waits for a look otherwise.
pub async fn resolve(req: Request, env: &Env, config: &Config, id: &str) -> Answer {
    let person = say::person(&req, env, config, "resolve a note").await?;
    let db = env.d1("DB")?;
    let row: Option<NoteRow> = db
        .prepare(format!("SELECT {NOTE_COLUMNS} FROM notes WHERE id = ?1"))
        .bind(&[id.into()])?
        .first(None)
        .await?;
    let row = row.ok_or_else(|| Refusal::new(404, "not found"))?;
    #[derive(serde::Deserialize)]
    struct WorkRow {
        repo: String,
        repository: String,
    }
    let work: WorkRow = db
        .prepare("SELECT repo, repository FROM works WHERE id = ?1")
        .bind(&[row.work.as_str().into()])?
        .first(None)
        .await?
        .ok_or_else(|| Refusal::new(404, "not found"))?;
    if !pages::open(&req, env, config, &work.repo).await? {
        return Err(Refusal::new(404, "not found"));
    }
    say::require(
        env,
        config,
        &person.login,
        &person.token,
        &work.repository,
        Act::Resolve,
    )
    .await?;
    let work_id = row.work.clone();
    let mut note = row.note();
    note.state = NoteState::Resolved;
    note.resolved_by = Some(person.login);
    let update = db
        .prepare("UPDATE notes SET state = 'resolved', resolved_by = ?2 WHERE id = ?1")
        .bind(&[
            id.into(),
            note.resolved_by.as_deref().unwrap_or_default().into(),
        ])?;
    changed(env, config, &db, &work_id, update, &note).await?;
    Ok(Response::from_json(&json!({ "note": note }))?)
}

/// Writes a note's change with the work's next event, sends it to the open pages, and derives
/// the review again.
async fn changed(
    env: &Env,
    config: &Config,
    db: &D1Database,
    work: &str,
    write: worker::D1PreparedStatement,
    note: &Note,
) -> Result<(), Refusal> {
    let seq = work::commit(db, vec![write, work::bump(db, work)?]).await?;
    let change = WorkChange::Note { note: note.clone() };
    work::notify(
        env,
        work,
        &Live::Work {
            seq,
            change: Box::new(change),
        },
    )
    .await;
    review::refresh(env, config, work, false).await?;
    Ok(())
}
