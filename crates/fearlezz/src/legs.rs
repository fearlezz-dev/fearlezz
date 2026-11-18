//! Proving a commit in several CI jobs. `[platform.<name>]` names a machine CI proves checks
//! on (its runner, its OS, values for its setup steps), and a check's `on` the platforms it
//! must pass on. Each platform, and each `job` a check names on it, is one leg: one job of a
//! matrix. `fearlezz run --ci --plan` prints the legs, each leg runs its own checks with
//! `--leg`, and `--gate` combines their bundles into one proof, the one required check.
//!
//! A pull request reuses a check's pass from the base commit's CI proof when it read the same
//! inputs on the same platform, so a change runs only the checks it can affect; a push proves
//! everything fresh.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::config::{Check, Config};
use crate::proof::{self, Proof, Ran};

/// The OSes a platform can name.
const OSES: [&str; 3] = ["linux", "macos", "windows"];

/// The file in a leg's bundle naming its run on the proof host, which the gate asks.
pub(crate) const HOST_RUN: &str = "host-run";

/// The leg of a project without platforms whose checks name no job.
const ONE_LEG: &str = "proof";

/// The runner of a leg without a platform.
const DEFAULT_RUNNER: &str = "ubuntu-latest";

/// Minutes a leg gets to set itself up (checkout, toolchain, devenv) beside its checks'
/// budgets.
const SETUP_MINUTES: u64 = 15;

/// GitHub's own limit on a job, for a leg with a check that has no budget.
const GITHUB_MINUTES: u64 = 360;

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub(crate) struct RawPlatform {
    runs_on: String,
    os: Option<String>,
    #[serde(default)]
    default: bool,
    #[serde(default)]
    matrix: toml::Table,
}

/// A machine CI proves checks on.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Platform {
    pub name: String,
    /// The GitHub runner label its leg runs on.
    pub runs_on: String,
    /// The OS it needs, when it needs one; a platform without one runs anywhere, so a
    /// developer's machine stands in for it.
    pub os: Option<String>,
    /// Where a check without `on` runs.
    pub default: bool,
    /// Values its leg's setup steps read, as `matrix.<key>`.
    pub matrix: toml::Table,
}

impl Platform {
    /// The platforms `raw` declares, by name. With several, one is the default.
    pub(crate) fn parse(raw: BTreeMap<String, RawPlatform>) -> Result<Vec<Platform>> {
        let mut platforms = Vec::new();
        for (name, p) in raw {
            if let Some(os) = &p.os
                && !OSES.contains(&os.as_str())
            {
                bail!(
                    "platform {name}: no os \"{os}\"; an os is one of {}",
                    OSES.join(", ")
                );
            }
            if !plain(&name) {
                bail!("platform {name}: a platform's name has no / or space");
            }
            platforms.push(Platform {
                name,
                runs_on: p.runs_on,
                os: p.os,
                default: p.default,
                matrix: p.matrix,
            });
        }
        let defaults = platforms.iter().filter(|p| p.default).count();
        match (platforms.len(), defaults) {
            (0, _) | (_, 1) => {}
            (1, 0) => platforms[0].default = true,
            (_, 0) => bail!(
                "platforms {}: mark the one a check without `on` runs on with default = true",
                names(&platforms)
            ),
            _ => bail!(
                "platforms {}: only one platform is the default",
                names(&platforms)
            ),
        }
        Ok(platforms)
    }

    /// The platforms a check's `on` names, each checked against `platforms`. When it names
    /// none: those of the OS its screen needs (`os`), or else the default one.
    pub(crate) fn on(
        platforms: &[Platform],
        written: Vec<String>,
        os: Option<&str>,
    ) -> Result<Vec<String>> {
        if written.is_empty() {
            let of_os: Vec<String> = platforms
                .iter()
                .filter(|p| os.is_some() && p.os.as_deref() == os)
                .map(|p| p.name.clone())
                .collect();
            if !of_os.is_empty() {
                return Ok(of_os);
            }
            return Ok(platforms
                .iter()
                .filter(|p| p.default)
                .map(|p| p.name.clone())
                .collect());
        }
        if platforms.is_empty() {
            bail!("on names platforms, but no [platform.<name>] table declares one");
        }
        for name in &written {
            if !platforms.iter().any(|p| &p.name == name) {
                bail!("no platform \"{name}\"; platforms are {}", names(platforms));
            }
        }
        Ok(written)
    }

