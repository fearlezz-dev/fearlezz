//! The few facts Fearlezz needs from git, read through the git CLI.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};

#[derive(Clone)]
pub(crate) struct Git {
    /// The worktree root.
    pub root: PathBuf,
    /// The git folder shared by every worktree; journals and results live here.
    pub common: PathBuf,
}

impl Git {
    pub(crate) fn discover(cwd: &Path) -> Result<Git> {
        let out = run_git(
            cwd,
            &[
                "rev-parse",
                "--path-format=absolute",
                "--show-toplevel",
                "--git-common-dir",
            ],
            &[],
        )
        .context("Fearlezz runs inside a git repository")?;
        let mut lines = out.lines();
        let root = PathBuf::from(lines.next().context("no worktree root")?);
        let common = PathBuf::from(lines.next().context("no git folder")?);
        Ok(Git { root, common })
    }

    fn git(&self, args: &[&str]) -> Result<String> {
        run_git(&self.root, args, &[])
    }

    pub(crate) fn branch(&self) -> Result<String> {
        self.git(&["symbolic-ref", "--short", "-q", "HEAD"])
            .map(|s| s.trim().to_string())
            .context("HEAD is detached; Fearlezz keeps a journal per branch, so switch to one")
    }

    /// Every file in the worktree that git would track: committed, changed or new, not ignored.
    pub(crate) fn files(&self) -> Result<Vec<String>> {
        let out = self.git(&[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])?;
        let mut files: Vec<String> = out
            .split('\0')
            .filter(|p| !p.is_empty() && self.root.join(p).is_file())
            .map(String::from)
            .collect();
        files.sort();
        files.dedup();
        Ok(files)
    }

    /// Paths that differ from where this branch left `base`, including uncommitted, new and
    /// deleted files. On `base` itself, only uncommitted changes count.
    pub(crate) fn changed(&self, base: &str) -> Result<BTreeSet<String>> {
        let from = self.base_or_err(base)?;
        let mut paths: BTreeSet<String> = BTreeSet::new();
        let diff = self.git(&["diff", "--name-only", "--no-renames", "-z", &from])?;
        let new = self.git(&["ls-files", "-z", "--others", "--exclude-standard"])?;
        for p in diff.split('\0').chain(new.split('\0')) {
            if !p.is_empty() {
                paths.insert(p.to_string());
            }
        }
        Ok(paths)
    }

    /// The lines that differ from where this branch left `base`, for judging findings.
    pub(crate) fn changed_lines(&self, base: &str) -> Result<crate::findings::Changed> {
        let from = self.base_or_err(base)?;
        // Fixed prefixes: the developer's diff settings (noprefix, mnemonicPrefix) must not
        // change how paths are read.
        let diff = self.git(&[
            "-c",
            "core.quotePath=false",
            "diff",
            "-U0",
            "--no-color",
            "--no-renames",
            "--no-ext-diff",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            &from,
        ])?;
        let new: Vec<String> = self
            .git(&["ls-files", "-z", "--others", "--exclude-standard"])?
            .split('\0')
            .filter(|p| !p.is_empty())
            .map(String::from)
            .collect();
        let all = self.changed(base)?;
        Ok(crate::findings::Changed::from_diff(&diff, &new, &all))
    }

    /// The commit this branch left `base` at, or an error saying the base is missing:
    /// guessing would make every change look untouched.
    fn base_or_err(&self, base: &str) -> Result<String> {
        self.merge_base(base).with_context(|| {
            format!(
                "the base branch {base} is not here (neither {base} nor origin/{base}); fetch \
                 it, or set base = \"<branch>\" in fearlezz.toml"
            )
        })
    }

    /// The short id of the commit checked out.
    pub(crate) fn head(&self) -> Result<String> {
        Ok(self.git(&["rev-parse", "HEAD"])?.trim().to_string())
    }

    pub(crate) fn head_short(&self) -> Result<String> {
        self.short("HEAD")
    }

    /// The first line of the message of the commit checked out.
    pub(crate) fn subject(&self) -> Result<String> {
        Ok(self.git(&["log", "-1", "--format=%s"])?.trim().to_string())
    }

    /// The short name of a commit.
    pub(crate) fn short(&self, rev: &str) -> Result<String> {
        Ok(self.git(&["rev-parse", "--short", rev])?.trim().to_string())
    }

    /// Whether every file in the worktree is committed; new files count, ignored ones do not.
    pub(crate) fn committed(&self) -> Result<bool> {
        Ok(self.git(&["status", "--porcelain"])?.trim().is_empty())
    }

