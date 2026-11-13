//! The proof host: takes the proof bundles CI sends (OIDC authenticated, files stored by
//! content in R2, records in D1) and shows each proof's page to the people GitHub lets read its
//! repository. `ee/README.md` is the protocol.
//!
//! It answers on two origins. The app origin holds sign in, the API and a shell page per proof;
//! the files origin serves the bundles' files, which CI wrote, so a proof's page never runs
//! where the session cookie lives.

mod api;
mod approve;
mod cache;
mod crypto;
mod files;
mod github;
mod oidc;
mod pages;
mod session;
mod share;

use worker::{Env, Request, Response, Result, event};

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: worker::Context) -> Result<Response> {
    let config = Config::from(&env)?;
    let path = req.path();
    // API callers and the shell's posts and asks read JSON; people read text.
    let json = path.starts_with("/api/")
        || req.method() == worker::Method::Post
        || path.ends_with("/share");
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
        ("GET", ["api", "v1", "installed"]) => api::installed(&req, env, config).await,
        ("POST", ["api", "v1", "proofs"]) => api::create(req, env, config).await,
        ("PUT", ["api", "v1", "blobs", sha256]) => api::blob(req, env, config, sha256).await,
        ("POST", ["api", "v1", "proofs", id, "done"]) => api::done(&req, env, config, id).await,
        ("GET", ["login"]) => session::login(&req, config).await,
        ("GET", ["callback"]) => session::callback(&req, env, config).await,
        ("GET", ["logout"]) => session::logout(&req, env).await,
        ("POST", [owner, repo, "proofs", id, action]) => match approve::Kind::from_path(action) {
            Some(kind) => approve::act(req, env, config, owner, repo, id, kind).await,
            None => Err(Refusal::new(404, "not found")),
        },
        ("POST", [owner, repo, "pull", number, "share"]) => {
            share::make(req, env, config, owner, repo, number).await
        }
        ("GET", [owner, repo, "pull", number, "share"]) => {
            share::list(&req, env, config, owner, repo, number).await
        }
        ("POST", [owner, repo, "pull", number, "share", token, "revoke"]) => {
            share::revoke(req, env, config, owner, repo, number, token).await
        }
        ("GET", ["s", token]) => share::view(env, config, token).await,
        ("GET", [owner, repo, "pull", number]) => {
            pages::pull(&req, env, config, owner, repo, number).await
        }
        ("GET", [owner, repo, "proofs", id, rest @ ..]) => {
            pages::shell(&req, env, config, owner, repo, id, rest).await
        }
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

/// Every setting, from wrangler vars and secrets.
pub struct Config {
    /// Where people sign in and CI uploads, and where the links the host hands out point.
    pub app_origin: String,
    /// Where the bundles' files are served from: another site than the app origin.
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
