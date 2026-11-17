//! Running one check: a command that writes its proof into a folder. Fearlezz records the
//! facts the command cannot fake (inputs, tree, exit code, time) itself.

use std::fs::{self, File};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
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
    /// The shell of the `[env]` the checkout at `root` runs its commands in.
    pub shell: Option<&'a str>,
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
    } else if check.needs_envtest() {
        match envtest_assets(check, at, &out)? {
            Ok(assets) => {
                let mut vars = at.vars.clone();
                vars.insert("KUBEBUILDER_ASSETS".into(), assets);
                spawn(check, &dir, out, &Where { vars: &vars, ..*at })?
            }
            Err(code) => (false, code),
        }
    } else {
        spawn(check, &dir, out, at)?
    };
    let duration_ms = u64::try_from(clock.elapsed().as_millis()).unwrap_or(u64::MAX);
    let stdout_path = dir.join("stdout.txt");

    let (mut proofs, mut proof_errors) = read_proofs(&dir.join("proof.jsonl"));
    proofs.extend(check.pictures.proofs(&dir));
    let mut passed = exit_ok;
    let (mut findings, mut findings_elsewhere) = (Vec::new(), 0);
    if let Some(spec) = &check.findings {
        let (found, errors) = spec
            .read
            .read(&stdout_path, &dir, at.root, check.dir.as_deref());
        proof_errors.extend(errors);
        let found = spec.levels.apply(&spec.tools, spec.defaults(found));
        let verdict = crate::findings::judge(found, spec.on, at.changed, passed);
        passed = verdict.passed;
        findings = verdict.counted;
        findings_elsewhere = verdict.elsewhere;
    }
    let mut cases = Vec::new();
    let tests = check.tests.as_ref().map(|reader| {
        match reader.count(&[&log, &stdout_path], &dir, at.root, started) {
            Ok((n, found)) => {
                cases = found;
                n
            }
            Err(why) => {
                proof_errors.push(why);
                0
            }
        }
    });
    // A test that passed only on a rerun counts as failed unless the check lists it in `flaky_tests`.
    for case in cases.iter_mut().filter(|c| c.flaky) {
        case.failed = !check.flaky_tests.is_match(&case.name);
    }
    if tests == Some(0) || cases.iter().any(|c| c.failed) {
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
        cases,
        dir,
        log,
    };
    store.save(&result)?;
    Ok(result)
}

/// One step of a check as a command in a clean env, inside the environment's shell if any.
fn command(
    check: &Check,
    step: &str,
    dir: &Path,
    stdout: File,
    log: &File,
    at: &Where<'_>,
) -> Result<Command> {
    // Inside a shell the command travels in a variable, so no quoting can break it. A check
    // with a folder starts there; the shell itself starts at the root, where it finds its own
    // files, and moves into the folder before the command.
    let folder = check.dir.as_ref().map(|d| at.root.join(d));
    let script = match (at.shell, &folder) {
        (Some(shell), None) => format!("{shell} sh -c \"$FEARLEZZ_RUN\""),
        (Some(shell), Some(_)) => {
            format!("{shell} sh -c 'cd \"$FEARLEZZ_DIR\" && eval \"$FEARLEZZ_RUN\"'")
        }
        (None, _) => step.to_string(),
    };
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(&script)
        .current_dir(match (at.shell, &folder) {
            (None, Some(folder)) => folder,
            _ => at.root,
        })
        .env_clear()
        .stdin(Stdio::null())
        .stdout(stdout)
        .stderr(log.try_clone()?);
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
    cmd.env("FEARLEZZ_PORT", free_port()?.to_string());
    cmd.envs(at.vars);
    if let Some(folder) = &folder {
        cmd.env("FEARLEZZ_DIR", folder);
    }
    cmd.env("FEARLEZZ_RUN", step)
        .env("FEARLEZZ_OUT", dir)
        .env("FEARLEZZ_CHECK", &check.id)
        .env("FEARLEZZ_STAGE", at.stage);
    Ok(cmd)
}

