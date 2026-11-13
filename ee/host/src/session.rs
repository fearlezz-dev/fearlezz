//! GitHub sign in and the session it leaves: a cookie holding the person's login and GitHub
//! token, signed with HMAC so the host can trust it without storing sessions.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use worker::{Headers, Request, Response, Url};

use crate::{Answer, Config, Refusal, crypto, github, now, origin};

const SESSION: &str = "session";
/// The sign in in flight: its state and where to go after.
const SIGNIN: &str = "signin";
/// GitHub App user tokens last eight hours; the session lasts as long.
const SESSION_SECONDS: i64 = 8 * 3600;
const SIGNIN_SECONDS: i64 = 600;

#[derive(Serialize, Deserialize)]
pub struct Session {
    pub login: String,
    pub token: String,
    exp: i64,
}

#[derive(Serialize, Deserialize)]
struct Signin {
    state: String,
    next: String,
    exp: i64,
}

/// The signed in person, if the request carries a valid session.
pub async fn read(req: &Request, config: &Config) -> worker::Result<Option<Session>> {
    let session: Option<Session> = cookie(req, SESSION, config).await?;
    Ok(session.filter(|s| s.exp > now()))
}

pub async fn login(req: &Request, config: &Config) -> Answer {
    let url = req.url()?;
    let next = url
        .query_pairs()
        .find(|(name, _)| name == "next")
        .map(|(_, value)| value.into_owned())
        .filter(|next| is_local(next))
        .unwrap_or_else(|| "/".to_owned());
    let state = crypto::random_id()?;
    let mut authorize = Url::parse(&format!("{}/login/oauth/authorize", config.github_web))
        .map_err(worker::Error::from)?;
    authorize
        .query_pairs_mut()
        .append_pair("client_id", &config.client_id)
        .append_pair("redirect_uri", &format!("{}/callback", origin(req)?))
        .append_pair("state", &state);
    let signin = Signin {
        state,
        next,
        exp: now() + SIGNIN_SECONDS,
    };
    let cookie = set_cookie(SIGNIN, &seal(&signin, config).await?, SIGNIN_SECONDS);
    Ok(redirect(authorize.as_str(), &[cookie])?)
}

pub async fn callback(req: &Request, config: &Config) -> Answer {
    let url = req.url()?;
    let query = |name: &str| {
        url.query_pairs()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.into_owned())
    };
    let signin: Option<Signin> = cookie(req, SIGNIN, config).await?;
    let signin = signin
        .filter(|s| s.exp > now() && query("state").as_deref() == Some(s.state.as_str()))
        .ok_or_else(|| Refusal::new(400, "this sign in was not started here; sign in again"))?;
    let code = query("code").ok_or_else(|| Refusal::new(400, "GitHub sent no code"))?;
    let redirect_uri = format!("{}/callback", origin(req)?);
    let token = github::exchange(config, &code, &redirect_uri)
        .await?
        .ok_or_else(|| Refusal::new(400, "GitHub refused the sign in; sign in again"))?;
    let login = github::login(config, &token)
        .await?
        .ok_or_else(|| Refusal::new(400, "GitHub did not say who signed in"))?;
    let session = Session {
        login,
        token,
        exp: now() + SESSION_SECONDS,
    };
    let cookies = [
        set_cookie(SESSION, &seal(&session, config).await?, SESSION_SECONDS),
        set_cookie(SIGNIN, "", 0),
    ];
    Ok(redirect(&signin.next, &cookies)?)
}

pub fn logout() -> Answer {
    Ok(redirect("/", &[set_cookie(SESSION, "", 0)])?)
}

/// Sends someone to sign in, and back to where they were.
pub fn to_sign_in(path: &str) -> Answer {
    let next: String = url::form_urlencoded::byte_serialize(path.as_bytes()).collect();
    Ok(redirect(&format!("/login?next={next}"), &[])?)
}

/// A 302 built by hand: `Response::redirect` has immutable headers, so it cannot carry cookies.
pub fn redirect(location: &str, cookies: &[String]) -> worker::Result<Response> {
    let headers = Headers::new();
    headers.set("Location", location)?;
    for cookie in cookies {
        headers.append("Set-Cookie", cookie)?;
    }
    Ok(Response::empty()?.with_status(302).with_headers(headers))
}

/// Only a path on this host, so sign in cannot send anyone elsewhere.
fn is_local(next: &str) -> bool {
    next.starts_with('/') && !next.starts_with("//") && !next.contains('\\')
}

fn set_cookie(name: &str, value: &str, max_age: i64) -> String {
    format!("{name}={value}; Max-Age={max_age}; Path=/; HttpOnly; Secure; SameSite=Lax")
}

/// `base64url(json).base64url(hmac)`.
async fn seal<T: Serialize>(value: &T, config: &Config) -> worker::Result<String> {
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(value)?);
    let mac = crypto::hmac_sign(&config.session_key, payload.as_bytes()).await?;
    Ok(format!("{payload}.{}", URL_SAFE_NO_PAD.encode(mac)))
}

/// A sealed cookie's value, if it is there and its seal holds.
async fn cookie<T: DeserializeOwned>(
    req: &Request,
    name: &str,
    config: &Config,
) -> worker::Result<Option<T>> {
    let header = req.headers().get("Cookie")?.unwrap_or_default();
    let Some(sealed) = header
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(n, _)| *n == name)
        .map(|(_, v)| v)
    else {
        return Ok(None);
    };
    let Some((payload, mac)) = sealed.split_once('.') else {
        return Ok(None);
    };
    let Ok(mac) = URL_SAFE_NO_PAD.decode(mac) else {
        return Ok(None);
    };
    if !crypto::hmac_verify(&config.session_key, payload.as_bytes(), &mac).await? {
        return Ok(None);
    }
    let Ok(json) = URL_SAFE_NO_PAD.decode(payload) else {
        return Ok(None);
    };
    Ok(serde_json::from_slice(&json).ok())
}
