//! Share links: a person with write access makes a link that shows anyone a work's live page,
//! view only, until someone with write access revokes it. A link names the work (one branch,
//! its pull request when it has one), so the people it reaches follow new runs.
//!
//! Opening a link skips the repository's access check for that work alone, and only views: the
//! page through a link offers no verdict, note, resolve or accept and asks no one to sign in,
//! and every write still takes a session (`say.rs`). The files come from the files origin
//! under the same one hour grant of one run as any other view, asked afresh by each page, for
//! the runs of the link's work alone. The page sends no referrer and asks not to be indexed.
//!
//! The token is 128 random bits, kept only as its HMAC with the signing key, like the CLI's
//! token: the list shows who made each link and when, and the link itself is shown once, when
//! it is made. Revoking deletes the row and ends the pages open on it.

use serde::Deserialize;
use serde_json::json;
use worker::{D1Database, Env, Headers, Request, Response};

use crate::say::Act;
use crate::session::{self, Session};
use crate::work::{self, Place};
use crate::{Answer, Config, Refusal, crypto, now, pages, repo_key, room, say};

fn not_found() -> Refusal {
    Refusal::new(404, "not found")
}

/// A live link, as the host keeps it.
#[derive(Deserialize)]
pub struct Link {
    pub id: String,
    pub work: String,
}

/// The tag of the pages open through link `id`, which revoking it ends.
fn tag(id: &str) -> String {
    format!("share:{id}")
}

async fn hash(config: &Config, token: &str) -> worker::Result<String> {
    let mac = crypto::hmac_sign(&config.signing_key, format!("share.{token}").as_bytes()).await?;
    Ok(hex::encode(mac))
}

/// The live link `token` opens, if any.
pub async fn find(env: &Env, config: &Config, token: &str) -> worker::Result<Option<Link>> {
    env.d1("DB")?
        .prepare("SELECT id, work FROM shares WHERE hash = ?1")
        .bind(&[hash(config, token).await?.into()])?
        .first(None)
        .await
}

/// `/-/share/<token>`: the work's page, for anyone holding the link. An unknown or revoked
/// token is a bare `404`, so it confirms no repository.
pub async fn page(env: &Env, config: &Config, token: &str) -> Answer {
    find(env, config, token).await?.ok_or_else(not_found)?;
    let headers = Headers::new();
    headers.set("Referrer-Policy", "no-referrer")?;
    headers.set("X-Robots-Tag", "noindex, nofollow")?;
    pages::app(env, config, &headers).await
}

/// `/live/-/share/<token>`: the live connection of the link's work, ended when it is revoked.
pub async fn live(req: &Request, env: &Env, config: &Config, token: &str) -> Answer {
    let link = find(env, config, token).await?.ok_or_else(not_found)?;
    pages::connect(req, env, &link.work, Some(&tag(&link.id))).await
}

/// The work a share call names: `{"repository": "owner/name", "branch"}`.
#[derive(Deserialize)]
struct Named {
    repository: String,
    branch: String,
}

/// The work `named` names, when the request's person may write its repository: `404` for a
/// work or private repository they may not see, `403` without write access, `401` when their
/// GitHub sign in expired.
async fn writable(
    req: &Request,
    env: &Env,
    config: &Config,
    person: &Session,
    named: &Named,
) -> Result<(D1Database, String, String), Refusal> {
    let (owner, name) = named.repository.split_once('/').ok_or_else(not_found)?;
    let repo = repo_key(owner, name);
    if !pages::open(req, env, config, &repo).await? {
        return Err(not_found());
    }
    let db = env.d1("DB")?;
    let work = work::find(&db, &repo, &Place::Branch(named.branch.clone()))
        .await?
        .ok_or_else(not_found)?;
    say::require(
        env,
        config,
        &person.login,
        &person.token,
        &named.repository,
        Act::Share,
    )
    .await?;
    Ok((db, work, repo))
}

