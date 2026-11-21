//! Running another program: [`Project::run`], the one runner of every command a project's
//! config names (its secrets fetched once, a clean env, its budget, every secret masked in
//! what it wrote), and [`read`], which runs one of Fearlezz's own tools to its end or a time
//! budget and turns a failure into what the program said.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::{self, File};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};

/// Env vars every project command gets, so ordinary tools work. Everything else must be
/// declared. The rustup ones choose the toolchain, as PATH does for other tools.
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

/// The project's own tool folders, put in front of PATH when they exist.
pub(crate) const TOOL_DIRS: [&str; 2] = ["node_modules/.bin", ".venv/bin"];

/// Where a project's commands run: from the checkout at `root`, inside the `[env]` `shell`
/// when there is one. Every command Fearlezz runs for a project (checks, envtest, services,
/// the preview, SecretSpec, releases) is built here, so what a command inherits is decided
/// in one place.
#[derive(Clone, Copy)]
pub(crate) struct Project<'a> {
    pub root: &'a Path,
    pub shell: Option<&'a str>,
}

impl<'a> Project<'a> {
    pub(crate) fn new(root: &'a Path, shell: Option<&'a str>) -> Self {
        Project { root, shell }
    }

    /// `script` through `sh -c` inside the shell, in the folder `dir` of the root, with only
    /// [`BASE_ENV`] and the variables `vars` names from Fearlezz's own, and the project's tool
    /// folders in front of PATH. `args` reach the script as its arguments (`"$@"`), whole,
    /// whatever they hold.
    fn clean(&self, dir: Option<&str>, script: &str, vars: &[String], args: &[String]) -> Command {
        let folder = dir.map(|d| self.root.join(d));
        self.shaped(folder.as_deref(), script, Some(vars), args)
    }

    /// `script` at the root, as [`Project::clean`] runs it with no variables of its own: a
    /// question Fearlezz asks the project's environment, such as where its ffmpeg is.
    pub(crate) fn script(&self, script: &str) -> Command {
        self.clean(None, script, &[], &[])
    }

    /// `script` as [`Project::clean`] runs it, but with all of Fearlezz's own variables: only
    /// for asking SecretSpec, whose provider's token must reach it and never a check.
    fn inherited(&self, dir: &Path, script: &str) -> Command {
        self.shaped(Some(dir), script, None, &[])
    }

    /// The command: its line, its folder, its variables (only [`BASE_ENV`] and `vars` when
    /// given, else all of Fearlezz's own) and the variables that carry the script. Inside a
    /// shell the script travels in a variable, so no quoting can break it. A command with a
    /// folder starts there; the shell itself starts at the root, where it finds its own files,
    /// and moves into the folder before the script.
    fn shaped(
        &self,
        folder: Option<&Path>,
        script: &str,
        vars: Option<&[String]>,
        args: &[String],
    ) -> Command {
        let line = match (self.shell, folder) {
            (Some(shell), None) => format!("{shell} sh -c \"$FEARLEZZ_RUN\" sh \"$@\""),
            (Some(shell), Some(_)) => {
                format!("{shell} sh -c 'cd \"$FEARLEZZ_DIR\" && eval \"$FEARLEZZ_RUN\"' sh \"$@\"")
            }
            (None, _) => script.to_string(),
        };
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(&line)
            .arg("sh")
            .args(args)
            .stdin(Stdio::null())
            .current_dir(match (self.shell, folder) {
                (None, Some(folder)) => folder,
                _ => self.root,
            });
        if let Some(vars) = vars {
            cleared(&mut cmd, vars);
        }
        if let Some(path) = self.path(folder) {
            cmd.env("PATH", path);
        }
        if let Some(folder) = folder {
            cmd.env("FEARLEZZ_DIR", folder);
        }
        cmd.env("FEARLEZZ_RUN", script);
        cmd
    }

