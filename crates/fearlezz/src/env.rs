//! The environment of a checkout: making a fresh one usable, and the services checks need,
//! warm while working, fresh at the final stage, stopped when idle.
//!
//! Fearlezz drives the project's own tools (devenv by preset) through commands; it keeps a
//! small state file per checkout so every command and every journal agrees on what runs.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::config::{Env, Ports};
use crate::run::{self, BASE_ENV};

/// Services unused this long are stopped by the next Fearlezz command of another journal.
pub(crate) const IDLE_SECONDS: i64 = 30 * 60;

/// What runs in one checkout.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Services {
    pub root: PathBuf,
    pub journal: String,
    pub started_at: Timestamp,
    pub last_used: Timestamp,
    pub ports: BTreeMap<String, String>,
    /// The journal whose final stage already reset these services, if any.
    pub fresh_for: Option<String>,
    /// False while up runs: a start that never finished is stopped before the next one.
    #[serde(default = "ready")]
    pub ready: bool,
}

fn ready() -> bool {
    true
}

/// The preview of one checkout: the tree it deployed and where it lives. Kept so a tree is
/// deployed once, whichever check or command needs it first.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Preview {
    pub root: PathBuf,
    pub tree: String,
    pub url: String,
}

/// Why a checkout has no preview, so the checks that need it fail without running.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NoPreview {
    /// The command failed, with its exit code (none when a signal stopped it).
    Failed { code: Option<i32> },
    /// The command passed but printed no URL.
    NoUrl,
    /// SecretSpec could not give it this secret; its reason is in the log.
    Secret { name: String },
    /// The shell found no `secretspec` to ask.
    NoSecretSpec,
    /// The checkout's `[env]` has no preview command, as on a base commit from before it.
    Unset,
}

/// A preview that could not be deployed, and the log that says why.
pub(crate) struct Undeployed {
    pub why: NoPreview,
    pub log: PathBuf,
}

/// The URL of a checkout's preview, and a line to print when it was deployed just now.
pub(crate) struct Deployed {
    pub url: String,
    pub line: Option<String>,
}

/// Everything a command needs to prepare checkouts and run services.
pub(crate) struct EnvCtx<'a> {
    pub cfg: &'a Env,
    pub state: EnvState,
    /// The main checkout, where ignored folders are cloned from.
    pub main: PathBuf,
    pub now: Timestamp,
}

impl EnvCtx<'_> {
    /// The main checkout of a repository whose shared git folder is `common`.
    pub(crate) fn main_checkout(common: &Path, fallback: &Path) -> PathBuf {
        match common.file_name() {
            Some(n) if n == ".git" => common.parent().unwrap_or(fallback).to_path_buf(),
            _ => fallback.to_path_buf(),
        }
    }
}

/// Where Fearlezz keeps environment state for the checkouts of one repository.
pub(crate) struct EnvState {
    dir: PathBuf,
}

impl EnvState {
    pub(crate) fn open(common: &Path) -> EnvState {
        EnvState {
            dir: common.join("fearlezz").join("env"),
        }
    }

    fn key(root: &Path) -> String {
        hex::encode(Sha256::digest(root.to_string_lossy().as_bytes()))[..16].to_string()
    }

    fn services_file(&self, root: &Path) -> PathBuf {
        self.dir
            .join("services")
            .join(format!("{}.json", Self::key(root)))
    }

    fn preview_file(&self, root: &Path) -> PathBuf {
        self.dir
            .join("previews")
            .join(format!("{}.json", Self::key(root)))
    }

    /// The URL of the preview of `tree` in `root`, when it is deployed.
    pub(crate) fn preview_of(&self, root: &Path, tree: &str) -> Option<String> {
        let text = fs::read_to_string(self.preview_file(root)).ok()?;
        let preview: Preview = serde_json::from_str(&text).ok()?;
        (preview.tree == tree).then_some(preview.url)
    }

