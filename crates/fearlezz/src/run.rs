//! Running one check: a command that writes its proof into a folder. Fearlezz records the
//! facts the command cannot fake (inputs, tree, exit code, time) itself.

use std::fs::{self, File};
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use jiff::Timestamp;

use fearlezz_proof::why::{Generated, Why};

use crate::config::Check;
use crate::record::Target;
use crate::store::{Inputs, Outcome, RunResult, Store};

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
    /// Why the preview the check needs could not be deployed: then its command never runs.
    pub no_preview: Option<&'a crate::env::Undeployed>,
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
    let log = dir.join(crate::read::LOG);
    let out = File::create(&log)?;

    let started_at = Timestamp::now();
    // Reports from before this moment are another run's, never proof of this one.
    let started = SystemTime::now();
    let clock = Instant::now();
    let generated = if check.generates.patterns().is_empty() || check.builtin.is_some() {
        None
    } else {
        Some(crate::generated::Snapshot::take(at.root, check)?)
    };
    let listed = Listed {
        root: at.root,
        files: std::cell::OnceCell::new(),
    };
    let (ended, mut builtin_found) = start(check, at, &dir, &listed, out)?;
    let (exit_ok, exit_code) = ended.exit();
    let never = ended.never(&log);
    let duration_ms = u64::try_from(clock.elapsed().as_millis()).unwrap_or(u64::MAX);
    let run = crate::read::Run {
        out: &dir,
        root: at.root,
        folder: check.dir.as_deref(),
        since: started,
    };

    let (mut proofs, mut errors) = read_proofs(&dir.join("proof.jsonl"));
    proofs.extend(check.pictures.proofs(&dir));
    let mut passed = exit_ok;
    let mut generated_failed = None;
    if let (true, Some(snapshot)) = (exit_ok, &generated) {
        let diff;
        let files = listed.get().map_err(anyhow::Error::msg)?;
        (generated_failed, diff) = snapshot.compare(files, at.root, check)?;
        if generated_failed.is_some() {
            passed = false;
        }
        if !diff.is_empty() {
            fs::write(dir.join(crate::generated::DIFF), diff)?;
        }
    }
    let (mut findings, mut findings_elsewhere) = (Vec::new(), 0);
    if let Some(spec) = &check.findings {
        // A built in check gives its findings in process, named from the root, deleted files
        // too; a tool's are read from what it wrote.
        let (found, unread) = match builtin_found.take() {
            Some(found) => (found, Vec::new()),
            None => spec.read.read(&run),
        };
        errors.extend(unread);
        let found = spec.levels.apply(&spec.tools, spec.defaults(found));
        let verdict = crate::findings::judge(found, spec.on, at.changed, passed);
        passed = verdict.passed;
        findings = verdict.counted;
        findings_elsewhere = verdict.elsewhere;
    }
    let measured = cover(
        check,
        &listed,
        &run,
        at.changed,
        &mut passed,
        &mut findings,
        &mut errors,
    )?;
    let (tests, cases) = count_tests(check, &run, &mut errors);
    if tests == Some(0) || cases.iter().any(|c| c.failed) {
        passed = false;
    }
    let why = (!passed).then(|| {
        never.unwrap_or_else(|| shown(&findings, generated_failed, tests, exit_code, errors))
    });
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
        findings,
        findings_elsewhere,
        coverage: measured,
        tests,
        cases,
        why,
        before_why: crate::store::BeforeWhy::default(),
        dir,
        log,
    };
    store.save(&result)?;
    Ok(result)
}

