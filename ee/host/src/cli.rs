//! The CLI's way in: `fearlezz setup login` runs GitHub's device flow through the host, which
//! holds the app's client id and asks GitHub, and ends in a host token for the CLI. The token
//! speaks for the person who entered the code, with their GitHub token deciding what it may
//! do: read the works of repositories they may read and the pictures of those works' runs, and
//! open and stream local runs of repositories they may write (`api.rs`). It can never give a
//! verdict, a note, a resolve or an accept: those take a browser session and refuse any
//! `Authorization` header (`access::speaker`).
//!
//! The token is the host's own: it lives 30 days from its last use, until `fearlezz setup
//! logout` ends it, while the host keeps the person's GitHub token fresh with its refresh token,
//! so the person signs in once. The host keeps a token as its HMAC with the signing key, so the
//! table alone opens nothing.

use fearlezz_proof::look::Look;
use fearlezz_proof::notes::{Feedback, Noted};
use fearlezz_proof::review::Review;
use serde::Deserialize;
use serde_json::json;
use wasm_bindgen::JsValue;
use worker::{Env, Headers, Request, Response};

use crate::access::{self, Action, Person, Via, Viewer};
use crate::github::{self, Device, UserToken};
use crate::work::{self, NOTE_COLUMNS, NoteRow, Place, RunRow, WorkRow, read};
use crate::{Answer, Config, Refusal, crypto, files, not_found, now, pages, repo_key};

/// What every CLI token starts with, so a reader tells it from CI's OIDC token at a glance.
const PREFIX: &str = "fzh_";

/// What the CLI is told when it has no token that holds.
pub(crate) const SIGN_IN: &str = "sign in to the proof host: fearlezz setup login";

/// How long a CLI token lives after its last use.
const IDLE_SECONDS: i64 = 30 * 86400;

/// How long before its GitHub token expires the host renews it, so a call never meets it
/// expired.
const RENEW_SECONDS: i64 = 300;

/// `POST /api/v2/login/device`: starts a device flow, answering what GitHub said: the code the
/// person enters (`user_code`) at `verification_uri`, and the `device_code` the CLI polls with.
pub async fn device(config: &Config) -> Answer {
    let codes = github::device_code(config).await?.ok_or_else(|| {
        Refusal::new(
            502,
            "GitHub refused to start a device sign in; the app's device flow may be off",
        )
    })?;
    let answer = json!({
        "device_code": codes["device_code"],
        "user_code": codes["user_code"],
        "verification_uri": codes["verification_uri"],
        "expires_in": codes["expires_in"],
        "interval": codes["interval"],
    });
    Ok(Response::from_json(&answer)?)
}

/// `POST /api/v2/login/token`: body `{"device_code"}`. While the person has not entered the code,
/// `{"pending": true, "interval"}`; once they did, `{"token", "login"}`; `400` when the flow
/// ended without them.
pub async fn token(mut req: Request, env: &Env, config: &Config) -> Answer {
    #[derive(Deserialize)]
    struct Ask {
        device_code: String,
    }
    let ask: Ask = req
        .json()
        .await
        .map_err(|_| Refusal::new(400, "send {\"device_code\"}"))?;
    let github_token = match github::device_token(config, &ask.device_code).await? {
        Device::Pending { interval } => {
            return Ok(Response::from_json(
                &json!({"pending": true, "interval": interval}),
            )?);
        }
        Device::Refused(why) => {
            return Err(Refusal::new(
                400,
                format!(
                    "the sign in ended without a token ({why}); run fearlezz setup login again"
                ),
            ));
        }
        Device::Token(token) => token,
    };
    let login = github::login(config, &github_token.token)
        .await?
        .ok_or_else(|| Refusal::new(400, "GitHub did not say who signed in"))?;
    let token = format!("{PREFIX}{}{}", crypto::random_id()?, crypto::random_id()?);
    let db = env.d1("DB")?;
    // Tokens nobody logged out of would pile up; each sign in sweeps the idle ones.
    db.prepare("DELETE FROM cli_tokens WHERE used <= ?1")
        .bind(&[((now() - IDLE_SECONDS) as f64).into()])?
        .run()
        .await?;
    db.prepare(
        "INSERT INTO cli_tokens (hash, login, token, token_expires, refresh, used) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
    )
    .bind(&[
        hash(config, &token).await?.into(),
        login.as_str().into(),
        github_token.token.as_str().into(),
        expiry(&github_token),
        github_token
            .refresh
            .as_deref()
            .map_or(JsValue::NULL, JsValue::from),
        (now() as f64).into(),
    ])?
    .run()
    .await?;
    Ok(Response::from_json(
        &json!({"token": token, "login": login}),
    )?)
}

