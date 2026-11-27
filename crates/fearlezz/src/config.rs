//! `fearlezz.toml`: the stages a change moves through, the checks that prove it, and the
//! environment that makes a checkout usable and runs its services.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use fearlezz_proof::floor::{Change, Name};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::Deserialize;

use crate::findings::{Finding, Level, Reader, Scope};
use crate::lints::{self, Lints};
use crate::look::MediaGlob;
use crate::presets;
use crate::rules::{self, Rules};
use crate::settings::Settings;

pub(crate) const FILE: &str = "fearlezz.toml";
const DEFAULT_STAGES: [&str; 4] = ["exploring", "working", "look", "merge"];

pub(crate) struct Config {
    pub stages: Stages,
    /// Every check, scheduled or not, by id. [`Config::due`] picks those this run proves.
    pub checks: Vec<Check>,
    /// Whether this run proves the scheduled checks instead of those of each change.
    pub scheduled: bool,
    /// The branch a change is compared against.
    pub base: String,
    pub env: Env,
    /// Tool settings Fearlezz writes into each tool's own file, lint tables included.
    pub settings: Settings,
    /// Globs of files that need no proof, such as prose. A change cannot add to them for
    /// itself: CI reads them from the base branch.
    pub no_proof: Globs,
    /// The proof host CI sends its bundle to, when it is not the default one.
    pub host: Option<String>,
    /// The repository sends nothing to the proof host (`[host] off = true`): neither CI nor a
    /// signed in CLI, and CI never says that its page was not sent.
    pub host_off: bool,
    /// The places a release goes to, `fearlezz run --release`.
    pub releases: Vec<crate::release::Place>,
    /// The machines CI proves the checks on, by name; empty for a project proven in one job.
    pub platforms: Vec<crate::legs::Platform>,
    /// The table it was read from, pieces merged in and checks turned off left out, so a part
    /// of it can be shown as written.
    table: toml::Table,
    /// The project folders of each check with `each`, by its id (`.` the root): a check of a
    /// folder that is no longer a project goes with it, which never weakens the proof.
    pub fanned: BTreeMap<String, Vec<String>>,
}

/// What makes a checkout usable and runs its services. Every part is optional; a project
/// without services needs none of it.
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Env {
    /// Makes a fresh checkout usable (installs, generated files, warming a Nix shell).
    pub prepare: Option<String>,
    /// Files whose change makes `prepare` run again (the Nix files for devenv). With a
    /// `shell`, they are inputs of every check too.
    pub inputs: Vec<String>,
    /// A command prefix that enters the environment; each check runs inside it, as
    /// `<shell> sh -c '<run>'`.
    pub shell: Option<String>,
    /// Ignored folders cloned copy-on-write from the main checkout into a fresh one.
    pub clone: Vec<String>,
    /// Files that must match the main checkout for cloning to be safe (lockfiles).
    pub clone_when: Vec<String>,
    pub up: Option<String>,
    pub down: Option<String>,
    /// Wipes service data, for the fresh run at the final stage.
    pub reset: Option<String>,
    /// Read from `ports` once serde is done, so its error reads as Fearlezz words it.
    #[serde(skip)]
    pub ports: Option<Ports>,
    #[serde(rename = "ports")]
    written_ports: Option<RawPorts>,
    /// Environment variables the commands above may see, shown as they are.
    #[serde(rename = "env")]
    pub vars: Vec<String>,
    /// Deploys the checkout somewhere remote and prints its URL: a service that lives
    /// remotely, deployed once per tree.
    pub preview: Option<String>,
    /// Secrets the preview gets from SecretSpec, by name, as a check gets its own.
    pub secrets: Vec<String>,
}

impl Env {
    /// One of its commands: at the root, with its variables.
    pub(crate) fn command(&self, script: &str) -> Command {
        Command {
            steps: vec![script.to_string()],
            env: self.vars.clone(),
            ..Command::default()
        }
    }

    /// What of it shapes a check's results besides its files, part of the inputs of each
    /// check it touches.
    pub(crate) fn key(&self) -> EnvKey {
        let services = self.up.as_ref().map(|up| {
            format!(
                "{up}\0{}\0{}\0{:?}",
                self.down.as_deref().unwrap_or(""),
                self.reset.as_deref().unwrap_or(""),
                self.ports
            )
        });
        let preview = self.preview.as_ref().map(|preview| {
            let mut names = self.secrets.clone();
            names.sort();
            format!("{preview}\0{}", names.join("\0"))
        });
        EnvKey {
            shell: self.shell.clone(),
            services,
            preview,
        }
    }
}

/// What of an `[env]` shapes a check's results besides its files.
#[derive(Clone, Default)]
pub(crate) struct EnvKey {
    /// The shell every check but a built in one runs inside.
    pub shell: Option<String>,
    /// The commands that make the services, for a check that needs them.
    pub services: Option<String>,
    /// The command that deploys the preview and the names of the secrets it gets, for a check
    /// that needs it.
    pub preview: Option<String>,
}

/// Where the ports of running services come from.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Ports {
    /// A command printing `NAME=VALUE` lines.
    Command(String),
    /// `devenv processes list`, read by Fearlezz.
    Devenv,
}

/// `ports = "<command>"`, or `ports = { read = "devenv" }` for a reader Fearlezz knows.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawPorts {
    Command(String),
    Read { read: String },
}

impl RawPorts {
    fn read(self) -> Result<Ports> {
        match self {
            RawPorts::Command(cmd) => Ok(Ports::Command(cmd)),
            RawPorts::Read { read } if read == "devenv" => Ok(Ports::Devenv),
            RawPorts::Read { read } => {
                bail!("[env] ports: no reader \"{read}\"; readers are devenv")
            }
        }
    }
}

/// A word in a check's `needs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Need {
    Services,
    Envtest,
    Preview,
    Ios,
    Linux,
}

/// What a check needs besides its files, read once from its `needs` with its `device` or
/// `display`.
#[derive(Default)]
pub(crate) struct Needs {
    /// The services `[env] up` starts; their ports reach the check as env vars.
    pub services: bool,
    /// The envtest binaries (etcd, kube-apiserver) of the Kubernetes version its go.mod
    /// requires, fetched by setup-envtest; their folder reaches it as `KUBEBUILDER_ASSETS`.
    pub envtest: bool,
    /// The deploy `[env] preview` makes of the checkout; its URL reaches the check as
    /// `FEARLEZZ_PREVIEW_URL`.
    pub preview: bool,
    /// Where its app shows: this machine's own screen, or the iOS simulator (`ios`) or Linux
    /// display (`linux`) it names, with the device type or display size it asks for.
    pub screen: crate::record::Target,
}

