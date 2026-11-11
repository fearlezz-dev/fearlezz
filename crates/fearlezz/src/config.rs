//! `fearlezz.toml`: the stages a change moves through, the checks that prove it, and the
//! environment that makes a checkout usable and runs its services.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::Deserialize;

use crate::findings::{Reader, Scope};
use crate::lints::{self, Lints};
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
    pub no_proof: Vec<String>,
    no_proof_globs: GlobSet,
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
    inputs: GlobSet,
    /// Environment variables the check may see. Their values are part of its inputs.
    pub env: Vec<String>,
    pub needs: Vec<Need>,
    /// For a check that needs services: the commands that make them, part of its inputs.
    pub services: Option<String>,
    /// For a check that reports findings: where they come from and which count.
    pub findings: Option<FindingsSpec>,
    /// The environment's shell it runs inside, part of its inputs.
    pub shell: Option<String>,
    /// Work Fearlezz does itself instead of running a command.
    pub builtin: Option<Builtin>,
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
}

/// `findings = "cargo"`, or
/// `findings = { read = "cargo", on = "everywhere", lints = ["clippy", "rustc"] }`.
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
            checks.push(Check {
                // Inside a shell, the environment's own files shape every result.
                inputs: globs(
                    c.inputs
                        .iter()
                        .chain(env.shell.iter().flat_map(|_| &env.inputs)),
                )
                .with_context(|| format!("check {id}: inputs"))?,
                shell: env.shell.clone(),
                builtin: None,
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
                    }),
                    Some(RawFindings::Spec { read, on, lints }) => {
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
            no_proof_globs: globs(&raw.no_proof).context("no_proof")?,
            no_proof: raw.no_proof,
            settings,
            stages,
            checks,
            base: raw.base.unwrap_or_else(|| "main".into()),
            env,
        })
    }

    pub(crate) fn needs_no_proof(&self, path: &str) -> bool {
        self.no_proof_globs.is_match(path)
    }

    /// Whether a change to `path` needs proof: a check reads it and `no_proof` does not
    /// declare it.
    pub(crate) fn needs_proof(&self, path: &str) -> bool {
        !self.needs_no_proof(path) && self.checks.iter().any(|c| c.reads(path))
    }

    pub(crate) fn check(&self, id: &str) -> Result<&Check> {
        self.checks.iter().find(|c| c.id == id).with_context(|| {
            let ids: Vec<&str> = self.checks.iter().map(|c| c.id.as_str()).collect();
            format!("no check \"{id}\"; checks are {}", ids.join(", "))
        })
    }
}

/// A check Fearlezz runs itself at the working stage, its findings judged in `on`.
fn builtin_check(id: &str, stages: &Stages, inputs: GlobSet, on: Scope, builtin: Builtin) -> Check {
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
        }),
        shell: None,
        builtin: Some(builtin),
    }
}

/// `*` stays inside one path segment; `**` crosses segments.
fn globs<'a>(patterns: impl IntoIterator<Item = &'a String>) -> Result<GlobSet> {
    let mut set = GlobSetBuilder::new();
    for p in patterns {
        set.add(GlobBuilder::new(p).literal_separator(true).build()?);
    }
    Ok(set.build()?)
}

impl Check {
    pub(crate) fn reads(&self, path: &str) -> bool {
        self.inputs.is_match(path)
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
    fn a_check_turned_off_is_gone() {
        let table = toml::from_str("[check.tests]\noff = true\n").unwrap();
        let checks = Config::from_table(table).unwrap().checks;
        assert!(!checks.iter().any(|c| c.id == "tests"));
    }
}
