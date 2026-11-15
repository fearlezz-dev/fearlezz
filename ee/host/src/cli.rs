//! The CLI's way in: `fearlezz setup login` runs GitHub's device flow through the host, which
//! holds the app's client id and asks GitHub, and ends in a host token for the CLI. The token
//! speaks for the person who entered the code, with their GitHub token deciding what it may
//! read: the works of repositories they may read, and the pictures of those works' runs. It
//! can never write: a verdict, a note, a resolve or an accept takes a browser session and
//! refuses any `Authorization` header (`say::person`), and the CI calls take only an OIDC token.
//!
//! The host keeps a token as its HMAC with the signing key, so the table alone opens nothing.

use fearlezz_proof::look::Look;
use fearlezz_proof::notes::{Feedback, Noted};
use fearlezz_proof::review::Review;
use serde::Deserialize;
use serde_json::json;
use worker::{Env, Headers, Request, Response};

use crate::github::{self, Device};
use crate::work::{self, NOTE_COLUMNS, NoteRow, Place, WorkRow, read};
use crate::{Answer, Config, Refusal, crypto, files, now, pages, repo_key};

/// What every CLI token starts with, so a reader tells it from CI's OIDC token at a glance.
const PREFIX: &str = "fzh_";

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
    let (github_token, seconds) = match github::device_token(config, &ask.device_code).await? {
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
        Device::Token { token, seconds } => (token, seconds),
    };
    let login = github::login(config, &github_token)
        .await?
        .ok_or_else(|| Refusal::new(400, "GitHub did not say who signed in"))?;
    let token = format!("{PREFIX}{}{}", crypto::random_id()?, crypto::random_id()?);
    let db = env.d1("DB")?;
    // Tokens nobody logged out of would pile up; each sign in sweeps the expired ones.
    db.prepare("DELETE FROM cli_tokens WHERE expires <= ?1")
        .bind(&[(now() as f64).into()])?
        .run()
        .await?;
    db.prepare("INSERT INTO cli_tokens (hash, login, token, expires) VALUES (?1, ?2, ?3, ?4)")
        .bind(&[
            hash(config, &token).await?.into(),
            login.as_str().into(),
            github_token.into(),
            ((now() + seconds) as f64).into(),
        ])?
        .run()
        .await?;
    Ok(Response::from_json(
        &json!({"token": token, "login": login}),
    )?)
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

/// The person a CLI token speaks for, and their GitHub token.
struct Reader {
    login: String,
    token: String,
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

/// The person the request's CLI token speaks for, when it may read `repo` (lowercased): `401`
/// without a token that holds, `404` for a repository they may not read.
async fn reader(req: &Request, env: &Env, config: &Config, repo: &str) -> Result<Reader, Refusal> {
    let expired = || Refusal::new(401, "sign in to the proof host: fearlezz setup login");
    let token = bearer(req)?.ok_or_else(expired)?;
    #[derive(Deserialize)]
    struct Row {
        login: String,
        token: String,
    }
    let row: Option<Row> = env
        .d1("DB")?
        .prepare("SELECT login, token FROM cli_tokens WHERE hash = ?1 AND expires > ?2")
        .bind(&[hash(config, &token).await?.into(), (now() as f64).into()])?
        .first(None)
        .await?;
    let row = row.ok_or_else(expired)?;
    let reader = Reader {
        login: row.login,
        token: row.token,
    };
    match pages::reads(env, config, &reader.login, &reader.token, repo).await? {
        Some(true) => Ok(reader),
        Some(false) => Err(Refusal::new(404, "not found")),
        None => Err(expired()),
    }
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
    reader(req, env, config, &repo).await?;
    let mut rest = vec!["work"];
    rest.extend_from_slice(branch);
    let place = pages::place(&rest).ok_or_else(|| Refusal::new(404, "not found"))?;
    let Place::Branch(branch) = place else {
        return Err(Refusal::new(404, "not found"));
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
    let row = row.ok_or_else(|| Refusal::new(404, "not found"))?;
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
    let repo: Option<String> = env
        .d1("DB")?
        .prepare("SELECT w.repo FROM runs r JOIN works w ON w.id = r.work WHERE r.id = ?1")
        .bind(&[run.into()])?
        .first(Some("repo"))
        .await?;
    // Without a token nothing is said, not even whether the run exists.
    bearer(req)?
        .ok_or_else(|| Refusal::new(401, "sign in to the proof host: fearlezz setup login"))?;
    let repo = repo.ok_or_else(|| Refusal::new(404, "not found"))?;
    reader(req, env, config, &repo).await?;
    let path = percent_encoding::percent_decode_str(&path.join("/"))
        .decode_utf8()
        .map_err(|_| Refusal::new(404, "not found"))?
        .into_owned();
    let kind = files::content_type(&path).ok_or_else(|| Refusal::new(404, "not found"))?;
    let sha: Option<String> = env
        .d1("DB")?
        .prepare("SELECT sha256 FROM files WHERE run = ?1 AND path = ?2")
        .bind(&[run.into(), path.as_str().into()])?
        .first(Some("sha256"))
        .await?;
    let sha = sha.ok_or_else(|| Refusal::new(404, "not found"))?;
    let object = env
        .bucket("BLOBS")?
        .get(format!("blobs/{repo}/{sha}"))
        .execute()
        .await?
        .ok_or_else(|| Refusal::new(404, "not found"))?;
    let body = object
        .body()
        .ok_or_else(|| Refusal::new(404, "not found"))?
        .response_body()?;
    let headers = Headers::new();
    headers.set("Content-Type", kind)?;
    headers.set("X-Content-Type-Options", "nosniff")?;
    // What CI wrote, on the origin of the sign in: never a page, only a download.
    headers.set("Content-Security-Policy", "sandbox; default-src 'none'")?;
    headers.set("Content-Disposition", "attachment")?;
    headers.set("Cache-Control", "no-store")?;
    Ok(Response::from_body(body)?.with_headers(headers))
}