/// The ordered stages of a change. The last one is the merge gate, where every check runs
/// fresh once.
pub(crate) struct Stages(Vec<String>);

/// A position in [`Stages`]. Only [`Stages`] creates one, so it is always valid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Stage(usize);

/// A command the project's config names: what it runs, where, and what it may see. Checks,
/// `[env]`, release places and envtest each fill one, and [`crate::process::Project::run`]
/// runs every one of them the same way.
#[derive(Clone, Debug, Default)]
pub(crate) struct Command {
    /// The plain commands it runs in order, the first that fails ending it.
    pub steps: Vec<String>,
    /// The folder of the repository it runs in; the root without one.
    pub dir: Option<String>,
    /// Variables it sees from Fearlezz's own, shown as they are.
    pub env: Vec<String>,
    /// Secrets it gets by name, from Fearlezz's own env or else SecretSpec, masked in
    /// everything it wrote.
    pub secrets: Vec<String>,
    /// How long it may run, all its steps together, before Fearlezz stops it.
    pub timeout: Option<crate::legs::Budget>,
}

#[derive(Default)]
pub(crate) struct Check {
    pub id: String,
    /// What it runs: its steps (empty for a built in check; a reader of stdout reads only the
    /// last one's), its folder (from `each` or written), its variables (their values part of
    /// its inputs), its secrets (their names part of its inputs, never their values) and its
    /// budget.
    pub command: Command,
    pub stage: Stage,
    /// What it reads, the key its results are reused by: the files `inputs` names, or
    /// without them every file outside `no_proof`. Only a speed hint; it never decides
    /// whether the check is needed.
    inputs: Globs,
    /// Too slow, costly or reliant on real infrastructure to run for each change: only a
    /// scheduled run (`fearlezz run --ci --scheduled`) or naming it runs it.
    pub schedule: bool,
    pub needs: Needs,
    /// For a check that reports findings: where they come from and which count.
    pub findings: Option<FindingsSpec>,
    /// For a check that runs tests: where it says how many ran. Zero tests fails it.
    pub tests: Option<crate::counts::Reader>,
    /// For a check that measures coverage while it tests: where the report is, and the
    /// `[lints.coverage]` table that gives its findings their level.
    pub coverage: Option<crate::coverage::Coverage>,
    /// Which of its inputs are tests, as written: CI runs the change's test files on the base
    /// commit to show they fail there (red before green).
    pub test_files: Globs,
    /// The stills and films it takes, and which of them are flaky.
    pub pictures: Pictures,
    /// Tests (names or globs, as written) that may pass only on a rerun. Listing more weakens
    /// the proof.
    pub flaky_tests: Globs,
    /// The files its command writes into the repository, relative to the folder it runs in:
    /// they must come out the same as the tree had them.
    pub generates: Globs,
    /// Work Fearlezz does itself instead of running a command.
    pub builtin: Option<Builtin>,
    /// The platforms it must pass on, by name: as written, or the default one. Empty in a
    /// project without platforms.
    pub on: Vec<String>,
    /// The CI job it runs in on each platform, with the other checks of that job; without one,
    /// the platform's shared job.
    pub job: Option<String>,
    /// Quick enough never to wait in the machine's heavy queue (`light = true`).
    pub light: bool,
}

/// What pictures and texts a check takes, in one place: the check's inputs, the floor and
/// whether a run needs ffmpeg all read this, so a new field is added here once.
#[derive(Default)]
pub(crate) struct Pictures {
    /// For a check that writes pictures into a folder: which files in it are stills and
    /// which are films.
    pub stills: Option<MediaGlob>,
    pub films: Option<MediaGlob>,
    /// For a check that writes text into a folder (rendered manifests, a plan): which files
    /// in it are texts, compared line by line.
    pub texts: Option<MediaGlob>,
    /// For a check that records an app's window while its command runs.
    pub record: Option<crate::record::Spec>,
    /// Pictures (ids or globs, as written) that are compared and shown but never wait for a
    /// person's look.
    pub flaky: Globs,
}

impl Pictures {
    /// Whether the check names pictures it takes, by glob or by recording a window. A check
    /// can also list pictures in `proof.jsonl` without naming them here.
    pub(crate) fn named(&self) -> bool {
        self.names_media() || self.texts.is_some()
    }

    /// Whether it names stills or films, by glob or by recording a window: what needs ffmpeg.
    pub(crate) fn names_media(&self) -> bool {
        self.stills.is_some() || self.films.is_some() || self.record.is_some()
    }

    /// The proofs for the files under `out` that the globs name.
    pub(crate) fn proofs(&self, out: &Path) -> Vec<serde_json::Value> {
        self.stills
            .iter()
            .chain(&self.films)
            .chain(&self.texts)
            .flat_map(|g| g.proofs(out))
            .collect()
    }

    /// Whether the picture named `name` is flaky, by `flaky`.
    pub(crate) fn is_flaky(&self, name: &str) -> bool {
        self.flaky.is_match(name)
    }

    /// Which pictures a run takes, part of the check's inputs. `flaky` is not: it decides how
    /// pictures are judged, never what a run records.
    pub(crate) fn taken(&self) -> Option<impl fmt::Debug + '_> {
        self.named().then(|| {
            (
                self.stills.as_ref().map(MediaGlob::glob),
                self.films.as_ref().map(MediaGlob::glob),
                self.record.as_ref().map(crate::record::Spec::fingerprint),
                self.texts.as_ref().map(MediaGlob::glob),
            )
        })
    }

    /// How `head` takes fewer pictures than these, the base branch's: a glob gone or changed
    /// (Fearlezz cannot tell a narrower glob from a wider one), a recording gone or recording
    /// less, and more pictures called flaky.
    pub(crate) fn weakenings(&self, head: &Pictures) -> Vec<Change> {
        let mut found = Vec::new();
        for (kind, b, h) in [
            ("stills", &self.stills, &head.stills),
            ("films", &self.films, &head.films),
            ("texts", &self.texts, &head.texts),
        ] {
            match (
                b.as_ref().map(MediaGlob::glob),
                h.as_ref().map(MediaGlob::glob),
            ) {
                (Some(b), None) => found.push(format!("it no longer takes {kind} by {b}").into()),
                (Some(b), Some(h)) if b != h => {
                    found.push(Change::replaced(
                        format!("it takes {kind} by another glob"),
                        b,
                        h,
                    ));
                }
                _ => {}
            }
        }
        if let Some(b) = &self.record {
            match &head.record {
                None => found.push("it no longer records its app's window".into()),
                Some(h) => found.extend(record_weakenings(b, h)),
            }
        }
        found.extend(head.flaky.widened(&self.flaky, "pictures"));
        found
    }
}