    /// PATH with the project's tool folders that exist in front, or None when there are none:
    /// those of the command's `folder` first, where a project in a folder keeps its own
    /// packages, then the root's.
    pub(crate) fn path(&self, folder: Option<&Path>) -> Option<OsString> {
        let dirs: Vec<PathBuf> = folder
            .into_iter()
            .chain([self.root])
            .flat_map(|base| TOOL_DIRS.iter().map(move |d| base.join(d)))
            .filter(|d| d.is_dir())
            .collect();
        if dirs.is_empty() {
            return None;
        }
        let rest = std::env::var_os("PATH").unwrap_or_default();
        std::env::join_paths(dirs.into_iter().chain(std::env::split_paths(&rest))).ok()
    }
}

/// Clears `cmd`'s environment down to [`BASE_ENV`] and the variables `vars` names, as
/// Fearlezz has them: all a program the project chose may see, whether a project's command or
/// a tool of the project's that Fearlezz runs itself, such as its ffmpeg. Fearlezz's own
/// secrets (a provider's token, the runner's OIDC request token) never reach it.
pub(crate) fn cleared<'c>(cmd: &'c mut Command, vars: &[String]) -> &'c mut Command {
    cmd.env_clear();
    for name in BASE_ENV
        .iter()
        .copied()
        .chain(vars.iter().map(String::as_str))
    {
        if let Some(value) = std::env::var_os(name) {
            cmd.env(name, value);
        }
    }
    cmd
}

/// How a project's command runs, besides what its config says.
#[derive(Clone, Default)]
pub(crate) struct Call<'a> {
    /// The variables Fearlezz gives it, such as `FEARLEZZ_OUT`.
    pub vars: Vec<(String, OsString)>,
    /// What reaches its script as its arguments (`"$@"`).
    pub args: &'a [String],
    /// Whether it starts outside the `[env]` shell, as the preparation and the services do,
    /// since they start it.
    pub outside: bool,
}

impl Call<'_> {
    /// With the variable `name` set to `value`.
    pub(crate) fn var(mut self, name: &str, value: impl AsRef<OsStr>) -> Self {
        self.vars
            .push((name.to_string(), value.as_ref().to_os_string()));
        self
    }
}

/// A step a caller starts itself, as a check does on its screen.
pub(crate) trait Start {
    /// Starts `cmd`, a step with its folder and variables set, and waits for it to end, or
    /// stops it with what it started at `until`; `last` for the command's last step.
    fn start(&mut self, cmd: Command, last: bool, until: Option<Instant>) -> Result<Exit>;
}

