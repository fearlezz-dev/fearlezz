//! The proof host: takes the runs CI streams (OIDC authenticated, files stored by content in
//! R2, records in D1) and shows each piece of work live to the people GitHub lets read its
//! repository, who review it there: a verdict per picture with notes of what should change, an
//! admin's accept of a weakening. The CLI reads what waits with a token of its own.
//! The host is the authority for review and writes it to GitHub as the app. `ee/README.md` is
//! the protocol.
//!
//! It answers on two origins. The app origin holds sign in, the API, the work pages (the app
//! in `ee/app`, served from static assets) and their live connections, each held by the work's
//! Durable Object; the files origin serves the runs' files, which CI wrote, so nothing of them
//! ever runs where the session cookie lives.

mod api;
mod cache;
mod cli;
mod crypto;
mod files;
mod github;
mod jobs;
mod notes;
mod oidc;
mod pages;
mod review;
mod room;
mod say;
mod session;
mod webhook;
mod work;

pub use room::WorkRoom;
use worker::{Env, Request, Response, Result, event};

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: worker::Context) -> Result<Response> {
    let config = Config::from(&env)?;
    let path = req.path();
    // API callers and the live connection read JSON; people read text.
    let json = path.starts_with("/api/") || path.starts_with("/live/");
    let on_files = req.url()?.origin().ascii_serialization() == config.files_origin;
    let answer = if on_files {
        files::serve(&req, &env, &config, &path).await
    } else {
        route(req, &env, &config, &path).await
    };
    match answer {
        Ok(response) => Ok(response),
        Err(refusal) => refusal.response(json && !on_files),
    }
}

async fn route(req: Request, env: &Env, config: &Config, path: &str) -> Answer {
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let method = req.method().to_string();
    match (method.as_str(), segments.as_slice()) {
        ("GET", ["api", "v2", "installed"]) => api::installed(&req, env, config).await,
        ("POST", ["api", "v2", "runs"]) => api::open(req, env, config).await,
        ("POST", ["api", "v2", "runs", run, "checks"]) => api::step(req, env, config, run).await,
        ("POST", ["api", "v2", "runs", run, "done"]) => api::done(req, env, config, run).await,
        ("PUT", ["api", "v2", "blobs", sha256]) => api::blob(req, env, config, sha256).await,
        ("GET", ["api", "v2", "runs", run, "grant"]) => pages::grant(&req, env, config, run).await,
        ("POST", ["api", "v2", "runs", run, "verdicts"]) => {
            say::verdict(req, env, config, run).await
        }
        ("POST", ["api", "v2", "runs", run, "accept"]) => say::accept(req, env, config, run).await,
        ("POST", ["api", "v2", "runs", run, "notes"]) => notes::make(req, env, config, run).await,
        ("POST", ["api", "v2", "notes", note, "resolve"]) => {
            notes::resolve(req, env, config, note).await
        }
        ("POST", ["api", "v2", "login", "device"]) => cli::device(config).await,
        ("POST", ["api", "v2", "login", "token"]) => cli::token(req, env, config).await,
        ("DELETE", ["api", "v2", "login"]) => cli::logout(&req, env, config).await,
        ("GET", ["api", "v2", "work", owner, repo, branch @ ..]) => {
            cli::feedback(&req, env, config, owner, repo, branch).await
        }
        ("GET", ["api", "v2", "runs", run, "files", path @ ..]) if !path.is_empty() => {
            cli::file(&req, env, config, run, path).await
        }
        ("GET", ["api", "v2", "viewer", owner, repo]) => {
            say::viewer(&req, env, config, owner, repo).await
        }
        ("POST", ["api", "github", "webhook"]) => webhook::receive(req, env, config).await,
        ("GET", ["login"]) => session::login(&req, config).await,
        ("GET", ["callback"]) => session::callback(&req, env, config).await,
        ("GET", ["logout"]) => session::logout(&req, env).await,
        ("GET", ["live", owner, repo, rest @ ..]) => match pages::place(rest) {
            Some(place) => pages::live(&req, env, config, owner, repo, &place).await,
            None => Err(Refusal::new(404, "not found")),
        },
        ("GET", [owner, repo, rest @ ..]) => match pages::place(rest) {
            Some(place) => pages::page(&req, env, config, owner, repo, &place).await,
            None => Err(Refusal::new(404, "not found")),
        },
        _ => Err(Refusal::new(404, "not found")),
    }
}