/// Runs the check to its end: a built in check in process (with its findings), the command,
/// or nothing when the preview it needs could not deploy.
fn start(
    check: &Check,
    at: &Where<'_>,
    dir: &Path,
    listed: &Listed<'_>,
    out: File,
) -> Result<(Ended, Option<Vec<crate::findings::Finding>>)> {
    if let Some(builtin) = &check.builtin {
        let found = run_builtin(builtin, at, listed, out)?;
        let ended = Ended::Ran {
            passed: true,
            code: Some(0),
        };
        return Ok((ended, Some(found)));
    }
    let ended = if let Some(undeployed) = at.no_preview {
        // The check's log ends as the preview's did, so its failure shows why.
        let mut out = out;
        for line in tail(&undeployed.log, 12) {
            writeln!(out, "{line}")?;
        }
        Ended::Never(Why::Preview(undeployed.why.clone()))
    } else if check.needs.envtest {
        match envtest_assets(check, at, &out)? {
            Ok(assets) => {
                let mut vars = at.vars.clone();
                vars.insert("KUBEBUILDER_ASSETS".into(), assets);
                run_command(check, &Where { vars: &vars, ..*at }, dir, out)?
            }
            Err(code) => Ended::Ran {
                passed: false,
                code,
            },
        }
    } else {
        run_command(check, at, dir, out)?
    };
    Ok((ended, None))
}

/// The check's coverage: its findings join `findings` and its notes the check's log. Coverage
/// adds findings but never excuses a failed run: only a gate of its own fails it, as does a
/// report it cannot read, whose reason goes first in `errors`, as it is why the run failed.
fn cover(
    check: &Check,
    listed: &Listed<'_>,
    run: &crate::read::Run<'_>,
    changed: &crate::findings::Changed,
    passed: &mut bool,
    findings: &mut Vec<crate::findings::Finding>,
    errors: &mut Vec<String>,
) -> Result<Option<crate::coverage::Measured>> {
    let Some(coverage) = &check.coverage else {
        return Ok(None);
    };
    match listed
        .code(check)
        .and_then(|code| coverage.findings(run, changed, &code))
    {
        Ok(m) => {
            *passed &= !m
                .found
                .iter()
                .any(|f| f.level == crate::findings::Level::Gate);
            findings.extend(m.found);
            append(&run.out.join(crate::read::LOG), &m.log)?;
            findings.sort_by(|a, b| (b.level, &a.file, a.line).cmp(&(a.level, &b.file, b.line)));
            Ok(Some(m.measured))
        }
        Err(why) => {
            errors.insert(0, why);
            *passed = false;
            Ok(None)
        }
    }
}

/// How many tests the check ran and each case, a reason going to `errors` when they cannot be
/// counted. A test that passed only on a rerun counts as failed unless the check lists it in
/// `flaky_tests`.
fn count_tests(
    check: &Check,
    run: &crate::read::Run<'_>,
    errors: &mut Vec<String>,
) -> (Option<u64>, Vec<crate::counts::Case>) {
    let mut cases = Vec::new();
    let tests = check.tests.as_ref().map(|reader| match reader.count(run) {
        Ok((n, found)) => {
            cases = found;
            n
        }
        Err(why) => {
            errors.push(why);
            0
        }
    });
    for case in cases.iter_mut().filter(|c| c.flaky) {
        case.failed = !check.flaky_tests.is_match(&case.name);
    }
    (tests, cases)
}

/// One step of a check as a command in a clean env, inside the environment's shell if any.
fn command(
    check: &Check,
    step: &str,
    dir: &Path,
    stdout: File,
    log: &File,
    at: &Where<'_>,
    secrets: &[(String, String)],
) -> Result<Command> {
    let mut cmd = crate::process::Project::new(at.root, at.shell).clean(
        check.dir.as_deref(),
        step,
        &check.env,
        &[],
    );
    cmd.stdin(Stdio::null())
        .stdout(stdout)
        .stderr(log.try_clone()?);
    cmd.env("FEARLEZZ_PORT", free_port()?.to_string());
    cmd.envs(at.vars);
    cmd.envs(secrets.iter().map(|(name, value)| (name, value)));
    // A Fearlezz the check starts runs under this slot instead of waiting for it.
    if crate::queue::holding() {
        cmd.env(crate::queue::HELD, "1");
    }
    cmd.env("FEARLEZZ_OUT", dir)
        .env("FEARLEZZ_CHECK", &check.id)
        .env("FEARLEZZ_STAGE", at.stage);
    Ok(cmd)
}