/// Where a project's command's output goes.
pub(crate) enum Output<'a> {
    /// Stdout and stderr appended to the file at this path, masked once it ends. Appended, so
    /// a process it leaves running keeps writing at the end, as do other writers of the file.
    Log(&'a Path),
    /// Stdout kept and returned, masked; stderr appended to the file at this path.
    Read(&'a Path),
    /// Fearlezz's own stdout and stderr as it comes, masked.
    Terminal,
    /// Nowhere.
    Nowhere,
    /// Wherever the caller starting each step sends it, every text file in the folder at this
    /// path masked once it ends.
    Started(&'a mut dyn Start, &'a Path),
}

/// How a project's command ended.
pub(crate) enum Exit {
    /// It ended: whether it passed, and its exit code (none when a signal stopped it): the
    /// last step's, or the first that failed.
    Ended { passed: bool, code: Option<i32> },
    /// It ran out of its budget and was stopped, with what it started.
    OverBudget,
}

impl Exit {
    /// A step that ended with `status`.
    pub(crate) fn of(status: ExitStatus) -> Exit {
        Exit::Ended {
            passed: status.success(),
            code: status.code(),
        }
    }

    /// Whether it passed.
    pub(crate) fn success(&self) -> bool {
        matches!(self, Exit::Ended { passed: true, .. })
    }

    /// Its exit code: none when a signal or its budget stopped it.
    pub(crate) fn code(&self) -> Option<i32> {
        match self {
            Exit::Ended { code, .. } => *code,
            Exit::OverBudget => None,
        }
    }
}

/// A project's command that ran.
pub(crate) struct Ran {
    pub exit: Exit,
    /// What it printed, masked: its stdout for [`Output::Read`], what it wrote into the log
    /// for [`Output::Log`], else nothing.
    pub printed: Vec<u8>,
}

/// Why a project's command did not get its secrets.
pub(crate) enum Unfetched {
    /// SecretSpec could not give this one; its reason is in the log.
    Secret(String),
    /// The shell found no `secretspec` to ask.
    NoSecretSpec,
}

impl Unfetched {
    /// Why a check could not run.
    pub(crate) fn why(self) -> fearlezz_proof::why::Why {
        use fearlezz_proof::why::Why;
        match self {
            Unfetched::Secret(name) => Why::Secret(name),
            Unfetched::NoSecretSpec => Why::MissingTool("secretspec".into()),
        }
    }

    /// Why the preview could not deploy.
    pub(crate) fn preview(self) -> fearlezz_proof::why::NoPreview {
        use fearlezz_proof::why::NoPreview;
        match self {
            Unfetched::Secret(name) => NoPreview::Secret { name },
            Unfetched::NoSecretSpec => NoPreview::NoSecretSpec,
        }
    }
}

/// A command's secrets, fetched once: each name with its value, and every form of every value,
/// longest first, masked in what a command wrote.
#[derive(Default)]
pub(crate) struct Secrets {
    values: Vec<(String, String)>,
    forms: Vec<Vec<u8>>,
}

impl Secrets {
    /// The value of the secret `name`, when it was given.
    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.values
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    fn of(values: Vec<(String, String)>) -> Self {
        Secrets {
            forms: masked_forms(&values),
            values,
        }
    }

    /// Masks every secret in the file at `path`, and returns what was written into it from
    /// byte `from` on, masked. Split before masking, which changes the length of what came
    /// before.
    fn mask_from(&self, path: &Path, from: u64) -> Result<Vec<u8>> {
        let text = fs::read(path)?;
        let (before, own) = text.split_at(usize::try_from(from)?.min(text.len()));
        let own = masked(own, &self.forms);
        if !self.forms.is_empty() {
            let whole = [masked(before, &self.forms), own.clone()].concat();
            if whole != text {
                fs::write(path, whole)?;
            }
        }
        Ok(own)
    }
}

impl Project<'_> {
    /// The secrets `command` names, fetched once, SecretSpec's own words going to `said` (or
    /// Fearlezz's own output without it), giving it `reason`.
    pub(crate) fn secrets(
        &self,
        command: &crate::config::Command,
        reason: &str,
        said: Option<&File>,
    ) -> Result<Result<Secrets, Unfetched>> {
        let mut values = Vec::new();
        for name in &command.secrets {
            match fetch_secret(name, reason, &self.folder(command), self, said)? {
                Ok(value) => values.push((name.clone(), value)),
                Err(unfetched) => return Ok(Err(unfetched)),
            }
        }
        Ok(Ok(Secrets::of(values)))
    }

    /// The secrets `command` names that are given, the others left out, so its tool falls back
    /// to its own login, as a release place's do. A missing one is no error, but one SecretSpec
    /// failed to give is said with SecretSpec's own words, so a broken provider shows; without
    /// SecretSpec nothing is said.
    pub(crate) fn given(&self, command: &crate::config::Command, reason: &str) -> Result<Secrets> {
        let mut values = Vec::new();
        for name in &command.secrets {
            let said = tempfile::NamedTempFile::new()?;
            match fetch_secret(
                name,
                reason,
                &self.folder(command),
                self,
                Some(said.as_file()),
            )? {
                Ok(value) => values.push((name.clone(), value)),
                Err(Unfetched::NoSecretSpec) => {}
                Err(Unfetched::Secret(_)) => {
                    let words = fs::read_to_string(said.path()).unwrap_or_default();
                    println!("{name} not given, so its tool uses its own login");
                    for line in words.lines().filter(|l| !l.trim().is_empty()) {
                        println!("  {line}");
                    }
                }
            }
        }
        Ok(Secrets::of(values))
    }

    /// The folder `command` runs in.
    fn folder(&self, command: &crate::config::Command) -> PathBuf {
        command
            .dir
            .as_ref()
            .map_or_else(|| self.root.to_path_buf(), |d| self.root.join(d))
    }

    /// Runs `command`'s steps in order, the first that fails ending them, with `call`'s
    /// variables and arguments and `secrets`, all of them within its budget, started when the
    /// first step starts. Its output goes to `output`, every secret masked in it, also when a
    /// step fails to start.
    pub(crate) fn run(
        &self,
        command: &crate::config::Command,
        call: &Call<'_>,
        secrets: &Secrets,
        mut output: Output<'_>,
    ) -> Result<Ran> {
        let log = match output {
            Output::Log(path) | Output::Read(path) => {
                Some((path, appended(path)?.metadata()?.len()))
            }
            Output::Terminal | Output::Nowhere | Output::Started(..) => None,
        };
        let mut printed = Vec::new();
        let exit = self.steps(command, call, secrets, &mut output, &mut printed);
        let masked = match (log, &output) {
            (Some((path, from)), _) => secrets.mask_from(path, from).map(|own| {
                if matches!(output, Output::Log(_)) {
                    printed = own;
                }
            }),
            (None, Output::Started(_, dir)) => mask(dir, &secrets.forms),
            (None, _) => Ok(()),
        };
        // A secret left unmasked matters more than why a step did not start.
        masked?;
        let exit = exit?;
        Ok(Ran { exit, printed })
    }

    /// The steps of [`Project::run`], before masking.
    fn steps(
        &self,
        command: &crate::config::Command,
        call: &Call<'_>,
        secrets: &Secrets,
        output: &mut Output<'_>,
        printed: &mut Vec<u8>,
    ) -> Result<Exit> {
        let until = command.timeout.as_ref().map(|b| Instant::now() + b.time);
        let mut exit = None;
        for (at, step) in command.steps.iter().enumerate() {
            let mut cmd = self.step(command, call, secrets, step);
            let ended = match output {
                Output::Log(path) => {
                    let file = appended(path)?;
                    cmd.stdout(file.try_clone()?).stderr(file);
                    wait(&mut cmd, until)?
                }
                Output::Nowhere => {
                    cmd.stdout(Stdio::null()).stderr(Stdio::null());
                    wait(&mut cmd, until)?
                }
                Output::Read(path) => {
                    cmd.stdout(Stdio::piped()).stderr(appended(path)?);
                    let mut child = spawn(&mut cmd, until)?;
                    let out = drain(child.stdout.take());
                    let ended = finish(&mut child, until)?;
                    printed.extend(masked(&out.join().unwrap_or_default(), &secrets.forms));
                    ended
                }
                Output::Terminal => {
                    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
                    let mut child = spawn(&mut cmd, until)?;
                    let out =
                        pass_masked(child.stdout.take(), secrets.forms.clone(), std::io::stdout);
                    let err =
                        pass_masked(child.stderr.take(), secrets.forms.clone(), std::io::stderr);
                    let ended = finish(&mut child, until)?;
                    drop(out.join());
                    drop(err.join());
                    ended
                }
                Output::Started(start, _) => {
                    start.start(cmd, at + 1 == command.steps.len(), until)?
                }
            };
            let passed = ended.success();
            exit = Some(ended);
            if !passed {
                break;
            }
        }
        exit.context("the command has no step to run")
    }

    /// `script`, one of `command`'s steps: through `sh -c` in its folder (inside the `[env]`
    /// shell unless the call starts outside it), in a clean env of [`BASE_ENV`], its
    /// variables, the call's and `secrets`, with the call's arguments.
    fn step(
        &self,
        command: &crate::config::Command,
        call: &Call<'_>,
        secrets: &Secrets,
        script: &str,
    ) -> Command {
        let project = if call.outside {
            Project::new(self.root, None)
        } else {
            *self
        };
        let mut cmd = project.clean(command.dir.as_deref(), script, &command.env, call.args);
        cmd.envs(call.vars.iter().map(|(name, value)| (name, value)));
        cmd.envs(secrets.values.iter().map(|(name, value)| (name, value)));
        cmd
    }
}