/// What a request gets: a response, or a refusal with its status.
pub type Answer = std::result::Result<Response, Refusal>;

#[derive(Debug)]
pub struct Refusal {
    status: u16,
    message: String,
}

impl Refusal {
    pub fn new(status: u16, message: impl Into<String>) -> Refusal {
        Refusal {
            status,
            message: message.into(),
        }
    }

    /// API callers read JSON; people read text.
    fn response(&self, json: bool) -> Result<Response> {
        let response = if json {
            Response::from_json(&serde_json::json!({"error": self.message}))?
        } else {
            Response::ok(self.message.clone())?
        };
        Ok(response.with_status(self.status))
    }
}

impl From<worker::Error> for Refusal {
    fn from(error: worker::Error) -> Refusal {
        worker::console_error!("{error}");
        Refusal::new(500, "the proof host failed; try again")
    }
}

impl From<serde_json::Error> for Refusal {
    fn from(error: serde_json::Error) -> Refusal {
        worker::Error::from(error).into()
    }
}

/// Every setting, from wrangler vars and secrets.
pub struct Config {
    /// Where people sign in and CI streams its runs, and where the links the host hands out
    /// point.
    pub app_origin: String,
    /// Where the runs' files are served from: another site than the app origin.
    pub files_origin: String,
    /// The host's origin, the `aud` CI asks its OIDC token for.
    pub audience: String,
    pub oidc_issuer: String,
    pub oidc_jwks: String,
    pub github_api: String,
    /// Where people sign in: GitHub's web origin.
    pub github_web: String,
    /// The GitHub App's client id: its OAuth client and the issuer of its JWTs.
    pub client_id: String,
    pub install_url: String,
    pub client_secret: String,
    /// The app's private key, PEM.
    pub app_key: String,
    /// Seals the sign in cookie and the grants of the files origin.
    pub signing_key: String,
    /// What GitHub signs the app's webhooks with.
    pub webhook_secret: String,
}

impl Config {
    fn from(env: &Env) -> Result<Config> {
        let var = |name: &str| env.var(name).map(|v| v.to_string());
        let secret = |name: &str| env.secret(name).map(|v| v.to_string());
        // Origins compare as the URL parser prints them: without a trailing slash.
        let origin = |name: &str| var(name).map(|v| v.trim_end_matches('/').to_owned());
        Ok(Config {
            app_origin: origin("APP_ORIGIN")?,
            files_origin: origin("FILES_ORIGIN")?,
            audience: var("AUDIENCE")?,
            oidc_issuer: var("OIDC_ISSUER")?,
            oidc_jwks: var("OIDC_JWKS")?,
            github_api: var("GITHUB_API")?,
            github_web: var("GITHUB_WEB")?,
            client_id: var("GITHUB_CLIENT_ID")?,
            install_url: var("INSTALL_URL")?,
            client_secret: secret("GITHUB_CLIENT_SECRET")?,
            app_key: secret("GITHUB_APP_KEY")?,
            signing_key: secret("SIGNING_KEY")?,
            webhook_secret: secret("WEBHOOK_SECRET")?,
        })
    }
}

/// Seconds since the epoch, by the runtime's clock.
pub fn now() -> i64 {
    (js_sys::Date::now() / 1000.0) as i64
}

/// A repository as the host keys it: GitHub ignores case, so the host does too.
pub fn repo_key(owner: &str, name: &str) -> String {
    format!("{owner}/{name}").to_lowercase()
}
