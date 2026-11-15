//! A piece of work as D1 records it, the one record of it: the work (one branch of one
//! repository), its runs and each run's checks as they stream, and its review. Every write bumps the work's
//! sequence number in the same batch, so a page can tell which events its snapshot already
//! holds. The work's room (`room.rs`) holds only the open pages and fans each event out.

use std::collections::BTreeMap;

use fearlezz_proof::events::{Keys, Live, Run, RunCheck, RunState, Snapshot, Source, Work};
use fearlezz_proof::look::Look;
use fearlezz_proof::notes::{Note, NoteState, Region, Side};
use fearlezz_proof::review::{self, Review, Verdict};
use fearlezz_proof::{Check, Proof, Pull};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::Deserialize;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, D1Result, Env};

use crate::{Config, jobs, repository, room};

/// How many runs a page shows, newest first.
const RUNS_SHOWN: f64 = 20.0;

/// Where a page of a work is: a branch, or the pull request the work has.
pub enum Place {
    Branch(String),
    Pull(u64),
}

/// What a branch's path segments must escape: the characters a URL path gives a meaning.
const SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}');

/// The host's page of a work: its pull request's when it has one, its branch's otherwise. The
/// host builds every URL; the CLI never does.
pub fn page_url(config: &Config, repository: &str, branch: &str, pull: Option<u64>) -> String {
    format!(
        "{}{}",
        config.app_origin,
        page_path(repository, branch, pull)
    )
}

/// The path of a work's page on the app origin, as `page_url` names it.
pub fn page_path(repository: &str, branch: &str, pull: Option<u64>) -> String {
    match pull {
        Some(number) => format!("/{repository}/pull/{number}"),
        None => {
            let branch = utf8_percent_encode(branch, SEGMENT);
            format!("/{repository}/work/{branch}")
        }
    }
}

/// The work at a place of a repository, by its id.
pub async fn find(db: &D1Database, repo: &str, place: &Place) -> worker::Result<Option<String>> {
    let statement = match place {
        Place::Branch(branch) => db
            .prepare("SELECT id FROM works WHERE repo = ?1 AND branch = ?2")
            .bind(&[repo.into(), branch.as_str().into()])?,
        Place::Pull(number) => db
            .prepare("SELECT id FROM works WHERE repo = ?1 AND pull = ?2 ORDER BY rowid DESC")
            .bind(&[repo.into(), (*number as f64).into()])?,
    };
    statement.first(Some("id")).await
}

/// A work as stored.
#[derive(Deserialize)]
pub struct WorkRow {
    pub id: String,
    pub repository: String,
    pub branch: String,
    pub pull: Option<f64>,
    pub pull_title: Option<String>,
    pub pull_url: Option<String>,
}

impl WorkRow {
    pub fn work(&self) -> Work {
        Work {
            repository: self.repository.clone(),
            branch: self.branch.clone(),
            pull: self.pull.map(|number| Pull {
                number: number as u64,
                title: self.pull_title.clone().unwrap_or_default(),
                url: self.pull_url.clone(),
            }),
        }
    }
}

/// The statement that bumps a work's sequence number, answering the new one, and notes when
/// it changed for the repository page; it ends every batch that changes the work.
pub fn bump(db: &D1Database, work: &str) -> worker::Result<D1PreparedStatement> {
    // In milliseconds, so works that change within one second still list in order.
    let at = js_sys::Date::now() / 1000.0;
    db.prepare("UPDATE works SET seq = seq + 1, active = ?2 WHERE id = ?1 RETURNING seq")
        .bind(&[work.into(), at.into()])
}

/// Runs a batch that ends with `bump`, answering the work's new sequence number.
pub async fn commit(db: &D1Database, statements: Vec<D1PreparedStatement>) -> worker::Result<u64> {
    #[derive(Deserialize)]
    struct Seq {
        seq: f64,
    }
    let results = db.batch(statements).await?;
    let seq = results
        .last()
        .map(|r| r.results::<Seq>())
        .transpose()?
        .and_then(|rows| rows.first().map(|row| row.seq as u64))
        .ok_or_else(|| worker::Error::RustError("the batch did not bump the work".into()))?;
    Ok(seq)
}

#[derive(Deserialize)]
struct RunRow {
    id: String,
    head: String,
    base: String,
    source: String,
    state: String,
    opened: f64,
    proof: Option<String>,
}

#[derive(Deserialize)]
struct CheckRow {
    run: String,
    id: String,
    started: Option<f64>,
    entry: Option<String>,
    look: Option<String>,
}

#[derive(Deserialize)]
struct HeldRow {
    run: String,
    path: String,
    sha256: String,
}

/// The files of a run whose bytes the host holds: their SHA-256 by path.
pub async fn held(db: &D1Database, run: &str) -> worker::Result<Files> {
    Ok(db
        .prepare(format!("{HELD} WHERE f.run = ?1"))
        .bind(&[run.into()])?
        .all()
        .await?
        .results::<HeldRow>()?
        .into_iter()
        .map(|row| (row.path, row.sha256))
        .collect())
}

