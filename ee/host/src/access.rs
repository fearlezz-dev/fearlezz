//! Who asks the host, and whether they may: the viewer a request resolves to, and the one
//! policy every endpoint asks (`can`), on GitHub's word kept for a few minutes.
//!
//! A request has one door, and its viewer comes from that door alone. A page, its live
//! connection, the files grant and the share links' list read the session cookie and nothing
//! else, so a CLI token never opens a page (`session::read`); the CLI's API reads its token
//! alone (`cli::person`); a share link is the token in its URL (`share::find`); and what a person
//! says (a verdict, an accept, a note, a share) takes a browser's session from the app origin
//! and refuses any token (`speaker`).
//!
//! Reading mirrors GitHub: anyone for a public repository, people who may read it for a private
//! one, and `404` for everyone else, so a private repository's name is not confirmed. A share
//! link reads its own work alone. Everything else needs a role on GitHub.

use fearlezz_proof::review::Role;
use worker::{Env, Request};

use crate::share::Link;
use crate::{Config, Refusal, cache, cli, github, not_found, session};

/// How long the host trusts GitHub's word on what a person may see or do.
pub const GITHUB_SECONDS: i64 = 300;

/// What a refusal says to someone who may read the repository once they sign in.
const SIGN_IN_TO_SEE: &str = "sign in to see this work";

/// Who asks, as the policy sees them: borrowed from what the request resolved.
#[derive(Clone, Copy)]
pub enum Viewer<'a> {
    /// Nobody signed in, or a session whose GitHub token no longer works.
    Anonymous,
    /// Someone holding a share link, which views its one work.
    Share(&'a Link),
    /// A person GitHub signed in.
    Person(&'a Person),
}

impl<'a> Viewer<'a> {
    /// Who a page is shown to: the session's person, or someone anonymous.
    pub fn of(person: Option<&'a Person>) -> Viewer<'a> {
        person.map_or(Viewer::Anonymous, Viewer::Person)
    }
}

/// A person, with the GitHub token that decides what they may do.
pub struct Person {
    pub login: String,
    pub token: String,
    pub via: Via,
}

/// How a person came in.
pub enum Via {
    /// A browser's session: the id of its row, which the cookie holds.
    Session(String),
    /// The CLI's own token.
    Cli,
}

impl Person {
    /// The id of the session the person came in by, if they came in by one.
    pub fn session(&self) -> Option<&str> {
        match &self.via {
            Via::Session(id) => Some(id),
            Via::Cli => None,
        }
    }
}

/// What a viewer asks to do on a repository.
#[derive(Clone, Copy)]
pub enum Action<'a> {
    /// See the repository's pages, and the works, runs and files on them. A share link sees
    /// only the work it names, so a caller that knows the work names it.
    Read {
        work: Option<&'a str>,
    },
    Verdict,
    Note,
    Resolve,
    Share,
    Accept,
    LocalRun,
}

impl Action<'_> {
    /// The role it needs on GitHub; None for reading, which mirrors who may read.
    fn needs(self) -> Option<Role> {
        match self {
            Action::Read { .. } => None,
            Action::Accept => Some(Role::Admin),
            Action::Verdict | Action::Note | Action::Resolve | Action::Share | Action::LocalRun => {
                Some(Role::Write)
            }
        }
    }

    /// What it does, as a refusal says it: "cannot give verdicts on owner/name".
    fn words(self) -> &'static str {
        match self {
            Action::Read { .. } => "see this work",
            Action::Verdict => "give verdicts",
            Action::Note => "make notes",
            Action::Resolve => "resolve notes",
            Action::Share => "share a work",
            Action::Accept => "accept a weakened proof",
            Action::LocalRun => "send local runs",
        }
    }
}

/// Refuses `viewer` the action on `repository` unless the policy allows it. Reading answers
/// `401` saying to sign in when signing in could help (`Refusal::asks_sign_in`), `404` for a
/// person GitHub shows nothing of a private repository or a share link of another work, and
/// `401` for a CLI token whose GitHub token no longer works. Anything else answers `403` saying
/// the role it needs, or `401` when the person's GitHub token no longer works.
pub async fn can(
    env: &Env,
    config: &Config,
    viewer: Viewer<'_>,
    action: Action<'_>,
    repository: &str,
) -> Result<(), Refusal> {
    let Some(needs) = action.needs() else {
        if let (Viewer::Share(link), Action::Read { work }) = (viewer, action) {
            return match work {
                Some(work) if work == link.work => Ok(()),
                _ => Err(not_found()),
            };
        }
        return match sees(env, config, viewer, repository, Asking::GitHub).await? {
            Some(true) => Ok(()),
            _ => Err(Refusal::new(401, SIGN_IN_TO_SEE)),
        };
    };
    let Viewer::Person(person) = viewer else {
        return Err(Refusal::new(401, format!("sign in to {}", action.words())));
    };
    if role(env, config, person, repository).await? >= needs {
        return Ok(());
    }
    let needs = match needs {
        Role::Admin => "admin rights",
        Role::Write | Role::Less => "write access",
    };
    Err(Refusal::new(
        403,
        format!(
            "@{} cannot {} on {repository}: that needs {needs} on GitHub",
            person.login,
            action.words()
        ),
    ))
}

