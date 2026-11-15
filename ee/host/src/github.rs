//! What the host asks GitHub and writes there: whether the app is installed on a repository,
//! whether a repository is public or a person may read it, what a person may do on it, who
//! signed in, and, as the app, its check runs and pull request comments.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde_json::{Value, json};
use worker::{Fetch, Headers, Method, Request, RequestInit, Result};

use fearlezz_proof::waits::Role;

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
    let Some(token) = installation_token(config, repo).await? else {
        return Ok(false);
    };
    let url = format!("{}/repos/{repo}", config.github_api);
    let reply = call(Method::Get, &url, Some(&token), None).await?;
    Ok(reply.status == 200 && reply.body["private"] == false)
}

/// A token of the app's installation on a repository, to ask GitHub as the app; None when the
/// app is not installed there.
pub async fn installation_token(config: &Config, repo: &str) -> Result<Option<String>> {
    let Some(installation) = installation(config, repo).await? else {
        return Ok(None);
    };
    let jwt = app_jwt(config).await?;
    let url = format!(
        "{}/app/installations/{installation}/access_tokens",
        config.github_api
    );
    let reply = call(Method::Post, &url, Some(&jwt), Some(&json!({}))).await?;
    match reply.body["token"].as_str() {
        Some(token) => Ok(Some(token.to_owned())),
        None => Err(worker::Error::RustError(format!(
            "GitHub answered {} for an installation token",
            reply.status
        ))),
    }
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

/// What a person may do on a repository, as GitHub's collaborator permission says it with
/// their own token; None when the token no longer works.
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

/// Writes as the app with an installation token: creates (POST to `path`) or updates (PATCH)
/// one of its records on `repo`, answering the record's id.
pub async fn write(
    token: &str,
    config: &Config,
    method: Method,
    path: &str,
    body: &Value,
) -> Result<u64> {
    let url = format!("{}{path}", config.github_api);
    let reply = call(method, &url, Some(token), Some(body)).await?;
    match (reply.status, reply.body["id"].as_u64()) {
        (200 | 201, Some(id)) => Ok(id),
        (status, _) => Err(worker::Error::RustError(format!(
            "GitHub answered {status} to {path}: {}",
            reply.body["message"].as_str().unwrap_or_default()
        ))),
    }
}

/// Starts GitHub's device flow for the app: the codes a person enters on GitHub, as GitHub
/// answers them (`device_code`, `user_code`, `verification_uri`, `expires_in`, `interval`); None
/// when GitHub refuses, as it does while the app's device flow is off.
pub async fn device_code(config: &Config) -> Result<Option<Value>> {
    let url = format!("{}/login/device/code", config.github_web);
    let body = json!({"client_id": config.client_id});
    let reply = call(Method::Post, &url, None, Some(&body)).await?;
    Ok((reply.status == 200 && reply.body["device_code"].is_string()).then_some(reply.body))
}

/// Where a device flow stands.
pub enum Device {
    /// The person has not entered the code yet; ask again after `interval` seconds.
    Pending { interval: u64 },
    /// They did: their token, and how long it lasts.
    Token { token: String, seconds: i64 },
    /// It ended without a token, in GitHub's words.
    Refused(String),
}

/// Asks GitHub whether the person entered the code of device flow `device_code`.
pub async fn device_token(config: &Config, device_code: &str) -> Result<Device> {
    let url = format!("{}/login/oauth/access_token", config.github_web);
    let body = json!({
        "client_id": config.client_id,
        "device_code": device_code,
        "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
    });
    let reply = call(Method::Post, &url, None, Some(&body)).await?;
    if let Some(token) = reply.body["access_token"].as_str() {
        return Ok(Device::Token {
            token: token.to_owned(),
            seconds: reply.body["expires_in"].as_i64().unwrap_or(8 * 3600),
        });
    }
    let interval = reply.body["interval"].as_u64().unwrap_or(5);
    Ok(match reply.body["error"].as_str() {
        Some("authorization_pending") => Device::Pending { interval },
        Some("slow_down") => Device::Pending { interval },
        Some(error) => Device::Refused(error.to_owned()),
        None => Device::Refused(format!("GitHub answered {}", reply.status)),
    })
}
