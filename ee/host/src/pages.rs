//! The pages on the app origin: a work's page, at its branch or its pull request, and its live
//! connection. The page is the app the worker serves from its static assets; it draws itself
//! from what the live connection sends (`room.rs`). Who sees them mirrors GitHub: anyone for a
//! public repository, people who may read it for a private one, and `404` for everyone else,
//! so a private repository's name is not confirmed.

use worker::{Env, Headers, Request, Response};

use crate::room::Viewer;
use crate::session::{self, Session, to_sign_in};
use crate::work::{self, Place};
use crate::{Answer, Config, Refusal, cache, files, github, repo_key, room, share};

/// How long the host trusts GitHub's word on who may see a repository.
pub const SIGHT_SECONDS: i64 = 300;

fn not_found() -> Refusal {
    Refusal::new(404, "not found")
}

/// Whether the request may see the repository's pages: `false` means sign in first, and a
/// person GitHub shows nothing of it gets `404`.
pub async fn open(req: &Request, env: &Env, config: &Config, repo: &str) -> Result<bool, Refusal> {
    let session = session::read(req, env).await?;
    sees(env, config, session.as_ref(), repo).await
}

/// Whether the person of `session`, or anyone without one, may see the repository's pages, as
/// `open` says it for a request. A live page asks it again while it stays open (`room.rs`).
pub async fn sees(
    env: &Env,
    config: &Config,
    session: Option<&Session>,
    repo: &str,
) -> Result<bool, Refusal> {
    if let Some(session) = session {
        match reads(env, config, &session.login, &session.token, repo).await? {
            Some(true) => return Ok(true),
            Some(false) => return Err(not_found()),
            None => {}
        }
    }
    let key = format!("public:{repo}");
    if let Some(public) = cache::get(env, &key).await? {
        return Ok(public == "yes");
    }
    let public = github::public(config, repo).await?;
    cache::put(env, &key, if public { "yes" } else { "no" }, SIGHT_SECONDS).await?;
    Ok(public)
}

/// Whether `login` may read the repository `repo` (lowercased), as their GitHub `token` sees
/// it, kept for a few minutes; None when the token no longer works.
pub async fn reads(
    env: &Env,
    config: &Config,
    login: &str,
    token: &str,
    repo: &str,
) -> worker::Result<Option<bool>> {
    let key = format!("reads:{}:{repo}", login.to_lowercase());
    if let Some(cached) = cache::get(env, &key).await? {
        return Ok(Some(cached == "yes"));
    }
    let reads = match github::sight(config, repo, token).await? {
        github::Sight::Reads => Some(true),
        github::Sight::Nothing => Some(false),
        // An expired token is no session; a public repository still opens.
        github::Sight::BadToken => None,
    };
    if let Some(reads) = reads {
        let value = if reads { "yes" } else { "no" };
        cache::put(env, &key, value, SIGHT_SECONDS).await?;
    }
    Ok(reads)
}

/// A work's page: the app, which draws the work from its live connection. Someone who may not
/// see the repository yet is sent to sign in first, and an unknown work is `404`.
pub async fn page(
    req: &Request,
    env: &Env,
    config: &Config,
    owner: &str,
    name: &str,
    place: &Place,
) -> Answer {
    let repo = repo_key(owner, name);
    if !open(req, env, config, &repo).await? {
        return to_sign_in(&req.path());
    }
    let db = env.d1("DB")?;
    work::find(&db, &repo, place).await?.ok_or_else(not_found)?;
    app(env, config, &Headers::new()).await
}

/// The app from the static assets, the same for every page, which draws itself from its live
/// connection, with `extra` headers over the ones every page gets.
pub async fn app(env: &Env, config: &Config, extra: &Headers) -> Answer {
    let mut index = env
        .assets("ASSETS")?
        .fetch("https://assets.local/index.html", None)
        .await?;
    if index.status_code() != 200 {
        return Err(worker::Error::RustError(format!(
            "the page's assets answered {}",
            index.status_code()
        ))
        .into());
    }
    let headers = Headers::new();
    headers.set("Content-Type", "text/html; charset=utf-8")?;
    // Who may see the work can change, so the page is asked for each time.
    headers.set("Cache-Control", "no-store")?;
    // Pictures and films are a pull request's content: they load from the files origin only.
    let files = &config.files_origin;
    headers.set(
        "Content-Security-Policy",
        &format!(
            "default-src 'self'; connect-src 'self'; img-src {files} 'self' data:; media-src {files}; frame-ancestors 'none'; base-uri 'none'; form-action 'none'"
        ),
    )?;
    headers.set("X-Frame-Options", "DENY")?;
    headers.set("Referrer-Policy", "same-origin")?;
    for (name, value) in extra.entries() {
        headers.set(&name, &value)?;
    }
    Ok(Response::ok(index.text().await?)?.with_headers(headers))
}

