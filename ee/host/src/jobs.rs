//! CI jobs as GitHub's `workflow_job` webhook names them, matched to Fearlezz runs:
//!
//! - A job is a Fearlezz run's own when it is of the run's workflow run and attempt and the
//!   run's runner took it (`own`): its name need not be the run's job id (`GITHUB_JOB`), as a
//!   workflow may name it (`name: Proof`). The runner tells the legs of a matrix apart, as every
//!   leg has the same job id. Only when one runner took several jobs of the attempt (a self
//!   hosted one) does the name choose: the one that is the job id, alone or with a matrix's
//!   values after it (`proof (macos-latest)`); when that leaves it unclear, no job is the run's.
//! - A queued job has no runner yet: one whose name a Fearlezz run of the repository named, by
//!   the same reading of names, waits
//!   on the page of the work of its branch until a runner takes it.
//! - A completed job closes, as cancelled, its own run if that is still running: the job ended
//!   without the run saying done.
//! - A job's display name, once a runner took it, is the leg of its own run, and the review
//!   counts the newest run of each leg (`fearlezz_proof::review::review`).
//! - A workflow run attempt that completed (GitHub's `workflow_run` webhook) ends every leg its
//!   plan made: one that opened no run by then never will.

use std::collections::{BTreeMap, BTreeSet};

use fearlezz_proof::review::Job;
use serde::Deserialize;
use worker::{D1Database, D1PreparedStatement, D1Result};

/// Whether a job named `name` is the job `job` (`GITHUB_JOB`) names, alone or in a matrix.
fn named(name: &str, job: &str) -> bool {
    name == job
        || name
            .strip_prefix(job)
            .is_some_and(|rest| rest.starts_with(" ("))
}

/// Which of `names`, the jobs of one workflow run attempt that a run's runner took, is the run's
/// own, `job` being its job id: the only one, or else the only one `job` names. None while
/// that stays unclear.
pub fn own<'a>(names: impl IntoIterator<Item = &'a str>, job: &str) -> Option<&'a str> {
    let names: Vec<&str> = names.into_iter().collect();
    if let [name] = names.as_slice() {
        return Some(name);
    }
    let mut named = names.into_iter().filter(|name| named(name, job));
    match (named.next(), named.next()) {
        (Some(name), None) => Some(name),
        _ => None,
    }
}

/// The statement that reads, for each CI run of work `work` at `head`, the names of the jobs
/// of its workflow run and attempt that its runner took.
pub fn legs_statement(
    db: &D1Database,
    work: &str,
    head: &str,
) -> worker::Result<D1PreparedStatement> {
    db.prepare(
        "SELECT r.id AS run, json_extract(r.source, '$.job') AS job, j.name FROM runs r \
         JOIN works w ON w.id = r.work \
         JOIN jobs j ON j.repo = w.repo AND j.workflow_run = json_extract(r.source, '$.run_id') \
           AND j.run_attempt = json_extract(r.source, '$.run_attempt') \
           AND j.runner = json_extract(r.source, '$.runner_name') \
         WHERE r.work = ?1 AND r.head = ?2 AND json_extract(r.source, '$.kind') = 'ci'",
    )
    .bind(&[work.into(), head.into()])
}

/// The leg of each run `legs_statement` read, by run id: the name of the job that is the run's
/// own (`own`). A run whose job no webhook named yet, or is unclear, has none.
pub fn legs(result: &D1Result) -> worker::Result<BTreeMap<String, String>> {
    #[derive(Deserialize)]
    struct LegRow {
        run: String,
        job: String,
        name: String,
    }
    let mut found: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
    for row in result.results::<LegRow>()? {
        found
            .entry(row.run)
            .or_insert((row.job, Vec::new()))
            .1
            .push(row.name);
    }
    Ok(found
        .into_iter()
        .filter_map(|(run, (job, names))| {
            let name = own(names.iter().map(String::as_str), &job)?.to_owned();
            Some((run, name))
        })
        .collect())
}

