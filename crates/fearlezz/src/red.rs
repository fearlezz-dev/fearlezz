//! Red before green: a check run on the base commit with the change's test files laid over
//! it. When it fails there and passes on the change, the change's tests show what the change
//! does; when it passes there too, they do not.
//!
//! Which files are tests comes from the check's `test_files`. The whole check runs, not only
//! the new tests, so it works for any runner. A check with a JUnit reader also names its tests:
//! those that failed on the base and pass on the change, or the change's tests that pass on
//! both. A test that does not build on the base commit (it calls new code) counts as failing
//! there, as SWE-bench counts it. When the check fails on the base without the change's test
//! files too, failing there shows nothing.

use std::fs;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::config::{Check, Config};
use crate::counts::{Case, Reader};
use crate::env::EnvCtx;
use crate::git::Git;
use crate::look::{self, Base};
use crate::proof::{RED_NEXT, Red, Verdict};
use crate::state::Worktree;
use crate::store::{Outcome, RunResult, Store};

/// The change's test files for `check`, by `test_files` alone: its inputs only decide reuse.
pub(crate) fn test_files(check: &Check, wt: &Worktree) -> Vec<String> {
    wt.changed
        .iter()
        .filter(|p| check.is_test(p))
        .cloned()
        .collect()
}

/// Runs `check` on the base commit with the change's test files laid over it, in a hidden
/// worktree named by their content, so the same test files reuse it and its result. When that
/// fails, also runs it on the plain base commit (or reuses that run) to tell a broken base
/// apart. `head` is the check's passing run on the change. None when the change touches none
/// of its test files. It never blocks: whether it should depends on the change being a fix.
pub(crate) fn run(
    git: &Git,
    config: &Config,
    store: &Store,
    (check, head): (&Check, &RunResult),
    wt: &Worktree,
    stage: &str,
    env: &EnvCtx<'_>,
) -> Result<Option<Red>> {
    let shown = |files: Vec<String>, verdict, tests, log| Red {
        verdict,
        files,
        tests,
        why: None,
        blocks: false,
        log,
    };
    let files = test_files(check, wt);
    if files.is_empty() {
        return Ok(None);
    }
    if !on_base_config(git, wt, &check.id)? {
        return Ok(Some(shown(files, Verdict::New, Vec::new(), None)));
    }
    let mut hash = Sha256::new();
    let mut overlay = Vec::new();
    for path in &files {
        let content = fs::read(wt.git.root.join(path)).ok();
        hash.update(path.as_bytes());
        hash.update([0]);
        match &content {
            Some(bytes) => hash.update(bytes),
            None => hash.update(b"\0(removed)"),
        }
        hash.update([0]);
        overlay.push((path, content));
    }
    let mut name = hex::encode(hash.finalize());
    name.truncate(10);
    let name = format!("-red-{name}");
    let mut base = Base::at(git, config, wt.base.clone(), &name)?;
    for (path, content) in overlay {
        let to = base.wt.git.root.join(path);
        match content {
            Some(bytes) => {
                if let Some(dir) = to.parent() {
                    fs::create_dir_all(dir)?;
                }
                fs::write(&to, bytes)?;
            }
            None if to.exists() => fs::remove_file(&to)?,
            None => {}
        }
    }
    base.reread(config)?;
    let result = on_base(store, check, &mut base, stage, env)?;
    let red = if result.outcome == Outcome::Passed {
        let tests = names(
            result
                .cases
                .iter()
                .filter(|c| !c.failed && files.iter().any(|f| crate::counts::is_from(c, f))),
        );
        shown(files, Verdict::NotRed, tests, None)
    } else {
        let mut tests: Vec<String> = names(
            result
                .cases
                .iter()
                .filter(|c| c.failed && passes(head, &c.name)),
        );
        let mut plain = Base::at(git, config, wt.base.clone(), "")?;
        let plain = on_base(store, check, &mut plain, stage, env)?;
        let names_tests = matches!(check.tests, Some(Reader::Junit { .. }));
        if plain.outcome == Outcome::Failed {
            tests.retain(|t| !fails(&plain, t));
            if tests.is_empty() {
                shown(files, Verdict::BaseFails, Vec::new(), Some(plain.log))
            } else {
                shown(files, Verdict::Red, tests, Some(result.log.clone()))
            }
        } else if tests.is_empty() && names_tests {
            shown(
                files,
                Verdict::Unbuilt,
                Vec::new(),
                Some(result.log.clone()),
            )
        } else {
            shown(files, Verdict::Red, tests, Some(result.log.clone()))
        }
    };
    Ok(Some(red))
}