/// Whether a live connection asks to upgrade to a WebSocket; otherwise it gets its snapshot as
/// JSON.
pub fn upgrades(req: &Request) -> worker::Result<bool> {
    let upgrade = req.headers().get("Upgrade")?.unwrap_or_default();
    Ok(upgrade.eq_ignore_ascii_case("websocket"))
}

/// A work's live connection, at `/live` before its page's path: a WebSocket that gets the
/// snapshot and then every event, handed to the work's room; or, asked without upgrading,
/// the snapshot alone as JSON. Who may see it is the page's rule, but without a session a
/// private repository answers `401` instead of a sign in.
pub async fn live(
    req: &Request,
    env: &Env,
    config: &Config,
    owner: &str,
    name: &str,
    place: &Place,
) -> Answer {
    let repo = repo_key(owner, name);
    let session = session::read(req, env).await?;
    if !sees(env, config, session.as_ref(), &repo).await? {
        return Err(Refusal::new(401, "sign in to see this work"));
    }
    let db = env.d1("DB")?;
    let id = work::find(&db, &repo, place).await?.ok_or_else(not_found)?;
    let viewer = Viewer::Person {
        session: session.map(|s| s.id),
        repo,
    };
    connect(req, env, &id, &viewer).await
}

/// The live connection to the work `id`, once the caller decided the request may see it, held
/// for `viewer` as long as they still may.
pub async fn connect(req: &Request, env: &Env, id: &str, viewer: &Viewer) -> Answer {
    if !upgrades(req)? {
        let db = env.d1("DB")?;
        let snapshot = work::snapshot(&db, id).await?.ok_or_else(not_found)?;
        let live = fearlezz_proof::events::Live::Snapshot(snapshot);
        let headers = Headers::new();
        headers.set("Cache-Control", "no-store")?;
        return Ok(Response::from_json(&live)?.with_headers(headers));
    }
    Ok(room::connect(env, req, id, ("work", id), viewer).await?)
}

/// A grant to the files of a run, for a page that shows it: the run's folder on the files
/// origin, under a seal that lasts an hour. Who may have it is the page's rule; without a
/// session a private repository answers `401`, and a run the person may not see is `404`.
/// Through a share link (`?share=<token>`) it is given for the runs of the link's work alone,
/// and a link that does not hold is `404`.
pub async fn grant(req: &Request, env: &Env, config: &Config, run: &str) -> Answer {
    #[derive(serde::Deserialize)]
    struct Row {
        repo: String,
        work: String,
    }
    let row: Option<Row> = env
        .d1("DB")?
        .prepare("SELECT w.repo, r.work FROM runs r JOIN works w ON w.id = r.work WHERE r.id = ?1")
        .bind(&[run.into()])?
        .first(None)
        .await?;
    let row = row.ok_or_else(not_found)?;
    let url = req.url()?;
    let shared = url.query_pairs().find(|(name, _)| name == "share");
    match shared {
        Some((_, token)) => {
            let link = share::find(env, config, &token).await?;
            if link.is_none_or(|link| link.work != row.work) {
                return Err(not_found());
            }
        }
        None => {
            if !open(req, env, config, &row.repo).await? {
                return Err(Refusal::new(401, "sign in to see this work"));
            }
        }
    }
    let granted = files::grant(config, &row.repo, run).await?;
    let headers = Headers::new();
    headers.set("Cache-Control", "no-store")?;
    Ok(Response::from_json(&granted)?.with_headers(headers))
}

/// The place a page's path names after `/<owner>/<repo>/`: `work/<branch>` (a branch may hold
/// slashes, each segment escaped) or `pull/<n>`.
pub fn place(rest: &[&str]) -> Option<Place> {
    match rest {
        ["pull", number] => number.parse().ok().map(Place::Pull),
        ["work", branch @ ..] if !branch.is_empty() && branch.iter().all(|s| !s.is_empty()) => {
            let joined = branch.join("/");
            let branch = percent_encoding::percent_decode_str(&joined)
                .decode_utf8()
                .ok()?
                .into_owned();
            Some(Place::Branch(branch))
        }
        _ => None,
    }
}
