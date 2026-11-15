//! CI jobs as GitHub's `workflow_job` webhook names them, matched to Fearlezz runs simply:
//!
//! - A job runs Fearlezz when its name is the job id a Fearlezz run of the repository named
//!   (`GITHUB_JOB`), or that id with a matrix's values after it (`proof (macos-latest)`). A job
//!   whose workflow gives it another name is not matched.
//! - A queued job of such a name waits on the page of the work of its branch until a runner
//!   takes it.
//! - A completed job closes, as cancelled, every run of the repository still running whose run
//!   page is the job's workflow run and whose job id the job's name matches: the job ended
//!   without the run saying done.

use fearlezz_proof::review::Job;
use serde::Deserialize;
use worker::{D1Database, D1PreparedStatement, D1Result};

/// Whether a job named `name` is the job `job` (`GITHUB_JOB`) names, alone or in a matrix.
pub fn runs(name: &str, job: &str) -> bool {
    name == job
        || name
            .strip_prefix(job)
            .is_some_and(|rest| rest.starts_with(" ("))
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