/// Runs `cmd`, a step of a project's command, to its end, or stops it with what it started at
/// `until`.
pub(crate) fn wait(cmd: &mut Command, until: Option<Instant>) -> Result<Exit> {
    let mut child = spawn(cmd, until)?;
    finish(&mut child, until)
}

/// Starts `cmd`; with a budget, in a process group of its own, so stopping it stops what it
/// started too.
fn spawn(cmd: &mut Command, until: Option<Instant>) -> Result<Child> {
    #[cfg(unix)]
    if until.is_some() {
        std::os::unix::process::CommandExt::process_group(cmd, 0);
    }
    cmd.spawn().context("starting the command")
}

/// Waits for `child` to end, or stops it with what it started at `until`.
fn finish(child: &mut Child, until: Option<Instant>) -> Result<Exit> {
    let Some(until) = until else {
        return Ok(Exit::of(child.wait()?));
    };
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Exit::of(status));
        }
        if Instant::now() >= until {
            crate::record::stop(child)?;
            return Ok(Exit::OverBudget);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The file at `path` opened to append, made when missing.
pub(crate) fn appended(path: &Path) -> Result<File> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))
}

/// Takes the secret `name` from Fearlezz's own env when it is set there (CI hands its tokens
/// over this way), or else fetches it from SecretSpec, giving it `reason`, outside a command's
/// clean env: in the project's shell and `folder`, so the project's SecretSpec and provider
/// settings apply, and with Fearlezz's own variables, so the provider's token (such as
/// 1Password's `OP_SERVICE_ACCOUNT_TOKEN`) reaches SecretSpec but never the command. One name
/// at a time, since SecretSpec refuses a whole profile when any required secret in it is
/// missing. SecretSpec's own words go to `said`, or Fearlezz's own output without it.
fn fetch_secret(
    name: &str,
    reason: &str,
    folder: &Path,
    project: &Project<'_>,
    said: Option<&File>,
) -> Result<std::result::Result<String, Unfetched>> {
    if let Some(value) = std::env::var(name).ok().filter(|v| !v.is_empty()) {
        return Ok(Ok(value));
    }
    // The value goes to a file of its own, so nothing the shell prints on entering can mix in.
    let get = r#"secretspec get "$FEARLEZZ_SECRET" --reason "$FEARLEZZ_REASON" --caller fearlezz > "$FEARLEZZ_SECRET_FILE""#;
    let file = tempfile::NamedTempFile::new()?;
    let mut cmd = project.inherited(folder, get);
    if let Some(said) = said {
        cmd.stdout(said.try_clone()?).stderr(said.try_clone()?);
    }
    cmd.env("FEARLEZZ_SECRET", name)
        .env("FEARLEZZ_SECRET_FILE", file.path())
        .env("FEARLEZZ_REASON", reason);
    let status = cmd
        .status()
        .with_context(|| format!("asking SecretSpec for {name}"))?;
    if status.code() == Some(crate::run::NOT_FOUND) {
        return Ok(Err(Unfetched::NoSecretSpec));
    }
    if !status.success() {
        return Ok(Err(Unfetched::Secret(name.to_string())));
    }
    let Ok(value) = String::from_utf8(fs::read(file.path())?) else {
        let line = format!("SecretSpec gave {name} as bytes that are not text\n");
        match said {
            Some(mut said) => said.write_all(line.as_bytes())?,
            None => eprint!("{line}"),
        }
        return Ok(Err(Unfetched::Secret(name.to_string())));
    };
    Ok(Ok(value))
}