/// How a check's command ended.
enum Ended {
    /// It ran: whether it passed, and its exit code (none when a signal or Fearlezz stopped
    /// it, or its screen failed).
    Ran { passed: bool, code: Option<i32> },
    /// It never ran, for this reason.
    Never(Why),
}

impl Ended {
    /// Whether it passed, and its exit code.
    fn exit(&self) -> (bool, Option<i32>) {
        match self {
            Ended::Ran { passed, code } => (*passed, *code),
            Ended::Never(_) => (false, None),
        }
    }

    /// What kept its command from running, the shell not finding it (named in `log`) included.
    fn never(self, log: &Path) -> Option<Why> {
        match self {
            Ended::Never(why) => Some(why),
            Ended::Ran {
                code: Some(NOT_FOUND),
                ..
            } => missing_command(log).map(Why::MissingTool),
            Ended::Ran { .. } => None,
        }
    }

    /// A command that ended with `status`.
    fn of(status: std::process::ExitStatus) -> Ended {
        Ended::Ran {
            passed: status.success(),
            code: status.code(),
        }
    }

    /// A command that failed without an exit code.
    const STOPPED: Ended = Ended::Ran {
        passed: false,
        code: None,
    };
}

/// Gets the check's secrets and runs its command with them, masking them in its files before
/// anything reads those, since they are kept, shown and uploaded.
fn run_command(check: &Check, at: &Where<'_>, dir: &Path, mut out: File) -> Result<Ended> {
    let folder = check
        .dir
        .as_ref()
        .map_or_else(|| at.root.to_path_buf(), |d| at.root.join(d));
    let reason = format!("the Fearlezz check {}", check.id);
    let asked = fetch_secrets(
        &check.secrets,
        &reason,
        &folder,
        at.root,
        at.shell,
        &mut out,
    )?;
    Ok(match asked {
        Ok(secrets) => {
            let ended = spawn(check, &secrets, dir, out, at)?;
            mask(dir, &masked_forms(&secrets))?;
            ended
        }
        Err(Unfetched::NoSecretSpec) => Ended::Never(Why::MissingTool("secretspec".into())),
        Err(Unfetched::Secret(name)) => Ended::Never(Why::Secret(name)),
    })
}

/// Runs the check's command in a clean env, inside the environment's shell if any, on the
/// screen it needs: this machine's own, or a Linux display or iOS simulator of Fearlezz's own
/// kept until its last step ends.
fn spawn(
    check: &Check,
    secrets: &[(String, String)],
    dir: &Path,
    mut out: File,
    at: &Where<'_>,
) -> Result<Ended> {
    let run = Run {
        check,
        secrets,
        dir,
        at,
    };
    match &check.needs.screen {
        Target::Window => run.on(None::<crate::record::System>, &[], out),
        Target::Linux { size } => {
            let display = match crate::record::display::Display::start(*size) {
                Ok(d) => d,
                Err(e) => return failed(&mut out, "a Linux display", &e),
            };
            // GTK and others take Wayland when they find it; this display is X.
            let env = [
                ("DISPLAY", display.name()),
                ("GDK_BACKEND", "x11".to_string()),
            ];
            run.on(Some(display), &env, out)
        }
        Target::Ios { device } => {
            let films = check
                .pictures
                .record
                .as_ref()
                .is_some_and(|r| r.film.is_some());
            let simulator = match crate::record::simulator::Simulator::lease(
                device.as_deref(),
                films,
                &mut out,
            ) {
                Ok(s) => s,
                Err(e) => return failed(&mut out, "the iOS simulator", &e),
            };
            let env = [("FEARLEZZ_DEVICE", simulator.udid().to_string())];
            run.on(Some(simulator), &env, out)
        }
    }
}