/// How the recording `h` records less than `b`: a still or film gone or renamed, another
/// window, less time, longer to wait for its window, or more cut from the top.
fn record_weakenings(b: &crate::record::Spec, h: &crate::record::Spec) -> Vec<Change> {
    let mut found = Vec::new();
    for (kind, was, now) in [("still", &b.still, &h.still), ("film", &b.film, &h.film)] {
        if let Some(was) = was.as_ref().filter(|w| now.as_ref() != Some(*w)) {
            found.push(format!("it no longer records the {kind} {was}").into());
        }
    }
    if b.window != h.window {
        let name = |w: &Option<String>| {
            w.clone()
                .unwrap_or_else(|| "the app its command starts".into())
        };
        found.push(Change::replaced(
            "it records another window",
            name(&b.window),
            name(&h.window),
        ));
    }
    let less = "it records for less time";
    match (b.seconds, h.seconds) {
        (Some(was), Some(now)) if now < was => {
            found.push(Change::replaced(
                less,
                format!("{was} s"),
                format!("{now} s"),
            ));
        }
        (None, Some(now)) => {
            found.push(Change::replaced(
                less,
                "until its command ends",
                format!("{now} s"),
            ));
        }
        _ => {}
    }
    let wait =
        |s: &crate::record::Spec| s.wait.unwrap_or(crate::record::DEFAULT_WAIT.as_secs_f64());
    if wait(h) > wait(b) {
        found.push(Change::replaced(
            "it waits longer for its window",
            format!("{} s", wait(b)),
            format!("{} s", wait(h)),
        ));
    }
    if h.crop_top > b.crop_top {
        found.push(Change::replaced(
            "it cuts more from the top",
            format!("{} points", b.crop_top),
            h.crop_top,
        ));
    }
    found
}

impl Needs {
    /// What `c` needs, its screen with the `record` it takes there.
    fn read(c: &RawCheck, record: Option<&crate::record::Spec>) -> Result<Needs> {
        use crate::record::Target;
        let has = |need| c.needs.contains(&need);
        let (ios, linux) = (has(Need::Ios), has(Need::Linux));
        if ios && linux {
            bail!("needs both ios and linux; a check runs on one machine, so split it in two");
        }
        if c.device.is_some() && !ios {
            bail!("device is only for a check that needs = [\"ios\"]");
        }
        if c.display.is_some() && !linux {
            bail!("display is only for a check that needs = [\"linux\"]");
        }
        let screen = if ios {
            Target::Ios {
                device: c.device.clone(),
            }
        } else if linux {
            let size = match &c.display {
                Some(text) => crate::record::parse_size(text).with_context(|| {
                    format!("display = \"{text}\" is not a width and height such as \"1280x720\"")
                })?,
                None => crate::record::DEFAULT_SIZE,
            };
            Target::Linux { size }
        } else {
            Target::Window
        };
        if screen.owned() && record.is_some_and(|r| r.crop_top > 0.0) {
            bail!(
                "record: crop_top is only for a window; the simulator and a Linux display have no title bar"
            );
        }
        Ok(Needs {
            services: has(Need::Services),
            envtest: has(Need::Envtest),
            preview: has(Need::Preview),
            screen,
        })
    }
}

/// What this binary ships (`src/shipped.rs`): its code and the files a built in check compares
/// against decide the check's verdict, and every development build says the same version, so
/// results shared across worktrees are keyed on this rather than the version.
pub(crate) const SHIPPED: &str = env!("FEARLEZZ_SHIPPED");

/// A check Fearlezz runs in process.
#[derive(Clone, Debug)]
pub(crate) enum Builtin {
    /// Fails when a tool's file drifted from the shared settings.
    Settings(Settings),
    /// The project's `[rule]` table.
    Rules(Rules),
    /// `[commits]`: the change's commit messages.
    Commits(crate::commits::Commits),
}

impl Builtin {
    /// A stable text of what it checks, part of its inputs beside [`SHIPPED`].
    pub(crate) fn fingerprint(&self) -> String {
        match self {
            Builtin::Settings(s) => format!("settings {}", s.fingerprint()),
            Builtin::Rules(r) => format!("rules {}", r.fingerprint()),
            Builtin::Commits(c) => format!("commits {}", c.fingerprint()),
        }
    }

    /// Files it reads whether or not git lists them.
    pub(crate) fn files(&self) -> Vec<String> {
        match self {
            Builtin::Settings(s) => s.files(),
            Builtin::Rules(_) | Builtin::Commits(_) => Vec::new(),
        }
    }
}

/// The id of the check that keeps tool files in line with the shared settings.
pub(crate) const SETTINGS_CHECK: &str = "settings";

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FindingsSpec {
    pub read: Reader,
    pub on: Scope,
    /// The `[lints]` tables that give its findings their levels, first match wins.
    pub tools: Vec<lints::Tool>,
    /// Those tables.
    pub levels: Lints,
    /// The level of every finding, for a tool that reports none (a scanner whose SARIF has
    /// no levels). The tables above can still move a rule.
    pub level: Option<Level>,
    /// What to do about a finding the tool gave no fix for.
    pub fix: Option<String>,
    /// Rules, as the tool reports them, that test code may break: the lints clippy's
    /// `allow-*-in-tests` settings name, for a check that reads clippy's table.
    pub in_tests: BTreeSet<String>,
}

impl FindingsSpec {
    /// Gives findings the level and fix this check declares, before the lint tables.
    pub(crate) fn defaults(&self, mut found: Vec<Finding>) -> Vec<Finding> {
        for f in &mut found {
            if let Some(level) = self.level {
                f.level = level;
            }
            if f.fix.is_none() {
                f.fix.clone_from(&self.fix);
            }
        }
        found
    }

    /// Part of the inputs of the check: another reader, scope or default is another proof.
    /// The lint tables are a part of their own.
    pub(crate) fn judged(&self) -> impl fmt::Debug + '_ {
        (&self.read, self.on, self.level, &self.fix, &self.in_tests)
    }
}

