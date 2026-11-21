//! Running one check: a command that writes its proof into a folder. Fearlezz records the
//! facts the command cannot fake (inputs, tree, exit code, time) itself.

use std::fs::{self, File};
use std::io::Write as _;
use std::path::Path;
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use jiff::Timestamp;

use fearlezz_proof::why::{Generated, Why};

use crate::config::Check;
use crate::process::{Call, Exit, Output};
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
    // Appended, as everything else that writes into it.
    let out = crate::process::appended(&log)?;

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
        folder: check.command.dir.as_deref(),
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
        let ended = Ended::Ran(Exit::Ended {
            passed: true,
            code: Some(0),
        });
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
        match envtest_assets(check, at, &out, &dir.join(crate::read::LOG))? {
            Ok(assets) => {
                let mut vars = at.vars.clone();
                vars.insert("KUBEBUILDER_ASSETS".into(), assets);
                run_command(check, &Where { vars: &vars, ..*at }, dir, out)?
            }
            Err(code) => Ended::Ran(Exit::Ended {
                passed: false,
                code,
            }),
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

/// How a check's command ended.
enum Ended {
    /// It ran, and ended so; without an exit code when its screen failed.
    Ran(Exit),
    /// It never ran, for this reason.
    Never(Why),
}

impl Ended {
    /// Whether it passed, and its exit code.
    fn exit(&self) -> (bool, Option<i32>) {
        match self {
            Ended::Ran(exit) => (exit.success(), exit.code()),
            Ended::Never(_) => (false, None),
        }
    }

    /// What kept its command from running, the shell not finding it (named in `log`) included.
    fn never(self, log: &Path) -> Option<Why> {
        match self {
            Ended::Never(why) => Some(why),
            Ended::Ran(exit) if exit.code() == Some(NOT_FOUND) => {
                missing_command(log).map(Why::MissingTool)
            }
            Ended::Ran(_) => None,
        }
    }
}

/// Gets the check's secrets and runs its command with them in a clean env, inside the
/// environment's shell if any, on the screen it needs: this machine's own, or a Linux display
/// or iOS simulator of Fearlezz's own kept until its last step ends. Its secrets are masked in
/// its files before anything reads those, since they are kept, shown and uploaded.
fn run_command(check: &Check, at: &Where<'_>, dir: &Path, mut out: File) -> Result<Ended> {
    let reason = format!("the Fearlezz check {}", check.id);
    let project = crate::process::Project::new(at.root, at.shell);
    let secrets = match project.secrets(&check.command, &reason, Some(&out))? {
        Ok(secrets) => secrets,
        Err(unfetched) => return Ok(Ended::Never(unfetched.why())),
    };
    let mut call = Call::default();
    call.vars.extend(
        at.vars
            .iter()
            .map(|(name, value)| (name.clone(), value.into())),
    );
    // A Fearlezz the check starts runs under this slot instead of waiting for it.
    if crate::queue::holding() {
        call = call.var(crate::queue::HELD, "1");
    }
    let call = call
        .var("FEARLEZZ_OUT", dir)
        .var("FEARLEZZ_CHECK", &check.id)
        .var("FEARLEZZ_STAGE", at.stage);
    match &check.needs.screen {
        Target::Window => {
            Steps::new(check, at, dir, &out, None::<crate::record::System>)?.run(&call, &secrets)
        }
        Target::Linux { size } => {
            let display = match crate::record::display::Display::start(*size) {
                Ok(d) => d,
                Err(e) => return failed(&mut out, "a Linux display", &e).map(Ended::Ran),
            };
            // GTK and others take Wayland when they find it; this display is X.
            let call = call
                .var("DISPLAY", display.name())
                .var("GDK_BACKEND", "x11");
            Steps::new(check, at, dir, &out, Some(display))?.run(&call, &secrets)
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
                Err(e) => return failed(&mut out, "the iOS simulator", &e).map(Ended::Ran),
            };
            let call = call.var("FEARLEZZ_DEVICE", simulator.udid());
            Steps::new(check, at, dir, &out, Some(simulator))?.run(&call, &secrets)
        }
    }
}

/// Says in the check's log why its screen or recording failed; the check fails.
fn failed(log: &mut File, what: &str, e: &anyhow::Error) -> Result<Exit> {
    writeln!(log, "{what}: {e:#}")?;
    Ok(Exit::Ended {
        passed: false,
        code: None,
    })
}

/// A check's steps as they start: their output into its log (the last one's stdout into
/// `stdout`), each with a free `FEARLEZZ_PORT`, the last recording `screen` (a screen of
/// Fearlezz's own kept until the steps end, or this machine's own without one) when the check
/// has `record`.
struct Steps<'a, S> {
    check: &'a Check,
    dir: &'a Path,
    at: &'a Where<'a>,
    stdout: File,
    log: File,
    screen: Option<S>,
}

