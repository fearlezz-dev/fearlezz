//! The result store: every check run, kept by the hash of what went into it, shared by all
//! worktrees. Journals point at results here; nothing about a result is stored twice.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::config::Check;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    Passed,
    Failed,
}

/// What went into a check: its command, declared env values and input files.
pub(crate) struct Inputs {
    pub key: String,
    /// Input path to content hash; env vars appear as `$NAME`.
    pub parts: BTreeMap<String, String>,
}

/// One run of one check. Written once by the run that made it, then only read.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct RunResult {
    pub id: String,
    pub check: String,
    pub key: String,
    pub parts: BTreeMap<String, String>,
    pub tree: String,
    /// The journal that ran it, so a result's origin is always visible.
    pub journal: String,
    pub stage: String,
    pub outcome: Outcome,
    pub exit_code: Option<i32>,
    pub started_at: Timestamp,
    pub duration_ms: u64,
    pub proofs: Vec<serde_json::Value>,
    pub proof_errors: Vec<String>,
    /// For a check that reports findings: the ones in scope, worst first.
    #[serde(default)]
    pub findings: Vec<crate::findings::Finding>,
    /// Findings outside the scope (on lines this change did not touch).
    #[serde(default)]
    pub findings_elsewhere: usize,
    /// For a check with a tests reader: how many tests ran; zero when there is no count.
    #[serde(default)]
    pub tests: Option<u64>,
    /// For a check with a JUnit reader: each test case that ran, and whether it failed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cases: Vec<crate::counts::Case>,
    pub dir: PathBuf,
    pub log: PathBuf,
}

impl RunResult {
    /// The shell could not find the check's command, so the run proves nothing either way.
    pub(crate) fn not_found(&self) -> bool {
        self.exit_code == Some(crate::run::NOT_FOUND)
    }

    /// Whether `journal` made this run at its final stage `final_stage`: the only kind of
    /// pass the final stage counts, so the proof before a merge is fresh.
    pub(crate) fn made_at_final(&self, journal: &str, final_stage: &str) -> bool {
        self.journal == journal && self.stage == final_stage
    }
}

/// What a passing run showed against one base commit: how its pictures compare and what red
/// before green found. A run compared with another base says nothing about this one, so the
/// record is kept in the run's folder under the base commit and read for the base the work
/// stands on now; none means the comparison is still to make.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct AgainstBase {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<Looked>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub red: Option<crate::red::Mark>,
}

/// How a run's pictures were compared with the base.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Looked {
    /// There was nothing to compare: no pictures on either side, or the same inputs.
    Nothing,
    /// The folder of the look.
    At(PathBuf),
}

impl AgainstBase {
    fn path(run: &RunResult, base: &str) -> PathBuf {
        run.dir.join("against").join(format!("{base}.json"))
    }