/// `findings = "cargo"`, or
/// `findings = { read = "cargo", on = "everywhere", lints = ["clippy", "rustc"] }`, and for a
/// scanner whose results carry no level, `level = "gate"` and `fix = "..."`.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawFindings {
    Read(Reader),
    Spec {
        read: Reader,
        #[serde(default)]
        on: Scope,
        #[serde(default)]
        lints: Vec<String>,
        level: Option<Level>,
        fix: Option<String>,
    },
}

impl RawFindings {
    /// The spec it says, its lint tables taken from `all`.
    fn resolve(self, all: &Lints) -> Result<FindingsSpec> {
        let (read, on, lints, level, fix) = match self {
            RawFindings::Read(read) => (read, Scope::Changed, Vec::new(), None, None),
            RawFindings::Spec {
                read,
                on,
                lints,
                level,
                fix,
            } => (read, on, lints, level, fix),
        };
        let tools = lints
            .iter()
            .map(|t| lints::Tool::parse(t))
            .collect::<Result<Vec<_>>>()
            .context("findings lints")?;
        Ok(FindingsSpec {
            read,
            on,
            levels: all.only(&tools),
            tools,
            level,
            fix,
            // Config::load fills it from the settings.
            in_tests: BTreeSet::new(),
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    stages: Option<RawStages>,
    #[serde(default)]
    check: BTreeMap<String, RawCheck>,
    base: Option<String>,
    env: Option<Env>,
    settings: Option<toml::Table>,
    lints: Option<BTreeMap<String, BTreeMap<String, lints::Level>>>,
    rule: Option<BTreeMap<String, toml::Table>>,
    commits: Option<crate::commits::RawCommits>,
    #[serde(default)]
    no_proof: Vec<String>,
    host: Option<RawHost>,
    release: Option<toml::Value>,
    #[serde(default)]
    platform: BTreeMap<String, crate::legs::RawPlatform>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHost {
    url: Option<String>,
    #[serde(default)]
    off: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStages {
    order: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCheck {
    run: RawRun,
    stage: String,
    dir: Option<String>,
    #[serde(default)]
    inputs: Vec<String>,
    #[serde(default)]
    env: Vec<String>,
    #[serde(default)]
    secrets: Vec<String>,
    #[serde(default)]
    schedule: bool,
    #[serde(default)]
    needs: Vec<Need>,
    /// The simulator's device type, with `needs = ["ios"]`.
    device: Option<String>,
    /// The Linux display's width and height, such as "1280x720", with `needs = ["linux"]`.
    display: Option<String>,
    findings: Option<RawFindings>,
    tests: Option<toml::Value>,
    coverage: Option<toml::Value>,
    #[serde(default)]
    test_files: Vec<String>,
    stills: Option<String>,
    films: Option<String>,
    texts: Option<String>,
    #[serde(default)]
    flaky: Vec<String>,
    #[serde(default)]
    flaky_tests: Vec<String>,
    #[serde(default)]
    light: bool,
    record: Option<toml::Value>,
    #[serde(default)]
    on: Vec<String>,
    job: Option<String>,
    timeout: Option<String>,
    #[serde(default)]
    generates: Vec<String>,
}

/// A check's `run`: one command, or steps run in order, so a check that needs two
/// commands (`terraform init`, then `terraform validate`) needs no shell glue.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawRun {
    One(String),
    Steps(Vec<String>),
}

impl Config {
    /// Reads `fearlezz.toml` in `root`, with the pieces it uses merged in (cached under
    /// `common`, the git folder every worktree shares).
    pub(crate) fn load(root: &Path, common: &Path) -> Result<Config> {
        let table = presets::resolve(root, common)?;
        let at = || format!("in {}", root.join(FILE).display());
        let no_proof: Vec<String> = table
            .get("no_proof")
            .and_then(toml::Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let no_proof = globs(&no_proof).with_context(at)?;
        let files = crate::git::Git {
            root: root.to_path_buf(),
            common: common.to_path_buf(),
        }
        .files()?;
        let found = |marker: &str| projects(root, &files, marker, &no_proof);
        Config::from_table(table, &found).with_context(at)
    }

    /// The config `table`, pieces merged in, says, with `found` the project folders of each
    /// marker file. One pass expands the table first: `each` fans checks out, what is turned
    /// off goes, and each release place adds its dry run.
    fn from_table(mut table: toml::Table, found: &dyn Fn(&str) -> Vec<String>) -> Result<Config> {
        let fanned = expand_each(&mut table, found)?;
        let off = drop_off(&mut table)?;
        crate::release::add_dry_checks(&mut table, DEFAULT_STAGES.last().copied(), &off);

        let raw: RawConfig = toml::Value::Table(table.clone()).try_into()?;
        let order = match raw.stages {
            Some(s) => s.order,
            None => DEFAULT_STAGES.iter().map(ToString::to_string).collect(),
        };
        if order.is_empty() {
            bail!("[stages] order needs at least one stage");
        }
        let stages = Stages(order);
        let mut env = raw.env.unwrap_or_default();
        env.ports = env.written_ports.take().map(RawPorts::read).transpose()?;
        if let Some(name) = env.secrets.iter().find(|n| replaces_given(n)) {
            bail!(
                "[env] secrets: the secret {name} would replace a variable the preview gets; \
                 give it another name in secretspec.toml"
            );
        }
        let lints = Lints::parse(raw.lints.unwrap_or_default())?;
        // Without inputs of its own, a check reads every file outside `no_proof`, so an edit
        // to prose reruns nothing: `no_proof` taken back out of `**`, in its own order.
        let whole_tree: Vec<String> = std::iter::once("**".to_string())
            .chain(raw.no_proof.iter().map(|p| match p.strip_prefix('!') {
                Some(back) => back.to_string(),
                None => format!("!{p}"),
            }))
            .collect();
        let platforms = crate::legs::Platform::parse(raw.platform)?;
        let around = Around {
            stages: &stages,
            env: &env,
            whole_tree: &whole_tree,
            platforms: &platforms,
            lints: &lints,
        };
        let mut checks = Vec::new();
        for (id, c) in raw.check {
            if c.needs.contains(&Need::Services) && env.up.is_none() {
                bail!("check {id} needs services, but [env] has no up command to start them");
            }
            if c.needs.contains(&Need::Preview) && env.preview.is_none() {
                bail!(
                    "check {id} needs the preview, but [env] has no preview command to deploy it"
                );
            }
            let check = c.resolve(&around).with_context(|| format!("check {id}"))?;
            checks.push(Check { id, ..check });
        }
        let read: BTreeSet<lints::Tool> = checks
            .iter()
            .filter_map(|c| c.findings.as_ref())
            .flat_map(|f| f.tools.iter().copied())
            .collect();
        let mut settings = Settings::parse(raw.settings.unwrap_or_default(), lints, &read)?;
        let in_tests = settings.clippy_in_tests();
        for spec in checks.iter_mut().filter_map(|c| c.findings.as_mut()) {
            if spec.tools.contains(&lints::Tool::Clippy) {
                spec.in_tests.clone_from(&in_tests);
            }
        }
        settings.cargo = found("Cargo.toml");
        settings.biome_runs = checks.iter().any(|c| {
            c.command.steps.iter().any(|step| {
                step.split_whitespace()
                    .any(|word| word == "biome" || word.ends_with("/biome"))
            })
        });
        settings.grow = crate::grow::Have::of(&checks);
        raw.host
            .as_ref()
            .and_then(|h| h.url.as_deref())
            .unwrap_or(crate::host::DEFAULT)
            .trim_end_matches('/')
            .clone_into(&mut settings.host);
        let rules = Rules::parse(raw.rule.unwrap_or_default())?;
        let base = raw.base.unwrap_or_else(|| "main".into());
        let built_in = built_in(&settings, rules, raw.commits, &base, &stages, &platforms)?;
        for check in built_in.into_iter().filter(|c| !off.contains(&c.id)) {
            match &check.builtin {
                _ if !checks.iter().any(|c| c.id == check.id) => checks.push(check),
                // A project's own settings check stands in for the built in one.
                Some(Builtin::Settings(_)) => {}
                Some(Builtin::Rules(_)) => {
                    bail!(
                        "check {} is the name of the check [rule] forms; rename it",
                        check.id
                    )
                }
                _ => bail!(
                    "check {} is the name of the check [commits] forms; rename it",
                    check.id
                ),
            }
        }
        checks.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(Config {
            no_proof: globs(&raw.no_proof).context("no_proof")?,
            settings,
            stages,
            checks,
            scheduled: false,
            base,
            env,
            host_off: raw.host.as_ref().is_some_and(|h| h.off),
            host: raw.host.and_then(|h| h.url),
            releases: crate::release::parse(raw.release.as_ref())?,
            platforms,
            table,
            fanned,
        })
    }

    /// The part at `path` (`check.tests`, `no_proof`, `lints.ruff.E501`) as TOML, nested
    /// under its own path, so a table keeps its header: `[check.tests]` with its fields, or
    /// `no_proof = [...]`. None when the config has no such part. A key may hold dots itself.
    pub(crate) fn part(&self, path: &str) -> Option<String> {
        fn find<'a>(table: &'a toml::Table, path: &str) -> Option<(Vec<&'a str>, &'a toml::Value)> {
            table.iter().find_map(|(key, value)| {
                if path == key {
                    return Some((vec![key.as_str()], value));
                }
                let rest = path.strip_prefix(key.as_str())?.strip_prefix('.')?;
                let (mut keys, found) = find(value.as_table()?, rest)?;
                keys.insert(0, key.as_str());
                Some((keys, found))
            })
        }
        let (keys, value) = find(&self.table, path)?;
        let nested = keys.iter().rev().fold(value.clone(), |inner, key| {
            toml::Value::Table(toml::Table::from_iter([(key.to_string(), inner)]))
        });
        let toml::Value::Table(nested) = nested else {
            return None;
        };
        toml::to_string(&nested).ok()
    }

    /// Whether a change to `path` needs proof: `no_proof` does not declare it.
    pub(crate) fn needs_proof(&self, path: &str) -> bool {
        !self.no_proof.is_match(path)
    }

    /// Whether `check` takes pictures: it names them, or by convention runs in the look stage
    /// and lists them in `proof.jsonl`. Such a check is compared with the base even when a
    /// run took none, so pictures that are all gone still show, and it needs ffmpeg.
    pub(crate) fn takes_pictures(&self, check: &Check) -> bool {
        check.pictures.named() || self.stages.name(check.stage) == "look"
    }

    /// Whether `check` needs ffmpeg: it names stills or films, or runs in the look stage, where
    /// it may list them in `proof.jsonl`. A check that names only texts needs none.
    pub(crate) fn needs_ffmpeg(&self, check: &Check) -> bool {
        check.pictures.names_media() || self.stages.name(check.stage) == "look"
    }

    /// The check called `id`, scheduled or not: naming a check runs it.
    pub(crate) fn check(&self, id: &str) -> Result<&Check> {
        self.checks.iter().find(|c| c.id == id).with_context(|| {
            let ids: Vec<&str> = self.checks.iter().map(|c| c.id.as_str()).collect();
            format!("no check \"{id}\"; checks are {}", ids.join(", "))
        })
    }

    /// The checks this run proves: those of each change, or on a scheduled run the scheduled
    /// ones.
    pub(crate) fn due(&self) -> impl Iterator<Item = &Check> {
        self.checks.iter().filter(|c| c.schedule == self.scheduled)
    }
}

/// Whether a secret named `name` would replace a variable Fearlezz itself gives every command.
pub(crate) fn replaces_given(name: &str) -> bool {
    name.starts_with("FEARLEZZ_") || crate::process::BASE_ENV.contains(&name)
}

/// The checks Fearlezz runs itself, at the working stage: the shared settings', `[rule]`'s
/// and `[commits]`'s, each where the config has them.
fn built_in(
    settings: &Settings,
    rules: Rules,
    commits: Option<crate::commits::RawCommits>,
    base: &str,
    stages: &Stages,
    platforms: &[crate::legs::Platform],
) -> Result<Vec<Check>> {
    // Its findings judged in `on`.
    let builtin = |id: &str, inputs: Globs, on: Scope, builtin: Builtin| Check {
        on: platforms
            .iter()
            .filter(|p| p.default)
            .map(|p| p.name.clone())
            .collect(),
        id: id.into(),
        stage: stages.find("working").unwrap_or(stages.first()),
        inputs,
        findings: Some(FindingsSpec {
            read: Reader::Jsonl,
            on,
            tools: Vec::new(),
            levels: Lints::default(),
            in_tests: BTreeSet::new(),
            level: None,
            fix: None,
        }),
        builtin: Some(builtin),
        light: true,
        ..Check::default()
    };
    let mut built_in = Vec::new();
    if !settings.is_empty() {
        built_in.push(builtin(
            SETTINGS_CHECK,
            globs(&settings.globs())?,
            Scope::Everywhere,
            Builtin::Settings(settings.clone()),
        ));
    }
    if !rules.is_empty() {
        // Names, companions and code can change with any file.
        built_in.push(builtin(
            rules::CHECK,
            globs(&["**".to_string()])?,
            Scope::Changed,
            Builtin::Rules(rules),
        ));
    }
    if let Some(commits) = commits {
        // What it reads is the messages, which its inputs hold; no file.
        built_in.push(builtin(
            crate::commits::CHECK,
            globs(&[])?,
            Scope::Everywhere,
            Builtin::Commits(crate::commits::Commits::parse(commits, base)),
        ));
    }
    Ok(built_in)
}

/// What every check of a config is read against.
struct Around<'a> {
    stages: &'a Stages,
    env: &'a Env,
    /// The inputs of a check that names none.
    whole_tree: &'a [String],
    platforms: &'a [crate::legs::Platform],
    lints: &'a Lints,
}

impl RawCheck {
    /// The check this table says, its id left for the caller to set; an error names no
    /// check, since the caller puts its id in front.
    fn resolve(mut self, at: &Around<'_>) -> Result<Check> {
        let stage = at.stages.find(&self.stage).with_context(|| {
            format!(
                "unknown stage \"{}\"; stages are {}",
                self.stage,
                at.stages.0.join(", ")
            )
        })?;
        let record = self
            .record
            .take()
            .map(crate::record::Spec::parse)
            .transpose()?;
        let needs = Needs::read(&self, record.as_ref())?;
        let steps = match self.run {
            RawRun::One(command) => vec![command],
            RawRun::Steps(steps) if steps.is_empty() => {
                bail!("run names no command; give it one, or a list of steps")
            }
            RawRun::Steps(steps) => steps,
        };
        if let Some(name) = self.secrets.iter().find(|n| replaces_given(n)) {
            bail!(
                "the secret {name} would replace a variable every check gets; give it another \
                 name in secretspec.toml"
            );
        }
        let own = if self.inputs.is_empty() {
            at.whole_tree
        } else {
            &self.inputs
        };
        // Inside a shell, the environment's own files shape every result.
        let shell_inputs = at.env.shell.iter().flat_map(|_| &at.env.inputs);
        let media =
            |kind, glob: Option<String>| glob.map(|g| MediaGlob::parse(kind, &g)).transpose();
        Ok(Check {
            inputs: globs(own.iter().chain(shell_inputs)).context("inputs")?,
            on: crate::legs::Platform::on(at.platforms, self.on, needs.screen.os())?,
            job: crate::legs::job(self.job)?,
            tests: self
                .tests
                .as_ref()
                .map(crate::counts::Reader::parse)
                .transpose()?,
            coverage: crate::coverage::Coverage::parse(self.coverage.as_ref(), at.lints)?,
            test_files: globs(&self.test_files).context("test_files")?,
            pictures: Pictures {
                stills: media("still", self.stills)?,
                films: media("film", self.films)?,
                texts: media("text", self.texts)?,
                record,
                flaky: globs(&self.flaky).context("flaky")?,
            },
            flaky_tests: globs(&self.flaky_tests).context("flaky_tests")?,
            generates: globs(&self.generates).context("generates")?,
            findings: self.findings.map(|f| f.resolve(at.lints)).transpose()?,
            light: self.light,
            command: Command {
                steps,
                dir: self
                    .dir
                    .map(|dir| folder(&dir))
                    .transpose()
                    .context("dir")?,
                env: self.env,
                secrets: self.secrets,
                timeout: self
                    .timeout
                    .as_deref()
                    .map(crate::legs::Budget::parse)
                    .transpose()
                    .context("timeout")?,
            },
            stage,
            schedule: self.schedule,
            needs,
            ..Check::default()
        })
    }
}

/// Leaves out what the merged pieces turn off with `off = true`: checks, rules, release places
/// and `[commits]`, and drops `off = false`, so nothing after reads the key. Returns the ids of
/// the checks turned off, which a check added later (a release's dry run, a built in check)
/// never comes back under.
fn drop_off(table: &mut toml::Table) -> Result<BTreeSet<String>> {
    fn off(part: &mut toml::Value, at: &str) -> Result<bool> {
        let Some(part) = part.as_table_mut() else {
            return Ok(false);
        };
        match part.remove("off") {
            None => Ok(false),
            Some(toml::Value::Boolean(off)) => Ok(off),
            Some(other) => bail!(
                "in `{at}`: off is {}, expected true or false",
                other.type_str()
            ),
        }
    }
    let mut dropped = std::collections::BTreeSet::new();
    for kind in ["check", "rule", "release"] {
        if let Some(toml::Value::Table(parts)) = table.get_mut(kind) {
            let mut gone = Vec::new();
            for (id, part) in parts.iter_mut() {
                if off(part, &format!("{kind}.{id}"))? {
                    gone.push(id.clone());
                }
            }
            for id in gone {
                parts.remove(&id);
                if kind == "check" {
                    dropped.insert(id);
                }
            }
        }
    }
    if let Some(commits) = table.get_mut("commits")
        && off(commits, "commits")?
    {
        table.remove("commits");
    }
    Ok(dropped)
}

/// Paths by a list of globs. `*` stays inside one path segment; `**` crosses segments. A
/// glob with a leading `!` takes back what earlier globs matched, and a later glob can match
/// it again: the last glob that matches a path decides, as in `.gitignore`.
#[derive(Default)]
pub(crate) struct Globs {
    /// As written, in order.
    patterns: Vec<String>,
    set: GlobSet,
}

impl Globs {
    /// Globs that match nothing.
    pub(crate) fn none() -> Globs {
        Globs {
            patterns: Vec::new(),
            set: GlobSet::empty(),
        }
    }

    pub(crate) fn is_match(&self, path: &str) -> bool {
        self.set
            .matches(path)
            .into_iter()
            .max()
            .and_then(|last| self.patterns.get(last))
            .is_some_and(|p| !p.starts_with('!'))
    }

    pub(crate) fn patterns(&self) -> &[String] {
        &self.patterns
    }

    /// How these globs, a list of flaky `what`, call more of them flaky than `base`, the base
    /// branch's: one line naming each glob added or `!` glob dropped, or the new order.
    pub(crate) fn widened(&self, base: &Globs, what: &str) -> Option<Change> {
        Change::names(format!("it calls more {what} flaky"), self.widenings(base))
    }

    /// How these globs match paths that `base` does not, by a sound rule over the lists as
    /// written: dropping a positive glob or putting in a `!` glob anywhere never matches more,
    /// and globs of one kind side by side mean the same in any order. Anything else may match
    /// more, and is named: positive globs they add, `!` globs they drop, or, with neither, the
    /// new order. Not [`fearlezz_proof::floor::names_diff`]: a dropped positive glob or an added
    /// `!` glob matches less, so it is not named.
    pub(crate) fn widenings(&self, base: &Globs) -> Vec<Name> {
        let (b, h) = (&base.patterns, &self.patterns);
        if narrows_to(b, h) {
            return Vec::new();
        }
        let positive = |p: &&String| !p.starts_with('!');
        let mut found: Vec<Name> = h
            .iter()
            .filter(positive)
            .filter(|p| !b.contains(p))
            .map(|p| Name::Added(p.clone()))
            .collect();
        found.extend(
            b.iter()
                .filter(|p| !positive(p))
                .filter(|p| !h.contains(p))
                .map(|p| Name::Dropped(p.clone())),
        );
        if found.is_empty() {
            found.push(Name::Reordered);
        }
        found
    }
}

/// Only the globs as written, so a key built from it stays the same across globset versions.
impl fmt::Debug for Globs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(&self.patterns).finish()
    }
}

/// Whether `head` comes from `base` by dropping positive globs and putting in `!` globs only,
/// where each run of one kind in `base` (globs side by side that all are, or all are not, `!`)
/// may come in any order.
fn narrows_to(base: &[String], head: &[String]) -> bool {
    let runs: Vec<&[String]> = base
        .chunk_by(|a, b| a.starts_with('!') == b.starts_with('!'))
        .collect();
    let narrowing = Narrowing { runs: &runs, head };
    let start = vec![false; runs.first().map_or(0, |r| r.len())];
    narrowing.from(0, 0, 0, start, &mut std::collections::HashSet::new())
}

struct Narrowing<'a> {
    runs: &'a [&'a [String]],
    head: &'a [String],
}