    fn log(&self, root: &Path, what: &str) -> PathBuf {
        self.dir
            .join("logs")
            .join(format!("{}-{what}.log", Self::key(root)))
    }

    pub(crate) fn services(&self, root: &Path) -> Option<Services> {
        let text = fs::read_to_string(self.services_file(root)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub(crate) fn all_services(&self) -> Vec<Services> {
        let Ok(entries) = fs::read_dir(self.dir.join("services")) else {
            return Vec::new();
        };
        entries
            .filter_map(std::result::Result::ok)
            .filter_map(|e| serde_json::from_str(&fs::read_to_string(e.path()).ok()?).ok())
            .collect()
    }

    fn save(&self, s: &Services) -> Result<()> {
        let file = self.services_file(&s.root);
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(file, serde_json::to_string_pretty(s)?)?;
        Ok(())
    }

    fn forget(&self, root: &Path) -> Result<()> {
        match fs::remove_file(self.services_file(root)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    /// Runs one environment command in `root` with a clean env, all output to its log. A
    /// process it starts in the background keeps writing there without holding Fearlezz up.
    fn sh(&self, env: &Env, root: &Path, what: &str, cmd: &str) -> Result<()> {
        let (mut command, log) = self.command(env, root, what, cmd)?;
        let out = fs::File::create(&log)?;
        command.stdout(out.try_clone()?).stderr(out);
        let status = command
            .status()
            .with_context(|| format!("starting [env] {what}"))?;
        check_status(status, what, root, &log)
    }

    /// Runs the command that prints the ports, returning its stdout; errors go to its log.
    fn sh_read(&self, env: &Env, root: &Path, what: &str, cmd: &str) -> Result<String> {
        let (mut command, log) = self.command(env, root, what, cmd)?;
        command
            .stdout(Stdio::piped())
            .stderr(fs::File::create(&log)?);
        let output = command
            .output()
            .with_context(|| format!("starting [env] {what}"))?;
        check_status(output.status, what, root, &log)?;
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    fn command(&self, env: &Env, root: &Path, what: &str, cmd: &str) -> Result<(Command, PathBuf)> {
        let log = self.log(root, what);
        if let Some(dir) = log.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg(cmd)
            .current_dir(root)
            .env_clear()
            .stdin(Stdio::null());
        for name in BASE_ENV
            .iter()
            .copied()
            .chain(env.vars.iter().map(String::as_str))
        {
            if let Ok(v) = std::env::var(name) {
                command.env(name, v);
            }
        }
        Ok((command, log))
    }

    /// Holds the services of `root` for one command's checks: another command waits, and
    /// idle cleanup leaves them alone. Released when dropped.
    pub(crate) fn hold(&self, root: &Path) -> Result<Hold> {
        let file = self.lock_file(root)?;
        file.lock()?;
        Ok(Hold { _file: file })
    }

    /// Whether a command holds the services of `root` right now.
    fn in_use(&self, root: &Path) -> bool {
        self.lock_file(root)
            .is_ok_and(|f| matches!(f.try_lock(), Err(fs::TryLockError::WouldBlock)))
    }

    fn lock_file(&self, root: &Path) -> Result<fs::File> {
        let path = self.dir.join("locks").join(Self::key(root));
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        Ok(fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)?)
    }

    /// Marks the services of `root` as used at `now`, when a check that needed them ends.
    pub(crate) fn touch(&self, root: &Path, now: Timestamp) -> Result<()> {
        match self.services(root) {
            Some(mut s) => {
                s.last_used = now;
                self.save(&s)
            }
            None => Ok(()),
        }
    }

    // ----- preview --------------------------------------------------------------------------

    /// The URL of the preview of `tree` in `root`, deploying it with `[env] preview` first
    /// when this tree has none yet. The deploy runs at the root, inside the environment's
    /// shell, in a clean env: the base variables, `[env] env`, the `[env] secrets` and
    /// `FEARLEZZ_PREVIEW_NAME` set to `name`. Its URL is the last one it printed; the secrets
    /// are masked in its log before anything reads it.
    pub(crate) fn preview(
        &self,
        env: &Env,
        root: &Path,
        tree: &str,
        name: &str,
    ) -> Result<std::result::Result<Deployed, Undeployed>> {
        let log = self.log(root, "preview");
        let Some(cmd) = &env.preview else {
            return Ok(Err(Undeployed {
                why: NoPreview::Unset,
                log,
            }));
        };
        if let Some(url) = self.preview_of(root, tree) {
            return Ok(Ok(Deployed { url, line: None }));
        }
        if let Some(dir) = log.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut out = fs::File::create(&log)?;
        let clock = std::time::Instant::now();
        let asked = run::fetch_secrets(
            &env.secrets,
            "the preview deploy of Fearlezz",
            root,
            root,
            env.shell.as_deref(),
            &mut out,
        )?;
        let secrets = match asked {
            Ok(secrets) => secrets,
            Err(run::Unfetched::Secret(name)) => {
                return Ok(Err(Undeployed {
                    why: NoPreview::Secret { name },
                    log,
                }));
            }
            Err(run::Unfetched::NoSecretSpec) => {
                return Ok(Err(Undeployed {
                    why: NoPreview::NoSecretSpec,
                    log,
                }));
            }
        };
        // Only what the deploy prints holds its URL, not what SecretSpec said before it.
        let asked_len = out.metadata()?.len();
        // The command travels in a variable, so no quoting inside the shell can break it.
        let script = match &env.shell {
            Some(shell) => format!("{shell} sh -c \"$FEARLEZZ_RUN\""),
            None => cmd.clone(),
        };
        // A clean env like the services' commands, so a deploy from the change's config never
        // sees the provider's token or CI's own; it gets its settings from `[env] env`.
        let (mut command, _) = self.command(env, root, "preview", &script)?;
        command
            .stdout(out.try_clone()?)
            .stderr(out)
            .env("FEARLEZZ_RUN", cmd)
            .env("FEARLEZZ_PREVIEW_NAME", name)
            .envs(secrets.iter().map(|(secret, value)| (secret, value)));
        if let Some(path) = run::project_path(root) {
            command.env("PATH", path);
        }
        let status = command.status().context("starting [env] preview")?;
        run::mask_file(&log, &run::masked_forms(&secrets))?;
        if !status.success() {
            return Ok(Err(Undeployed {
                why: NoPreview::Failed {
                    code: status.code(),
                },
                log,
            }));
        }
        let text = fs::read(&log).unwrap_or_default();
        let printed = text
            .get(usize::try_from(asked_len).unwrap_or(usize::MAX)..)
            .unwrap_or_default();
        let Some(url) = last_url(&String::from_utf8_lossy(printed)) else {
            return Ok(Err(Undeployed {
                why: NoPreview::NoUrl,
                log,
            }));
        };
        let preview = Preview {
            root: root.to_path_buf(),
            tree: tree.to_string(),
            url: url.clone(),
        };
        let file = self.preview_file(root);
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir)?;
        }
        // Written whole and then moved, so a reader never sees half of it.
        let partial = file.with_extension("json.partial");
        fs::write(&partial, serde_json::to_string_pretty(&preview)?)?;
        fs::rename(&partial, &file)?;
        let line = format!(
            "preview deployed: {url} ({:.1} s)",
            clock.elapsed().as_secs_f64()
        );
        Ok(Ok(Deployed {
            url,
            line: Some(line),
        }))
    }

    // ----- preparing a checkout ---------------------------------------------------------

    /// Makes a checkout usable: clones ignored folders from the main checkout when the
    /// lockfiles match, then runs `prepare`. Does nothing when nothing it depends on changed
    /// since the last time. Returns what it did, for a line of output.
    pub(crate) fn prepare(&self, env: &Env, main: &Path, root: &Path) -> Result<Option<String>> {
        if env.prepare.is_none() && env.clone.is_empty() {
            return Ok(None);
        }
        let stamp_file = self.dir.join("prepared").join(Self::key(root));
        let stamp = prepare_stamp(env, root);
        if fs::read_to_string(&stamp_file).is_ok_and(|s| s == stamp) {
            return Ok(None);
        }
        let clock = std::time::Instant::now();
        let mut cloned = Vec::new();
        if root != main && same_files(main, root, &env.clone_when) {
            for rel in expand(main, &env.clone) {
                let (from, to) = (main.join(&rel), root.join(&rel));
                if !to.exists() {
                    if let Err(e) = clone_dir(&from, &to) {
                        // A half copy would be taken as done by every later prepare.
                        if let Err(cleanup) = fs::remove_dir_all(&to)
                            && cleanup.kind() != std::io::ErrorKind::NotFound
                        {
                            return Err(e.context(format!(
                                "cloning {rel} from the main checkout; removing the partial \
                                 copy failed too: {cleanup}"
                            )));
                        }
                        return Err(e.context(format!("cloning {rel} from the main checkout")));
                    }
                    cloned.push(rel);
                }
            }
        }
        if let Some(cmd) = &env.prepare {
            self.sh(env, root, "prepare", cmd)?;
        }
        if let Some(dir) = stamp_file.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(&stamp_file, stamp)?;
        let what = if cloned.is_empty() {
            "prepared".to_string()
        } else {
            format!("prepared (cloned {})", cloned.join(", "))
        };
        Ok(Some(format!(
            "{what}, {:.1} s",
            clock.elapsed().as_secs_f64()
        )))
    }

    // ----- services -----------------------------------------------------------------------

    /// The ports of the services in `root`, starting them first when they do not run.
    /// At the final stage the services restart from reset data once per journal.
    pub(crate) fn ensure_up(
        &self,
        env: &Env,
        root: &Path,
        journal: &str,
        fresh: bool,
        now: Timestamp,
    ) -> Result<(Services, Option<String>)> {
        let up = env.up.as_deref().context("[env] has no up command")?;
        // Services run when the state says so and their ports can still be read; the
        // ports are read again because a restart may have moved them.
        let recorded = self.services(root);
        let unfinished = recorded.as_ref().is_some_and(|s| !s.ready);
        let running = recorded
            .filter(|s| s.ready)
            .and_then(|s| Some((s, self.read_ports(env, root).ok()?)));
        let reuse = running.filter(|(s, _)| !fresh || s.fresh_for.as_deref() == Some(journal));
        if let Some((mut s, ports)) = reuse {
            s.last_used = now;
            s.journal = journal.to_string();
            s.ports = ports;
            self.save(&s)?;
            return Ok((s, None));
        }
        let clock = std::time::Instant::now();
        let mut said = if env.ports.is_some() {
            "services up"
        } else {
            "services started"
        }
        .to_string();
        // Down only what is recorded: down on nothing fails with some tools.
        if unfinished || (fresh && self.services(root).is_some()) {
            self.stop(env, root)?;
        }
        if fresh {
            if let Some(reset) = &env.reset {
                self.sh(env, root, "reset", reset)?;
            }
            said = "services fresh".to_string();
        }
        // Recorded before starting, so services an interrupted start leaves behind are known
        // to down and to idle cleanup.
        self.save(&Services {
            root: root.to_path_buf(),
            journal: journal.to_string(),
            started_at: now,
            last_used: now,
            ports: BTreeMap::new(),
            fresh_for: None,
            ready: false,
        })?;
        if let Err(e) = self.sh(env, root, "up", up) {
            // A failed start can leave services running with no owner.
            return Err(self.stop_after(env, root, e));
        }
        let ports = match self.read_ports(env, root) {
            Ok(p) => p,
            Err(e) => return Err(self.stop_after(env, root, e)),
        };
        let s = Services {
            root: root.to_path_buf(),
            journal: journal.to_string(),
            started_at: now,
            last_used: now,
            ports,
            fresh_for: fresh.then(|| journal.to_string()),
            ready: true,
        };
        self.save(&s)?;
        let ports: Vec<String> = s.ports.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let line = format!(
            "{said} ({:.1} s){}{}",
            clock.elapsed().as_secs_f64(),
            if ports.is_empty() { "" } else { ": " },
            ports.join(" ")
        );
        Ok((s, Some(line)))
    }

    fn read_ports(&self, env: &Env, root: &Path) -> Result<BTreeMap<String, String>> {
        match &env.ports {
            None => Ok(BTreeMap::new()),
            Some(Ports::Command(cmd)) => {
                Ok(parse_assignments(&self.sh_read(env, root, "ports", cmd)?))
            }
            Some(Ports::Devenv) => Ok(parse_devenv_list(&self.sh_read(
                env,
                root,
                "ports",
                "devenv processes list",
            )?)),
        }
    }

    /// Stops the services in `root`, whatever state they are in, and forgets them. When down
    /// fails the record stays, so a later down or idle cleanup tries again.
    pub(crate) fn stop(&self, env: &Env, root: &Path) -> Result<()> {
        if let Some(down) = &env.down {
            self.sh(env, root, "down", down)?;
        }
        self.forget(root)
    }

    /// The error of a failed start, after stopping what it may have left running.
    fn stop_after(&self, env: &Env, root: &Path, failed: anyhow::Error) -> anyhow::Error {
        match self.stop(env, root) {
            Ok(()) => failed,
            Err(e) => failed.context(format!(
                "stopping the services afterwards failed too: {e:#}"
            )),
        }
    }

    /// Stops services other journals left idle for too long, skipping any a running command
    /// holds. Returns each one it tried, with how that went.
    pub(crate) fn stop_idle(
        &self,
        env: &Env,
        current: &Path,
        now: Timestamp,
    ) -> Vec<(Services, Result<Idle>)> {
        let mut stopped = Vec::new();
        for s in self.all_services() {
            let idle = now.as_second() - s.last_used.as_second();
            if s.root == current || idle <= IDLE_SECONDS || self.in_use(&s.root) {
                continue;
            }
            let outcome = if s.root.exists() {
                self.stop(env, &s.root).map(|()| Idle::Stopped)
            } else {
                self.forget(&s.root).map(|()| Idle::Forgotten)
            };
            stopped.push((s, outcome));
        }
        stopped
    }

    /// Stops the services of `root` for `fearlezz setup down`, unless a running command holds them.
    pub(crate) fn stop_unless_in_use(&self, env: &Env, root: &Path) -> Result<()> {
        if self.in_use(root) {
            bail!(
                "the services in {} are in use by a running check; stop it first",
                root.display()
            );
        }
        self.stop(env, root)
    }
}

/// The services of one checkout held by this command; see [`EnvState::hold`].
pub(crate) struct Hold {
    _file: fs::File,
}

/// What stopping idle services did.
pub(crate) enum Idle {
    Stopped,
    /// The checkout is gone, so its down command cannot run; the services may still run.
    Forgotten,
}

fn check_status(
    status: std::process::ExitStatus,
    what: &str,
    root: &Path,
    log: &Path,
) -> Result<()> {
    if status.success() {
        return Ok(());
    }
    let tail = run::tail(log, 6).join("\n  ");
    bail!(
        "[env] {what} failed in {} (exit {}):\n  {tail}\n  log: {}",
        root.display(),
        status.code().map_or("signal".into(), |c| c.to_string()),
        log.display()
    );
}

/// A fingerprint of everything preparing depends on.
fn prepare_stamp(env: &Env, root: &Path) -> String {
    let mut h = Sha256::new();
    h.update(env.prepare.as_deref().unwrap_or("").as_bytes());
    for c in &env.clone {
        h.update(b"\0");
        h.update(c.as_bytes());
    }
    for f in env.clone_when.iter().chain(&env.inputs) {
        h.update(b"\0");
        h.update(f.as_bytes());
        h.update(b"\0");
        h.update(fs::read(root.join(f)).unwrap_or_default());
    }
    hex::encode(h.finalize())
}

/// Whether every file in `files` has the same content in both checkouts.
fn same_files(a: &Path, b: &Path, files: &[String]) -> bool {
    files
        .iter()
        .all(|f| fs::read(a.join(f)).ok() == fs::read(b.join(f)).ok())
}

/// Folders under `root` matching each pattern; `*` matches within one path segment.
fn expand(root: &Path, patterns: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for p in patterns {
        let mut found = vec![String::new()];
        for seg in p.split('/').filter(|s| !s.is_empty()) {
            let matcher = globset::Glob::new(seg).map(|g| g.compile_matcher()).ok();
            let mut next = Vec::new();
            for base in &found {
                let dir = root.join(base);
                if !seg.contains('*') {
                    if dir.join(seg).is_dir() {
                        next.push(join(base, seg));
                    }
                    continue;
                }
                let Ok(entries) = fs::read_dir(&dir) else {
                    continue;
                };
                for e in entries.filter_map(std::result::Result::ok) {
                    let name = e.file_name().to_string_lossy().to_string();
                    if e.path().is_dir() && matcher.as_ref().is_some_and(|m| m.is_match(&name)) {
                        next.push(join(base, &name));
                    }
                }
            }
            found = next;
        }
        out.extend(found.into_iter().filter(|f| !f.is_empty()));
    }
    out.sort();
    out.dedup();
    out
}

fn join(base: &str, name: &str) -> String {
    if base.is_empty() {
        name.to_string()
    } else {
        format!("{base}/{name}")
    }
}

/// Copy-on-write clone of a folder: one clonefile call on macOS, a reflink copy elsewhere.
fn clone_dir(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt as _;
        let src = CString::new(from.as_os_str().as_bytes())?;
        let dst = CString::new(to.as_os_str().as_bytes())?;
        // SAFETY: both are valid, NUL terminated paths for the duration of the call.
        if unsafe { libc::clonefile(src.as_ptr(), dst.as_ptr(), 0) } == 0 {
            return Ok(());
        }
    }
    let status = Command::new("cp")
        .args(if cfg!(target_os = "linux") {
            vec!["-a", "--reflink=auto"]
        } else {
            vec!["-R", "-p"]
        })
        .arg(from)
        .arg(to)
        .status()?;
    if !status.success() {
        bail!("cp {} {} failed", from.display(), to.display());
    }
    Ok(())
}

/// The last `http://` or `https://` URL in `text`: deploy tools print the preview's URL after
/// links to their docs and dashboards. It ends at whitespace, a quote, a bracket or a control
/// character (colour codes), and loses a trailing full stop or comma.
pub(crate) fn last_url(text: &str) -> Option<String> {
    let starts = text
        .match_indices("http://")
        .chain(text.match_indices("https://"))
        .map(|(at, _)| at);
    starts
        .filter_map(|at| {
            let rest = text.get(at..)?;
            let end = rest
                .find(|c: char| c.is_whitespace() || c.is_control() || "\"'<>()[]{}`".contains(c))
                .unwrap_or(rest.len());
            let url = rest.get(..end)?.trim_end_matches(['.', ',', ';', ':']);
            let host = url.split_once("://").map_or("", |(_, h)| h);
            (!host.is_empty()).then(|| (at, url.to_string()))
        })
        .max_by_key(|(at, _)| *at)
        .map(|(_, url)| url)
}

/// The name of a checkout's preview: its branch made safe for a URL (lowercase, each run of
/// anything outside `a-z0-9` one `-`, at most 40 characters, no `-` at either end), or for a
/// detached HEAD, or a branch with nothing left, `commit-<short sha>`.
pub(crate) fn preview_name(branch: Option<&str>, short: &str) -> String {
    let mut name = String::new();
    for c in branch.unwrap_or_default().chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            name.push(c);
        } else if !name.ends_with('-') {
            name.push('-');
        }
    }
    let name: String = name.trim_start_matches('-').chars().take(40).collect();
    let name = name.trim_end_matches('-');
    if name.is_empty() {
        format!("commit-{short}")
    } else {
        name.to_string()
    }
}

