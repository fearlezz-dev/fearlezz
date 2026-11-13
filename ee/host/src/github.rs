//! What the host asks GitHub: whether the app is installed on a repository, whether a
//! repository is public or a person may read it, who signed in, and, for approving on a page,
//! a person's permission, a pull request's head, and posting a comment as that person.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use fearlezz_proof::waits::Role;
use serde::Deserialize;
use serde_json::{Value, json};
use worker::{Fetch, Headers, Method, Request, RequestInit, Result};

use crate::{Config, crypto, now};

/// GitHub's answer: its status and JSON body.
struct Reply {
    status: u16,
    body: Value,
}

async fn call(
    method: Method,
    url: &str,
    auth: Option<&str>,
    body: Option<&Value>,
) -> Result<Reply> {
    let headers = Headers::new();
    headers.set("Accept", "application/json")?;
    headers.set("User-Agent", "fearlezz-host")?;
    headers.set("X-GitHub-Api-Version", "2022-11-28")?;
    if let Some(auth) = auth {
        headers.set("Authorization", &format!("Bearer {auth}"))?;
    }
    let mut init = RequestInit::new();
    init.with_method(method).with_headers(headers);
    if let Some(body) = body {
        init.headers.set("Content-Type", "application/json")?;
        init.with_body(Some(body.to_string().into()));
    }
    let mut response = Fetch::Request(Request::new_with_init(url, &init)?)
        .send()
        .await?;
    let status = response.status_code();
    let body = response.json().await.unwrap_or(Value::Null);
    Ok(Reply { status, body })
}

/// A JWT the app signs to speak as itself: short lived, backdated a minute for clock drift, as
/// GitHub asks.
async fn app_jwt(config: &Config) -> Result<String> {
    let header = json!({"alg": "RS256", "typ": "JWT"});
    let now = now();
    let claims = json!({"iat": now - 60, "exp": now + 540, "iss": config.client_id});
    let signed = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    let key = crypto::rsa_private_key(&config.app_key).await?;
    let signature = crypto::rs256_sign(&key, signed.as_bytes()).await?;
    Ok(format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature)))
}

/// The app's installation on a repository, if it is installed there.
pub async fn installation(config: &Config, repo: &str) -> Result<Option<u64>> {
    let jwt = app_jwt(config).await?;
    let url = format!("{}/repos/{repo}/installation", config.github_api);
    let reply = call(Method::Get, &url, Some(&jwt), None).await?;
    match reply.status {
        200 => Ok(reply.body["id"].as_u64()),
        404 => Ok(None),
        status => Err(worker::Error::RustError(format!(
            "GitHub answered {status} for the installation on {repo}"
        ))),
    }
}

/// Whether anyone may read a repository. Only the app can tell for someone without a session;
/// a repository it is not installed on counts as private.
pub async fn public(config: &Config, repo: &str) -> Result<bool> {
    let Some(installation) = installation(config, repo).await? else {
        return Ok(false);
    };
    let jwt = app_jwt(config).await?;
    let url = format!(
        "{}/app/installations/{installation}/access_tokens",
        config.github_api
    );
    let reply = call(Method::Post, &url, Some(&jwt), Some(&json!({}))).await?;
    let Some(token) = reply.body["token"].as_str() else {
        return Err(worker::Error::RustError(format!(
            "GitHub answered {} for an installation token",
            reply.status
        )));
    };
    let url = format!("{}/repos/{repo}", config.github_api);
    let reply = call(Method::Get, &url, Some(token), None).await?;
    Ok(reply.status == 200 && reply.body["private"] == false)
}

/// What a person's token sees of a repository.
pub enum Sight {
    /// Public, or one they may read.
    Reads,
    /// Private and not theirs, or not there: GitHub does not tell these apart.
    Nothing,
    /// The token no longer works.
    BadToken,
}