    /// The id of the tree the worktree holds right now, uncommitted and new files included.
    /// Proof is named by this, not by the last commit.
    pub(crate) fn tree(&self) -> Result<String> {
        let index = PathBuf::from(
            self.git(&["rev-parse", "--path-format=absolute", "--git-path", "index"])?
                .trim(),
        );
        let tmp_dir = self.common.join("fearlezz").join("tmp");
        std::fs::create_dir_all(&tmp_dir)?;
        let tmp = tmp_dir.join(format!("index-{}", std::process::id()));
        if index.exists() {
            std::fs::copy(&index, &tmp)?;
        }
        let tmp_str = tmp.to_string_lossy().to_string();
        let env = [("GIT_INDEX_FILE", tmp_str.as_str())];
        let result = run_git(&self.root, &["add", "-A"], &env)
            .and_then(|_| run_git(&self.root, &["write-tree"], &env));
        let removed = match std::fs::remove_file(&tmp) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
        let tree = result?;
        removed?;
        Ok(tree.trim().to_string())
    }
}

impl Git {
    /// Each checked out branch and the worktree it lives in.
    pub(crate) fn worktrees(&self) -> Result<Vec<(String, Git)>> {
        let out = self.git(&["worktree", "list", "--porcelain"])?;
        let mut found = Vec::new();
        let mut path = None;
        for line in out.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                path = Some(PathBuf::from(p));
            } else if let Some(b) = line.strip_prefix("branch refs/heads/")
                && let Some(root) = path.take()
            {
                let common = self.common.clone();
                found.push((b.to_string(), Git { root, common }));
            }
        }
        Ok(found)
    }
}

impl Git {
    /// Where this branch left `base`, if `base` exists here, locally or as `origin/<base>`
    /// (a CI checkout often has only the remote branch).
    pub(crate) fn merge_base(&self, base: &str) -> Option<String> {
        [base.to_string(), format!("origin/{base}")]
            .iter()
            .find_map(|b| self.git(&["merge-base", "HEAD", b]).ok())
            .map(|s| s.trim().to_string())
    }

    /// A hidden worktree with `commit` checked out, shared by every journal, and a hold on it:
    /// while the returned file lives, no other command removes it. Keeps the three most
    /// recently used and removes older ones nobody holds. `name` tells apart worktrees of one
    /// commit, such as one with a change's test files laid over it.
    pub(crate) fn base_worktree(&self, commit: &str, name: &str) -> Result<(Git, std::fs::File)> {
        let dir = self.common.join("fearlezz").join("base");
        std::fs::create_dir_all(&dir)?;
        // One command at a time adds or prunes bases, so two never add the same one and none
        // prunes a base another just took.
        let lock = std::fs::File::create(self.common.join("fearlezz").join("base.lock"))?;
        lock.lock()?;
        let id = &format!("{}{name}", commit.get(..12).unwrap_or(commit));
        let holds = self.common.join("fearlezz").join("base-holds");
        std::fs::create_dir_all(&holds)?;
        let hold_file = |id: &std::ffi::OsStr| {
            std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(holds.join(id))
        };
        let hold = hold_file(std::ffi::OsStr::new(id))?;
        hold.lock_shared()?;
        let root = dir.join(id);
        if !root.exists() {
            let path = root.to_string_lossy().to_string();
            self.git(&["worktree", "add", "--detach", "--force", &path, commit])?;
        }
        // Touch it so pruning keeps the bases in use.
        let now = std::time::SystemTime::now();
        std::fs::File::open(&root)?.set_modified(now)?;
        let mut bases: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&dir)?
            .filter_map(std::result::Result::ok)
            .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
            .collect();
        bases.sort_by_key(|b| std::cmp::Reverse(b.0));
        for (_, old) in bases.into_iter().skip(3) {
            let Some(name) = old.file_name() else {
                continue;
            };
            // A base another command holds is in use; it goes on a later run.
            let free = hold_file(name)?;
            if let Err(e) = free.try_lock() {
                match e {
                    std::fs::TryLockError::WouldBlock => continue,
                    std::fs::TryLockError::Error(e) => return Err(e.into()),
                }
            }
            let path = old.to_string_lossy().to_string();
            if let Err(e) = self.git(&["worktree", "remove", "--force", &path]) {
                eprintln!("could not remove the old base {path}: {e:#}");
            }
        }
        Ok((
            Git {
                root: root.canonicalize()?,
                common: self.common.clone(),
            },
            hold,
        ))
    }
}

fn run_git(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<String> {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(dir);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().context("running git")?;
    if !out.status.success() {
        bail!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8(out.stdout)?)
}