/// Where the policy may take GitHub's word from.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Asking {
    /// Only the word the host holds: a work's room asks this while it runs a write, which must
    /// not wait for GitHub (`room.rs`).
    Cache,
    /// GitHub, when the host holds no word.
    GitHub,
}

/// Whether `viewer` may see the repository `repo` (lowercased): `Err` `404` for a person GitHub
/// shows nothing of it, and None when `asking` cannot say. A session whose GitHub token no
/// longer works is someone anonymous, while a CLI token's is `401`.
pub async fn sees(
    env: &Env,
    config: &Config,
    viewer: Viewer<'_>,
    repo: &str,
    asking: Asking,
) -> Result<Option<bool>, Refusal> {
    let person = match viewer {
        Viewer::Person(person) => Some(person),
        Viewer::Anonymous => None,
        // A share link sees its work alone, never a repository (`can`).
        Viewer::Share(_) => return Err(not_found()),
    };
    if let Some(person) = person {
        match word(env, config, Question::Reads(person), repo, asking).await? {
            Word::Is(reads) if reads == "yes" => return Ok(Some(true)),
            Word::Is(_) => return Err(not_found()),
            Word::BadToken if matches!(person.via, Via::Cli) => {
                return Err(Refusal::new(401, cli::SIGN_IN));
            }
            Word::BadToken | Word::Unknown => {}
        }
    }
    Ok(
        match word(env, config, Question::Public, repo, asking).await? {
            Word::Is(public) if public == "yes" => Some(true),
            // Not public: no for someone anonymous, and for a person whose token GitHub
            // refused; a person whose word the cache lacks only GitHub can say.
            Word::Is(_) if person.is_none() || asking == Asking::GitHub => Some(false),
            Word::Is(_) | Word::BadToken | Word::Unknown => None,
        },
    )
}

/// What a person may do on a repository: GitHub's word, kept for a few minutes. `401` when
/// their GitHub token no longer works.
pub async fn role(
    env: &Env,
    config: &Config,
    person: &Person,
    repository: &str,
) -> Result<Role, Refusal> {
    match word(
        env,
        config,
        Question::Role(person),
        repository,
        Asking::GitHub,
    )
    .await?
    {
        Word::Is(role) => Ok(Role::of(&role)),
        Word::BadToken | Word::Unknown => Err(Refusal::new(
            401,
            "your GitHub sign in expired; sign in again",
        )),
    }
}

/// The person a browser's ask speaks for, after the checks every saying passes: no token, from
/// the app origin, JSON, and a session. `what` is what they ask, as "sign in to <what>" says.
pub async fn speaker(
    req: &Request,
    env: &Env,
    config: &Config,
    what: &str,
) -> Result<Person, Refusal> {
    if req.headers().get("Authorization")?.is_some() {
        return Err(Refusal::new(
            403,
            format!("refused: only a person on the work's page can {what}, never a token"),
        ));
    }
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
    session::read(req, env)
        .await?
        .ok_or_else(|| Refusal::new(401, format!("sign in to {what}")))
}

/// What the host asks GitHub about a repository.
#[derive(Clone, Copy)]
enum Question<'a> {
    /// Whether anyone may read it, as the app sees it.
    Public,
    /// Whether the person may read it, as their token sees it.
    Reads(&'a Person),
    /// The person's role on it.
    Role(&'a Person),
}

/// GitHub's word on a question.
enum Word {
    Is(String),
    /// The person's token no longer works: never kept.
    BadToken,
    /// Not kept, and the asking may not ask GitHub.
    Unknown,
}

/// GitHub's word on `question` about `repo`, the one lookup every rule reads: kept for
/// `GITHUB_SECONDS` under one key per question, person and repository, and asked of GitHub
/// only when nothing is kept and `asking` allows it.
async fn word(
    env: &Env,
    config: &Config,
    question: Question<'_>,
    repo: &str,
    asking: Asking,
) -> Result<Word, Refusal> {
    let key = match question {
        Question::Public => format!("public:{}", repo.to_lowercase()),
        Question::Reads(person) => key("reads", person, repo),
        Question::Role(person) => key("role", person, repo),
    };
    if let Some(kept) = cache::get(env, &key).await? {
        return Ok(Word::Is(kept));
    }
    if asking == Asking::Cache {
        return Ok(Word::Unknown);
    }
    let yes = |b: bool| if b { "yes" } else { "no" };
    let word = match question {
        Question::Public => Some(yes(github::public(config, repo).await?)),
        Question::Reads(person) => match github::sight(config, repo, &person.token).await? {
            github::Sight::Reads => Some(yes(true)),
            github::Sight::Nothing => Some(yes(false)),
            github::Sight::BadToken => None,
        },
        Question::Role(person) => github::permission(config, repo, &person.login, &person.token)
            .await?
            .map(Role::word),
    };
    let Some(word) = word else {
        return Ok(Word::BadToken);
    };
    cache::put(env, &key, word, GITHUB_SECONDS).await?;
    Ok(Word::Is(word.to_owned()))
}

fn key(question: &str, person: &Person, repo: &str) -> String {
    format!(
        "{question}:{}:{}",
        person.login.to_lowercase(),
        repo.to_lowercase()
    )
}
