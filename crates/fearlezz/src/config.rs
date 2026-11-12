//! `fearlezz.toml`: the stages a change moves through, the checks that prove it, and the
//! environment that makes a checkout usable and runs its services.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
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
    pub checks: Vec<Check>,
    /// The branch a change is compared against.
    pub base: String,
    pub env: Env,
    /// Tool settings Fearlezz writes into each tool's own file, lint tables included.
    pub settings: Settings,
    /// Globs of files that need no proof, such as prose. A change cannot add to them for
    /// itself: CI reads them from the base branch.
    pub no_proof: Globs,
}

/// What makes a checkout usable and runs its services. Every part is optional; a project
/// without services needs none of it.
#[derive(Default)]
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
    pub ports: Option<Ports>,
    /// Environment variables the commands above may see.
    pub vars: Vec<String>,
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

/// What a check needs besides its files.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Need {
    /// The services `[env] up` starts; their ports reach the check as env vars.
    Services,
}

/// The ordered stages of a change. The last one is the merge gate, where every check runs
/// fresh once.
pub(crate) struct Stages(Vec<String>);

/// A position in [`Stages`]. Only [`Stages`] creates one, so it is always valid.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Stage(usize);

pub(crate) struct Check {
    pub id: String,
    pub run: String,
    pub stage: Stage,
    /// What it reads, the key its results are reused by: the files `inputs` names, or
    /// without them every file outside `no_proof`. Only a speed hint; it never decides
    /// whether the check is needed.
    inputs: Globs,
    /// Environment variables the check may see. Their values are part of its inputs.
    pub env: Vec<String>,
    pub needs: Vec<Need>,
    /// For a check that needs services: the commands that make them, part of its inputs.
    pub services: Option<String>,
    /// For a check that reports findings: where they come from and which count.
    pub findings: Option<FindingsSpec>,
    /// For a check that runs tests: where it says how many ran. Zero tests fails it.
    pub tests: Option<crate::counts::Reader>,
    /// Which of its inputs are tests, as written: CI runs the change's test files on the base
    /// commit to show they fail there (red before green).
    pub test_files: Vec<String>,
    test_globs: Globs,
    /// The stills and films it takes, and which of them are flaky.
    pub pictures: Pictures,
    /// The environment's shell it runs inside, part of its inputs.
    pub shell: Option<String>,
    /// Work Fearlezz does itself instead of running a command.
    pub builtin: Option<Builtin>,
}

/// What pictures a check takes, in one place: the check's inputs, the floor and whether a run
/// needs ffmpeg all read this, so a new picture field is added here once.
#[derive(Default)]
pub(crate) struct Pictures {
    /// For a check that writes pictures into a folder: which files in it are stills and
    /// which are films.
    pub stills: Option<MediaGlob>,
    pub films: Option<MediaGlob>,
    /// For a check that records an app's window while its command runs.
    pub record: Option<crate::record::Spec>,
    /// Pictures (ids or globs, as written) that are compared and shown but never block or
    /// wait for approval.
    pub flaky: Globs,
}

impl Pictures {
    /// Whether the check names pictures it takes, by glob or by recording a window. A check
    /// can also list pictures in `proof.jsonl` without naming them here.
    pub(crate) fn named(&self) -> bool {
        self.stills.is_some() || self.films.is_some() || self.record.is_some()
    }