/// Runs `cmd` to its end, or stops it at `until`, when the check's budget runs out.
fn budgeted(check: &Check, mut cmd: Command, until: Option<Instant>, out: File) -> Result<Ended> {
    let Some(until) = until else {
        let status = cmd
            .status()
            .with_context(|| format!("starting check {}", check.id))?;
        return Ok(Ended::of(status));
    };
    // Its own process group, so stopping it stops what it started too.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = cmd
        .spawn()
        .with_context(|| format!("starting check {}", check.id))?;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Ended::of(status));
        }
        if Instant::now() >= until {
            crate::record::stop(&mut child)?;
            return over_budget(check, out);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Says in the check's log that it ran out of its budget and was stopped; it fails.
fn over_budget(check: &Check, mut log: File) -> Result<Ended> {
    if let Some(budget) = &check.timeout {
        writeln!(
            log,
            "fearlezz: {} exceeded its {budget} budget and was stopped",
            check.id
        )?;
    }
    Ok(Ended::STOPPED)
}

/// Says in the check's log why its screen or recording failed; the check fails.
fn failed(log: &mut File, what: &str, e: &anyhow::Error) -> Result<Ended> {
    writeln!(log, "{what}: {e:#}")?;
    Ok(Ended::STOPPED)
}

/// A check about to run, with what its commands get.
struct Run<'a> {
    check: &'a Check,
    secrets: &'a [(String, String)],
    dir: &'a Path,
    at: &'a Where<'a>,
}