/// When a GitHub user token expires, as D1 keeps it; null when it does not.
fn expiry(token: &UserToken) -> JsValue {
    token
        .seconds
        .map_or(JsValue::NULL, |s| ((now() + s) as f64).into())
}

/// `DELETE /api/v2/login`: the CLI logs out, which ends its token on the host. `204`.
pub async fn logout(req: &Request, env: &Env, config: &Config) -> Answer {
    if let Some(token) = bearer(req)? {
        env.d1("DB")?
            .prepare("DELETE FROM cli_tokens WHERE hash = ?1")
            .bind(&[hash(config, &token).await?.into()])?
            .run()
            .await?;
    }
    Ok(Response::empty()?.with_status(204))
}

/// Whether the request carries a CLI token (`fzh_…`), whether or not it holds.
pub fn carries(req: &Request) -> worker::Result<bool> {
    Ok(bearer(req)?.is_some())
}

fn bearer(req: &Request) -> worker::Result<Option<String>> {
    Ok(req
        .headers()
        .get("Authorization")?
        .and_then(|a| a.strip_prefix("Bearer ").map(str::to_owned))
        .filter(|t| t.starts_with(PREFIX)))
}

async fn hash(config: &Config, token: &str) -> worker::Result<String> {
    let mac = crypto::hmac_sign(&config.signing_key, format!("cli.{token}").as_bytes()).await?;
    Ok(hex::encode(mac))
}

/// The person the request's CLI token speaks for: `401` without a token used in the last 30
/// days, or when their GitHub token can no longer be renewed. Each use keeps the token alive
/// for 30 more days, and a GitHub token about to expire is renewed first.
pub async fn person(req: &Request, env: &Env, config: &Config) -> Result<Person, Refusal> {
    let expired = || Refusal::new(401, SIGN_IN);
    let token = bearer(req)?.ok_or_else(expired)?;
    #[derive(Deserialize)]
    struct Row {
        login: String,
        token: String,
        token_expires: Option<f64>,
        refresh: Option<String>,
        used: f64,
    }
    let db = env.d1("DB")?;
    let hash = hash(config, &token).await?;
    let now = now();
    let row: Option<Row> = db
        .prepare("SELECT login, token, token_expires, refresh, used FROM cli_tokens WHERE hash = ?1 AND used > ?2")
        .bind(&[hash.as_str().into(), ((now - IDLE_SECONDS) as f64).into()])?
        .first(None)
        .await?;
    let row = row.ok_or_else(expired)?;
    let mut person = Person {
        login: row.login,
        token: row.token,
        via: Via::Cli,
    };
    let stale = row
        .token_expires
        .is_some_and(|at| at as i64 <= now + RENEW_SECONDS);
    if stale {
        let renewed = match &row.refresh {
            Some(refresh) => github::refresh(config, refresh).await?,
            None => None,
        };
        let Some(renewed) = renewed else {
            db.prepare("DELETE FROM cli_tokens WHERE hash = ?1")
                .bind(&[hash.as_str().into()])?
                .run()
                .await?;
            return Err(expired());
        };
        db.prepare("UPDATE cli_tokens SET token = ?2, token_expires = ?3, refresh = COALESCE(?4, refresh) WHERE hash = ?1")
            .bind(&[
                hash.as_str().into(),
                renewed.token.as_str().into(),
                expiry(&renewed),
                renewed.refresh.as_deref().map_or(JsValue::NULL, JsValue::from),
            ])?
            .run()
            .await?;
        person.token = renewed.token;
    }
    // Written at most once an hour, not on every call.
    if (row.used as i64) < now - 3600 {
        db.prepare("UPDATE cli_tokens SET used = ?2 WHERE hash = ?1")
            .bind(&[hash.as_str().into(), (now as f64).into()])?
            .run()
            .await?;
    }
    Ok(person)
}

/// Refuses the request unless its CLI token speaks for a person who may read `repo`
/// (lowercased): `401` without a token that holds, `404` for a repository they may not read.
async fn reads(req: &Request, env: &Env, config: &Config, repo: &str) -> Result<(), Refusal> {
    let reader = person(req, env, config).await?;
    let read = Action::Read { work: None };
    access::can(env, config, Viewer::Person(&reader), read, repo).await
}

