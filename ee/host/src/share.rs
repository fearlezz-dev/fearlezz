//! Share links: a person with write access makes a link anyone can open, until revoked. A link
//! names one pull request and always shows its latest whole proof, the one
//! `/<owner>/<repo>/pull/<n>` leads to, so the people it reaches follow the work as it moves.
//!
//! Opening a link skips the repository's access check for that pull request alone, and only
//! views: its shell never posts approvals or accepts, whoever holds it. The files still come
//! from the files origin under the same grant of one proof that expires within the hour; the
//! link lives here, on the app origin, and never reaches the files origin.

use fearlezz_proof::waits::Role;
use serde::Deserialize;
use serde_json::json;
use worker::{Env, Request, Response};

use crate::approve::{same_origin, sign_in_again};
use crate::github;
use crate::session::{self, Session};
use crate::{Answer, Config, Refusal, crypto, files, now, pages, repo_key};

fn not_found() -> Refusal {
    Refusal::new(404, "not found")
}

/// Makes a link to the pull request and answers it: `201 {"url", "token", "made_by", "made_at"}`.
pub async fn make(
    req: Request,
    env: &Env,
    config: &Config,
    owner: &str,
    name: &str,
    number: &str,
) -> Answer {
    same_origin(&req, config)?;
    let pull = pull(number)?;
    let person = writer(&req, env, config, owner, name, "share").await?;
    // Only a pull request with a whole proof has anything to show.
    if pages::latest(env, &repo_key(owner, name), pull)
        .await?
        .is_none()
    {
        return Err(not_found());
    }
    let token = crypto::random_id()?;
    let made_at = now();
    env.d1("DB")?
        .prepare(
            "INSERT INTO shares (token, repo, pull, made_by, made_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(&[
            token.as_str().into(),
            repo_key(owner, name).into(),
            (pull as f64).into(),
            person.login.as_str().into(),
            (made_at as f64).into(),
        ])?
        .run()
        .await?;
    let link = Link {
        token,
        made_by: person.login,
        made_at: made_at as f64,
    };
    Ok(Response::from_json(&link.json(config))?.with_status(201))
}

/// The pull request's live links, newest first: `200 {"links": [...]}`.
pub async fn list(
    req: &Request,
    env: &Env,
    config: &Config,
    owner: &str,
    name: &str,
    number: &str,
) -> Answer {
    let pull = pull(number)?;
    writer(req, env, config, owner, name, "see its share links").await?;
    let links: Vec<Link> = env
        .d1("DB")?
        .prepare(
            "SELECT token, made_by, made_at FROM shares WHERE repo = ?1 AND pull = ?2 AND revoked_at IS NULL ORDER BY made_at DESC, rowid DESC",
        )
        .bind(&[repo_key(owner, name).into(), (pull as f64).into()])?
        .all()
        .await?
        .results()?;
    let links: Vec<_> = links.iter().map(|link| link.json(config)).collect();
    Ok(Response::from_json(&json!({ "links": links }))?)
}

/// Revokes a live link of the pull request: `200 {"revoked": "<token>"}`. A link of another
/// pull request, or one already revoked, is `404`.
pub async fn revoke(
    req: Request,
    env: &Env,
    config: &Config,
    owner: &str,
    name: &str,
    number: &str,
    token: &str,
) -> Answer {
    same_origin(&req, config)?;
    let pull = pull(number)?;
    writer(&req, env, config, owner, name, "revoke its share links").await?;
    let revoked = env
        .d1("DB")?
        .prepare(
            "UPDATE shares SET revoked_at = ?1 WHERE token = ?2 AND repo = ?3 AND pull = ?4 AND revoked_at IS NULL",
        )
        .bind(&[
            (now() as f64).into(),
            token.into(),
            repo_key(owner, name).into(),
            (pull as f64).into(),
        ])?
        .run()
        .await?
        .meta()?
        .and_then(|meta| meta.changes)
        .unwrap_or(0);
    if revoked == 0 {
        return Err(not_found());
    }
    Ok(Response::from_json(&json!({ "revoked": token }))?)
}

/// `/s/<token>`: the shell of the pull request's latest whole proof, without a session. An
/// unknown or revoked token is a bare `404`, so it confirms no repository.
pub async fn view(env: &Env, config: &Config, token: &str) -> Answer {
    #[derive(Deserialize)]
    struct Shared {
        repo: String,
        pull: f64,
    }
    let shared: Option<Shared> = env
        .d1("DB")?
        .prepare("SELECT repo, pull FROM shares WHERE token = ?1 AND revoked_at IS NULL")
        .bind(&[token.into()])?
        .first(None)
        .await?;
    let shared = shared.ok_or_else(not_found)?;
    let pull = shared.pull as u64;
    let id = pages::latest(env, &shared.repo, pull)
        .await?
        .ok_or_else(not_found)?;
    let proof = pages::whole(env, &shared.repo, &id)
        .await?
        .ok_or_else(not_found)?;
    // The repository as GitHub spells it, from the proof; the host keeps it lowercased.
    let full = serde_json::from_str::<serde_json::Value>(&proof.proof)
        .ok()
        .and_then(|p| p["repository"].as_str().map(str::to_owned))
        .filter(|full| full.to_lowercase() == shared.repo)
        .unwrap_or_else(|| shared.repo.clone());
    let view = pages::View {
        title: pages::title(&full, &proof.proof),
        frame: format!(
            "{}index.html",
            files::folder(config, &shared.repo, &id).await?
        ),
        act: pages::Act::ViewOnly {
            pull: format!("/{full}/pull/{pull}"),
        },
    };
    pages::render(config, view)
}

fn pull(number: &str) -> Result<u64, Refusal> {
    number.parse().map_err(|_| not_found())
}

/// The signed in person, when GitHub gives them write access to the repository: `401` without a
/// session, `404` for a private repository they cannot read, `403` without write access, as
/// approving refuses.
async fn writer(
    req: &Request,
    env: &Env,
    config: &Config,
    owner: &str,
    name: &str,
    what: &str,
) -> Result<Session, Refusal> {
    let Some(person) = session::read(req, env).await? else {
        return Err(Refusal::new(401, format!("sign in to {what} on this page")));
    };
    if !pages::open(req, env, config, &repo_key(owner, name)).await? {
        return Err(sign_in_again());
    }
    let full = format!("{owner}/{name}");
    match github::permission(config, &full, &person.login, &person.token).await? {
        Some(role) if role >= Role::Write => Ok(person),
        Some(_) => Err(Refusal::new(
            403,
            format!(
                "@{} cannot {what} here: that needs write access to {full} on GitHub",
                person.login
            ),
        )),
        None => Err(sign_in_again()),
    }
}

#[derive(Deserialize)]
struct Link {
    token: String,
    made_by: String,
    made_at: f64,
}

impl Link {
    fn json(&self, config: &Config) -> serde_json::Value {
        json!({
            "url": format!("{}/s/{}", config.app_origin, self.token),
            "token": self.token,
            "made_by": self.made_by,
            "made_at": self.made_at as i64,
        })
    }
}