/// A run's held files, by path: what a look's pictures and their keys are read from.
pub type Files = BTreeMap<String, String>;

/// The files whose bytes the host holds, of the runs a `WHERE` after it picks.
const HELD: &str = "SELECT f.run, f.path, f.sha256 FROM files f JOIN runs r ON r.id = f.run JOIN works w ON w.id = r.work JOIN blobs b ON b.repo = w.repo AND b.sha256 = f.sha256";

/// Whether every picture a look names is held: until then a page could only show it broken,
/// so pages get a look only once its files are all in.
pub fn complete(look: &Look, held: &Files) -> bool {
    look.items
        .iter()
        .flat_map(|item| [&item.before, &item.after])
        .flatten()
        .all(|path| held.contains_key(path))
}

/// A look as pages get it: only once complete, with the keys of its pictures.
pub fn shown(look: Look, held: &Files) -> Option<(Look, Keys)> {
    if !complete(&look, held) {
        return None;
    }
    let keys = review::keys(&look.items, held);
    Some((look, keys))
}

#[derive(Deserialize)]
struct SeqRow {
    seq: f64,
}

#[derive(Deserialize)]
struct ReviewRow {
    review: Option<String>,
}

#[derive(Deserialize)]
struct VerdictRow {
    check_id: String,
    picture: String,
    key: String,
    login: String,
    at: f64,
    run: String,
    head: String,
}

impl VerdictRow {
    fn verdict(self) -> Verdict {
        Verdict {
            check: self.check_id,
            picture: self.picture,
            key: self.key,
            by: self.login,
            at: self.at,
            run: self.run,
            head: self.head,
        }
    }
}

/// The work's verdicts, in the order given.
const VERDICTS: &str =
    "SELECT check_id, picture, key, login, at, run, head FROM verdicts WHERE work = ?1 ORDER BY at";

/// A note as stored.
#[derive(Deserialize)]
pub struct NoteRow {
    id: String,
    pub work: String,
    check_id: String,
    picture: String,
    run: String,
    head: String,
    side: String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    time: Option<f64>,
    words: String,
    login: String,
    at: f64,
    state: String,
    resolved_by: Option<String>,
}

impl NoteRow {
    pub fn note(self) -> Note {
        Note {
            id: self.id,
            check: self.check_id,
            picture: self.picture,
            run: self.run,
            head: self.head,
            side: match self.side.as_str() {
                "before" => Side::Before,
                "both" => Side::Both,
                _ => Side::After,
            },
            region: Region {
                x: self.x,
                y: self.y,
                w: self.w,
                h: self.h,
            },
            time: self.time,
            words: self.words,
            by: self.login,
            at: self.at,
            state: if self.state == "resolved" {
                NoteState::Resolved
            } else {
                NoteState::Open
            },
            resolved_by: self.resolved_by,
        }
    }
}

/// Every column of a note, as `NoteRow` reads it.
pub const NOTE_COLUMNS: &str = "id, work, check_id, picture, run, head, side, x, y, w, h, time, words, login, at, state, resolved_by";

/// The work's open notes, oldest first.
fn open_notes(db: &D1Database, work: &str) -> worker::Result<D1PreparedStatement> {
    db.prepare(format!(
        "SELECT {NOTE_COLUMNS} FROM notes WHERE work = ?1 AND state = 'open' ORDER BY at, rowid"
    ))
    .bind(&[work.into()])
}

fn notes_of(result: &D1Result) -> worker::Result<Vec<Note>> {
    Ok(result
        .results::<NoteRow>()?
        .into_iter()
        .map(NoteRow::note)
        .collect())
}

/// The runs a `WHERE` picks, as three statements of a batch: the runs, their checks, and their
/// held files; `assemble` reads their results.
fn run_statements(
    db: &D1Database,
    picks: &str,
    binds: &[JsValue],
) -> worker::Result<Vec<D1PreparedStatement>> {
    let runs = format!(
        "SELECT id, head, base, source, state, opened, proof FROM runs WHERE {picks} ORDER BY opened DESC, rowid DESC"
    );
    let ids = format!("SELECT id FROM ({runs})");
    Ok(vec![
        db.prepare(&runs).bind(binds)?,
        db.prepare(format!(
            "SELECT c.run, c.id, c.started, c.entry, c.look FROM checks c WHERE c.run IN ({ids}) ORDER BY c.position"
        ))
        .bind(binds)?,
        db.prepare(format!("{HELD} WHERE f.run IN ({ids})"))
            .bind(binds)?,
    ])
}