/// What stands in for a secret's value in what a command wrote.
const MASK: &[u8] = b"***";

/// Values shorter than this are not masked: replacing a word as short as `dev` everywhere
/// would garble every report, and a value that short protects nothing.
const MASKED_FROM: usize = 4;

/// Pictures hold no text a tool printed, and masking would break them.
const UNMASKED: [&str; 9] = [
    "png", "jpg", "jpeg", "gif", "webp", "mp4", "webm", "mov", "mkv",
];

/// Every form a secret's value can take in a command's text: as it is, as JSON escapes it
/// (findings, SARIF, logs), and each of its lines, for a value of several lines (a PEM key)
/// printed line by line.
fn masked_forms(values: &[(String, String)]) -> Vec<Vec<u8>> {
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

/// Replaces every form of a secret's value in the text files at `path`, a file or a folder of
/// them, with [`MASK`].
fn mask(path: &Path, forms: &[Vec<u8>]) -> Result<()> {
    if forms.is_empty() {
        return Ok(());
    }
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            mask(&entry?.path(), forms)?;
        }
        return Ok(());
    }
    let picture = path
        .extension()
        .and_then(|x| x.to_str())
        .is_some_and(|x| UNMASKED.contains(&x.to_ascii_lowercase().as_str()));
    if picture {
        return Ok(());
    }
    let bytes = fs::read(path)?;
    let masked = masked(&bytes, forms);
    if masked != bytes {
        fs::write(path, masked)?;
    }
    Ok(())
}

