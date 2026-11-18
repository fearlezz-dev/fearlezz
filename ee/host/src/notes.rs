//! Notes on a work's page: a person with write access marks what should change on a picture of
//! a run, CI's or a local one (a region of its before, its after or both, a moment of a film,
//! their words), and resolves a note once it is settled. Both are what a person says, so they pass the same
//! checks as a verdict (`access::speaker`): a browser session from the app origin as JSON, never a
//! token. Every change goes to the work's open pages and moves its review, whose check run
//! summary lists the open notes.

use fearlezz_proof::events::WorkChange;
use fearlezz_proof::look::Look;
use fearlezz_proof::notes::{Ask, Note, NoteState, Side, WORDS_MAX};
use serde_json::json;
use worker::{Env, Request, Response};

use crate::access::{self, Action, Viewer};
use crate::work::{Derive, Event, NOTE_COLUMNS, NoteRow, Statement, Write, read};
use crate::{Answer, Config, Refusal, crypto, not_found, now, room, say};

/// `POST /api/v2/runs/<run>/notes`: a person with write access notes what should change on a
/// picture of a run. A local run takes notes too, though never a verdict: that is how feedback
/// reaches the agent before a pull request exists. `201 {"note"}`.
pub async fn make(mut req: Request, env: &Env, config: &Config, id: &str) -> Answer {
    let person = access::speaker(&req, env, config, "make a note").await?;
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
    let viewer = Viewer::Person(&person);
    let run = say::run(env, config, &db, viewer, id).await?;
    access::can(env, config, viewer, Action::Note, &run.repository).await?;
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
    let insert = Statement::new(
        "INSERT INTO notes (id, work, check_id, picture, run, head, side, x, y, w, h, time, words, login, at, state) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, 'open')",
        [
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
            note.time.into(),
            note.words.as_str().into(),
            note.by.as_str().into(),
            note.at.into(),
        ],
    );
    changed(env, &run.work, insert, &note).await?;
    Ok(Response::from_json(&json!({ "note": note }))?.with_status(201))
}

/// `POST /api/v2/notes/<id>/resolve`: a person with write access resolves a note, which then
/// leaves the pages and the agent's reading. `200 {"note"}`. Once a picture's last note is
/// resolved it no longer needs change: it looks right when a verdict holds for its content, and
/// waits for a look otherwise.
pub async fn resolve(req: Request, env: &Env, config: &Config, id: &str) -> Answer {
    let person = access::speaker(&req, env, config, "resolve a note").await?;
    let db = env.d1("DB")?;
    let row: Option<NoteRow> = db
        .prepare(format!("SELECT {NOTE_COLUMNS} FROM notes WHERE id = ?1"))
        .bind(&[id.into()])?
        .first(None)
        .await?;
    let row = row.ok_or_else(not_found)?;
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
        .ok_or_else(not_found)?;
    let viewer = Viewer::Person(&person);
    let read = Action::Read {
        work: Some(&row.work),
    };
    access::can(env, config, viewer, read, &work.repo)
        .await
        .map_err(Refusal::hidden)?;
    access::can(env, config, viewer, Action::Resolve, &work.repository).await?;
    let work_id = row.work.clone();
    let mut note = row.note();
    note.state = NoteState::Resolved;
    note.resolved_by = Some(person.login);
    let update = Statement::new(
        "UPDATE notes SET state = 'resolved', resolved_by = ?2 WHERE id = ?1",
        [id.into(), note.resolved_by.clone().into()],
    );
    changed(env, &work_id, update, &note).await?;
    Ok(Response::from_json(&json!({ "note": note }))?)
}

/// Writes a note's change with the work's next event, which goes to the open pages, and derives
/// the review again.
async fn changed(env: &Env, work: &str, write: Statement, note: &Note) -> Result<(), Refusal> {
    let write = Write {
        statements: vec![write],
        event: Some(Event::work(WorkChange::Note { note: note.clone() })),
        derive: Derive::Review,
    };
    room::write(env, work, &write).await?;
    Ok(())
}