impl Run<'_> {
    /// One step as a command, with what its screen gives it in `env`.
    fn command(
        &self,
        step: &str,
        stdout: File,
        log: &File,
        env: &[(&str, String)],
    ) -> Result<Command> {
        let mut cmd = command(
            self.check,
            step,
            self.dir,
            stdout,
            log,
            self.at,
            self.secrets,
        )?;
        cmd.envs(env.iter().map(|(name, value)| (name, value)));
        Ok(cmd)
    }

    /// Runs its steps on `owned`, a screen of Fearlezz's own kept until the last step ends, or
    /// on this machine's own screen without one; recording the last when the check has
    /// `record`, and stopping it at its budget otherwise.
    fn on<S: crate::record::Screen>(
        self,
        owned: Option<S>,
        env: &[(&str, String)],
        out: File,
    ) -> Result<Ended> {
        let check = self.check;
        // A tool that prints findings on stdout keeps them apart from its log; only the last
        // step's stdout holds them, the steps before it prepare.
        let stdout_path = self.dir.join(crate::read::STDOUT);
        let stdout = match &check.findings {
            Some(f) if f.read.reads_stdout() => File::create(&stdout_path)?,
            _ => out.try_clone()?,
        };
        let Some((last, before)) = check.steps.split_last() else {
            bail!("check {} has no command to run", check.id);
        };
        // One budget for all its steps and its recording, from when the first starts.
        let until = check.timeout.as_ref().map(|b| Instant::now() + b.time);
        for step in before {
            let cmd = self.command(step, out.try_clone()?, &out, env)?;
            let ended = budgeted(check, cmd, until, out.try_clone()?)?;
            if !matches!(ended, Ended::Ran { passed: true, .. }) {
                return Ok(ended);
            }
        }
        let cmd = self.command(last, stdout, &out, env)?;
        let Some(record) = &check.pictures.record else {
            let ran = budgeted(check, cmd, until, out);
            drop(owned);
            return ran;
        };
        match owned {
            Some(screen) => self.recording(cmd, record, screen, until, out),
            None => self.recording(cmd, record, crate::record::System::default(), until, out),
        }
    }

    /// Runs `cmd` while recording `screen`.
    fn recording<S: crate::record::Screen>(
        self,
        mut cmd: Command,
        record: &crate::record::Spec,
        screen: S,
        until: Option<Instant>,
        mut log: File,
    ) -> Result<Ended> {
        let Run { check, dir, at, .. } = self;
        let place = crate::record::Place::of(&check.needs.screen);
        // Its own process group, so stopping it after `seconds` stops the app it started too.
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        // The windows open before the command starts are never recorded.
        let mut recorder = match crate::record::Recorder::new(screen, record, place, until) {
            Ok(r) => r,
            Err(e) => return failed(&mut log, "record", &e),
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
        // Every film of a window, failed ones too, so how often films fail can be counted
        // machine wide.
        let films = recorder.films();
        let noted = if films.is_empty() || place == crate::record::Place::Own {
            Ok(())
        } else {
            crate::files::UserDir::Cache.path().and_then(|dir| {
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
                crate::record::stop(&mut child)?;
                return failed(&mut log, "record", &e);
            }
        };
        let code = match &recorded.ended {
            crate::record::Ended::Exited(status) => status.code(),
            crate::record::Ended::OverBudget => return over_budget(check, log),
            crate::record::Ended::AfterSeconds | crate::record::Ended::WaitedTooLong => None,
        };
        Ok(Ended::Ran {
            passed: recorded.passed(),
            code,
        })
    }
}

/// Why a check or the preview did not get its secrets.
pub(crate) enum Unfetched {
    /// SecretSpec could not give this one; its reason is in the log.
    Secret(String),
    /// The shell found no `secretspec` to ask.
    NoSecretSpec,
}

/// Fetches each secret in `names` from SecretSpec, giving it `reason`, outside a check's clean
/// env: in `shell` (the environment's, started at `root`) and `folder`, so the project's
/// SecretSpec and provider settings apply, and with Fearlezz's own variables, so the
/// provider's token (such as 1Password's `OP_SERVICE_ACCOUNT_TOKEN`) reaches SecretSpec but
/// never the check. One name at a time, since SecretSpec refuses a whole profile when any
/// required secret in it is missing. SecretSpec's own words go to `out`.
pub(crate) fn fetch_secrets(
    names: &[String],
    reason: &str,
    folder: &Path,
    root: &Path,
    shell: Option<&str>,
    out: &mut File,
) -> Result<std::result::Result<Vec<(String, String)>, Unfetched>> {
    // The value goes to a file of its own, so nothing the shell prints on entering can mix in.
    let get = r#"secretspec get "$FEARLEZZ_SECRET" --reason "$FEARLEZZ_REASON" --caller fearlezz > "$FEARLEZZ_SECRET_FILE""#;
    let project = crate::process::Project::new(root, shell);
    let mut secrets = Vec::new();
    for name in names {
        let file = tempfile::NamedTempFile::new()?;
        let mut cmd = project.inherited(folder, get);
        cmd.stdout(out.try_clone()?)
            .stderr(out.try_clone()?)
            .env("FEARLEZZ_SECRET", name)
            .env("FEARLEZZ_SECRET_FILE", file.path())
            .env("FEARLEZZ_REASON", reason);
        let status = cmd
            .status()
            .with_context(|| format!("asking SecretSpec for {name}"))?;
        if status.code() == Some(NOT_FOUND) {
            return Ok(Err(Unfetched::NoSecretSpec));
        }
        if !status.success() {
            return Ok(Err(Unfetched::Secret(name.clone())));
        }
        let Ok(value) = String::from_utf8(fs::read(file.path())?) else {
            writeln!(out, "SecretSpec gave {name} as bytes that are not text")?;
            return Ok(Err(Unfetched::Secret(name.clone())));
        };
        secrets.push((name.clone(), value));
    }
    Ok(Ok(secrets))
}

/// What stands in for a secret's value in a run's files.
const MASK: &[u8] = b"***";

/// Values shorter than this are not masked: replacing a word as short as `dev` everywhere
/// would garble every report, and a value that short protects nothing.
const MASKED_FROM: usize = 4;

/// Pictures hold no text a tool printed, and masking would break them.
const UNMASKED: [&str; 9] = [
    "png", "jpg", "jpeg", "gif", "webp", "mp4", "webm", "mov", "mkv",
];

/// Every form a secret's value can take in a run's text files: as it is, as JSON escapes it
/// (findings, SARIF, logs), and each of its lines, for a value of several lines (a PEM key)
/// printed line by line.
pub(crate) fn masked_forms(values: &[(String, String)]) -> Vec<Vec<u8>> {
    let mut forms: Vec<String> = Vec::new();
    for (_, value) in values {
        forms.push(value.clone());
        let json = serde_json::to_string(value).unwrap_or_default();
        forms.push(json.trim_matches('"').to_string());
        if value.contains('\n') {
            forms.extend(value.lines().map(str::to_string));
        }
    }
    forms.retain(|f| f.len() >= MASKED_FROM);
    // The longest first, so a line never masks part of the whole value before it is matched.
    forms.sort_by_key(|f| std::cmp::Reverse(f.len()));
    forms.dedup();
    forms.into_iter().map(String::into_bytes).collect()
}

/// Replaces every form of a secret's value in the files under `dir` with [`MASK`].
fn mask(dir: &Path, forms: &[Vec<u8>]) -> Result<()> {
    if forms.is_empty() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            mask(&path, forms)?;
            continue;
        }
        let picture = path
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| UNMASKED.contains(&x.to_ascii_lowercase().as_str()));
        if picture {
            continue;
        }
        mask_file(&path, forms)?;
    }
    Ok(())
}