pub async fn sight(config: &Config, repo: &str, token: &str) -> Result<Sight> {
    #[derive(Deserialize)]
    struct Repo {
        private: bool,
        #[serde(default)]
        permissions: Permissions,
    }
    #[derive(Default, Deserialize)]
    struct Permissions {
        #[serde(default)]
        pull: bool,
    }
    let url = format!("{}/repos/{repo}", config.github_api);
    let reply = call(Method::Get, &url, Some(token), None).await?;
    Ok(match reply.status {
        200 => match serde_json::from_value::<Repo>(reply.body) {
            Ok(repo) if !repo.private || repo.permissions.pull => Sight::Reads,
            _ => Sight::Nothing,
        },
        401 => Sight::BadToken,
        _ => Sight::Nothing,
    })
}

/// Trades an OAuth code for the person's token.
pub async fn exchange(config: &Config, code: &str, redirect_uri: &str) -> Result<Option<String>> {
    let url = format!("{}/login/oauth/access_token", config.github_web);
    let body = json!({
        "client_id": config.client_id,
        "client_secret": config.client_secret,
        "code": code,
        "redirect_uri": redirect_uri,
    });
    let reply = call(Method::Post, &url, None, Some(&body)).await?;
    Ok(reply.body["access_token"].as_str().map(str::to_owned))
}

/// The login a person's token belongs to.
pub async fn login(config: &Config, token: &str) -> Result<Option<String>> {
    let url = format!("{}/user", config.github_api);
    let reply = call(Method::Get, &url, Some(token), None).await?;
    Ok(reply.body["login"].as_str().map(str::to_owned))
}

/// The role of `login` on `repo`, asked with their own token: the legacy role GitHub gives
/// (`admin`, `write`, `read`, `none`), the one CI counts. None when the token no longer works.
pub async fn permission(
    config: &Config,
    repo: &str,
    login: &str,
    token: &str,
) -> Result<Option<Role>> {
    let url = format!(
        "{}/repos/{repo}/collaborators/{login}/permission",
        config.github_api
    );
    let reply = call(Method::Get, &url, Some(token), None).await?;
    Ok(match (reply.status, reply.body["permission"].as_str()) {
        (401, _) => None,
        (200, Some(permission)) => Some(Role::of(permission)),
        _ => Some(Role::Less),
    })
}

/// A pull request's head: its commit and the repository it comes from.
pub struct Head {
    pub sha: String,
    pub from: String,
}

/// The head of pull request `number`, or None when the token no longer works.
pub async fn head(config: &Config, repo: &str, number: u64, token: &str) -> Result<Option<Head>> {
    let url = format!("{}/repos/{repo}/pulls/{number}", config.github_api);
    let reply = call(Method::Get, &url, Some(token), None).await?;
    match reply.status {
        200 => Ok(Some(Head {
            sha: reply.body["head"]["sha"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            from: reply.body["head"]["repo"]["full_name"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        })),
        401 => Ok(None),
        status => Err(worker::Error::RustError(format!(
            "GitHub answered {status} for pull request {number} of {repo}"
        ))),
    }
}

/// What came of posting a comment.
pub enum Posted {
    /// Its page.
    Url(String),
    BadToken,
    /// GitHub's status, such as 403 when the app lacks the permission to comment.
    Refused(u16),
}

/// Posts `body` on pull request `number` as the person whose token it is.
pub async fn comment(
    config: &Config,
    repo: &str,
    number: u64,
    token: &str,
    body: &str,
) -> Result<Posted> {
    let url = format!(
        "{}/repos/{repo}/issues/{number}/comments",
        config.github_api
    );
    let reply = call(
        Method::Post,
        &url,
        Some(token),
        Some(&json!({"body": body})),
    )
    .await?;
    Ok(match (reply.status, reply.body["html_url"].as_str()) {
        (201, Some(url)) => Posted::Url(url.to_owned()),
        (401, _) => Posted::BadToken,
        (status, _) => Posted::Refused(status),
    })
}
