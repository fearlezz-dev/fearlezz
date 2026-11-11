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
use std::path::PathBuf;

use anyhow::Result;
use sha2::{Digest as _, Sha256};

use crate::config::{Check, Config};
use crate::counts::{Case, Reader};
use crate::env::EnvCtx;
use crate::git::Git;
use crate::look::{self, Base};
use crate::state::Worktree;
use crate::store::{Outcome, RunResult, Store};

/// What the check did on the base commit with the change's test files.
pub(crate) struct Red {
    /// The test files the change added, changed or removed that the check reads.
    pub files: Vec<String>,
    /// The run on the base commit with those files.
    pub base: RunResult,
    pub verdict: Verdict,
}

/// What the run on the base commit shows.
pub(crate) enum Verdict {
    /// The check failed on the base and passes here. `tests` names the tests that failed there
    /// and pass here, when its reader names tests.
    Red { tests: Vec<String> },
    /// The check failed on the base before any of the change's tests ran: they do not build
    /// there. Counted as red.
    Unbuilt,
    /// The check fails on the base without the change's test files too.
    BaseFails { log: PathBuf },
    /// The check passed on the base too. `tests` names the change's tests, when its reader
    /// names them.
    NotRed { tests: Vec<String> },
}

impl Red {
    /// `tests/a.sh` or `2 test files`, with the verbs agreeing.
    pub(crate) fn subject(&self) -> (String, &'static str, &'static str) {
        match self.files.as_slice() {
            [one] => (one.clone(), "fails", "passes"),
            many => (format!("{} test files", many.len()), "fail", "pass"),
        }
    }
}

/// The change's test files for `check`: changed paths it reads that are tests.
fn test_files(check: &Check, wt: &Worktree) -> Vec<String> {
    wt.changed
        .iter()
        .filter(|p| check.reads(p) && check.is_test(p))
        .cloned()
        .collect()
}

/// Runs `check` on the base commit with the change's test files laid over it, in a hidden
/// worktree named by their content, so the same test files reuse it and its result. When that
/// fails, also runs it on the plain base commit (or reuses that run) to tell a broken base
/// apart. `head` is the check's passing run on the change. None when the change touches none
/// of its test files, or there is no base branch.
pub(crate) fn run(
    git: &Git,
    config: &Config,
    store: &Store,
    (check, head): (&Check, &RunResult),
    wt: &Worktree,
    stage: &str,
    env: &EnvCtx<'_>,
) -> Result<Option<Red>> {
    let files = test_files(check, wt);
    if files.is_empty() {
        return Ok(None);
    }
    let Some(commit) = git.merge_base(&config.base) else {
        return Ok(None);
    };
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
    let mut base = Base::at(git, config, commit, &name)?;
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
    let verdict = if result.outcome == Outcome::Passed {
        Verdict::NotRed {
            tests: names(
                result
                    .cases
                    .iter()
                    .filter(|c| !c.failed && files.iter().any(|f| c.is_from(f))),
            ),
        }
    } else {
        let mut tests: Vec<String> = names(
            result
                .cases
                .iter()
                .filter(|c| c.failed && passes(head, &c.name)),
        );
        let plain = match Base::open(git, config)? {
            Some(mut plain) => Some(on_base(store, check, &mut plain, stage, env)?),
            None => None,
        };
        let names_tests = matches!(check.tests, Some(Reader::Junit { .. }));
        match plain.filter(|p| p.outcome == Outcome::Failed) {
            Some(plain) => {
                tests.retain(|t| !fails(&plain, t));
                if tests.is_empty() {
                    Verdict::BaseFails { log: plain.log }
                } else {
                    Verdict::Red { tests }
                }
            }
            None if tests.is_empty() && names_tests => Verdict::Unbuilt,
            None => Verdict::Red { tests },
        }
    };
    Ok(Some(Red {
        files,
        base: result,
        verdict,
    }))
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

/// Whether the change says it fixes a bug: the pull request title, else the head commit's
/// subject, is a conventional `fix:` commit.
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
