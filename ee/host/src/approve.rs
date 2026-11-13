//! Approving pictures and accepting a weakened proof from a proof's page. The host never
//! approves anything itself: it posts `/fearlezz approve <words>` or `/fearlezz accept <words>`
//! on the pull request as the signed in person, with their own GitHub token, and CI judges that
//! comment as it judges any other. The host only refuses early what CI would not count, and
//! says why.
//!
//! The page that asks runs on the files origin, which never gets the sign in; its shell on the
//! app origin passes the ask on, so these endpoints take only same origin JSON.

use serde::Deserialize;
use serde_json::{Value, json};
use worker::{Env, Request, Response};

use crate::github::{self, Permission};
use crate::{Answer, Config, Refusal, pages, repo_key, session};

/// Longer words than this are no approval but an essay; the pull request has room for that.
const MAX_WORDS: usize = 500;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Approve,
    Accept,
}

impl Kind {
    pub fn from_path(segment: &str) -> Option<Kind> {
        match segment {
            "approve" => Some(Kind::Approve),
            "accept" => Some(Kind::Accept),
            _ => None,
        }
    }

    fn word(self) -> &'static str {
        match self {
            Kind::Approve => "approve",
            Kind::Accept => "accept",
        }
    }
}

#[derive(Deserialize)]
struct Ask {
    /// The check whose pictures are approved; an accept covers the whole weakened proof.
    #[serde(default)]
    check: Option<String>,
    words: String,
}

pub async fn act(
    mut req: Request,
    env: &Env,
    config: &Config,
    owner: &str,
    name: &str,
    id: &str,
    kind: Kind,
) -> Answer {
    same_origin(&req, config)?;
    let Some(person) = session::read(&req, env).await? else {
        return Err(Refusal::new(
            401,
            format!("sign in to {} on this page", kind.word()),
        ));
    };
    let ask: Ask = req
        .json()
        .await
        .map_err(|_| Refusal::new(400, "send {\"check\": <check>, \"words\": <your words>}"))?;
    let words = words(&ask.words, kind)?;

    let repo = repo_key(owner, name);
    // Someone GitHub shows nothing of a private repository learns nothing of it here either.
    if !pages::open(&req, env, config, &repo).await? {
        return Err(sign_in_again());
    }
    let row = pages::whole(env, &repo, id)
        .await?
        .ok_or_else(|| Refusal::new(404, "not found"))?;
    let proof: Value = serde_json::from_str(&row.proof)
        .map_err(|e| worker::Error::RustError(format!("a stored proof does not read back: {e}")))?;
    let Some(pull) = row.pull.map(|n| n as u64) else {
        return Err(Refusal::new(
            409,
            "this proof is of no pull request, so there is nothing to comment on",
        ));
    };
    let latest = pages::latest(env, &repo, pull).await?;
    if latest.as_deref() != Some(id) {
        let newer = latest
            .map(|l| format!(": open {}/{owner}/{name}/proofs/{l}/", config.app_origin))
            .unwrap_or_default();
        return Err(Refusal::new(
            409,
            format!(
                "a newer proof of pull request {pull} exists{newer}, and {} what it shows",
                kind.word()
            ),
        ));
    }
    waits(&proof, kind, ask.check.as_deref())?;

    let full = format!("{owner}/{name}");
    let needs = match kind {
        Kind::Approve => "write access to",
        Kind::Accept => "admin rights on",
    };
    let allowed = match github::permission(config, &full, &person.login, &person.token).await? {
        Permission::Admin => true,
        Permission::Write => kind == Kind::Approve,
        Permission::Less => false,
        Permission::BadToken => return Err(sign_in_again()),
    };
    if !allowed {
        return Err(Refusal::new(
            403,
            format!(
                "@{} cannot {} here: that needs {needs} {full} on GitHub",
                person.login,
                kind.word()
            ),
        ));
    }

    let head = proof["head"].as_str().unwrap_or_default();
    let Some(now) = github::head(config, &full, pull, &person.token).await? else {
        return Err(sign_in_again());
    };
    if now.sha != head {
        return Err(Refusal::new(
            409,
            format!(
                "pull request {pull} is at {} now, newer than this proof of {}: wait for its proof, then {} that",
                short(&now.sha),
                short(head),
                kind.word()
            ),
        ));
    }
    // A fork's run writes no status comment, so its approval names the head the person saw.
    let fork = !now.from.eq_ignore_ascii_case(&full);
    let body = match (kind, fork) {
        (Kind::Approve, true) => format!("/fearlezz approve {} {words}", short(head)),
        _ => format!("/fearlezz {} {words}", kind.word()),
    };
    let url = match github::comment(config, &full, pull, &person.token, &body).await? {
        github::Posted::Url(url) => url,
        github::Posted::BadToken => return Err(sign_in_again()),
        github::Posted::Refused(status) => {
            return Err(Refusal::new(
                502,
                format!(
                    "GitHub refused the comment ({status}); the Fearlezz app needs the pull requests write permission, or comment `{body}` on the pull request yourself"
                ),
            ));
        }
    };
    let answer = json!({"url": url, "pull": pull, "comment": body, "login": person.login});
    Ok(Response::from_json(&answer)?.with_status(201))
}

