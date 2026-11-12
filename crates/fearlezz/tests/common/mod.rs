//! A real git repository in a temporary folder, driven through the `fearlezz` binary.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]
#![allow(dead_code, reason = "each test file uses only some of the helpers")]

pub(crate) mod github;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;

use assert_cmd::Command;
use tempfile::TempDir;

pub(crate) struct Repo {
    _dir: TempDir,
    pub root: PathBuf,
    /// The per user cache folder Fearlezz sees (`XDG_CACHE_HOME`), apart from the repository.
    _cache: TempDir,
    pub cache: PathBuf,
}

pub(crate) struct Output {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub(crate) fn lines(&self) -> Vec<&str> {
        self.stdout.lines().collect()
    }

    pub(crate) fn last_line(&self) -> &str {
        self.stdout.lines().last().unwrap_or("")
    }
}

impl Repo {
    /// A repository on `main` with one commit holding `fearlezz.toml` and `files`. It opts out
    /// of the agent skill, which is the subject of `with_skill` alone.
    pub(crate) fn new(config: &str, files: &[(&str, &str)]) -> Repo {
        Repo::with_skill(
            &format!("{config}\n[settings.skill]\nagents = []\n\n[settings.hooks]\nagents = []\n"),
            files,
        )
    }

    /// A repository as a real project has it: `fearlezz.toml` exactly as given.
    pub(crate) fn with_skill(config: &str, files: &[(&str, &str)]) -> Repo {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let cache_dir = tempfile::tempdir().unwrap();
        let cache = cache_dir.path().canonicalize().unwrap();
        let repo = Repo {
            _dir: dir,
            root,
            _cache: cache_dir,
            cache,
        };
        repo.git(&["init", "-q", "-b", "main"]);
        repo.git(&["config", "user.email", "dev@example.com"]);
        repo.git(&["config", "user.name", "Dev"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo.write("fearlezz.toml", config);
        for (path, content) in files {
            repo.write(path, content);
        }
        repo.commit("initial");
        repo
    }

    pub(crate) fn write(&self, path: &str, content: &str) {
        let full = self.root.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, content).unwrap();
    }

    pub(crate) fn remove(&self, path: &str) {
        fs::remove_file(self.root.join(path)).unwrap();
    }

    pub(crate) fn path(&self, path: &str) -> PathBuf {
        self.root.join(path)
    }

    pub(crate) fn git(&self, args: &[&str]) -> String {
        git_in(&self.root, args)
    }

    pub(crate) fn commit(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", message]);
    }

    pub(crate) fn branch(&self, name: &str) {
        self.git(&["switch", "-q", "-c", name]);
    }

    /// A second worktree of the same repository on a new branch.
    pub(crate) fn worktree(&self, branch: &str) -> Worktree {
        let path = self.root.with_file_name(format!(
            "{}-{}",
            self.root.file_name().unwrap().to_string_lossy(),
            branch.replace('/', "-")
        ));
        self.git(&[
            "worktree",
            "add",
            "-q",
            "-b",
            branch,
            path.to_str().unwrap(),
        ]);
        Worktree {
            root: path,
            cache: self.cache.clone(),
        }
    }

    pub(crate) fn fz(&self, args: &[&str]) -> Output {
        fz_in(&self.root, Some(&self.cache), args, &[])
    }

    pub(crate) fn fz_env(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        fz_in(&self.root, Some(&self.cache), args, env)
    }

    /// The folder holding this repository's base checkouts, one per base commit.
    pub(crate) fn bases(&self) -> PathBuf {
        let dir = self.cache.join("fearlezz").join("bases");
        let mut found: Vec<PathBuf> = fs::read_dir(&dir)
            .map(|d| d.filter_map(Result::ok).map(|e| e.path()).collect())
            .unwrap_or_default();
        assert_eq!(
            found.len(),
            1,
            "one repository's bases in {}",
            dir.display()
        );
        found.remove(0)
    }

    /// Runs `fearlezz` with `input` on stdin, as an agent runs a hook.
    pub(crate) fn fz_input(&self, args: &[&str], input: &str) -> Output {
        let out = Command::cargo_bin("fearlezz")
            .unwrap()
            .args(args)
            .current_dir(&self.root)
            .env("XDG_CACHE_HOME", &self.cache)
            .env_remove("CI")
            .env_remove("FEARLEZZ_JOURNAL")
            .write_stdin(input)
            .output()
            .unwrap();
        Output {
            ok: out.status.success(),
            stdout: String::from_utf8(out.stdout).unwrap(),
            stderr: String::from_utf8(out.stderr).unwrap(),
        }
    }
}

pub(crate) struct Worktree {
    pub root: PathBuf,
    cache: PathBuf,
}

impl Worktree {
    pub(crate) fn fz(&self, args: &[&str]) -> Output {
        fz_in(&self.root, Some(&self.cache), args, &[])
    }