/// Whether the base commit's config has check `id`: one the change adds has no run there to
/// compare with.
fn on_base_config(git: &Git, wt: &Worktree, id: &str) -> Result<bool> {
    let (base, _hold) = git.base_worktree(&wt.base, "")?;
    if !base.root.join(crate::config::FILE).exists() {
        return Ok(false);
    }
    let config = Config::load(&base.root, &git.common)?;
    Ok(config.checks.iter().any(|c| c.id == id))
}

/// The check's run on `base`, reused when its inputs ran there before.
fn on_base(
    store: &Store,
    check: &Check,
    base: &mut Base,
    stage: &str,
    env: &EnvCtx<'_>,
) -> Result<RunResult> {
    let inputs = base.wt.inputs(check)?;
    match store.for_key(&inputs.key).pop() {
        Some(r) => Ok(r),
        None => look::run_on_base(store, check, &inputs, base, stage, env),
    }
}

/// The names of `cases`, each once, in report order.
fn names<'a>(cases: impl Iterator<Item = &'a Case>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for case in cases {
        if !out.contains(&case.name) {
            out.push(case.name.clone());
        }
    }
    out
}

/// Whether the test `name` ran in `run` and passed every time.
fn passes(run: &RunResult, name: &str) -> bool {
    let mut same = run.cases.iter().filter(|c| c.name == name).peekable();
    same.peek().is_some() && same.all(|c| !c.failed)
}

/// Whether the test `name` failed in `run`.
fn fails(run: &RunResult, name: &str) -> bool {
    run.cases.iter().any(|c| c.name == name && c.failed)
}

/// What red before green found for a check's passing run against one base commit, kept with
/// its look (see [`crate::store::AgainstBase`]) so the run, `fearlezz` and the hooks all read
/// one verdict.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Mark {
    /// Why the change's tests do not show what it does, when they do not.
    pub unshown: Option<Unshown>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Unshown {
    /// They pass on the base commit too.
    PassesOnBase,
    /// The check could not run on the base commit, for this reason.
    DidNotRun(String),
}

impl Mark {
    /// What to do when a fix is held by it, or None when it holds nothing.
    pub(crate) fn next(&self, check: &str) -> Option<String> {
        Some(match self.unshown.as_ref()? {
            Unshown::PassesOnBase => format!("{RED_NEXT}, then fearlezz run"),
            // A next line is one line; the run printed the whole reason.
            Unshown::DidNotRun(why) => format!(
                "red before green {check} did not run ({}); make the check run on the base \
                 commit, then fearlezz run",
                why.lines().next().unwrap_or_default().trim_end_matches(':')
            ),
        })
    }
}

/// Why `red` does not show the change, when it does not.
pub(crate) fn unshown(red: &Red) -> Option<Unshown> {
    (red.verdict == Verdict::NotRed).then_some(Unshown::PassesOnBase)
}

/// Whether the change says it fixes a bug. The pull request title decides when it is known
/// (in CI), since with a squash merge it is the commit: a `feat:` title with an interim
/// `fix:` commit is no fix. Without a title (locally), any commit since `base`, the base
/// commit, whose subject is a conventional `fix:` commit makes it one, so a later format or
/// merge commit does not hide an earlier fix; uncommitted work with no commits of its own is
/// not a fix yet.
pub(crate) fn is_fix_change(git: &Git, base: &str) -> Result<bool> {
    if let Some(title) = crate::action::Workflow::read().title() {
        return Ok(is_fix(title));
    }
    Ok(git.subjects_since(base)?.iter().any(|s| is_fix(s)))
}

/// Whether a commit subject or title is a conventional `fix:` commit.
pub(crate) fn is_fix(subject: &str) -> bool {
    let Some((kind, _)) = subject.split_once(':') else {
        return false;
    };
    let kind = kind.strip_suffix('!').unwrap_or(kind);
    let kind = kind.split_once('(').map_or(kind, |(k, _)| k);
    kind.trim() == "fix"
}

#[cfg(test)]
mod tests {
    use super::is_fix;

    #[test]
    fn a_fix_is_a_conventional_fix_commit() {
        assert!(is_fix("fix: a"));
        assert!(is_fix("fix(core): a"));
        assert!(is_fix("fix!: a"));
        assert!(!is_fix("feat: a"));
        assert!(!is_fix("fixes the thing"));
        assert!(!is_fix("prefix: a"));
    }
}
