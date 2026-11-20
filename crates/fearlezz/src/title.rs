//! The title of this change: the pull request's, since with a squash merge it is the commit.
//! In CI it is the event file's; locally it is the title of the current branch's open pull
//! request as `gh` tells it, so local runs and CI judge the same words. Red before green and
//! the `commits` check both read it here, so they never disagree.
//!
//! Asking `gh` takes most of a second over the network, and every `fearlezz` (each hook too)
//! hashes the `commits` check's inputs, so an answer is kept a minute per branch in the user's
//! cache folder, and once per command in memory. A missing `gh`, no GitHub remote, no sign in,
//! no network or no open pull request all mean no title; asking never fails a run.

use std::collections::HashMap;
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
static ASKED: OnceLock<Mutex<HashMap<PathBuf, Option<Title>>>> = OnceLock::new();

/// The title of the change's pull request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Title(pub String);

/// The change's title: CI's event file first, else (outside GitHub Actions) the open pull
/// request of the branch `git` has checked out, else none.
pub(crate) fn of(git: &Git) -> Option<Title> {
    let workflow = crate::action::Workflow::read();
    if let Some(title) = workflow.title() {
        return Some(Title(title.to_string()));
    }
    // On a runner the event is the whole truth; a push has no pull request to ask about.
    if workflow.actions {
        return None;
    }
    let asked = ASKED.get_or_init(Mutex::default);
    if let Some(known) = asked.lock().ok()?.get(&git.root) {
        return known.clone();
    }
    let title = local(git);
    asked.lock().ok()?.insert(git.root.clone(), title.clone());
    title
}

/// One answer kept in the user's cache folder.
#[derive(Serialize, Deserialize)]
struct Kept {
    /// Seconds since the Unix epoch when `gh` gave it.
    at: u64,
    title: Option<Title>,
}

/// The branch's open pull request's title, from the kept answer while it is fresh, else from
/// `gh`. None without a GitHub remote or a branch, before `gh` is ever asked.
fn local(git: &Git) -> Option<Title> {
    git.github_repository()?;
    let branch = git.branch().ok()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
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
fn ask(git: &Git) -> Option<Title> {
    #[derive(Deserialize)]
    struct Pull {
        title: String,
        state: String,
    }
    let mut cmd = Command::new("gh");
    cmd.args(["pr", "view", "--json", "title,state"])
        .env("GH_PROMPT_DISABLED", "1")
        .current_dir(&git.root);
    let out = crate::process::read(&mut cmd, Some(BUDGET)).ok()?;
    let pull: Pull = serde_json::from_slice(&out).ok()?;
    (pull.state == "OPEN").then_some(Title(pull.title))
}
