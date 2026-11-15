//! CI jobs as GitHub's `workflow_job` webhook names them, matched to Fearlezz runs:
//!
//! - A job is a Fearlezz run's own when it is of the run's workflow run and attempt, the run's
//!   runner took it, and its name is the run's job id (`GITHUB_JOB`), alone or with a matrix's
//!   values after it (`proof (macos-latest)`). The runner tells the legs of a matrix apart, as
//!   every leg has the same job id. A job whose workflow gives it another name is not matched.
//! - A queued job has no runner yet: one whose name a Fearlezz run of the repository named waits
//!   on the page of the work of its branch until a runner takes it.
//! - A completed job closes, as cancelled, its own run if that is still running: the job ended
//!   without the run saying done.
//! - A job's display name, once a runner took it, is the leg of its own run, and the review
//!   counts the newest run of each leg (`fearlezz_proof::review::review`).

use std::collections::BTreeMap;

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

/// Whether the job named `name` on `runner`, of a run's workflow run and attempt, is the run's
/// own: its job id `job` on its runner `runner_name`.
pub fn is(name: &str, runner: Option<&str>, job: &str, runner_name: &str) -> bool {
    runner == Some(runner_name) && named(name, job)
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

/// The leg of each run `legs_statement` read, by run id: the name of the one job that is the
/// run's own, as `is` names it. A run whose job no webhook named yet, or whose runner took
/// more than one job of its name, has none.
pub fn legs(result: &D1Result) -> worker::Result<BTreeMap<String, String>> {
    #[derive(Deserialize)]
    struct LegRow {
        run: String,
        job: String,
        name: String,
    }
    let mut found: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in result.results::<LegRow>()? {
        if named(&row.name, &row.job) {
            found.entry(row.run).or_default().push(row.name);
        }
    }
    Ok(found
        .into_iter()
        .filter_map(|(run, mut names)| (names.len() == 1).then(|| (run, names.remove(0))))
        .collect())
}

#[derive(Deserialize)]
struct Row {
    name: String,
    head: String,
    url: Option<String>,
    since: f64,
}

/// The statement that reads the jobs waiting for a runner on work `work`'s page.
pub fn statement(db: &D1Database, work: &str) -> worker::Result<D1PreparedStatement> {
    db.prepare(
        "SELECT j.name, j.head, j.url, j.since FROM jobs j JOIN works w ON w.repo = j.repo AND w.branch = j.branch \
         WHERE w.id = ?1 AND j.state IN ('queued', 'waiting') AND EXISTS ( \
           SELECT 1 FROM runs r JOIN works o ON o.id = r.work, json_each(r.source) s \
           WHERE o.repo = j.repo AND s.key = 'job' AND (j.name = s.value OR substr(j.name, 1, length(s.value) + 2) = s.value || ' (')) \
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