/// `bytes` with every form of a secret's value replaced by [`MASK`].
fn masked(bytes: &[u8], forms: &[Vec<u8>]) -> Vec<u8> {
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

/// Copies `pipe` into `to` as it comes, masked, on a thread of its own. Only an end that may
/// be the start of a value waits for what follows, so a question without a newline shows.
fn pass_masked<W: std::io::Write + 'static>(
    pipe: Option<impl std::io::Read + Send + 'static>,
    forms: Vec<Vec<u8>>,
    to: fn() -> W,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let Some(mut pipe) = pipe else { return };
        let (mut held, mut chunk) = (Vec::new(), [0u8; 8192]);
        loop {
            let n = pipe.read(&mut chunk).unwrap_or(0);
            held.extend_from_slice(chunk.get(..n).unwrap_or_default());
            let masked = masked(&held, &forms);
            let keep = if n == 0 {
                0
            } else {
                open_start(&masked, &forms)
            };
            let (shown, open) = masked.split_at(masked.len().saturating_sub(keep));
            let mut to = to();
            drop(to.write_all(shown));
            drop(to.flush());
            if n == 0 {
                return;
            }
            held = open.to_vec();
        }
    })
}

/// How many bytes at the end of `bytes` begin one of `forms` without completing it.
fn open_start(bytes: &[u8], forms: &[Vec<u8>]) -> usize {
    let longest = forms.iter().map(Vec::len).max().unwrap_or(0);
    (1..longest.min(bytes.len() + 1))
        .rev()
        .find(|&k| {
            forms
                .iter()
                .any(|f| f.len() > k && f.get(..k).is_some_and(|start| bytes.ends_with(start)))
        })
        .unwrap_or(0)
}

/// Why a program gave no answer.
#[derive(Debug)]
pub(crate) enum Failed {
    /// It did not start, most often because it is not installed.
    Start {
        program: String,
        error: std::io::Error,
    },
    /// It was still running when its budget ran out, and was stopped.
    Late { command: String, budget: Duration },
    /// It ended unsuccessfully; what it printed on stderr, trimmed.
    Exit { command: String, stderr: String },
}

impl fmt::Display for Failed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failed::Start { program, error } => write!(f, "running {program}: {error}"),
            Failed::Late { command, budget } => {
                write!(f, "{command} gave no answer in {budget:?}")
            }
            Failed::Exit { command, stderr } => write!(f, "{command} failed: {stderr}"),
        }
    }
}

impl std::error::Error for Failed {}

/// Runs `cmd` with no input until it ends, or until `budget` when one is given, and returns
/// what it printed on stdout. Both outputs are read while it runs, so a program that prints a
/// lot never blocks on a full pipe.
pub(crate) fn read(cmd: &mut Command, budget: Option<Duration>) -> Result<Vec<u8>, Failed> {
    read_fed(cmd, &[], budget)
}