impl Narrowing<'_> {
    /// Whether `head[j..]` comes from the runs from `r` on, run `r` entered at `head[entry]`
    /// with its members in `taken` matched since. A head glob equal to a member not yet matched
    /// is matched at once: putting in a `!` glob or dropping a positive one is allowed anyway,
    /// so matching later never does better. The search then branches only on leaving a run,
    /// and `taken` follows from `(r, entry, j)`, which is what `failed` remembers.
    fn from(
        &self,
        r: usize,
        entry: usize,
        j: usize,
        mut taken: Vec<bool>,
        failed: &mut std::collections::HashSet<(usize, usize, usize)>,
    ) -> bool {
        let rest = self.head.get(j..).unwrap_or_default();
        let Some(run) = self.runs.get(r) else {
            return rest.iter().all(|p| p.starts_with('!'));
        };
        if failed.contains(&(r, entry, j)) {
            return false;
        }
        let negated = run.first().is_some_and(|p| p.starts_with('!'));
        let h = rest.first();
        let same = h.and_then(|h| run.iter().zip(&taken).position(|(p, done)| !done && p == h));
        let ok = if let Some(i) = same {
            if let Some(t) = taken.get_mut(i) {
                *t = true;
            }
            self.from(r, entry, j + 1, taken, failed)
        } else {
            // Positive globs left in a run may be dropped; `!` globs may not.
            let leave = (!negated || taken.iter().all(|t| *t)) && {
                let next = vec![false; self.runs.get(r + 1).map_or(0, |r| r.len())];
                self.from(r + 1, j, j, next, failed)
            };
            leave
                || (h.is_some_and(|h| h.starts_with('!'))
                    && self.from(r, entry, j + 1, taken, failed))
        };
        if !ok {
            failed.insert((r, entry, j));
        }
        ok
    }
}

