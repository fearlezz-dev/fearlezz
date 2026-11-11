//! The few facts Fearlezz needs from git, read through the git CLI.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use sha2::{Digest as _, Sha256};

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

    /// A checkout of `commit` shared by every journal, and a hold on it: while the returned
    /// file lives, no other command removes it. Keeps the three most recently used and
    /// removes older ones nobody holds. `name` tells apart checkouts of one commit, such as
    /// one with a change's test files laid over it. They live in [`Git::bases_dir`].
    pub(crate) fn base_worktree(&self, commit: &str, name: &str) -> Result<(Git, std::fs::File)> {
        let bases = self.bases_dir()?;
        let dir = bases.join("checkouts");
        std::fs::create_dir_all(&dir)?;
        let repo = bases.join("repo");
        if !repo.exists() {
            std::fs::write(&repo, self.common.to_string_lossy().as_bytes())?;
        }
        // One command at a time adds or prunes bases, so two never add the same one and none
        // prunes a base another just took.
        let lock = std::fs::File::create(bases.join("lock"))?;
        lock.lock()?;
        self.drop_bases_in_git_folder();
        let id = &format!("{}{name}", commit.get(..12).unwrap_or(commit));
        let hold = hold_file(&bases, std::ffi::OsStr::new(id))?;
        hold.lock_shared()?;
        let root = dir.join(id);
        if !root.exists() {
            let path = root.to_string_lossy().to_string();
            // Forced: git may still list the folder after the cache was cleared.
            self.git(&["worktree", "add", "--detach", "--force", &path, commit])?;
        }
        // Touch it so pruning keeps the bases in use.
        let now = std::time::SystemTime::now();
        std::fs::File::open(&root)?.set_modified(now)?;
        let mut used = checkouts(&dir);
        used.sort_by_key(|b| std::cmp::Reverse(b.0));
        for (_, old) in used.into_iter().skip(3) {
            self.remove_base(&bases, &old)?;
        }
        Ok((
            Git {
                root: root.canonicalize()?,
                common: self.common.clone(),
            },
            hold,
        ))
    }

    /// Where this repository keeps its base checkouts: in the per user cache, outside the
    /// repository, since tools refuse or skip files below `.git` (Vite's dev server answers
    /// 403 there). One folder per repository, told apart by its shared git folder, which a
    /// `repo` file in it names so pruning can tell when the repository is gone.
    pub(crate) fn bases_dir(&self) -> Result<PathBuf> {
        let main = match self.common.file_name() {
            Some(n) if n == ".git" => self.common.parent().unwrap_or(&self.common),
            _ => &self.common,
        };
        let label: String = main
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let hash = hex::encode(Sha256::digest(self.common.to_string_lossy().as_bytes()));
        Ok(cache_dir()?
            .join("bases")
            .join(format!("{label}-{}", hash.get(..12).unwrap_or(&hash))))
    }

    /// Removes one base checkout unless another command holds it; then it goes on a later run.
    fn remove_base(&self, bases: &Path, checkout: &Path) -> Result<()> {
        let Some(name) = checkout.file_name() else {
            return Ok(());
        };
        let free = hold_file(bases, name)?;
        if let Err(e) = free.try_lock() {
            return match e {
                std::fs::TryLockError::WouldBlock => Ok(()),
                std::fs::TryLockError::Error(e) => Err(e.into()),
            };
        }
        let path = checkout.to_string_lossy().to_string();
        if let Err(e) = self.git(&["worktree", "remove", "--force", &path]) {
            eprintln!("could not remove the old base {path}: {e:#}");
        }
        Ok(())
    }

    /// Earlier versions kept base checkouts below `.git/fearlezz/base`; they go once.
    fn drop_bases_in_git_folder(&self) {
        let fearlezz = self.common.join("fearlezz");
        let old = fearlezz.join("base");
        let Ok(entries) = std::fs::read_dir(&old) else {
            return;
        };
        for entry in entries.filter_map(std::result::Result::ok) {
            let path = entry.path().to_string_lossy().to_string();
            if let Err(e) = self.git(&["worktree", "remove", "--force", &path]) {
                eprintln!("could not remove the old base {path}: {e:#}");
            }
        }
        for gone in [old, fearlezz.join("base-holds")] {
            if let Err(e) = std::fs::remove_dir_all(&gone)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                eprintln!("could not remove {}: {e}", gone.display());
            }
        }
        if let Err(e) = std::fs::remove_file(fearlezz.join("base.lock"))
            && e.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("could not remove the old base lock: {e}");
        }
    }

    /// Cleans the base cache at most once a day: this repository's checkouts unused for
    /// `keep_days` (unless held), git's records of checkouts whose folder is gone, and the
    /// folders of repositories that no longer exist.
    pub(crate) fn prune_bases(&self, keep_days: i64) -> Result<()> {
        let bases = self.bases_dir()?;
        let marker = bases.join("pruned");
        let day = std::time::Duration::from_secs(24 * 3600);
        let now = std::time::SystemTime::now();
        let age = |path: &Path| {
            std::fs::metadata(path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| now.duration_since(t).ok())
        };
        if !bases.exists() || age(&marker).is_some_and(|d| d < day) {
            return Ok(());
        }
        let lock = std::fs::File::create(bases.join("lock"))?;
        lock.lock()?;
        let keep = day * u32::try_from(keep_days).unwrap_or(u32::MAX);
        for (_, checkout) in checkouts(&bases.join("checkouts")) {
            if age(&checkout).is_some_and(|d| d > keep) {
                self.remove_base(&bases, &checkout)?;
            }
        }
        self.git(&["worktree", "prune"])?;
        if let Some(all) = bases.parent() {
            for other in std::fs::read_dir(all)?.filter_map(std::result::Result::ok) {
                let gone = std::fs::read_to_string(other.path().join("repo"))
                    .is_ok_and(|common| !Path::new(common.trim()).exists());
                if gone && let Err(e) = std::fs::remove_dir_all(other.path()) {
                    eprintln!("could not remove {}: {e}", other.path().display());
                }
            }
        }
        std::fs::write(&marker, b"")?;
        Ok(())
    }
}

/// The per user cache folder of Fearlezz: below `$XDG_CACHE_HOME` when set, else the
/// platform's (`~/Library/Caches` on macOS, `~/.cache` elsewhere).
pub(crate) fn cache_dir() -> Result<PathBuf> {
    let xdg = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute());
    if let Some(dir) = xdg {
        return Ok(dir.join("fearlezz"));
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set, so there is no cache folder for base checkouts")?;
    let caches = if cfg!(target_os = "macos") {
        home.join("Library").join("Caches")
    } else {
        home.join(".cache")
    };
    Ok(caches.join("fearlezz"))
}

/// The file whose shared lock holds the base checkout `id`.
fn hold_file(bases: &Path, id: &std::ffi::OsStr) -> std::io::Result<std::fs::File> {
    let holds = bases.join("holds");
    std::fs::create_dir_all(&holds)?;
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(holds.join(id))
}

/// Each checkout in `dir`, with when it was last used.
fn checkouts(dir: &Path) -> Vec<(std::time::SystemTime, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(std::result::Result::ok)
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect()
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