impl<'a, S: crate::record::Screen> Steps<'a, S> {
    fn new(
        check: &'a Check,
        at: &'a Where<'a>,
        dir: &'a Path,
        log: &File,
        screen: Option<S>,
    ) -> Result<Self> {
        // A tool that prints findings on stdout keeps them apart from its log; only the last
        // step's stdout holds them, the steps before it prepare.
        let stdout = match &check.findings {
            Some(f) if f.read.reads_stdout() => File::create(dir.join(crate::read::STDOUT))?,
            _ => log.try_clone()?,
        };
        Ok(Steps {
            check,
            dir,
            at,
            stdout,
            log: log.try_clone()?,
            screen,
        })
    }

    /// Runs the check's steps with `call` and `secrets`, within its budget, started when the
    /// first step starts; how they ended.
    fn run(mut self, call: &Call<'_>, secrets: &crate::process::Secrets) -> Result<Ended> {
        let (check, dir, at) = (self.check, self.dir, self.at);
        let ran = crate::process::Project::new(at.root, at.shell).run(
            &check.command,
            call,
            secrets,
            Output::Started(&mut self, dir),
        )?;
        // Its log says it ran out of its budget and was stopped; it fails.
        if let (Exit::OverBudget, Some(budget)) = (&ran.exit, &check.command.timeout) {
            writeln!(
                self.log,
                "fearlezz: {} exceeded its {budget} budget and was stopped",
                check.id
            )?;
        }
        Ok(Ended::Ran(ran.exit))
    }
}

impl<S: crate::record::Screen> crate::process::Start for Steps<'_, S> {
    fn start(&mut self, mut cmd: Command, last: bool, until: Option<Instant>) -> Result<Exit> {
        let stdout = if last { &self.stdout } else { &self.log };
        cmd.stdout(stdout.try_clone()?)
            .stderr(self.log.try_clone()?)
            .env("FEARLEZZ_PORT", free_port()?.to_string());
        match &self.check.pictures.record {
            Some(record) if last => match self.screen.take() {
                Some(screen) => self.recording(cmd, record, screen, until),
                None => self.recording(cmd, record, crate::record::System::default(), until),
            },
            _ => crate::process::wait(&mut cmd, until)
                .with_context(|| format!("starting check {}", self.check.id)),
        }
    }
}

impl<S> Steps<'_, S> {
    /// Runs `cmd` while recording `screen`.
    fn recording<R: crate::record::Screen>(
        &self,
        mut cmd: Command,
        record: &crate::record::Spec,
        screen: R,
        until: Option<Instant>,
    ) -> Result<Exit> {
        let (check, dir, at) = (self.check, self.dir, self.at);
        let mut log = self.log.try_clone()?;
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
            crate::record::Ended::OverBudget => return Ok(Exit::OverBudget),
            crate::record::Ended::AfterSeconds | crate::record::Ended::WaitedTooLong => None,
        };
        Ok(Exit::Ended {
            passed: recorded.passed(),
            code,
        })
    }
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
    out: &File,
    log: &Path,
) -> Result<Result<String, Option<i32>>> {
    let folder = check
        .command
        .dir
        .as_ref()
        .map_or(at.root.to_path_buf(), |d| at.root.join(d));
    let go_mod = fs::read_to_string(folder.join("go.mod")).unwrap_or_default();
    let Some(version) = kubernetes_version(&go_mod) else {
        writeln!(
            &*out,
            "envtest: go.mod in {} requires no k8s.io/api, so the Kubernetes version is unknown",
            folder.display()
        )?;
        return Ok(Err(None));
    };
    let bin = crate::files::UserDir::Cache.path()?.join("envtest");
    // The check's own variables too, such as a proxy the download needs.
    let command = crate::config::Command {
        steps: vec![
            "setup-envtest use \"$FEARLEZZ_K8S\" --bin-dir \"$FEARLEZZ_ENVTEST\" -p path".into(),
        ],
        env: check.command.env.clone(),
        ..crate::config::Command::default()
    };
    let call = Call::default()
        .var("FEARLEZZ_K8S", &version)
        .var("FEARLEZZ_ENVTEST", &bin);
    let ran = crate::process::Project::new(at.root, at.shell)
        .run(
            &command,
            &call,
            &crate::process::Secrets::default(),
            Output::Read(log),
        )
        .context("starting setup-envtest")?;
    let assets = String::from_utf8_lossy(&ran.printed).trim().to_string();
    if !ran.exit.success() || assets.is_empty() {
        writeln!(
            &*out,
            "envtest: setup-envtest found no binaries for Kubernetes {version}"
        )?;
        return Ok(Err(ran.exit.code()));
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
            let mut found = settings.drift(at.root)?;
            for f in &found {
                writeln!(out, "{} drifted from the shared settings", f.file)?;
            }
            if found.is_empty() {
                writeln!(out, "every managed file matches the shared settings")?;
            }
            let files = listed.get().map_err(anyhow::Error::msg)?;
            let changed: Vec<&str> = at.changed.paths().collect();
            let grow = settings.grow(at.root, files, &changed);
            for f in &grow {
                writeln!(out, "{}", f.message)?;
            }
            found.extend(grow);
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