/// A folder of the repository as a check names it: relative, without `.` segments, never
/// leaving the repository; `.` alone is the root, which needs no folder.
fn folder(dir: &str) -> Result<String> {
    let parts: Vec<&str> = dir
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    if dir.starts_with('/') || parts.contains(&"..") || parts.is_empty() {
        bail!("\"{dir}\": name a folder inside the repository, relative to its root");
    }
    Ok(parts.join("/"))
}

/// The project folders `files` show: each folder holding a file named `marker`, outside
/// `no_proof`, and not inside another such folder (a project's own crates or packages are its
/// business). A Cargo.toml is a project unless a workspace above it names it a member, as
/// Cargo decides, so a workspace of its own below the root is one. `.` is the root.
pub(crate) fn projects(
    root: &Path,
    files: &[String],
    marker: &str,
    no_proof: &Globs,
) -> Vec<String> {
    let suffix = format!("/{marker}");
    let mut found: Vec<String> = files
        .iter()
        .filter(|f| !no_proof.is_match(f))
        .filter_map(|f| {
            if f == marker {
                Some(".".to_string())
            } else {
                f.strip_suffix(&suffix).map(String::from)
            }
        })
        .collect();
    found.sort();
    found.dedup();
    let all = found.clone();
    let above = |o: &String, f: &String| o != f && (o == "." || f.starts_with(&format!("{o}/")));
    if marker == "Cargo.toml" {
        found.retain(|f| {
            !all.iter()
                .any(|o| above(o, f) && crate::settings::workspace_has(root, o, f))
        });
    } else {
        found.retain(|f| !all.iter().any(|o| above(o, f)));
    }
    found
}