    /// Whether this machine can stand in for it: it needs no OS, or this one.
    pub(crate) fn fits_here(&self) -> bool {
        self.os
            .as_deref()
            .is_none_or(|os| os == std::env::consts::OS)
    }
}

/// Whether `name` can be part of a leg's id: no `/`, which joins a platform and a job, and no
/// whitespace.
fn plain(name: &str) -> bool {
    !name.is_empty() && !name.contains('/') && !name.chars().any(char::is_whitespace)
}

/// A check's `job`, checked to be part of a leg's id.
pub(crate) fn job(written: Option<String>) -> Result<Option<String>> {
    match written {
        Some(job) if !plain(&job) => bail!("a job's name has no / or space"),
        job => Ok(job),
    }
}

fn names(platforms: &[Platform]) -> String {
    platforms
        .iter()
        .map(|p| p.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// How long a check may run, as written (`90s`, `20m`, `1h`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Budget {
    pub time: Duration,
    pub written: String,
}

impl Budget {
    pub(crate) fn parse(written: &str) -> Result<Budget> {
        let span: jiff::SignedDuration = written.parse().with_context(|| {
            format!("\"{written}\" is no duration; write one such as \"90s\", \"20m\" or \"1h\"")
        })?;
        let time = Duration::try_from(span)
            .ok()
            .filter(|t| !t.is_zero())
            .with_context(|| format!("\"{written}\" is no time to run in"))?;
        Ok(Budget {
            time,
            written: written.to_string(),
        })
    }
}

impl fmt::Display for Budget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.written)
    }
}

/// The platforms a check is on that this machine cannot stand in for, when it can stand in for
/// none of them: the check is left to CI. None when it runs here. A machine never stands in
/// for a screen it can never have (a Linux display on a Mac); without platforms such a check
/// is left to CI with none named, and [`Check::screen`] says what it needs.
pub(crate) fn left_to_ci<'a>(config: &'a Config, check: &Check) -> Option<Vec<&'a str>> {
    let screen_here = check.needs.screen.never_here().is_none();
    let on: Vec<&Platform> = config
        .platforms
        .iter()
        .filter(|p| check.on.contains(&p.name))
        .collect();
    if on.is_empty() {
        return (!screen_here).then(Vec::new);
    }
    (!on.iter().any(|p| p.fits_here() && screen_here))
        .then(|| on.iter().map(|p| p.name.as_str()).collect())
}

/// One job of the matrix: a platform, or one job a check names on it, with its checks.
pub(crate) struct Leg<'a> {
    pub id: String,
    pub platform: Option<&'a Platform>,
    pub checks: Vec<&'a Check>,
}

impl Leg<'_> {
    /// The platform its checks run on, as the proof names it.
    pub(crate) fn platform_name(&self) -> Option<String> {
        self.platform.map(|p| p.name.clone())
    }
}

/// The leg `job` on `platform` is.
fn leg_id(platform: Option<&str>, job: Option<&str>) -> String {
    match (platform, job) {
        (Some(p), Some(j)) => format!("{p}/{j}"),
        (Some(p), None) => p.to_string(),
        (None, Some(j)) => j.to_string(),
        (None, None) => ONE_LEG.to_string(),
    }
}

/// Every leg of the config, by id, each check once on each of its platforms.
pub(crate) fn legs(config: &Config) -> Vec<Leg<'_>> {
    let mut legs: Vec<Leg<'_>> = Vec::new();
    for check in config.due() {
        let on: Vec<Option<&Platform>> = if check.on.is_empty() {
            vec![None]
        } else {
            config
                .platforms
                .iter()
                .filter(|p| check.on.contains(&p.name))
                .map(Some)
                .collect()
        };
        for platform in on {
            let id = leg_id(platform.map(|p| p.name.as_str()), check.job.as_deref());
            match legs.iter_mut().find(|l| l.id == id) {
                Some(leg) => leg.checks.push(check),
                None => legs.push(Leg {
                    id,
                    platform,
                    checks: vec![check],
                }),
            }
        }
    }
    legs.sort_by(|a, b| a.id.cmp(&b.id));
    legs
}