/// `GET /api/v2/work/<owner>/<repo>/<branch>`: what waits on the host for a work, as the CLI
/// shows it (`notes::Feedback`): its page, the review of its head, and its open notes with
/// the paths of their pictures in the runs they were made on.
pub async fn feedback(
    req: &Request,
    env: &Env,
    config: &Config,
    owner: &str,
    name: &str,
    branch: &[&str],
) -> Answer {
    let repo = repo_key(owner, name);
    reads(req, env, config, &repo).await?;
    let mut rest = vec!["work"];
    rest.extend_from_slice(branch);
    let place = pages::place(&rest).ok_or_else(not_found)?;
    let Place::Branch(branch) = place else {
        return Err(not_found());
    };
    let db = env.d1("DB")?;
    #[derive(Deserialize)]
    struct Row {
        id: String,
        repository: String,
        branch: String,
        pull: Option<f64>,
        pull_title: Option<String>,
        pull_url: Option<String>,
        review: Option<String>,
    }
    let row: Option<Row> = db
        .prepare("SELECT id, repository, branch, pull, pull_title, pull_url, review FROM works WHERE repo = ?1 AND branch = ?2")
        .bind(&[repo.as_str().into(), branch.as_str().into()])?
        .first(None)
        .await?;
    let row = row.ok_or_else(not_found)?;
    let stored = WorkRow {
        id: row.id,
        repository: row.repository,
        branch: row.branch,
        pull: row.pull,
        pull_title: row.pull_title,
        pull_url: row.pull_url,
    };
    let rows = db
        .prepare(format!(
            "SELECT {NOTE_COLUMNS} FROM notes WHERE work = ?1 AND state = 'open' ORDER BY at, rowid"
        ))
        .bind(&[stored.id.as_str().into()])?
        .all()
        .await?
        .results::<NoteRow>()?;
    let mut notes = Vec::new();
    for note in rows.into_iter().map(NoteRow::note) {
        let look: Option<String> = db
            .prepare("SELECT look FROM checks WHERE run = ?1 AND id = ?2")
            .bind(&[note.run.as_str().into(), note.check.as_str().into()])?
            .first(Some("look"))
            .await?;
        let look = look.map(|l| read::<Look>(&l)).transpose()?;
        let item = look
            .as_ref()
            .and_then(|l| l.items.iter().find(|i| i.name == note.picture));
        notes.push(Noted {
            kind: item.map_or_else(|| "still".to_owned(), |i| i.kind.clone()),
            before: item.and_then(|i| i.before.clone()),
            after: item.and_then(|i| i.after.clone()),
            note,
        });
    }
    let work = stored.work();
    let answer = Feedback {
        url: work::page_url(
            config,
            &work.repository,
            &work.branch,
            work.pull.as_ref().map(|p| p.number),
        ),
        work,
        review: row.review.as_deref().map(read::<Review>).transpose()?,
        notes,
    };
    let headers = Headers::new();
    headers.set("Cache-Control", "no-store")?;
    Ok(Response::from_json(&answer)?.with_headers(headers))
}

/// `GET /api/v2/runs/<run>/files/<path>`: one picture or film of a run, for the CLI to draw a
/// note's region on. Only the types the files origin serves; anything else is `404`.
pub async fn file(req: &Request, env: &Env, config: &Config, run: &str, path: &[&str]) -> Answer {
    let row = RunRow::find(&env.d1("DB")?, run).await?;
    // Without a token nothing is said, not even whether the run exists.
    bearer(req)?.ok_or_else(|| Refusal::new(401, SIGN_IN))?;
    let row = row.ok_or_else(not_found)?;
    reads(req, env, config, &row.repo).await?;
    let stored = files::stored(env, &row.repo, run, &path.join("/")).await?;
    let body = files::body(env, &stored.key, None).await?;
    let headers = Headers::new();
    headers.set("Content-Type", stored.kind)?;
    headers.set("X-Content-Type-Options", "nosniff")?;
    // What CI wrote, on the origin of the sign in: never a page, only a download.
    headers.set("Content-Security-Policy", "sandbox; default-src 'none'")?;
    headers.set("Content-Disposition", "attachment")?;
    headers.set("Cache-Control", "no-store")?;
    Ok(Response::from_body(body)?.with_headers(headers))
}