/// Runs the check's command in a clean env, inside the environment's shell if any, recording
/// its app's window when it has `record`. Returns whether it passed and its exit code.
fn spawn(check: &Check, dir: &Path, out: File, at: &Where<'_>) -> Result<(bool, Option<i32>)> {
    // A tool that prints findings on stdout keeps them apart from its log; only the last
    // step's stdout holds them, the steps before it prepare.
    let stdout_path = dir.join("stdout.txt");
    let stdout = match &check.findings {
        Some(f) if f.read.reads_stdout() => File::create(&stdout_path)?,
        _ => out.try_clone()?,
    };
    let Some((last, before)) = check.steps.split_last() else {
        bail!("check {} has no command to run", check.id);
    };
    for step in before {
        let status = command(check, step, dir, out.try_clone()?, &out, at)?
            .status()
            .with_context(|| format!("starting check {}", check.id))?;
        if !status.success() {
            return Ok((false, status.code()));
        }
    }
    let mut cmd = command(check, last, dir, stdout, &out, at)?;
    let Some(record) = &check.pictures.record else {
        let status = cmd
            .status()
            .with_context(|| format!("starting check {}", check.id))?;
        return Ok((status.success(), status.code()));
    };
    // Its own process group, so stopping it after `seconds` stops the app it started too.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut log = out;
    // The windows open before the command starts are never recorded.
    let mut recorder = match crate::record::Recorder::new(
        crate::record::System::default(),
        record,
        crate::record::Place::from_env(),
    ) {
        Ok(r) => r,
        Err(e) => {
            use std::io::Write as _;
            writeln!(log, "record: {e:#}")?;
            return Ok((false, None));
        }
    };
    let mut child = cmd
        .spawn()
        .with_context(|| format!("starting check {}", check.id))?;
    // A recorder that panics must still stop the app the command started.
    let recorded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        recorder.during(&mut child, dir, &mut log)
    }))
    .unwrap_or_else(|panic| {
        Err(anyhow::anyhow!(
            "the recorder failed: {}",
            crate::panic_text(&*panic)
        ))
    });
    // Every film, failed ones too, so how often films fail can be counted machine wide.
    let films = recorder.films();
    let noted = if films.is_empty() {
        Ok(())
    } else {
        crate::git::cache_dir().and_then(|dir| {
            crate::record::note_films(
                &dir.join("films.jsonl"),
                at.root,
                &check.id,
                films,
                &crate::record::Machine::now(),
            )
        })
    };
    if let Err(e) = noted {
        use std::io::Write as _;
        writeln!(
            log,
            "record: could not note the film in the machine's film log: {e:#}"
        )?;
    }
    let recorded = match recorded {
        Ok(r) => r,
        Err(e) => {
            use std::io::Write as _;
            writeln!(log, "record: {e:#}")?;
            crate::record::stop(&mut child)?;
            return Ok((false, None));
        }
    };
    let code = match &recorded.ended {
        crate::record::Ended::Exited(status) => status.code(),
        crate::record::Ended::AfterSeconds | crate::record::Ended::WaitedTooLong => None,
    };
    Ok((recorded.passed(), code))
}

/// A port nothing listens on right now, for a server the check starts itself (such as
/// Playwright's `webServer`), so checks in two worktrees or on the base never collide. The
/// system picks it from its wide ephemeral range, so another process taking it in the
/// moment before the server starts is unlikely.
fn free_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
        .context("finding a free port for FEARLEZZ_PORT")?;
    Ok(listener.local_addr()?.port())
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

/// The Kubernetes version whose envtest binaries a module needs: `k8s.io/api v0.31.2` in its
/// go.mod (a replacement's version when it has one) is Kubernetes 1.31.
fn kubernetes_version(go_mod: &str) -> Option<String> {
    let minor = |version: &str| -> Option<String> {
        let minor = version.strip_prefix("v0.")?.split('.').next()?;
        minor.parse::<u32>().ok().map(|m| format!("1.{m}"))
    };
    let (mut required, mut replaced) = (None, None);
    for line in go_mod.lines() {
        let line = line.trim();
        let line = line
            .strip_prefix("require")
            .or_else(|| line.strip_prefix("replace"))
            .unwrap_or(line)
            .trim();
        let mut words = line.split_whitespace();
        if words.next() != Some("k8s.io/api") {
            continue;
        }
        let words: Vec<&str> = words.collect();
        // `k8s.io/api [v0.31.2] => k8s.io/api v0.30.1` replaces the required version; a
        // replacement by a local folder has none, and the required one stands.
        match words.iter().position(|w| *w == "=>") {
            Some(arrow) => replaced = words.get(arrow + 2).and_then(|v| minor(v)).or(replaced),
            None => required = words.first().and_then(|v| minor(v)).or(required),
        }
    }
    replaced.or(required)
}