/// Replaces every form of a secret's value in the file at `path` with [`MASK`].
fn mask_file(path: &Path, forms: &[Vec<u8>]) -> Result<()> {
    let bytes = fs::read(path)?;
    let masked = masked(&bytes, forms);
    if masked != bytes {
        fs::write(path, masked)?;
    }
    Ok(())
}

/// `bytes` with every form of a secret's value replaced by [`MASK`].
pub(crate) fn masked(bytes: &[u8], forms: &[Vec<u8>]) -> Vec<u8> {
    forms
        .iter()
        .fold(bytes.to_vec(), |masked, form| replace(&masked, form, MASK))
}

/// `bytes` with every `from` replaced by `to`.
fn replace(bytes: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut rest = bytes;
    while let Some(at) = rest.windows(from.len()).position(|w| w == from) {
        let (before, found) = rest.split_at(at);
        out.extend_from_slice(before);
        out.extend_from_slice(to);
        rest = found.get(from.len()..).unwrap_or_default();
    }
    out.extend_from_slice(rest);
    out
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
    let bin = crate::files::UserDir::Cache.path()?.join("envtest");
    let command = "setup-envtest use \"$FEARLEZZ_K8S\" --bin-dir \"$FEARLEZZ_ENVTEST\" -p path";
    // The check's own variables too, such as a proxy the download needs.
    let mut cmd =
        crate::process::Project::new(at.root, at.shell).clean(None, command, &check.env, &[]);
    cmd.stderr(log.try_clone()?);
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
pub(crate) const NIX_TOOLS: [&str; 11] = [
    "oxlint",
    "prettier",
    "stylua",
    "svelte-check",
    "setup-envtest",
    "secretspec",
    "gitleaks",
    "trufflehog",
    "betterleaks",
    "osv-scanner",
    "zizmor",
];

/// How to make a missing tool reachable for checks.
pub(crate) fn install(tool: &str) -> String {
    let nix = if NIX_TOOLS.contains(&tool) {
        format!(" (in devenv.nix: add `pkgs.{tool}` to packages)")
    } else {
        String::new()
    };
    format!(
        "install {tool} where checks find it: the project's dev dependencies ({}), its devenv{nix}, \
         or PATH",
        crate::process::TOOL_DIRS.join(", ")
    )
}

/// Why a run whose command ran failed, from what it showed, the first that applies: a
/// blocking finding, its generated files, no tests, what its readers could not read
/// (`unread`, the first one the cause), then its exit.
pub(crate) fn shown(
    findings: &[crate::findings::Finding],
    generated: Option<Generated>,
    tests: Option<u64>,
    exit_code: Option<i32>,
    unread: Vec<String>,
) -> Why {
    if let Some(found) = findings
        .iter()
        .find(|x| x.level == crate::findings::Level::Gate)
    {
        return Why::Gate(found.to_string());
    }
    if let Some(failed) = generated {
        return Why::Generated(failed);
    }
    if tests == Some(0) && exit_code == Some(0) {
        return Why::NoTests(unread);
    }
    match (exit_code, unread.into_iter().next()) {
        (Some(0), Some(why)) => Why::Unread(why),
        _ => Why::Exit(exit_code),
    }
}

pub(crate) use fearlezz_proof::why::NOT_FOUND;

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

/// The files git tracks or would track in a checkout, listed once after a check's command
/// ran: its generated files, its coverage and the rules check all read the same list.
struct Listed<'a> {
    root: &'a Path,
    files: std::cell::OnceCell<Result<Vec<String>, String>>,
}