/// Each check with `each` made one check per project folder `found` names for its marker
/// file: `<id>` for the root, `<id>/<folder>` running in that folder otherwise. A check the
/// project writes under that name tunes the one folder. Without a project folder, the check
/// stays at the root, where its tool says what is missing. Its `inputs` stay with the root:
/// they name the root project's files, so a folder's check reads every file until its own
/// table names them. Returns the folders of each, by id.
fn expand_each(
    table: &mut toml::Table,
    found: &dyn Fn(&str) -> Vec<String>,
) -> Result<BTreeMap<String, Vec<String>>> {
    let mut fanned = BTreeMap::new();
    let Some(toml::Value::Table(checks)) = table.get_mut("check") else {
        return Ok(fanned);
    };
    let templates: Vec<String> = checks
        .iter()
        .filter(|(_, c)| c.get("each").is_some())
        .map(|(id, _)| id.clone())
        .collect();
    for id in templates {
        let Some(toml::Value::Table(mut check)) = checks.remove(&id) else {
            continue;
        };
        let Some(toml::Value::String(marker)) = check.remove("each") else {
            bail!(
                "check {id}: each names the file that marks a project folder, such as \"Cargo.toml\""
            );
        };
        let mut folders = found(&marker);
        if folders.is_empty() {
            folders.push(".".into());
        }
        for dir in &folders {
            let mut one = check.clone();
            let name = if dir == "." {
                id.clone()
            } else {
                one.insert("dir".into(), toml::Value::String(dir.clone()));
                one.remove("inputs");
                format!("{id}/{dir}")
            };
            if let Some(toml::Value::Table(own)) = checks.remove(&name) {
                presets::merge(&mut one, own);
            }
            checks.insert(name, toml::Value::Table(one));
        }
        // A table left for a folder that is no longer a project tunes nothing.
        let prefix = format!("{id}/");
        let stale = checks.iter().find(|(name, c)| {
            name.strip_prefix(&prefix)
                .is_some_and(|dir| !folders.iter().any(|f| f == dir))
                && c.get("run").is_none()
        });
        if let Some((name, _)) = stale {
            bail!(
                "[check.\"{name}\"] tunes the {id} check of {}, which has no {marker}: remove the table, or put the project back",
                name.strip_prefix(&prefix).unwrap_or(name)
            );
        }
        fanned.insert(id, folders);
    }
    Ok(fanned)
}