    pub(crate) fn fz_env(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        fz_in(&self.root, Some(&self.cache), args, env)
    }

    pub(crate) fn write(&self, path: &str, content: &str) {
        let full = self.root.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, content).unwrap();
    }
}

impl Drop for Worktree {
    fn drop(&mut self) {
        if let Err(e) = fs::remove_dir_all(&self.root) {
            eprintln!("could not remove {}: {e}", self.root.display());
        }
    }
}

pub(crate) fn git_in(dir: &Path, args: &[&str]) -> String {
    let out = StdCommand::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// The variables GitHub Actions sets for every step, which tests set only on purpose.
const ACTIONS_ENV: [&str; 15] = [
    "GITHUB_ACTIONS",
    "GITHUB_EVENT_NAME",
    "GITHUB_EVENT_PATH",
    "GITHUB_STEP_SUMMARY",
    "GITHUB_OUTPUT",
    "GITHUB_TOKEN",
    "GH_TOKEN",
    "GITHUB_API_URL",
    "GITHUB_GRAPHQL_URL",
    "GITHUB_REPOSITORY",
    "GITHUB_SERVER_URL",
    "GITHUB_RUN_ID",
    "GITHUB_RUN_ATTEMPT",
    "RUNNER_TEMP",
    "RUNNER_TOOL_CACHE",
];

/// Runs `fearlezz` in a folder that need not be a repository, as a workflow without a checkout
/// does.
pub(crate) fn fz_at_env(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    fz_in(dir, None, args, env)
}

fn fz_in(dir: &Path, cache: Option<&Path>, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::cargo_bin("fearlezz").unwrap();
    if let Some(cache) = cache {
        cmd.env("XDG_CACHE_HOME", cache);
    }
    // Tests choose for themselves whether they run as on CI.
    cmd.args(args)
        .current_dir(dir)
        .env_remove("CI")
        .env_remove("FEARLEZZ_JOURNAL");
    // On GitHub's runners these say the run is a workflow's, which talks to GitHub.
    for name in ACTIONS_ENV {
        cmd.env_remove(name);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    Output {
        ok: out.status.success(),
        stdout: String::from_utf8(out.stdout).unwrap(),
        stderr: String::from_utf8(out.stderr).unwrap(),
    }
}

/// Counts how often a check really ran, from outside its inputs.
pub(crate) fn runs(counter: &Path) -> usize {
    fs::read_to_string(counter).map_or(0, |s| s.lines().count())
}

/// Renders a small film or still with ffmpeg's test sources: a white canvas of 160x120 at
/// 10 frames per second for `seconds`, with `filter` applied (empty for none). The file
/// type follows the extension (`.gif` for films, `.png` for stills).
pub(crate) fn render(path: &Path, seconds: f64, filter: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut cmd = StdCommand::new("ffmpeg");
    cmd.args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg(format!("color=c=white:s=160x120:r=10:d={seconds}"));
    if !filter.is_empty() {
        cmd.args(["-vf", filter]);
    }
    if path.extension().is_some_and(|e| e == "png") {
        cmd.args(["-frames:v", "1"]);
    }
    let out = cmd
        .arg(path)
        .output()
        .expect("ffmpeg is needed for media tests");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A blue box at `x`, shown from `from` seconds on.
pub(crate) fn box_at(x: u32, from: f64) -> String {
    format!("drawbox=x={x}:y=30:w=50:h=40:color=blue:t=fill:enable='gte(t,{from})'")
}

/// `fearlezz` run in `dir` without a repository.
pub(crate) fn fz_at(dir: &Path, args: &[&str]) -> Output {
    fz_in(dir, None, args, &[])
}