/// Fetches, once into the user cache, the envtest binaries of the Kubernetes version the
/// check's go.mod requires, with setup-envtest run like the check (clean env, the project's
/// shell), and returns their folder. Its output goes to the check's log; when it fails, the
/// error is the exit code the check fails with.
fn envtest_assets(
    check: &Check,
    at: &Where<'_>,
    log: &File,
) -> Result<Result<String, Option<i32>>> {
    use std::io::Write as _;
    let folder = check
        .dir
        .as_ref()
        .map_or(at.root.to_path_buf(), |d| at.root.join(d));
    let go_mod = fs::read_to_string(folder.join("go.mod")).unwrap_or_default();
    let Some(version) = kubernetes_version(&go_mod) else {
        writeln!(
            &*log,
            "envtest: go.mod in {} requires no k8s.io/api, so the Kubernetes version is unknown",
            folder.display()
        )?;
        return Ok(Err(None));
    };
    let bin = crate::git::cache_dir()?.join("envtest");
    let command = "setup-envtest use \"$FEARLEZZ_K8S\" --bin-dir \"$FEARLEZZ_ENVTEST\" -p path";
    let script = match at.shell {
        Some(shell) => format!("{shell} sh -c '{command}'"),
        None => command.to_string(),
    };
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(&script)
        .current_dir(at.root)
        .env_clear()
        .stdin(Stdio::null())
        .stderr(log.try_clone()?);
    // The check's own variables too, such as a proxy the download needs.
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
    cmd.env("FEARLEZZ_K8S", &version)
        .env("FEARLEZZ_ENVTEST", &bin);
    let output = cmd.output().context("starting setup-envtest")?;
    let assets = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || assets.is_empty() {
        writeln!(
            &*log,
            "envtest: setup-envtest found no binaries for Kubernetes {version}"
        )?;
        return Ok(Err(output.status.code()));
    }
    Ok(Ok(assets))
}

/// Tools the built in pieces run that nixpkgs packages under the same name, so a missing one
/// can be answered with the line that adds it to a project's devenv.
pub(crate) const NIX_TOOLS: [&str; 10] = [
    "oxlint",
    "prettier",
    "stylua",
    "svelte-check",
    "setup-envtest",
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
        crate::config::Builtin::Commits(commits) => {
            let found = commits.check(at.root)?;
            for f in &found {
                writeln!(out, "{}", f.message)?;
            }
            if found.is_empty() {
                writeln!(out, "every message is a conventional commit")?;
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

#[cfg(test)]
mod tests {
    use super::kubernetes_version;

    #[test]
    fn the_kubernetes_version_follows_k8s_io_api_and_its_replacement() {
        let required =
            "module op\n\nrequire (\n\tk8s.io/api v0.31.2\n\tk8s.io/apimachinery v0.31.2\n)\n";
        assert_eq!(kubernetes_version(required).as_deref(), Some("1.31"));
        let one_line = "module op\nrequire k8s.io/api v0.29.0 // indirect\n";
        assert_eq!(kubernetes_version(one_line).as_deref(), Some("1.29"));
        let replaced = format!("{required}\nreplace k8s.io/api => k8s.io/api v0.30.1\n");
        assert_eq!(kubernetes_version(&replaced).as_deref(), Some("1.30"));
        // A replace before the require, naming the version it replaces, still wins.
        let first = "module op\nreplace k8s.io/api v0.31.2 => k8s.io/api v0.30.1\nrequire k8s.io/api v0.31.2\n";
        assert_eq!(kubernetes_version(first).as_deref(), Some("1.30"));
        // A local replacement has no version; the required one stands.
        let local = format!("{required}\nreplace k8s.io/api => ../api\n");
        assert_eq!(kubernetes_version(&local).as_deref(), Some("1.31"));
        assert_eq!(kubernetes_version("module op\n"), None);
    }
}