/// `POST /api/v2/shares`: makes a link to a work. `201 {"url", "share": {"id", "made_by",
/// "made_at"}}`; the URL holds the token and is shown only now.
pub async fn make(mut req: Request, env: &Env, config: &Config) -> Answer {
    let person = say::person(&req, env, config, "share a work").await?;
    let named: Named = req
        .json()
        .await
        .map_err(|_| Refusal::new(400, "send {\"repository\": \"owner/name\", \"branch\"}"))?;
    let (db, work, _) = writable(&req, env, config, &person, &named).await?;
    let token = crypto::random_id()?;
    let id = crypto::random_id()?;
    let made_at = now() as f64;
    db.prepare("INSERT INTO shares (id, hash, work, made_by, made_at) VALUES (?1, ?2, ?3, ?4, ?5)")
        .bind(&[
            id.as_str().into(),
            hash(config, &token).await?.into(),
            work.as_str().into(),
            person.login.as_str().into(),
            made_at.into(),
        ])?
        .run()
        .await?;
    let answer = json!({
        "url": format!("{}/-/share/{token}", config.app_origin),
        "share": {"id": id, "made_by": person.login, "made_at": made_at},
    });
    Ok(Response::from_json(&answer)?.with_status(201))
}

/// `GET /api/v2/shares?repository=<owner/name>&branch=<branch>`: the work's live links, newest
/// first, `200 {"shares": [{"id", "made_by", "made_at"}]}`.
pub async fn list(req: &Request, env: &Env, config: &Config) -> Answer {
    let person = session::read(req, env)
        .await?
        .ok_or_else(|| Refusal::new(401, "sign in to see the share links"))?;
    let url = req.url()?;
    let param = |name: &str| {
        url.query_pairs()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.into_owned())
    };
    let (Some(repository), Some(branch)) = (param("repository"), param("branch")) else {
        return Err(Refusal::new(
            400,
            "name the work: ?repository=owner/name&branch=",
        ));
    };
    let named = Named { repository, branch };
    let (db, work, _) = writable(req, env, config, &person, &named).await?;
    #[derive(Deserialize, serde::Serialize)]
    struct Listed {
        id: String,
        made_by: String,
        made_at: f64,
    }
    let shares: Vec<Listed> = db
        .prepare("SELECT id, made_by, made_at FROM shares WHERE work = ?1 ORDER BY made_at DESC, rowid DESC")
        .bind(&[work.as_str().into()])?
        .all()
        .await?
        .results()?;
    let headers = Headers::new();
    headers.set("Cache-Control", "no-store")?;
    Ok(Response::from_json(&json!({ "shares": shares }))?.with_headers(headers))
}

/// `POST /api/v2/shares/<id>/revoke`: ends a link, and every page open on it. `200
/// {"revoked": "<id>"}`; a link that is gone, or one of a repository the person may not see,
/// is `404`.
pub async fn revoke(req: Request, env: &Env, config: &Config, id: &str) -> Answer {
    let person = say::person(&req, env, config, "revoke a share link").await?;
    #[derive(Deserialize)]
    struct Row {
        work: String,
        repo: String,
        repository: String,
    }
    let db = env.d1("DB")?;
    let row: Option<Row> = db
        .prepare("SELECT s.work, w.repo, w.repository FROM shares s JOIN works w ON w.id = s.work WHERE s.id = ?1")
        .bind(&[id.into()])?
        .first(None)
        .await?;
    let row = row.ok_or_else(not_found)?;
    if !pages::open(&req, env, config, &row.repo).await? {
        return Err(not_found());
    }
    say::require(
        env,
        config,
        &person.login,
        &person.token,
        &row.repository,
        Act::Share,
    )
    .await?;
    db.prepare("DELETE FROM shares WHERE id = ?1")
        .bind(&[id.into()])?
        .run()
        .await?;
    if let Err(error) = room::close(env, &row.work, &tag(id), "This link was revoked.").await {
        worker::console_error!("the pages of share link {id} were not ended: {error}");
    }
    Ok(Response::from_json(&json!({ "revoked": id }))?)
}
