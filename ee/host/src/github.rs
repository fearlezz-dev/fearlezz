//! What the host asks GitHub: whether the app is installed on a repository, whether a
//! repository is public or a person may read it, and who signed in.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
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
