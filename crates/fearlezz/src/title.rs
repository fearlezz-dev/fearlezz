//! The title of this change: the pull request's, since with a squash merge it is the commit.
//! In CI it is the event file's; locally it is the title of the current branch's open pull
//! request as `gh` tells it, so local runs and CI judge the same words. Red before green and
//! the `commits` check both read it here, so they never disagree.
//!
//! Asking `gh` takes most of a second over the network, and every `fearlezz` (each hook too)
//! hashes the `commits` check's inputs, so an answer is kept a minute per branch in the user's
//! cache folder, and once per command in memory. A cold cache waits for `gh` at most five
//! seconds. `gh` is asked only when the `origin` remote is on GitHub; another remote, or none,
//! never runs it. A missing `gh`, no sign in, no network or no open pull request all mean no
//! title, kept with why (`NoTitle`) so a run can say it; asking never fails a run.

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::files::{UserDir, sha256_hex};
use crate::git::Git;

/// How long `gh` may take to answer before the change has no title.
const BUDGET: Duration = Duration::from_secs(5);

/// How long a kept answer holds: a renamed or newly opened pull request shows within it.
const FRESH: Duration = Duration::from_secs(60);

/// The titles this command found, per worktree root, so it asks at most once.
static ASKED: OnceLock<Mutex<HashMap<PathBuf, Found>>> = OnceLock::new();

/// The change's title, or why it has none.
pub(crate) type Found = Result<Title, NoTitle>;

/// The title of the change's pull request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Title(pub String);

/// Why the change has no title.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum NoTitle {
    /// The GitHub Actions run is not for a pull request, such as a push.
    NoPullRequestEvent,
    /// The `origin` remote is missing or not on GitHub, so `gh` was not asked.
    NotOnGitHub,
    /// No branch is checked out.
    NoBranch,
    /// `gh` is not installed.
    NoGh,
    /// The branch has no open pull request.
    NoOpenPull,
    /// `gh` gave no answer within its budget.
    Late,
    /// `gh` failed, with the first line it printed on stderr.
    GhFailed(String),
}

impl fmt::Display for NoTitle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NoTitle::NoPullRequestEvent => write!(f, "the workflow runs for no pull request"),
            NoTitle::NotOnGitHub => write!(f, "origin is not on GitHub"),
            NoTitle::NoBranch => write!(f, "no branch is checked out"),
            NoTitle::NoGh => write!(f, "gh is not installed"),
            NoTitle::NoOpenPull => write!(f, "no open pull request"),
            NoTitle::Late => write!(f, "gh gave no answer in {} s", BUDGET.as_secs()),
            NoTitle::GhFailed(said) if said.is_empty() => write!(f, "gh failed"),
            NoTitle::GhFailed(said) => write!(f, "gh failed: {said}"),
        }
    }
}

/// The change's title: CI's event file first, else (outside GitHub Actions) the open pull
/// request of the branch `git` has checked out, else why there is none.
pub(crate) fn of(git: &Git) -> Found {
    let workflow = crate::action::Workflow::read();
    if let Some(title) = workflow.title() {
        return Ok(Title(title.to_string()));
    }
    // On a runner the event is the whole truth; a push has no pull request to ask about.
    if workflow.actions {
        return Err(NoTitle::NoPullRequestEvent);
    }
    let asked = ASKED.get_or_init(Mutex::default);
    if let Some(known) = asked.lock().ok().and_then(|a| a.get(&git.root).cloned()) {
        return known;
    }
    let title = local(git);
    if let Ok(mut asked) = asked.lock() {
        asked.insert(git.root.clone(), title.clone());
    }
    title
}

/// One answer kept in the user's cache folder.
#[derive(Serialize, Deserialize)]
struct Kept {
    /// Seconds since the Unix epoch when `gh` gave it.
    at: u64,
    title: Found,
}

/// The branch's open pull request's title, from the kept answer while it is fresh, else from
/// `gh`. Without a GitHub `origin` or a branch it has none, before `gh` is ever asked.
fn local(git: &Git) -> Found {
    git.github_repository().ok_or(NoTitle::NotOnGitHub)?;
    let branch = git.branch().ok().ok_or(NoTitle::NoBranch)?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let path = UserDir::Cache.path().ok().map(|dir| {
        let key = format!("{}\n{branch}", git.common.display());
        dir.join("titles").join(format!("{}.json", sha256_hex(key)))
    });
    let kept = path
        .as_ref()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|bytes| serde_json::from_slice::<Kept>(&bytes).ok())
        .filter(|k| now.saturating_sub(k.at) < FRESH.as_secs());
    if let Some(kept) = kept {
        return kept.title;
    }
    let title = ask(git);
    if let Some(path) = path {
        let kept = Kept {
            at: now,
            title: title.clone(),
        };
        // A cache that cannot be written only means asking again next time.
        let written = serde_json::to_vec(&kept)
            .map_err(anyhow::Error::from)
            .and_then(|json| crate::files::write_atomic(&path, json));
        drop(written);
    }
    title
}

/// What `gh pr view` says about the checked out branch's pull request, when it is open.
fn ask(git: &Git) -> Found {
    use crate::process::Failed;
    #[derive(Deserialize)]
    struct Pull {
        title: String,
        state: String,
    }
    let mut cmd = Command::new("gh");
    cmd.args(["pr", "view", "--json", "title,state"])
        .env("GH_PROMPT_DISABLED", "1")
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .current_dir(&git.root);
    let out = match crate::process::read(&mut cmd, Some(BUDGET)) {
        Ok(out) => out,
        Err(Failed::Start { .. }) => return Err(NoTitle::NoGh),
        Err(Failed::Late { .. }) => return Err(NoTitle::Late),
        // gh's own words for a branch with no pull request at all.
        Err(Failed::Exit { stderr, .. }) if stderr.starts_with("no pull requests found") => {
            return Err(NoTitle::NoOpenPull);
        }
        Err(Failed::Exit { stderr, .. }) => {
            let said = stderr.lines().next().unwrap_or_default().trim();
            return Err(NoTitle::GhFailed(said.to_string()));
        }
    };
    let pull: Pull = serde_json::from_slice(&out)
        .map_err(|e| NoTitle::GhFailed(format!("its answer is not the JSON asked for: {e}")))?;
    if pull.state == "OPEN" {
        Ok(Title(pull.title))
    } else {
        Err(NoTitle::NoOpenPull)
    }
}
