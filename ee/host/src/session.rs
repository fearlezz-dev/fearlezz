//! GitHub sign in and the session it leaves: a row in D1 holding the person's login and GitHub
//! token, and a cookie holding only the row's random id, so the token never leaves the host and
//! logging out ends the session for good.

use serde::{Deserialize, Serialize};
use worker::{Env, Headers, Request, Response, Url};

use crate::{Answer, Config, Refusal, crypto, github, now};

const SESSION: &str = "session";
/// The sign in in flight: its state and where to go after.
const SIGNIN: &str = "signin";
/// GitHub App user tokens last eight hours; the session lasts as long.
const SESSION_SECONDS: i64 = 8 * 3600;
const SIGNIN_SECONDS: i64 = 600;

#[derive(Deserialize)]
pub struct Session {
    pub login: String,
    pub token: String,
}

#[derive(Serialize, Deserialize)]
struct Signin {
    state: String,
    next: String,
    exp: i64,
}

/// The signed in person, if the request carries the id of a session that has not expired.
pub async fn read(req: &Request, env: &Env) -> worker::Result<Option<Session>> {
    let Some(id) = cookie(req, SESSION)? else {
        return Ok(None);
    };
    env.d1("DB")?
        .prepare("SELECT login, token FROM sessions WHERE id = ?1 AND expires > ?2")
        .bind(&[id.into(), (now() as f64).into()])?
        .first(None)
        .await
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
        .append_pair("redirect_uri", &callback_url(config))
        .append_pair("state", &state);
    let signin = Signin {
        state,
        next,
        exp: now() + SIGNIN_SECONDS,
    };
    let sealed = crypto::seal(&config.signing_key, SIGNIN, &signin).await?;
    let cookie = set_cookie(SIGNIN, &sealed, SIGNIN_SECONDS);
    Ok(redirect(authorize.as_str(), &[cookie])?)
}

pub async fn callback(req: &Request, env: &Env, config: &Config) -> Answer {
    let url = req.url()?;
    let query = |name: &str| {
        url.query_pairs()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.into_owned())
    };
    let signin: Option<Signin> = match cookie(req, SIGNIN)? {
        Some(sealed) => crypto::unseal(&config.signing_key, SIGNIN, &sealed).await?,
        None => None,
    };
    let signin = signin
        .filter(|s| s.exp > now() && query("state").as_deref() == Some(s.state.as_str()))
        .ok_or_else(|| Refusal::new(400, "this sign in was not started here; sign in again"))?;
    let code = query("code").ok_or_else(|| Refusal::new(400, "GitHub sent no code"))?;
    let token = github::exchange(config, &code, &callback_url(config))
        .await?
        .ok_or_else(|| Refusal::new(400, "GitHub refused the sign in; sign in again"))?;
    let login = github::login(config, &token)
        .await?
        .ok_or_else(|| Refusal::new(400, "GitHub did not say who signed in"))?;

    let id = crypto::random_id()?;
    let db = env.d1("DB")?;
    // Sessions nobody logged out of would pile up; each sign in sweeps the expired ones.
    db.prepare("DELETE FROM sessions WHERE expires <= ?1")
        .bind(&[(now() as f64).into()])?
        .run()
        .await?;
    db.prepare("INSERT INTO sessions (id, login, token, expires) VALUES (?1, ?2, ?3, ?4)")
        .bind(&[
            id.as_str().into(),
            login.into(),
            token.into(),
            ((now() + SESSION_SECONDS) as f64).into(),
        ])?
        .run()
        .await?;
    let cookies = [
        set_cookie(SESSION, &id, SESSION_SECONDS),
        set_cookie(SIGNIN, "", 0),
    ];
    Ok(redirect(&signin.next, &cookies)?)
}

pub async fn logout(req: &Request, env: &Env) -> Answer {
    if let Some(id) = cookie(req, SESSION)? {
        env.d1("DB")?
            .prepare("DELETE FROM sessions WHERE id = ?1")
            .bind(&[id.into()])?
            .run()
            .await?;
    }
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

fn callback_url(config: &Config) -> String {
    format!("{}/callback", config.app_origin)
}

/// Only a path on this host, so sign in cannot send anyone elsewhere.
fn is_local(next: &str) -> bool {
    next.starts_with('/') && !next.starts_with("//") && !next.contains('\\')
}

fn set_cookie(name: &str, value: &str, max_age: i64) -> String {
    format!("{name}={value}; Max-Age={max_age}; Path=/; HttpOnly; Secure; SameSite=Lax")
}

/// A cookie's value, as the request sent it.
fn cookie(req: &Request, name: &str) -> worker::Result<Option<String>> {
    let header = req.headers().get("Cookie")?.unwrap_or_default();
    Ok(header
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(n, _)| *n == name)
        .map(|(_, v)| v.to_owned()))
}
