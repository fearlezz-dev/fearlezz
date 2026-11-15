//! A piece of work as D1 records it, the one record of it: the work (one branch of one
//! repository), its runs and each run's checks as they stream. Every write bumps the work's
//! sequence number in the same batch, so a page can tell which events its snapshot already
//! holds. The work's room (`room.rs`) holds only the open pages and fans each event out.

use fearlezz_proof::Check;
use fearlezz_proof::Pull;
use fearlezz_proof::events::{Live, Run, RunCheck, RunState, Snapshot, Source, Work};
use fearlezz_proof::look::Look;
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use serde::Deserialize;
use wasm_bindgen::JsValue;
use worker::{D1Database, D1PreparedStatement, Env, Method, Request, RequestInit};

use crate::Config;

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
    match pull {
        Some(number) => format!("{}/{repository}/pull/{number}", config.app_origin),
        None => {
            let branch = utf8_percent_encode(branch, SEGMENT);
            format!("{}/{repository}/work/{branch}", config.app_origin)
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

/// The statement that bumps a work's sequence number, answering the new one; it ends every
/// batch that changes the work.
pub fn bump(db: &D1Database, work: &str) -> worker::Result<D1PreparedStatement> {
    db.prepare("UPDATE works SET seq = seq + 1 WHERE id = ?1 RETURNING seq")
        .bind(&[work.into()])
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
struct SeqRow {
    seq: f64,
}

/// Everything a page needs of a work, read in one batch so it is one consistent moment: the
/// work, its newest runs with their checks, and the sequence number of the last event it holds.
pub async fn snapshot(db: &D1Database, work: &str) -> worker::Result<Option<Snapshot>> {
    let shown = RUNS_SHOWN.into();
    let results = db
        .batch(vec![
            db.prepare(
                "SELECT id, repository, branch, pull, pull_title, pull_url, seq FROM works WHERE id = ?1",
            )
            .bind(&[work.into()])?,
            db.prepare(
                "SELECT id, head, base, source, state, opened FROM runs WHERE work = ?1 ORDER BY opened DESC, rowid DESC LIMIT ?2",
            )
            .bind(&[work.into(), JsValue::clone(&shown)])?,
            db.prepare(
                "SELECT c.run, c.id, c.started, c.entry, c.look FROM checks c WHERE c.run IN (SELECT id FROM runs WHERE work = ?1 ORDER BY opened DESC, rowid DESC LIMIT ?2) ORDER BY c.position",
            )
            .bind(&[work.into(), shown])?,
        ])
        .await?;
    let [works, runs, checks] = results.as_slice() else {
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
    let checks = checks.results::<CheckRow>()?;
    let runs = runs
        .results::<RunRow>()?
        .into_iter()
        .map(|run| {
            let checks = checks
                .iter()
                .filter(|c| c.run == run.id)
                .map(|c| {
                    Ok(RunCheck {
                        id: c.id.clone(),
                        started: c.started,
                        entry: c.entry.as_deref().map(read::<Check>).transpose()?,
                        look: c.look.as_deref().map(read::<Look>).transpose()?,
                    })
                })
                .collect::<worker::Result<Vec<_>>>()?;
            Ok(Run {
                source: read::<Source>(&run.source)?,
                state: state(&run.state),
                id: run.id,
                head: run.head,
                base: run.base,
                opened: run.opened,
                checks,
            })
        })
        .collect::<worker::Result<Vec<_>>>()?;
    Ok(Some(Snapshot {
        seq,
        work: row.work(),
        runs,
    }))
}

/// A run's state as stored.
pub fn state(word: &str) -> RunState {
    match word {
        "done" => RunState::Done,
        _ => RunState::Running,
    }
}

/// JSON the host stored, read back.
pub fn read<T: serde::de::DeserializeOwned>(json: &str) -> worker::Result<T> {
    serde_json::from_str(json)
        .map_err(|e| worker::Error::RustError(format!("a stored record does not read back: {e}")))
}

/// Hands an event to the work's room, which sends it to every page open on the work. The
/// event is in D1 already, so a room that cannot be reached loses nothing: a page that
/// reconnects gets it in its snapshot. It is logged and the call goes on.
pub async fn notify(env: &Env, work: &str, live: &Live) {
    let sent = async {
        let stub = env.durable_object("WORK")?.id_from_name(work)?.get_stub()?;
        let mut init = RequestInit::new();
        init.with_method(Method::Post)
            .with_body(Some(serde_json::to_string(live)?.into()));
        let request = Request::new_with_init("https://room/event", &init)?;
        stub.fetch_with_request(request).await?;
        Ok::<(), worker::Error>(())
    };
    if let Err(error) = sent.await {
        worker::console_error!("the room of work {work} missed an event: {error}");
    }
}