impl Listed<'_> {
    fn get(&self) -> Result<&[String], String> {
        self.files
            .get_or_init(|| {
                crate::git::Git::discover(self.root)
                    .and_then(|git| git.files())
                    .map_err(|e| format!("{e:#}"))
            })
            .as_deref()
            .map_err(Clone::clone)
    }

    /// The project's code: the files git keeps that `check` reads, tests aside.
    fn code(&self, check: &Check) -> Result<std::collections::BTreeSet<String>, String> {
        let files = self.get().map_err(|e| {
            format!("could not list the repository's files to measure coverage: {e}")
        })?;
        Ok(files
            .iter()
            .filter(|p| check.reads(p) && !check.is_test(p))
            .cloned()
            .collect())
    }
}

/// Adds `lines` to the end of the check's log.
fn append(log: &Path, lines: &[String]) -> Result<()> {
    if lines.is_empty() {
        return Ok(());
    }
    let mut file = fs::OpenOptions::new().append(true).open(log)?;
    for line in lines {
        writeln!(file, "{line}")?;
    }
    Ok(())
}

/// Does a built in check's work and gives its findings, saying in its log what it found.
fn run_builtin(
    builtin: &crate::config::Builtin,
    at: &Where<'_>,
    listed: &Listed<'_>,
    mut out: File,
) -> Result<Vec<crate::findings::Finding>> {
    Ok(match builtin {
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
            let files = listed.get().map_err(anyhow::Error::msg)?;
            let found = rules.check(at.root, files, at.changed)?;
            for f in &found {
                writeln!(out, "{f}")?;
            }
            found
        }
    })
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
    all.iter()
        .skip(all.len().saturating_sub(lines))
        .map(ToString::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_is_masked_as_it_is_as_json_escapes_it_and_line_by_line() {
        let value = "first-line\"quoted\nsecond-line";
        let dir = tempfile::tempdir().unwrap();
        let text = format!(
            "raw {value} json {} lines {} {} short ab\n",
            serde_json::to_string(value).unwrap(),
            "first-line\"quoted",
            "second-line"
        );
        fs::write(dir.path().join("log.txt"), &text).unwrap();
        fs::write(dir.path().join("still.png"), value).unwrap();
        let secrets = [
            ("KEY".to_string(), value.to_string()),
            ("SHORT".to_string(), "ab".to_string()),
        ];
        mask(dir.path(), &masked_forms(&secrets)).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("log.txt")).unwrap(),
            "raw *** json \"***\" lines *** *** short ab\n"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("still.png")).unwrap(),
            value,
            "pictures are left whole"
        );
    }

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