/// [`read`], with `input` written to the program's stdin first, such as a token that must not
/// show in its arguments.
pub(crate) fn read_fed(
    cmd: &mut Command,
    input: &[u8],
    budget: Option<Duration>,
) -> Result<Vec<u8>, Failed> {
    let stdin = if input.is_empty() {
        Stdio::null()
    } else {
        Stdio::piped()
    };
    let mut child = cmd
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| Failed::Start {
            program: cmd.get_program().to_string_lossy().into_owned(),
            error,
        })?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    if let Some(mut stdin) = child.stdin.take() {
        // On a thread of its own, so a program that prints before it reads all of its input
        // never blocks; one that stops reading early has said what it will say.
        let input = input.to_vec();
        std::thread::spawn(move || drop(stdin.write_all(&input)));
    }
    let status = match budget {
        None => child.wait(),
        Some(budget) => {
            let since = Instant::now();
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => break Ok(status),
                    Ok(None) if since.elapsed() >= budget => {
                        drop(child.kill());
                        drop(child.wait());
                        return Err(Failed::Late {
                            command: shown(cmd),
                            budget,
                        });
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                    Err(e) => break Err(e),
                }
            }
        }
    };
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    match status {
        Ok(status) if status.success() => Ok(stdout),
        Ok(_) => Err(Failed::Exit {
            command: shown(cmd),
            stderr: String::from_utf8_lossy(&stderr).trim().to_string(),
        }),
        Err(error) => Err(Failed::Start {
            program: cmd.get_program().to_string_lossy().into_owned(),
            error,
        }),
    }
}

/// Reads `pipe` to its end on a thread of its own.
fn drain(pipe: Option<impl std::io::Read + Send + 'static>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            drop(pipe.read_to_end(&mut bytes));
        }
        bytes
    })
}

/// The program and its arguments, as a person would type them (without quoting).
fn shown(cmd: &Command) -> String {
    std::iter::once(cmd.get_program())
        .chain(cmd.get_args())
        .map(|a| a.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
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
    fn only_an_end_that_may_start_a_value_waits() {
        let forms = vec![b"s3cret-token".to_vec()];
        assert_eq!(open_start(b"Enter OTP: ", &forms), 0);
        assert_eq!(open_start(b"token is s3cr", &forms), 4);
        assert_eq!(open_start(b"token is s3cret-token", &forms), 0);
        let (one, two) = (b"with s3cr".to_vec(), b"et-token done".to_vec());
        let keep = open_start(&one, &forms);
        let (shown, open) = one.split_at(one.len() - keep);
        let mut rest = open.to_vec();
        rest.extend_from_slice(&two);
        let mut all = shown.to_vec();
        all.extend(masked(&rest, &forms));
        assert_eq!(all, b"with *** done");
    }

    #[test]
    fn what_a_program_prints_is_read() {
        let out = read(Command::new("sh").args(["-c", "echo hi"]), None).unwrap();
        assert_eq!(out, b"hi\n");
    }

    #[test]
    fn a_failure_says_what_the_program_said() {
        let err = read(Command::new("sh").args(["-c", "echo no >&2; exit 3"]), None)
            .unwrap_err()
            .to_string();
        assert_eq!(err, "sh -c echo no >&2; exit 3 failed: no");
    }

    #[test]
    fn a_program_past_its_budget_is_stopped() {
        let since = Instant::now();
        let err = read(
            Command::new("sleep").arg("5"),
            Some(Duration::from_millis(50)),
        )
        .unwrap_err();
        assert!(matches!(err, Failed::Late { .. }), "{err}");
        assert!(since.elapsed() < Duration::from_secs(4));
    }

    #[test]
    fn a_missing_program_says_it_did_not_start() {
        let err = read(&mut Command::new("fearlezz-no-such-tool"), None).unwrap_err();
        assert!(matches!(err, Failed::Start { .. }), "{err}");
    }

    #[test]
    fn a_long_output_never_blocks_the_program() {
        let out = read(
            Command::new("sh").args(["-c", "yes x | head -c 1000000; yes y | head -c 1000000 >&2"]),
            Some(Duration::from_secs(20)),
        )
        .unwrap();
        assert_eq!(out.len(), 1_000_000);
    }
}