    /// What `run` showed against `base`, if it was compared with it.
    pub(crate) fn of(run: &RunResult, base: &str) -> Option<AgainstBase> {
        let bytes = fs::read(Self::path(run, base)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Records another part of what `run` showed against `base`.
    pub(crate) fn update(
        run: &RunResult,
        base: &str,
        change: impl FnOnce(&mut AgainstBase),
    ) -> Result<()> {
        let mut record = Self::of(run, base).unwrap_or_default();
        change(&mut record);
        let path = Self::path(run, base);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, serde_json::to_vec_pretty(&record)?)?;
        Ok(())
    }
}

/// Results unused this long, that no journal active in that time points at, are deleted.
pub(crate) const KEEP_DAYS: i64 = 30;

pub(crate) struct Store {
    root: PathBuf,
}

/// Hashes file contents once per command, however many checks read the same file.
#[derive(Default)]
pub(crate) struct Hasher {
    files: HashMap<String, String>,
}

impl Hasher {
    /// `changed_lines` is the digest of the lines a findings check is judged on, if any.
    pub(crate) fn inputs(
        &mut self,
        root: &Path,
        check: &Check,
        files: &[String],
        changed_lines: Option<&str>,
    ) -> Result<Inputs> {
        let mut parts = BTreeMap::new();
        if let Some(digest) = changed_lines {
            parts.insert("(changed lines)".into(), digest.to_string());
        }
        for name in &check.env {
            let value = std::env::var(name).unwrap_or_default();
            parts.insert(format!("${name}"), sha256(value.as_bytes()));
        }
        if let Some(builtin) = &check.builtin {
            parts.insert("(builtin)".into(), sha256(builtin.fingerprint().as_bytes()));
            for path in builtin.files() {
                let hash = match fs::read(root.join(&path)) {
                    Ok(bytes) => sha256(&bytes),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => "(absent)".into(),
                    Err(e) => return Err(e).with_context(|| format!("reading {path}")),
                };
                parts.insert(path, hash);
            }
        }
        if let Some(spec) = check.findings.as_ref().filter(|s| !s.tools.is_empty()) {
            let levels = spec.levels.fingerprint(&spec.tools);
            parts.insert("(lints)".into(), sha256(levels.as_bytes()));
        }
        if let Some(spec) = &check.findings {
            parts.insert("(findings)".into(), definition(&spec.judged()));
        }
        if let Some(reader) = &check.tests {
            parts.insert("(tests)".into(), definition(reader));
        }
        if let Some(pictures) = check.pictures.taken() {
            parts.insert("(pictures)".into(), definition(&pictures));
        }
        if let Some(shell) = &check.shell {
            parts.insert("(shell)".into(), sha256(shell.as_bytes()));
        }
        if let Some(services) = &check.services {
            parts.insert("(services)".into(), sha256(services.as_bytes()));
        }
        for path in files.iter().filter(|p| check.reads(p)) {
            let hash = if let Some(h) = self.files.get(path) {
                h.clone()
            } else {
                let bytes = fs::read(root.join(path)).with_context(|| format!("reading {path}"))?;
                let h = sha256(&bytes);
                self.files.insert(path.clone(), h.clone());
                h
            };
            parts.insert(path.clone(), hash);
        }
        let mut key = Sha256::new();
        key.update(RESULT_FORMAT.as_bytes());
        key.update(b"\0");
        key.update(check.run.as_bytes());
        for (name, hash) in &parts {
            key.update(b"\0");
            key.update(name.as_bytes());
            key.update(b"\0");
            key.update(hash.as_bytes());
        }
        Ok(Inputs {
            key: hex::encode(key.finalize()),
            parts,
        })
    }
}

/// The format of a stored result, part of every key. Bump it when what a result holds or
/// means changes, so results stored before run again (2: JUnit results keep each case).
const RESULT_FORMAT: &str = "fearlezz/2";

/// A part of the key from a definition as Fearlezz parsed it, so a field added to it joins
/// the key without anyone writing it down.
fn definition(parsed: &impl std::fmt::Debug) -> String {
    sha256(format!("{parsed:?}").as_bytes())
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

impl Store {
    pub(crate) fn open(common: &Path) -> Store {
        Store {
            root: common.join("fearlezz").join("results"),
        }
    }

    /// A fresh folder for a run's proof files.
    pub(crate) fn new_run_dir(&self, key: &str, id: &str) -> Result<PathBuf> {
        let dir = self.root.join(key).join(id);
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    pub(crate) fn save(&self, result: &RunResult) -> Result<()> {
        let path = self
            .root
            .join(&result.key)
            .join(format!("{}.json", result.id));
        fs::write(path, serde_json::to_vec_pretty(result)?)?;
        Ok(())
    }

    /// Every result for these inputs, oldest first.
    pub(crate) fn for_key(&self, key: &str) -> Vec<RunResult> {
        let Ok(entries) = fs::read_dir(self.root.join(key)) else {
            return Vec::new();
        };
        let mut results: Vec<RunResult> = entries
            .filter_map(std::result::Result::ok)
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| fs::read(e.path()).ok())
            .filter_map(|b| serde_json::from_slice(&b).ok())
            .collect();
        results.sort_by(|a, b| a.id.cmp(&b.id));
        results
    }

    pub(crate) fn get(&self, key: &str, id: &str) -> Option<RunResult> {
        let bytes = fs::read(self.root.join(key).join(format!("{id}.json"))).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Deletes the results nothing reads again. For each set of inputs it keeps the latest
    /// result (reuse and the look's before), the latest pass and the latest fail (so an
    /// unstable check stays known), and every result in `named` (what journals point at).
    /// Inputs whose newest result is older than [`KEEP_DAYS`] and that `named` does not
    /// mention go entirely; they cost one rerun if that code returns. Runs at most once a
    /// day; returns how many results it deleted.
    pub(crate) fn prune(
        &self,
        named: &BTreeSet<(String, String)>,
        now: Timestamp,
    ) -> Result<usize> {
        let marker = self.root.join(".pruned");
        let day = 24 * 3600;
        if let Ok(text) = fs::read_to_string(&marker)
            && let Ok(last) = text.trim().parse::<Timestamp>()
            && now.as_second() - last.as_second() < day
        {
            return Ok(0);
        }
        let Ok(keys) = fs::read_dir(&self.root) else {
            return Ok(0);
        };
        let mut deleted = 0;
        for entry in keys.filter_map(std::result::Result::ok) {
            if !entry.path().is_dir() {
                continue;
            }
            let key = entry.file_name().to_string_lossy().to_string();
            let results = self.for_key(&key);
            let Some(newest) = results.last() else {
                continue;
            };
            let mentioned = results
                .iter()
                .any(|r| named.contains(&(key.clone(), r.id.clone())));
            let old = now.as_second() - newest.started_at.as_second() > KEEP_DAYS * day;
            let keep: BTreeSet<&str> = if old && !mentioned {
                BTreeSet::new()
            } else {
                let last_of = |o: Outcome| results.iter().rfind(|r| r.outcome == o);
                [
                    Some(newest),
                    last_of(Outcome::Passed),
                    last_of(Outcome::Failed),
                ]
                .into_iter()
                .flatten()
                .chain(
                    results
                        .iter()
                        .filter(|r| named.contains(&(key.clone(), r.id.clone()))),
                )
                .map(|r| r.id.as_str())
                .collect()
            };
            for r in results.iter().filter(|r| !keep.contains(r.id.as_str())) {
                // The record first: a folder without one is a run still going, never touched.
                fs::remove_file(entry.path().join(format!("{}.json", r.id)))?;
                let dir = entry.path().join(&r.id);
                if dir.exists() {
                    fs::remove_dir_all(&dir)?;
                }
                deleted += 1;
            }
            if keep.is_empty() {
                // Only when empty: another command may be starting a run here.
                if let Err(e) = fs::remove_dir(entry.path())
                    && !matches!(
                        e.kind(),
                        std::io::ErrorKind::DirectoryNotEmpty | std::io::ErrorKind::NotFound
                    )
                {
                    return Err(e.into());
                }
            }
        }
        fs::write(&marker, now.to_string())?;
        Ok(deleted)
    }

    /// Same inputs gave both outcomes: a flaky check or an input nobody declared.
    pub(crate) fn unstable(&self, key: &str) -> bool {
        let results = self.for_key(key);
        // A command the shell could not find says nothing about the inputs.
        results.iter().any(|r| r.outcome == Outcome::Passed)
            && results
                .iter()
                .any(|r| r.outcome == Outcome::Failed && !r.not_found())
    }
}