/// Only the shell on the app origin may ask, and only with JSON: a page on another site can send
/// the session cookie with a form post, never this.
pub fn same_origin(req: &Request, config: &Config) -> Result<(), Refusal> {
    let origin = req.headers().get("Origin")?;
    if origin.as_deref() != Some(config.app_origin.as_str()) {
        return Err(Refusal::new(
            403,
            "refused: the ask did not come from this site",
        ));
    }
    let json = req
        .headers()
        .get("Content-Type")?
        .is_some_and(|t| t.trim_start().starts_with("application/json"));
    if !json {
        return Err(Refusal::new(415, "send JSON"));
    }
    Ok(())
}

/// The person's own words, on one line, as CI reads them from the comment.
fn words(said: &str, kind: Kind) -> Result<String, Refusal> {
    if said.contains(['\n', '\r']) {
        return Err(Refusal::new(400, "keep your words on one line"));
    }
    let words = said.split_whitespace().collect::<Vec<_>>().join(" ");
    if words.is_empty() {
        let what = match kind {
            Kind::Approve => "what you think of the pictures",
            Kind::Accept => "why the weaker proof is right",
        };
        return Err(Refusal::new(400, format!("say {what}, in your own words")));
    }
    if words.chars().count() > MAX_WORDS {
        return Err(Refusal::new(
            400,
            format!("say it in at most {MAX_WORDS} characters"),
        ));
    }
    Ok(words)
}

/// Whether the proof shows something `kind` settles: pictures of `check` that changed, or a
/// weakening that waits for an admin rather than failing the proof.
fn waits(proof: &Value, kind: Kind, check: Option<&str>) -> Result<(), Refusal> {
    match kind {
        Kind::Approve => {
            let check = check.unwrap_or_default();
            let changed = proof["looks"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|look| look["check"] == check)
                .flat_map(|look| look["items"].as_array().into_iter().flatten())
                .any(|item| item["change"] != "same" && item["flaky"] != true);
            if !changed {
                return Err(Refusal::new(
                    409,
                    format!("no changed pictures of `{check}` wait in this proof"),
                ));
            }
        }
        Kind::Accept => {
            let weakened = proof["weakened"].as_array().is_some_and(|w| !w.is_empty());
            if !weakened || proof["weakenings_wait"] != true {
                return Err(Refusal::new(
                    409,
                    "no weakened proof waits for an accept in this proof",
                ));
            }
        }
    }
    Ok(())
}

pub fn sign_in_again() -> Refusal {
    Refusal::new(401, "your GitHub sign in expired; sign in again")
}

/// A commit as people read it.
fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}
