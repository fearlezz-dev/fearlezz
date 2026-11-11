//! Running one check: a command that writes its proof into a folder. Fearlezz records the
//! facts the command cannot fake (inputs, tree, exit code, time) itself.

use std::fs::{self, File};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use jiff::Timestamp;

use crate::config::Check;
use crate::store::{Inputs, Outcome, RunResult, Store};

/// Env vars every check gets, so ordinary tools work. Everything else must be declared.
/// The rustup ones choose the toolchain, as PATH does for other tools.
pub(crate) const BASE_ENV: [&str; 11] = [
    "PATH",
    "HOME",
    "TMPDIR",
    "TERM",
    "LANG",
    "LC_ALL",
    "USER",
    "SHELL",
    "RUSTUP_TOOLCHAIN",
    "RUSTUP_HOME",
    "CARGO_HOME",
];

pub(crate) struct Where<'a> {
    pub root: &'a Path,
    pub tree: &'a str,
    pub journal: &'a str,
    pub stage: &'a str,
    /// Extra variables for this run, such as the ports of running services. Not inputs:
    /// a port says where a service listens, not what the check proves.
    pub vars: &'a std::collections::BTreeMap<String, String>,
    /// The lines the change touched, for judging findings.
    pub changed: &'a crate::findings::Changed,
}

pub(crate) fn execute(
    store: &Store,
    check: &Check,
    inputs: &Inputs,
    at: &Where<'_>,
) -> Result<RunResult> {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let id = format!("{nanos:024}-{}", std::process::id());
    let dir = store.new_run_dir(&inputs.key, &id)?;
    let log = dir.join("output.log");
    let out = File::create(&log)?;

    let started_at = Timestamp::now();
    // Reports from before this moment are another run's, never proof of this one.
    let started = SystemTime::now();
    let clock = Instant::now();
    let (exit_ok, exit_code) = if let Some(builtin) = &check.builtin {
        run_builtin(builtin, at, &dir, out)?;
        (true, Some(0))
    } else {
        let status = spawn(check, &dir, out, at)?;
        (status.success(), status.code())
    };
    let duration_ms = u64::try_from(clock.elapsed().as_millis()).unwrap_or(u64::MAX);
    let stdout_path = dir.join("stdout.txt");

    let (mut proofs, mut proof_errors) = read_proofs(&dir.join("proof.jsonl"));
    if let Some(stills) = &check.stills {
        proofs.extend(stills.proofs(&dir));
    }
    let mut passed = exit_ok;
    let (mut findings, mut findings_elsewhere) = (Vec::new(), 0);
    if let Some(spec) = &check.findings {
        let (found, errors) = spec.read.read(&stdout_path, &dir, at.root);
        proof_errors.extend(errors);
        let found = spec.levels.apply(&spec.tools, spec.defaults(found));
        let verdict = crate::findings::judge(found, spec.on, at.changed, passed);
        passed = verdict.passed;
        findings = verdict.counted;
        findings_elsewhere = verdict.elsewhere;
    }
    let tests = check.tests.as_ref().map(|reader| {
        reader
            .count(&[&log, &stdout_path], &dir, at.root, started)
            .unwrap_or_else(|why| {
                proof_errors.push(why);
                0
            })
    });
    if tests == Some(0) {
        passed = false;
    }
    let result = RunResult {
        id,
        check: check.id.clone(),
        key: inputs.key.clone(),
        parts: inputs.parts.clone(),
        tree: at.tree.to_string(),
        journal: at.journal.to_string(),
        stage: at.stage.to_string(),
        outcome: if passed {
            Outcome::Passed
        } else {
            Outcome::Failed
        },
        exit_code,
        started_at,
        duration_ms,
        proofs,
        proof_errors,
        findings,
        findings_elsewhere,
        tests,
        dir,
        log,
    };
    store.save(&result)?;
    Ok(result)
}

