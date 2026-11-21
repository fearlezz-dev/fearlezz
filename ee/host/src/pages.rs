//! The pages on the app origin: a work's page, at its branch or its pull request, and its live
//! connection. The page is the app the worker serves from its static assets; it draws itself
//! from what the live connection sends (`room.rs`). Who sees them mirrors GitHub: anyone for a
//! public repository, people who may read it for a private one, and `404` for everyone else,
//! so a private repository's name is not confirmed.

use worker::{Env, Headers, Request, Response};

use crate::access::{self, Action, Viewer};
use crate::room::Held;
use crate::session::{self, to_sign_in};
use crate::work::{self, Place, RunRow};
use crate::{Answer, Config, Refusal, files, not_found, repo_key, room, share};

/// Whether a page of the repository `repo` (lowercased) is shown to the request: None when it
/// is, and otherwise what it answers instead, sending to sign in first someone who may see it
/// once signed in.
pub(crate) async fn barred(
    req: &Request,
    env: &Env,
    config: &Config,
    repo: &str,
) -> Result<Option<Answer>, Refusal> {
    let person = session::read(req, env).await?;
    let read = Action::Read { work: None };
    match access::can(env, config, Viewer::of(person.as_ref()), read, repo).await {
        Ok(()) => Ok(None),
        Err(refusal) if refusal.asks_sign_in() => Ok(Some(to_sign_in(&req.path()))),
        Err(refusal) => Err(refusal),
    }
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
    if let Some(answer) = barred(req, env, config, &repo).await? {
        return answer;
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
    // Pictures, films and texts are a pull request's content: they load from the files origin
    // only. The page reads a text from there to diff it, and fetches two pictures there to
    // compare them in its worker, which runs the comparison as wasm.
    let files = &config.files_origin;
    headers.set(
        "Content-Security-Policy",
        &format!(
            "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'self' {files}; img-src {files} 'self' data:; media-src {files}; frame-ancestors 'none'; base-uri 'none'; form-action 'none'"
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
    let person = session::read(req, env).await?;
    let viewer = Viewer::of(person.as_ref());
    access::can(env, config, viewer, Action::Read { work: None }, &repo).await?;
    let db = env.d1("DB")?;
    let id = work::find(&db, &repo, place).await?.ok_or_else(not_found)?;
    connect(req, env, &id, &Held::of(viewer, &repo)).await
}

/// The live connection to the work `id`, once the caller decided the request may see it, held
/// as `held` says, as long as its viewer still may.
pub(crate) async fn connect(req: &Request, env: &Env, id: &str, held: &Held) -> Answer {
    if !upgrades(req)? {
        let db = env.d1("DB")?;
        let snapshot = work::snapshot(&db, id).await?.ok_or_else(not_found)?;
        let live = fearlezz_proof::events::Live::Snapshot(snapshot);
        let headers = Headers::new();
        headers.set("Cache-Control", "no-store")?;
        return Ok(Response::from_json(&live)?.with_headers(headers));
    }
    Ok(room::connect(env, req, id, ("work", id), held).await?)
}

/// A grant to the files of a run, for a page that shows it: the run's folder on the files
/// origin, under a seal that lasts an hour. Who may have it is the page's rule; without a
/// session a private repository answers `401`, and a run the person may not see is `404`.
/// Through a share link (`?share=<token>`) it is given for the runs of the link's work alone,
/// and a link that does not hold is `404`.
pub async fn grant(req: &Request, env: &Env, config: &Config, run: &str) -> Answer {
    let row = RunRow::find(&env.d1("DB")?, run)
        .await?
        .ok_or_else(not_found)?;
    let url = req.url()?;
    let read = Action::Read {
        work: Some(&row.work),
    };
    match url.query_pairs().find(|(name, _)| name == "share") {
        Some((_, token)) => {
            let link = share::find(env, config, &token)
                .await?
                .ok_or_else(not_found)?;
            access::can(env, config, Viewer::Share(&link), read, &row.repo).await?;
        }
        None => {
            let person = session::read(req, env).await?;
            access::can(env, config, Viewer::of(person.as_ref()), read, &row.repo).await?;
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