fn globs<'a>(patterns: impl IntoIterator<Item = &'a String>) -> Result<Globs> {
    let mut set = GlobSetBuilder::new();
    let mut written = Vec::new();
    for p in patterns {
        let glob = p.strip_prefix('!').unwrap_or(p);
        set.add(GlobBuilder::new(glob).literal_separator(true).build()?);
        written.push(p.clone());
    }
    Ok(Globs {
        patterns: written,
        set: set.build()?,
    })
}

impl Check {
    /// What it runs, as shown: its one command, its steps one per line, or the work Fearlezz
    /// does itself.
    pub(crate) fn run(&self) -> String {
        if self.builtin.is_some() {
            format!("(fearlezz {})", self.id)
        } else {
            self.command.steps.join("\n")
        }
    }

    /// Whether its results depend on `path`: one of its inputs, or a file it generates,
    /// which it compares with what its command writes.
    pub(crate) fn reads(&self, path: &str) -> bool {
        self.inputs.is_match(path) || self.generated(path)
    }

    /// Whether `path`, from the root, is a file its `generates` names.
    pub(crate) fn generated(&self, path: &str) -> bool {
        match &self.command.dir {
            Some(dir) => path
                .strip_prefix(dir.as_str())
                .and_then(|p| p.strip_prefix('/'))
                .is_some_and(|p| self.generates.is_match(p)),
            None => self.generates.is_match(path),
        }
    }

    /// Whether `path` is one of its tests, by `test_files`.
    pub(crate) fn is_test(&self, path: &str) -> bool {
        self.test_files.is_match(path)
    }
}

impl Stages {
    pub(crate) fn find(&self, name: &str) -> Option<Stage> {
        self.0.iter().position(|s| s == name).map(Stage)
    }

    pub(crate) fn first(&self) -> Stage {
        Stage(0)
    }

    pub(crate) fn last(&self) -> Stage {
        Stage(self.0.len() - 1)
    }

    pub(crate) fn after(&self, stage: Stage) -> Option<Stage> {
        (stage.0 + 1 < self.0.len()).then_some(Stage(stage.0 + 1))
    }

    pub(crate) fn name(&self, stage: Stage) -> &str {
        &self.0[stage.0]
    }

    pub(crate) fn all(&self) -> impl Iterator<Item = Stage> + '_ {
        (0..self.0.len()).map(Stage)
    }
}

impl fmt::Display for Stages {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_devenv_piece_runs_services_and_reads_their_ports() {
        let table = toml::from_str(include_str!("../presets/devenv.toml")).unwrap();
        let env = Config::from_table(table, &|_| Vec::new()).unwrap().env;
        assert_eq!(env.up.as_deref(), Some("devenv up -d"));
        assert_eq!(env.ports, Some(Ports::Devenv));
        assert_eq!(env.inputs, ["devenv.nix", "devenv.yaml", "devenv.lock"]);
        assert_eq!(env.shell.as_deref(), Some("devenv shell --"));
    }

    #[test]
    fn every_built_in_piece_is_a_valid_config() {
        for (name, piece) in presets::BUILTIN {
            let parse = |name: &str, text: &str| -> toml::Table {
                toml::from_str(text).unwrap_or_else(|e| panic!("{name}: {e}"))
            };
            let mut own = parse(name, piece);
            // Merged over the built in pieces it builds on, as `use` does.
            let mut table = toml::Table::new();
            if let Some(toml::Value::Array(used)) = own.remove("use") {
                for used in used.iter().filter_map(toml::Value::as_str) {
                    let (_, text) = presets::BUILTIN
                        .iter()
                        .find(|(n, _)| *n == used)
                        .unwrap_or_else(|| panic!("{name} uses {used}"));
                    let mut base = parse(used, text);
                    base.remove("use");
                    presets::merge(&mut table, base);
                }
            }
            presets::merge(&mut table, own);
            // As in a project with a nested project of each marker, so `each` is read too.
            let found = |_: &str| vec![".".to_string(), "sub".to_string()];
            if let Err(e) = Config::from_table(table, &found) {
                panic!("{name}: {e:#}");
            }
        }
    }

    #[test]
    fn a_long_run_of_negations_is_compared_quickly() {
        let mut base = vec!["src/**".to_string()];
        base.extend((0..40).map(|i| format!("!src/x{i}/**")));
        let globs = |list: &[String]| globs(list).unwrap();
        let started = std::time::Instant::now();
        let dropped = &base[..base.len() - 1];
        assert_eq!(
            globs(dropped).widenings(&globs(&base)),
            [Name::Dropped("!src/x39/**".into())]
        );
        let mut reversed = base.clone();
        reversed[1..].reverse();
        assert_eq!(globs(&reversed).widenings(&globs(&base)), []);
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn a_check_turned_off_is_gone() {
        let table = toml::from_str("[check.tests]\noff = true\n").unwrap();
        let checks = Config::from_table(table, &|_| Vec::new()).unwrap().checks;
        assert!(!checks.iter().any(|c| c.id == "tests"));
    }
}