/// The runs `run_statements` read, newest first, each with its proof once done.
fn assemble(results: &[D1Result]) -> worker::Result<Vec<(Run, Option<Proof>)>> {
    let [runs, checks, held] = results else {
        return Err(worker::Error::RustError(
            "D1 answered another number of results".into(),
        ));
    };
    let checks = checks.results::<CheckRow>()?;
    let held = held.results::<HeldRow>()?;
    runs.results::<RunRow>()?
        .into_iter()
        .map(|run| {
            let files: Files = held
                .iter()
                .filter(|h| h.run == run.id)
                .map(|h| (h.path.clone(), h.sha256.clone()))
                .collect();
            let checks = checks
                .iter()
                .filter(|c| c.run == run.id)
                .map(|c| {
                    let look = c.look.as_deref().map(read::<Look>).transpose()?;
                    let (look, keys) = look.and_then(|l| shown(l, &files)).unzip();
                    Ok(RunCheck {
                        id: c.id.clone(),
                        started: c.started,
                        entry: c.entry.as_deref().map(read::<Check>).transpose()?,
                        look,
                        keys: keys.unwrap_or_default(),
                    })
                })
                .collect::<worker::Result<Vec<_>>>()?;
            let proof = run.proof.as_deref().map(read::<Proof>).transpose()?;
            Ok((
                Run {
                    source: read::<Source>(&run.source)?,
                    state: state(&run.state),
                    id: run.id,
                    head: run.head,
                    base: run.base,
                    opened: run.opened,
                    checks,
                },
                proof,
            ))
        })
        .collect()
}

/// Everything a page needs of a work, read in one batch so it is one consistent moment: the
/// work, its newest runs with their checks, its review, verdicts and waiting jobs, and the
/// sequence number of the last event it holds.
pub async fn snapshot(db: &D1Database, work: &str) -> worker::Result<Option<Snapshot>> {
    let mut statements = vec![
        db.prepare(
            "SELECT id, repository, branch, pull, pull_title, pull_url, seq, review FROM works WHERE id = ?1",
        )
        .bind(&[work.into()])?,
        db.prepare(VERDICTS).bind(&[work.into()])?,
        jobs::statement(db, work)?,
        open_notes(db, work)?,
    ];
    statements.extend(run_statements(
        db,
        "id IN (SELECT id FROM runs WHERE work = ?1 ORDER BY opened DESC, rowid DESC LIMIT ?2)",
        &[work.into(), RUNS_SHOWN.into()],
    )?);
    let results = db.batch(statements).await?;
    let [works, verdicts, waiting, notes, runs @ ..] = results.as_slice() else {
        return Err(worker::Error::RustError(
            "D1 answered another number of results".into(),
        ));
    };
    let Some(row) = works.results::<WorkRow>()?.into_iter().next() else {
        return Ok(None);
    };
    let seq = works
        .results::<SeqRow>()?
        .first()
        .map_or(0, |s| s.seq as u64);
    let review = works
        .results::<ReviewRow>()?
        .into_iter()
        .next()
        .and_then(|r| r.review)
        .map(|r| read::<Review>(&r))
        .transpose()?;
    let verdicts = verdicts
        .results::<VerdictRow>()?
        .into_iter()
        .map(VerdictRow::verdict)
        .collect();
    Ok(Some(Snapshot {
        seq,
        work: row.work(),
        runs: assemble(runs)?.into_iter().map(|(run, _)| run).collect(),
        review,
        verdicts,
        waiting: jobs::read(waiting)?,
        notes: notes_of(notes)?,
    }))
}

/// What a work's review is derived from: its runs of `head`, newest first, with their
/// proofs, its verdicts and its open notes.
pub struct OfHead {
    pub runs: Vec<(Run, Option<Proof>)>,
    pub verdicts: Vec<Verdict>,
    pub notes: Vec<Note>,
}

pub async fn of_head(db: &D1Database, work: &str, head: &str) -> worker::Result<OfHead> {
    let mut statements = vec![
        db.prepare(VERDICTS).bind(&[work.into()])?,
        open_notes(db, work)?,
    ];
    statements.extend(run_statements(
        db,
        "work = ?1 AND head = ?2",
        &[work.into(), head.into()],
    )?);
    let results = db.batch(statements).await?;
    let [verdicts, notes, runs @ ..] = results.as_slice() else {
        return Err(worker::Error::RustError(
            "D1 answered another number of results".into(),
        ));
    };
    let verdicts = verdicts
        .results::<VerdictRow>()?
        .into_iter()
        .map(VerdictRow::verdict)
        .collect();
    Ok(OfHead {
        runs: assemble(runs)?,
        verdicts,
        notes: notes_of(notes)?,
    })
}

/// A run's state as stored.
pub fn state(word: &str) -> RunState {
    match word {
        "done" => RunState::Done,
        "cancelled" => RunState::Cancelled,
        _ => RunState::Running,
    }
}

/// JSON the host stored, read back.
pub fn read<T: serde::de::DeserializeOwned>(json: &str) -> worker::Result<T> {
    serde_json::from_str(json)
        .map_err(|e| worker::Error::RustError(format!("a stored record does not read back: {e}")))
}

/// Hands an event to the work's room, which sends it to every page open on the work, and the
/// work as it now stands to its repository's room. The event is in D1 already, so a room that
/// cannot be reached loses nothing: a page that reconnects gets it in its snapshot. It is
/// logged and the call goes on.
pub async fn notify(env: &Env, work: &str, live: &Live) {
    let sent = async { room::send(env, work, &serde_json::to_string(live)?).await };
    if let Err(error) = sent.await {
        worker::console_error!("the room of work {work} missed an event: {error}");
    }
    repository::changed(env, work).await;
}