/// The leg `id`, with only the checks `ids` names when it names any.
pub(crate) fn leg<'a>(config: &'a Config, id: &str, ids: &[String]) -> Result<Leg<'a>> {
    let all = legs(config);
    let ids_of = |legs: &[Leg<'_>]| {
        legs.iter()
            .map(|l| l.id.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let known = ids_of(&all);
    let Some(mut leg) = all.into_iter().find(|l| l.id == id) else {
        bail!("no leg \"{id}\"; legs are {known}");
    };
    if !ids.is_empty() {
        for name in ids {
            if !leg.checks.iter().any(|c| &c.id == name) {
                let check = config.check(name)?;
                let theirs: Vec<String> = legs(config)
                    .into_iter()
                    .filter(|l| l.checks.iter().any(|c| c.id == check.id))
                    .map(|l| l.id)
                    .collect();
                bail!(
                    "check {name} is not in leg {id}; its legs are {}",
                    theirs.join(", ")
                );
            }
        }
        leg.checks.retain(|c| ids.contains(&c.id));
    }
    Ok(leg)
}

/// What `--leg` names: a leg's id, or its entry in the plan as JSON (`toJSON(matrix)` in a
/// workflow), which also names its checks and every leg of the plan.
pub(crate) struct Asked {
    pub id: String,
    pub checks: Vec<String>,
    pub planned: Vec<String>,
}

impl Asked {
    pub(crate) fn read(arg: &str, checks: Vec<String>) -> Result<Asked> {
        if !arg.trim_start().starts_with('{') {
            return Ok(Asked {
                id: arg.to_string(),
                checks,
                planned: Vec::new(),
            });
        }
        let entry: Value = serde_json::from_str(arg).context("reading the leg's plan entry")?;
        let text = |key: &str| entry.get(key).and_then(Value::as_str).unwrap_or_default();
        let words = |key: &str| -> Vec<String> {
            text(key).split_whitespace().map(str::to_string).collect()
        };
        if text("leg").is_empty() {
            bail!(
                "the leg's plan entry names no leg; pass the entry fearlezz run --ci --plan made"
            );
        }
        Ok(Asked {
            id: text("leg").to_string(),
            checks: if checks.is_empty() {
                words("checks")
            } else {
                checks
            },
            planned: words("planned"),
        })
    }
}

/// The base commit's CI proof, whose passes a pull request may reuse.
pub(crate) struct Reuse {
    base: Proof,
}

impl Reuse {
    /// The proof in `dir` (a bundle with `proof.json`), when a pull request is proved: a push
    /// reuses nothing. None also when the base commit has no proof yet.
    pub(crate) fn read(dir: Option<&Path>, pull: bool) -> Result<Option<Reuse>> {
        let Some(dir) = dir.filter(|_| pull) else {
            return Ok(None);
        };
        let path = dir.join("proof.json");
        if !path.exists() {
            // Stderr: the plan's stdout is JSON alone.
            eprintln!(
                "reuse: no proof of the base commit in {}, so every check runs",
                dir.display()
            );
            return Ok(None);
        }
        let base: Proof = serde_json::from_slice(&std::fs::read(&path)?)
            .with_context(|| format!("reading {}", path.display()))?;
        Ok(Some(Reuse { base }))
    }

    /// The base's pass of `id` on `platform` that read exactly `key`, when it saw one there.
    /// A pass it reused itself counts too: it read the same inputs.
    pub(crate) fn pass(&self, id: &str, platform: Option<&str>, key: &str) -> Option<proof::Check> {
        let found = self.base.checks.iter().find(|c| {
            c.id == id
                && c.platform.as_deref() == platform
                && c.key.as_deref() == Some(key)
                && c.outcome == Ran::Passed
        })?;
        let from = found
            .reused
            .clone()
            .or_else(|| self.base.run_url.clone())
            .unwrap_or_else(|| format!("the CI of {}", self.base.commit));
        Some(proof::Check {
            reused: Some(from),
            output: None,
            log: None,
            red: None,
            ..found.clone()
        })
    }
}

/// The plan's entry of `leg` for GitHub's `matrix.include`: its id, platform, runner, checks
/// (space separated), time limit, every leg of the plan, and its platform's matrix values.
pub(crate) fn entry(leg: &Leg<'_>, planned: &[String]) -> Value {
    let mut entry = Map::new();
    entry.insert("leg".into(), leg.id.clone().into());
    if let Some(p) = leg.platform {
        entry.insert("platform".into(), p.name.clone().into());
    }
    let runs_on = leg.platform.map_or(DEFAULT_RUNNER, |p| p.runs_on.as_str());
    entry.insert("runs-on".into(), runs_on.into());
    let ids: Vec<&str> = leg.checks.iter().map(|c| c.id.as_str()).collect();
    entry.insert("checks".into(), ids.join(" ").into());
    let minutes = leg
        .checks
        .iter()
        .map(|c| c.timeout.as_ref().map(|b| b.time.as_secs().div_ceil(60)))
        .sum::<Option<u64>>()
        .map_or(GITHUB_MINUTES, |m| m + SETUP_MINUTES);
    entry.insert("timeout-minutes".into(), minutes.into());
    entry.insert("planned".into(), planned.join(" ").into());
    if let Some(p) = leg.platform {
        for (key, value) in &p.matrix {
            if !entry.contains_key(key) {
                entry.insert(key.clone(), toml_to_json(value));
            }
        }
    }
    Value::Object(entry)
}

fn toml_to_json(value: &toml::Value) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// The proof of one leg as its bundle holds it, and where that bundle is.
pub(crate) struct LegProof {
    pub dir: PathBuf,
    pub proof: Proof,
}

/// Every leg bundle in `dir`: each folder in it holding a `proof.json`, as downloading the
/// legs' artifacts lays them out.
pub(crate) fn read_legs(dir: &Path) -> Result<Vec<LegProof>> {
    let mut found = Vec::new();
    let entries =
        std::fs::read_dir(dir).with_context(|| format!("reading the legs in {}", dir.display()))?;
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.join("proof.json").is_file())
        .collect();
    dirs.sort();
    for dir in dirs {
        let path = dir.join("proof.json");
        let proof: Proof = serde_json::from_slice(&std::fs::read(&path)?)
            .with_context(|| format!("reading {}", path.display()))?;
        found.push(LegProof { dir, proof });
    }
    Ok(found)
}

