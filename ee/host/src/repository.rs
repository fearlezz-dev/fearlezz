//! The repository page: `/<owner>/<repo>` lists every work of the repository, newest activity
//! first, with where its newest run stands and what waits on it (pictures without a verdict,
//! open notes, a weakening to accept), live. Who sees it is the work page's rule.
//!
//! The listing is derived from D1 when a page connects, and again for one work each time that
//! work changes (`work::commit`), which the repository's room sends to its open pages.

use fearlezz_proof::events::{Latest, Listed, Listing, Source};
use fearlezz_proof::review::Review;
use serde::Deserialize;
use worker::{D1Database, Env, Headers, Request, Response};

use crate::session::to_sign_in;
use crate::work::{self, WorkRow};
use crate::{Answer, Config, Refusal, pages, repo_key, room};

/// Every column a listed work is read from; a `WHERE` on `w` follows.
const LISTED: &str = "SELECT w.id, w.repo, w.repository, w.branch, w.pull, w.pull_title, w.pull_url, w.seq, w.review, \
     COALESCE(w.active, 0) AS active, r.head AS run_head, r.source AS run_source, r.state AS run_state, \
     (SELECT COUNT(*) FROM checks c WHERE c.run = r.id AND json_extract(c.entry, '$.outcome') = 'failed') AS failed, \
     (SELECT COUNT(*) FROM notes n WHERE n.work = w.id AND n.state = 'open') AS notes \
     FROM works w LEFT JOIN runs r ON r.id = (SELECT id FROM runs WHERE work = w.id ORDER BY opened DESC, rowid DESC LIMIT 1)";

#[derive(Deserialize)]
struct Row {
    #[serde(flatten)]
    work: WorkRow,
    repo: String,
    seq: f64,
    review: Option<String>,
    active: f64,
    run_head: Option<String>,
    run_source: Option<String>,
    run_state: Option<String>,
    failed: f64,
    notes: f64,
}

impl Row {
    fn listed(self) -> worker::Result<Listed> {
        let review = self
            .review
            .as_deref()
            .map(work::read::<Review>)
            .transpose()?;
        let latest = match (self.run_head, self.run_source, self.run_state) {
            (Some(head), Some(source), Some(state)) => Some(Latest {
                head,
                source: work::read::<Source>(&source)?,
                state: work::state(&state),
                failed: self.failed as u32,
            }),
            _ => None,
        };
        let work = self.work.work();
        Ok(Listed {
            path: work::page_path(
                &work.repository,
                &work.branch,
                work.pull.as_ref().map(|p| p.number),
            ),
            work,
            seq: self.seq as u64,
            active: self.active,
            latest,
            pictures: review.as_ref().map_or(0, |r| r.waiting().count() as u32),
            notes: self.notes as u32,
            weakened: review.as_ref().is_some_and(Review::weakenings_wait),
        })
    }
}

/// Every work of the repository `repo` (lowercased), newest activity first; none when the host
/// holds no work of it.
pub async fn listing(db: &D1Database, repo: &str) -> worker::Result<Option<Listing>> {
    let works = db
        .prepare(format!(
            "{LISTED} WHERE w.repo = ?1 ORDER BY w.active DESC, w.rowid DESC"
        ))
        .bind(&[repo.into()])?
        .all()
        .await?
        .results::<Row>()?
        .into_iter()
        .map(Row::listed)
        .collect::<worker::Result<Vec<_>>>()?;
    let Some(repository) = works.first().map(|w| w.work.repository.clone()) else {
        return Ok(None);
    };
    Ok(Some(Listing::Snapshot { repository, works }))
}

/// Sends the work `work` as it now stands to the open pages of its repository. Like every
/// event, a failure is logged: a page that reconnects reads it from D1.
pub async fn changed(env: &Env, work: &str) {
    let sent = async {
        let row: Option<Row> = env
            .d1("DB")?
            .prepare(format!("{LISTED} WHERE w.id = ?1"))
            .bind(&[work.into()])?
            .first(None)
            .await?;
        let Some(row) = row else {
            return Ok(());
        };
        let repo = row.repo.clone();
        let text = serde_json::to_string(&Listing::Work(row.listed()?))?;
        room::send(env, &room::of_repository(&repo), &text).await
    };
    if let Err(error) = sent.await {
        worker::console_error!("the repository page missed a change of work {work}: {error}");
    }
}

/// The repository page: the app, which lists the works from its live connection. Someone who
/// may not see the repository yet is sent to sign in first; a repository the host holds no
/// work of is `404`.
pub async fn page(req: &Request, env: &Env, config: &Config, owner: &str, name: &str) -> Answer {
    let repo = repo_key(owner, name);
    if !pages::open(req, env, config, &repo).await? {
        return to_sign_in(&req.path());
    }
    if !holds(&env.d1("DB")?, &repo).await? {
        return Err(Refusal::new(404, "not found"));
    }
    pages::app(env, config, &Headers::new()).await
}

/// The repository page's live connection, at `/live/<owner>/<repo>`: a WebSocket that gets the
/// listing and then each work as it changes, or, asked without upgrading, the listing as JSON.
/// Without a session a private repository answers `401`.
pub async fn live(req: &Request, env: &Env, config: &Config, owner: &str, name: &str) -> Answer {
    let repo = repo_key(owner, name);
    if !pages::open(req, env, config, &repo).await? {
        return Err(Refusal::new(401, "sign in to see this repository"));
    }
    let db = env.d1("DB")?;
    if !pages::upgrades(req)? {
        let listing = listing(&db, &repo)
            .await?
            .ok_or_else(|| Refusal::new(404, "not found"))?;
        let headers = Headers::new();
        headers.set("Cache-Control", "no-store")?;
        return Ok(Response::from_json(&listing)?.with_headers(headers));
    }
    if !holds(&db, &repo).await? {
        return Err(Refusal::new(404, "not found"));
    }
    Ok(room::connect(env, req, &room::of_repository(&repo), ("repo", &repo), None).await?)
}

/// Whether the host holds any work of the repository `repo`.
async fn holds(db: &D1Database, repo: &str) -> worker::Result<bool> {
    let any: Option<f64> = db
        .prepare("SELECT 1 AS any FROM works WHERE repo = ?1 LIMIT 1")
        .bind(&[repo.into()])?
        .first(Some("any"))
        .await?;
    Ok(any.is_some())
}
