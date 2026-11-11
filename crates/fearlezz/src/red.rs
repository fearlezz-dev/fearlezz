//! Red before green: a check run on the base commit with the change's test files laid over
//! it. When it fails there and passes on the change, the change's tests show what the change
//! does; when it passes there too, they do not.
//!
//! Which files are tests comes from the check's `test_files`. The whole check runs, not only
//! the new tests, so it works for any runner; a test that does not build on the base commit
//! (it calls new code) counts as failing there, as SWE-bench counts it.

use std::fs;

use anyhow::Result;
use sha2::{Digest as _, Sha256};

use crate::config::{Check, Config};
use crate::env::EnvCtx;
use crate::git::Git;
use crate::look::{self, Base};
use crate::state::Worktree;
use crate::store::{Outcome, RunResult, Store};

/// What the check did on the base commit with the change's test files.
pub(crate) struct Red {
    /// The test files the change added, changed or removed that the check reads.
    pub files: Vec<String>,
    /// The run on the base commit.
    pub base: RunResult,
}

impl Red {
    /// Whether the check failed on the base commit, as it should.
    pub(crate) fn is_red(&self) -> bool {
        self.base.outcome == Outcome::Failed
    }

    /// `tests/a.sh` or `2 test files`, with the verb agreeing.
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
/// worktree named by their content, so the same test files reuse it and its result. None when
/// the change touches none of its test files, or there is no base branch.
pub(crate) fn run(
    git: &Git,
    config: &Config,
    store: &Store,
    check: &Check,
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
    let inputs = base.wt.inputs(check)?;
    let result = match store.for_key(&inputs.key).pop() {
        Some(r) => r,
        None => look::run_on_base(store, check, &inputs, &mut base, stage, env)?,
    };
    Ok(Some(Red {
        files,
        base: result,
    }))
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