/// The statement that reads the workflow run attempts of work `work`'s repository at `head`
/// that completed, each with whether its workflow is one the repository proves with.
pub(crate) fn ended_statement(
    db: &D1Database,
    work: &str,
    head: &str,
) -> worker::Result<D1PreparedStatement> {
    db.prepare(
        "SELECT a.workflow_run, a.run_attempt, \
           EXISTS (SELECT 1 FROM proof_workflows p WHERE p.repo = a.repo AND p.path = a.path) AS proof \
         FROM attempts a JOIN works w ON w.repo = a.repo WHERE w.id = ?1 AND a.head = ?2",
    )
    .bind(&[work.into(), head.into()])
}

/// The attempts `ended_statement` read, as (`run_id`, `run_attempt`): every one, and those of
/// a proof workflow.
pub(crate) fn ended(
    result: &D1Result,
) -> worker::Result<(BTreeSet<(u64, u32)>, BTreeSet<(u64, u32)>)> {
    #[derive(Deserialize)]
    struct AttemptRow {
        workflow_run: f64,
        run_attempt: f64,
        proof: f64,
    }
    let mut ended = BTreeSet::new();
    let mut proof = BTreeSet::new();
    for row in result.results::<AttemptRow>()? {
        let attempt = (row.workflow_run as u64, row.run_attempt as u32);
        if row.proof != 0.0 {
            proof.insert(attempt);
        }
        ended.insert(attempt);
    }
    Ok((ended, proof))
}

/// The statement that reads the names of the jobs of work `work`'s repository at `head`, by
/// workflow run and attempt.
pub(crate) fn names_statement(
    db: &D1Database,
    work: &str,
    head: &str,
) -> worker::Result<D1PreparedStatement> {
    db.prepare(
        "SELECT j.workflow_run, j.run_attempt, j.name FROM jobs j JOIN works w ON w.repo = j.repo \
         WHERE w.id = ?1 AND j.head = ?2 ORDER BY j.id",
    )
    .bind(&[work.into(), head.into()])
}

/// The job names `names_statement` read, by (`run_id`, `run_attempt`).
pub(crate) fn names(result: &D1Result) -> worker::Result<BTreeMap<(u64, u32), Vec<String>>> {
    #[derive(Deserialize)]
    struct NameRow {
        workflow_run: f64,
        run_attempt: f64,
        name: String,
    }
    let mut names: BTreeMap<(u64, u32), Vec<String>> = BTreeMap::new();
    for row in result.results::<NameRow>()? {
        names
            .entry((row.workflow_run as u64, row.run_attempt as u32))
            .or_default()
            .push(row.name);
    }
    Ok(names)
}

#[derive(Deserialize)]
struct Row {
    name: String,
    head: String,
    url: Option<String>,
    since: f64,
}

/// The statement that reads the jobs waiting for a runner on work `work`'s page: those whose job
/// id some run of the repository streamed as its own. A matrix job is named `<id> (<leg>)`, and
/// a job id holds no spaces, so the id is the name up to its first ` (`; matching on the id
/// lets `runs_by_job` find the runs instead of reading every run of the repository.
pub fn statement(db: &D1Database, work: &str) -> worker::Result<D1PreparedStatement> {
    db.prepare(
        "SELECT j.name, j.head, j.url, j.since FROM jobs j JOIN works w ON w.repo = j.repo AND w.branch = j.branch \
         WHERE w.id = ?1 AND j.state IN ('queued', 'waiting') AND EXISTS ( \
           SELECT 1 FROM runs r JOIN works o ON o.id = r.work \
           WHERE json_extract(r.source, '$.job') IN (j.name, substr(j.name, 1, instr(j.name, ' (') - 1)) \
             AND o.repo = j.repo) \
         ORDER BY j.since, j.id",
    )
    .bind(&[work.into()])
}

/// The jobs `statement` read.
pub fn read(result: &D1Result) -> worker::Result<Vec<Job>> {
    Ok(result
        .results::<Row>()?
        .into_iter()
        .map(|row| Job {
            name: row.name,
            head: row.head,
            url: row.url,
            since: row.since,
        })
        .collect())
}

/// The jobs waiting for a runner on work `work`'s page.
pub async fn waiting(db: &D1Database, work: &str) -> worker::Result<Vec<Job>> {
    read(&statement(db, work)?.all().await?)
}