/// `NAME=VALUE` lines; anything else is ignored.
pub(crate) fn parse_assignments(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let (k, v) = l.trim().split_once('=')?;
            let k = k.trim();
            (!k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
                .then(|| (k.to_string(), v.trim().to_string()))
        })
        .collect()
}

/// `devenv processes list` lines such as `web   ready restarts: 0 ports: http:8800` become
/// `WEB_HTTP_PORT=8800`.
pub(crate) fn parse_devenv_list(text: &str) -> BTreeMap<String, String> {
    let name = |s: &str| -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect()
    };
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        let Some(process) = words.next() else {
            continue;
        };
        let rest: Vec<&str> = words.collect();
        let Some(at) = rest.iter().position(|w| *w == "ports:") else {
            continue;
        };
        for spec in rest[at + 1..].iter().flat_map(|w| w.split(',')) {
            if let Some((port_name, port)) = spec.split_once(':')
                && port.chars().all(|c| c.is_ascii_digit())
                && !port.is_empty()
            {
                out.insert(
                    format!("{}_{}_PORT", name(process), name(port_name)),
                    port.to_string(),
                );
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devenv_ports_become_env_vars() {
        let list = "web                            ready restarts: 0 ports: http:8800\nworker  ready restarts: 1 ports: http:8791,inspector:9229\ndb  ready restarts: 0\n";
        let ports = parse_devenv_list(list);
        assert_eq!(ports.get("WEB_HTTP_PORT").map(String::as_str), Some("8800"));
        assert_eq!(
            ports.get("WORKER_HTTP_PORT").map(String::as_str),
            Some("8791")
        );
        assert_eq!(
            ports.get("WORKER_INSPECTOR_PORT").map(String::as_str),
            Some("9229")
        );
        assert_eq!(ports.len(), 3);
    }

    #[test]
    fn the_preview_url_is_the_last_one_printed() {
        let log = "see https://docs.example.dev/deploys.\n\u{1b}[32mVersion URL: https://abc.site.dev/\u{1b}[0m\nfinished (http://).";
        assert_eq!(last_url(log).as_deref(), Some("https://abc.site.dev/"));
        assert_eq!(last_url("no url here"), None);
    }

    #[test]
    fn a_preview_name_is_safe_for_a_url() {
        assert_eq!(
            preview_name(Some("Feat/Site_Look--New"), "abc"),
            "feat-site-look-new"
        );
        assert_eq!(preview_name(Some("___"), "abc"), "commit-abc");
        assert_eq!(preview_name(None, "abc"), "commit-abc");
        assert_eq!(
            preview_name(
                Some("feature/a-very-long-branch-name-that-goes-on-and-on"),
                "abc"
            ),
            "feature-a-very-long-branch-name-that-goe"
        );
    }

    #[test]
    fn assignments_skip_noise() {
        let p =
            parse_assignments("starting...\nAPI_PORT=4100\n  DB_URL = postgres://x\nnot a=line\n");
        assert_eq!(p.len(), 2);
        assert_eq!(p["API_PORT"], "4100");
        assert_eq!(p["DB_URL"], "postgres://x");
    }
}