/// Runs the check's command in a clean env, inside the environment's shell if any.
fn spawn(check: &Check, dir: &Path, out: File, at: &Where<'_>) -> Result<std::process::ExitStatus> {
    // A tool that prints findings on stdout keeps them apart from its log.
    let stdout_path = dir.join("stdout.txt");
    let stdout = match &check.findings {
        Some(f) if f.read.reads_stdout() => File::create(&stdout_path)?,
        _ => out.try_clone()?,
    };
    // Inside a shell the command travels in a variable, so no quoting can break it.
    let script = match &check.shell {
        Some(shell) => format!("{shell} sh -c \"$FEARLEZZ_RUN\""),
        None => check.run.clone(),
    };
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(&script)
        .current_dir(at.root)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(out);
    for name in BASE_ENV
        .iter()
        .copied()
        .chain(check.env.iter().map(String::as_str))
    {
        if let Ok(value) = std::env::var(name) {
            cmd.env(name, value);
        }
    }
    if let Some(path) = project_path(at.root) {
        cmd.env("PATH", path);
    }
    cmd.envs(at.vars);
    cmd.env("FEARLEZZ_RUN", &check.run)
        .env("FEARLEZZ_OUT", dir)
        .env("FEARLEZZ_CHECK", &check.id)
        .env("FEARLEZZ_STAGE", at.stage);

    cmd.status()
        .with_context(|| format!("starting check {}", check.id))
}

/// Folders where a project installs its own tools, found before the rest of PATH, as a
/// package manager's scripts find them.
pub(crate) const TOOL_DIRS: [&str; 2] = ["node_modules/.bin", ".venv/bin"];

/// PATH with the project's tool folders that exist in front, or None when there are none.
fn project_path(root: &Path) -> Option<std::ffi::OsString> {
    let dirs: Vec<std::path::PathBuf> = TOOL_DIRS
        .iter()
        .map(|d| root.join(d))
        .filter(|d| d.is_dir())
        .collect();
    if dirs.is_empty() {
        return None;
    }
    let rest = std::env::var_os("PATH").unwrap_or_default();
    std::env::join_paths(dirs.into_iter().chain(std::env::split_paths(&rest))).ok()
}

/// Tools the built in pieces run that nixpkgs packages under the same name, so a missing one
/// can be answered with the line that adds it to a project's devenv.
pub(crate) const NIX_TOOLS: [&str; 5] = [
    "gitleaks",
    "trufflehog",
    "betterleaks",
    "osv-scanner",
    "zizmor",
];

/// The exit code a shell gives when it cannot find the command.
pub(crate) const NOT_FOUND: i32 = 127;

/// The command the shell could not find, from the end of a check's log.
pub(crate) fn missing_command(log: &Path) -> Option<String> {
    // bash says "sh: line 1: biome: command not found", dash "sh: 1: biome: not found".
    tail(log, 12).iter().rev().find_map(|line| {
        let rest = line
            .strip_suffix(": command not found")
            .or_else(|| line.strip_suffix(": not found"))?;
        rest.rsplit(": ").next().map(str::to_string)
    })
}

/// Does a built in check's work, leaving its findings where the jsonl reader looks.
fn run_builtin(
    builtin: &crate::config::Builtin,
    at: &Where<'_>,
    dir: &Path,
    mut out: File,
) -> Result<()> {
    use std::io::Write as _;
    let found = match builtin {
        crate::config::Builtin::Settings(settings) => {
            let found = settings.drift(at.root)?;
            for f in &found {
                writeln!(out, "{} drifted from the shared settings", f.file)?;
            }
            if found.is_empty() {
                writeln!(out, "every managed file matches the shared settings")?;
            }
            found
        }
        crate::config::Builtin::Rules(rules) => {
            let files = crate::git::Git::discover(at.root)?.files()?;
            let found = rules.check(at.root, &files, at.changed)?;
            for f in &found {
                writeln!(out, "{f}")?;
            }
            found
        }
    };
    let mut lines = String::new();
    for f in &found {
        lines.push_str(&serde_json::to_string(f)?);
        lines.push('\n');
    }
    fs::write(dir.join("findings.jsonl"), lines)?;
    Ok(())
}

fn read_proofs(path: &Path) -> (Vec<serde_json::Value>, Vec<String>) {
    let Ok(text) = fs::read_to_string(path) else {
        return (Vec::new(), Vec::new());
    };
    let mut proofs = Vec::new();
    let mut errors = Vec::new();
    for (n, line) in text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
    {
        match serde_json::from_str(line) {
            Ok(v) => proofs.push(v),
            Err(e) => errors.push(format!("proof.jsonl line {}: {e}", n + 1)),
        }
    }
    (proofs, errors)
}

/// The last lines of a run's output, for a failure message.
pub(crate) fn tail(log: &Path, lines: usize) -> Vec<String> {
    let text = fs::read_to_string(log).unwrap_or_default();
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..]
        .iter()
        .map(ToString::to_string)
        .collect()
}