    /// The proofs for the files under `out` that the globs name.
    pub(crate) fn proofs(&self, out: &Path) -> Vec<serde_json::Value> {
        self.stills
            .iter()
            .chain(&self.films)
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
            )
        })
    }

    /// How `head` takes fewer pictures than these, the base branch's: a glob gone or changed
    /// (Fearlezz cannot tell a narrower glob from a wider one), a recording gone or recording
    /// less, and more pictures called flaky.
    pub(crate) fn weakenings(&self, head: &Pictures) -> Vec<String> {
        let mut found = Vec::new();
        for (kind, b, h) in [
            ("stills", &self.stills, &head.stills),
            ("films", &self.films, &head.films),
        ] {
            match (
                b.as_ref().map(MediaGlob::glob),
                h.as_ref().map(MediaGlob::glob),
            ) {
                (Some(b), None) => found.push(format!("it no longer takes {kind} by {b}")),
                (Some(b), Some(h)) if b != h => {
                    found.push(format!("it takes {kind} by another glob ({b}, now {h})"));
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
        let flaky: Vec<String> = head
            .flaky
            .widenings(&self.flaky)
            .into_iter()
            .map(|w| match w {
                Widening::Added(p) => format!("+{p}"),
                Widening::Dropped(p) => format!("-{p}"),
                Widening::Reordered => "reordered".into(),
            })
            .collect();
        if !flaky.is_empty() {
            found.push(format!(
                "it calls more pictures flaky ({})",
                flaky.join(", ")
            ));
        }
        found
    }
}

/// How the recording `h` records less than `b`: a still or film gone or renamed, another
/// window, less time, longer to wait for its window, or more cut from the top.
fn record_weakenings(b: &crate::record::Spec, h: &crate::record::Spec) -> Vec<String> {
    let mut found = Vec::new();
    for (kind, was, now) in [("still", &b.still, &h.still), ("film", &b.film, &h.film)] {
        if let Some(was) = was.as_ref().filter(|w| now.as_ref() != Some(*w)) {
            found.push(format!("it no longer records the {kind} {was}"));
        }
    }
    if b.window != h.window {
        let name = |w: &Option<String>| {
            w.clone()
                .unwrap_or_else(|| "the app its command starts".into())
        };
        found.push(format!(
            "it records another window ({}, now {})",
            name(&b.window),
            name(&h.window)
        ));
    }
    match (b.seconds, h.seconds) {
        (Some(was), Some(now)) if now < was => {
            found.push(format!("it records for less time ({was} s, now {now} s)"));
        }
        (None, Some(now)) => found.push(format!(
            "it records for less time (until its command ends, now {now} s)"
        )),
        _ => {}
    }
    let wait =
        |s: &crate::record::Spec| s.wait.unwrap_or(crate::record::DEFAULT_WAIT.as_secs_f64());
    if wait(h) > wait(b) {
        found.push(format!(
            "it waits longer for its window ({} s, now {} s)",
            wait(b),
            wait(h)
        ));
    }
    if h.crop_top > b.crop_top {
        found.push(format!(
            "it cuts more from the top ({} points, now {})",
            b.crop_top, h.crop_top
        ));
    }
    found
}

/// A check Fearlezz runs in process.
#[derive(Clone, Debug)]
pub(crate) enum Builtin {
    /// Fails when a tool's file drifted from the shared settings.
    Settings(Settings),
    /// The project's `[rule]` table.
    Rules(Rules),
}

impl Builtin {
    /// A stable text of what it checks, part of its inputs. It holds the Fearlezz version,
    /// since the check's code is part of what decides its result.
    pub(crate) fn fingerprint(&self) -> String {
        let version = env!("CARGO_PKG_VERSION");
        match self {
            Builtin::Settings(s) => format!("{version} settings {}", s.fingerprint()),
            Builtin::Rules(r) => format!("{version} rules {}", r.fingerprint()),
        }
    }

    /// Files it reads whether or not git lists them.
    pub(crate) fn files(&self) -> Vec<String> {
        match self {
            Builtin::Settings(s) => s.files(),
            Builtin::Rules(_) => Vec::new(),
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
        (&self.read, self.on, self.level, &self.fix)
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    stages: Option<RawStages>,
    #[serde(default)]
    check: BTreeMap<String, RawCheck>,
    base: Option<String>,
    env: Option<RawEnv>,
    settings: Option<toml::Table>,
    lints: Option<BTreeMap<String, BTreeMap<String, lints::Level>>>,
    rule: Option<BTreeMap<String, toml::Table>>,
    #[serde(default)]
    no_proof: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStages {
    order: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCheck {
    run: String,
    stage: String,
    #[serde(default)]
    inputs: Vec<String>,
    #[serde(default)]
    env: Vec<String>,
    #[serde(default)]
    needs: Vec<Need>,
    findings: Option<RawFindings>,
    tests: Option<toml::Value>,
    #[serde(default)]
    test_files: Vec<String>,
    stills: Option<String>,
    films: Option<String>,
    #[serde(default)]
    flaky: Vec<String>,
    record: Option<toml::Value>,
    /// Turns off a check a piece from `use` brings in.
    #[serde(default)]
    #[expect(
        dead_code,
        reason = "pieces are merged as TOML and off checks are dropped there; the field only \
                  lets serde accept the key"
    )]
    off: bool,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawEnv {
    prepare: Option<String>,
    #[serde(default)]
    inputs: Vec<String>,
    shell: Option<String>,
    #[serde(default)]
    clone: Vec<String>,
    #[serde(default)]
    clone_when: Vec<String>,
    up: Option<String>,
    down: Option<String>,
    reset: Option<String>,
    ports: Option<RawPorts>,
    #[serde(default)]
    env: Vec<String>,
}

impl RawEnv {
    fn resolve(self) -> Result<Env> {
        Ok(Env {
            prepare: self.prepare,
            inputs: self.inputs,
            shell: self.shell,
            clone: self.clone,
            clone_when: self.clone_when,
            up: self.up,
            down: self.down,
            reset: self.reset,
            ports: match self.ports {
                None => None,
                Some(RawPorts::Command(cmd)) => Some(Ports::Command(cmd)),
                Some(RawPorts::Read { read }) if read == "devenv" => Some(Ports::Devenv),
                Some(RawPorts::Read { read }) => {
                    bail!("[env] ports: no reader \"{read}\"; readers are devenv")
                }
            },
            vars: self.env,
        })
    }
}

impl Config {
    /// Reads `fearlezz.toml` in `root`, with the pieces it uses merged in (cached under
    /// `common`, the git folder every worktree shares).
    pub(crate) fn load(root: &Path, common: &Path) -> Result<Config> {
        let table = presets::resolve(root, common)?;
        Config::from_table(table).with_context(|| format!("in {}", root.join(FILE).display()))
    }

    fn from_table(mut table: toml::Table) -> Result<Config> {
        // The settings check is added below, so turning it off is read before the others go.
        let settings_off = table
            .get("check")
            .and_then(|c| c.get(SETTINGS_CHECK))
            .and_then(|c| c.get("off"))
            .and_then(toml::Value::as_bool)
            == Some(true);
        if let Some(toml::Value::Table(checks)) = table.get_mut("check") {
            checks.retain(|_, c| c.get("off").and_then(toml::Value::as_bool) != Some(true));
        }

        let raw: RawConfig = toml::Value::Table(table).try_into()?;
        let order = match raw.stages {
            Some(s) => s.order,
            None => DEFAULT_STAGES.iter().map(ToString::to_string).collect(),
        };
        if order.is_empty() {
            bail!("[stages] order needs at least one stage");
        }
        let stages = Stages(order);
        let env = raw.env.unwrap_or_default().resolve()?;
        let services = env.up.as_ref().map(|up| {
            format!(
                "{up}\0{}\0{}\0{:?}",
                env.down.as_deref().unwrap_or(""),
                env.reset.as_deref().unwrap_or(""),
                env.ports
            )
        });
        let all_lints = Lints::parse(raw.lints.unwrap_or_default())?;
        // Without inputs of its own, a check reads every file outside `no_proof`, so an edit
        // to prose reruns nothing: `no_proof` taken back out of `**`, in its own order.
        let whole_tree: Vec<String> = std::iter::once("**".to_string())
            .chain(raw.no_proof.iter().map(|p| match p.strip_prefix('!') {
                Some(back) => back.to_string(),
                None => format!("!{p}"),
            }))
            .collect();
        let mut checks = Vec::new();
        for (id, c) in raw.check {
            let stage = stages.find(&c.stage).with_context(|| {
                format!(
                    "check {id}: unknown stage \"{}\"; stages are {}",
                    c.stage,
                    stages.0.join(", ")
                )
            })?;
            let needs_services = c.needs.contains(&Need::Services);
            if needs_services && services.is_none() {
                bail!("check {id} needs services, but [env] has no up command to start them");
            }
            // Inside a shell, the environment's own files shape every result.
            let shell_inputs = || env.shell.iter().flat_map(|_| &env.inputs);
            let own = if c.inputs.is_empty() {
                &whole_tree
            } else {
                &c.inputs
            };
            let inputs = globs(own.iter().chain(shell_inputs()))
                .with_context(|| format!("check {id}: inputs"))?;
            checks.push(Check {
                inputs,
                shell: env.shell.clone(),
                builtin: None,
                tests: c
                    .tests
                    .as_ref()
                    .map(crate::counts::Reader::parse)
                    .transpose()
                    .with_context(|| format!("check {id}"))?,
                test_globs: globs(&c.test_files)
                    .with_context(|| format!("check {id}: test_files"))?,
                test_files: c.test_files,
                pictures: Pictures {
                    stills: c
                        .stills
                        .as_deref()
                        .map(|g| MediaGlob::parse("still", g))
                        .transpose()
                        .with_context(|| format!("check {id}"))?,
                    films: c
                        .films
                        .as_deref()
                        .map(|g| MediaGlob::parse("film", g))
                        .transpose()
                        .with_context(|| format!("check {id}"))?,
                    record: c
                        .record
                        .map(crate::record::Spec::parse)
                        .transpose()
                        .with_context(|| format!("check {id}"))?,
                    flaky: globs(&c.flaky).with_context(|| format!("check {id}: flaky"))?,
                },
                id: id.clone(),
                run: c.run,
                stage,
                env: c.env,
                needs: c.needs,
                services: if needs_services {
                    services.clone()
                } else {
                    None
                },
                findings: match c.findings {
                    None => None,
                    Some(RawFindings::Read(read)) => Some(FindingsSpec {
                        read,
                        on: Scope::Changed,
                        tools: Vec::new(),
                        levels: Lints::default(),
                        level: None,
                        fix: None,
                    }),
                    Some(RawFindings::Spec {
                        read,
                        on,
                        lints,
                        level,
                        fix,
                    }) => {
                        let tools = lints
                            .iter()
                            .map(|t| lints::Tool::parse(t))
                            .collect::<Result<Vec<_>>>()
                            .with_context(|| format!("check {id}: findings lints"))?;
                        Some(FindingsSpec {
                            read,
                            on,
                            levels: all_lints.only(&tools),
                            tools,
                            level,
                            fix,
                        })
                    }
                },
            });
        }
        let settings = Settings::parse(raw.settings.unwrap_or_default(), all_lints)?;
        let own_settings_check = checks.iter().any(|c| c.id == SETTINGS_CHECK);
        if !settings.is_empty() && !own_settings_check && !settings_off {
            checks.push(builtin_check(
                SETTINGS_CHECK,
                &stages,
                globs(&settings.globs())?,
                Scope::Everywhere,
                Builtin::Settings(settings.clone()),
            ));
        }
        let rules = Rules::parse(raw.rule.unwrap_or_default())?;
        if !rules.is_empty() {
            if checks.iter().any(|c| c.id == rules::CHECK) {
                bail!(
                    "check {} is the name of the check [rule] forms; rename it",
                    rules::CHECK
                );
            }
            // Names, companions and code can change with any file.
            let inputs = globs(&["**".to_string()])?;
            checks.push(builtin_check(
                rules::CHECK,
                &stages,
                inputs,
                Scope::Changed,
                Builtin::Rules(rules),
            ));
        }
        checks.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(Config {
            no_proof: globs(&raw.no_proof).context("no_proof")?,
            settings,
            stages,
            checks,
            base: raw.base.unwrap_or_else(|| "main".into()),
            env,
        })
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

    pub(crate) fn check(&self, id: &str) -> Result<&Check> {
        self.checks.iter().find(|c| c.id == id).with_context(|| {
            let ids: Vec<&str> = self.checks.iter().map(|c| c.id.as_str()).collect();
            format!("no check \"{id}\"; checks are {}", ids.join(", "))
        })
    }
}

/// A check Fearlezz runs itself at the working stage, its findings judged in `on`.
fn builtin_check(id: &str, stages: &Stages, inputs: Globs, on: Scope, builtin: Builtin) -> Check {
    Check {
        id: id.into(),
        run: format!("(fearlezz {id})"),
        stage: stages.find("working").unwrap_or(stages.first()),
        inputs,
        env: Vec::new(),
        needs: Vec::new(),
        services: None,
        findings: Some(FindingsSpec {
            read: Reader::Jsonl,
            on,
            tools: Vec::new(),
            levels: Lints::default(),
            level: None,
            fix: None,
        }),
        shell: None,
        builtin: Some(builtin),
        tests: None,
        test_files: Vec::new(),
        test_globs: Globs::default(),
        pictures: Pictures::default(),
    }
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

/// How a glob list matches more than the base branch's: positive globs it adds, `!` globs it
/// drops, or, with neither, an order that lets a positive glob win over a `!` glob again.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Widening {
    Added(String),
    Dropped(String),
    Reordered,
}

impl Globs {
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

    /// How these globs match paths that `base` does not, by a sound rule over the lists as
    /// written: dropping a positive glob or putting in a `!` glob anywhere never matches more,
    /// and globs of one kind side by side mean the same in any order. Anything else may match
    /// more, and is named.
    pub(crate) fn widenings(&self, base: &Globs) -> Vec<Widening> {
        let (b, h) = (&base.patterns, &self.patterns);
        if narrows_to(b, h) {
            return Vec::new();
        }
        let positive = |p: &&String| !p.starts_with('!');
        let mut found: Vec<Widening> = h
            .iter()
            .filter(positive)
            .filter(|p| !b.contains(p))
            .map(|p| Widening::Added(p.clone()))
            .collect();
        found.extend(
            b.iter()
                .filter(|p| !positive(p))
                .filter(|p| !h.contains(p))
                .map(|p| Widening::Dropped(p.clone())),
        );
        if found.is_empty() {
            found.push(Widening::Reordered);
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
    pub(crate) fn reads(&self, path: &str) -> bool {
        self.inputs.is_match(path)
    }

    /// Whether `path` is one of its tests, by `test_files`.
    pub(crate) fn is_test(&self, path: &str) -> bool {
        self.test_globs.is_match(path)
    }

    pub(crate) fn needs_services(&self) -> bool {
        self.needs.contains(&Need::Services)
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
        let env = Config::from_table(table).unwrap().env;
        assert_eq!(env.up.as_deref(), Some("devenv up -d"));
        assert_eq!(env.ports, Some(Ports::Devenv));
        assert_eq!(env.inputs, ["devenv.nix", "devenv.yaml", "devenv.lock"]);
        assert_eq!(env.shell.as_deref(), Some("devenv shell --"));
    }

    #[test]
    fn every_built_in_piece_is_a_valid_config() {
        for (name, piece) in presets::BUILTIN {
            let table = toml::from_str(piece).unwrap_or_else(|e| panic!("{name}: {e}"));
            if let Err(e) = Config::from_table(table) {
                panic!("{name}: {e:#}");
            }
        }
    }

    #[test]
    fn a_long_run_of_negations_is_compared_quickly() {
        let mut base = vec!["src/**".to_string()];
        base.extend((0..40).map(|i| format!("!src/x{i}/**")));
        let globs = |list: &[String]| super::globs(list).unwrap();
        let started = std::time::Instant::now();
        let dropped = &base[..base.len() - 1];
        assert_eq!(
            globs(dropped).widenings(&globs(&base)),
            [Widening::Dropped("!src/x39/**".into())]
        );
        let mut reversed = base.clone();
        reversed[1..].reverse();
        assert_eq!(globs(&reversed).widenings(&globs(&base)), []);
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn a_check_turned_off_is_gone() {
        let table = toml::from_str("[check.tests]\noff = true\n").unwrap();
        let checks = Config::from_table(table).unwrap().checks;
        assert!(!checks.iter().any(|c| c.id == "tests"));
    }
}