/// Why a check the gate expected from a leg that never brought it is not proven.
pub(crate) fn missing(check: &Check, platform: Option<&str>, leg: &str) -> proof::Item {
    let on = platform.map(|p| format!(" on {p}")).unwrap_or_default();
    proof::Item {
        what: format!("check `{}`{on} did not run", check.id),
        why: format!("its leg {leg} did not finish"),
        fix: format!("run the job of leg {leg} again; its page on GitHub shows why it stopped"),
    }
}

/// Why a check CI without a leg cannot prove is not proven: this machine cannot stand in for
/// `platform`.
pub(crate) fn left_out(check: &Check, platform: &str) -> proof::Item {
    proof::Item {
        what: format!("check `{}` on {platform} did not run", check.id),
        why: format!(
            "this machine cannot stand in for {platform}, and the run is not split into legs"
        ),
        fix: "prove the commit in legs: the action's plan, leg and gate, or its reusable workflow"
            .into(),
    }
}

/// A check of a leg's bundle as the gate's bundle holds it: its output and pictures named from
/// the leg's folder, so writing the gate's bundle copies them.
pub(crate) fn from_leg(leg: &LegProof, check: &proof::Check) -> proof::Check {
    let mut entry = check.clone();
    entry.log = check.output.as_ref().map(|rel| leg.dir.join(rel));
    entry.output = None;
    entry
}

/// The looks of a leg's bundle, their pictures found in the leg's folder.
pub(crate) fn looks_of(leg: &LegProof) -> Vec<crate::look::Look> {
    leg.proof
        .looks
        .iter()
        .cloned()
        .map(|mut look| {
            look.dir.clone_from(&leg.dir);
            look
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_budget_reads_friendly_durations() {
        assert_eq!(Budget::parse("90s").unwrap().time, Duration::from_secs(90));
        assert_eq!(
            Budget::parse("20m").unwrap().time,
            Duration::from_secs(1200)
        );
        assert_eq!(Budget::parse("1h").unwrap().time, Duration::from_secs(3600));
        Budget::parse("soon").unwrap_err();
        Budget::parse("0s").unwrap_err();
    }

    #[test]
    fn a_leg_is_its_platform_and_job() {
        assert_eq!(leg_id(Some("mac"), Some("ui")), "mac/ui");
        assert_eq!(leg_id(Some("mac"), None), "mac");
        assert_eq!(leg_id(None, Some("ui")), "ui");
        assert_eq!(leg_id(None, None), "proof");
    }
}
