//! What one proving run showed: the bundle model lives in `fearlezz-proof`, shared with the
//! proof host. This adds what only the machine that ran the checks knows: each check's entry
//! from its run, and writing the bundle (`proof.json` with outputs and media) for the host.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

pub(crate) use fearlezz_proof::{Check, Item, Proof, Pull, RED_NEXT, Ran, Red, Verdict};

use crate::look::Look;
use crate::store::{Outcome, RunResult};

/// The entry of a check that ran, with why it failed when it did.
pub(crate) fn ran(
    check: &crate::config::Check,
    stage: &str,
    result: &RunResult,
    failure: Option<Item>,
) -> Check {
    #[expect(clippy::cast_precision_loss, reason = "a duration shown in seconds")]
    let seconds = result.duration_ms as f64 / 1000.0;
    Check {
        id: check.id.clone(),
        platform: None,
        key: Some(result.key.clone()),
        reused: None,
        run: check.run().trim().to_string(),
        stage: stage.to_string(),
        outcome: match result.outcome {
            Outcome::Passed => Ran::Passed,
            Outcome::Failed => Ran::Failed,
        },
        seconds,
        exit_code: result.exit_code,
        tests: result.tests,
        cases: result.cases.clone(),
        findings: result.findings.clone(),
        findings_elsewhere: result.findings_elsewhere,
        output: None,
        failure,
        not_run: None,
        unstable: false,
        red: None,
        log: Some(result.log.clone()),
    }
}

/// Why a check did not run here.
pub(crate) enum Away {
    /// This machine cannot run it and need not: CI's runners cannot record a window, so a
    /// developer's machine proves it.
    Skipped(String),
    /// This machine cannot run it and should, which fails the run: `why` as the proof holds
    /// it, `said` as the terminal gives it, and the `fix`.
    Cannot {
        why: String,
        said: String,
        fix: String,
    },
    /// Nothing proved it: CI without legs cannot stand in for its platform, or its leg did
    /// not finish.
    Unproven(Item),
}

impl Away {
    /// What the terminal says of it.
    pub(crate) fn lines(&self, id: &str) -> Vec<String> {
        match self {
            Away::Skipped(why) => vec![format!("not run {id}: {why}")],
            Away::Cannot { said, fix, .. } => {
                vec![format!("fail {id}: {said}"), format!("  fix: {fix}")]
            }
            Away::Unproven(item) => vec![
                format!("blocked: {}: {}", item.what, item.why),
                format!("  fix: {}", item.fix),
            ],
        }
    }
}

/// The entry of a check that did not run here: not run with why, or failed with its item when
/// nothing proved it.
pub(crate) fn not_run(check: &crate::config::Check, stage: &str, away: &Away) -> Check {
    let (outcome, not_run, failure) = match away {
        Away::Skipped(why) | Away::Cannot { why, .. } => (Ran::NotRun, Some(why.clone()), None),
        Away::Unproven(item) => (Ran::Failed, None, Some(item.clone())),
    };
    Check {
        id: check.id.clone(),
        platform: None,
        key: None,
        reused: None,
        run: check.run().trim().to_string(),
        stage: stage.to_string(),
        outcome,
        seconds: 0.0,
        exit_code: None,
        tests: None,
        cases: Vec::new(),
        findings: Vec::new(),
        findings_elsewhere: 0,
        output: None,
        failure,
        not_run,
        unstable: false,
        red: None,
        log: None,
    }
}

/// Where a check's output lives in the bundle, relative to it: under its platform when it
/// names one, as the gate's bundle holds the same check from several.
pub(crate) fn output_path(check: &Check) -> String {
    match &check.platform {
        Some(platform) => format!("outputs/{platform}/{}.log", check.id),
        None => format!("outputs/{}.log", check.id),
    }
}

/// Writes `proof.json` into `dir` with every check's output and every picture's media beside
/// it, named relative to `dir`. Only the proof host draws a page from it.
pub(crate) fn write(proof: &Proof, dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut out = proof.clone();
    for check in &mut out.checks {
        let Some(log) = &check.log else { continue };
        let rel = output_path(check);
        copy(log, &dir.join(&rel))?;
        check.output = Some(rel);
    }
    for look in &mut out.looks {
        let (in_bundle, files) = bundled(look);
        for (rel, from) in files {
            copy(&from, &dir.join(rel))?;
        }
        *look = in_bundle;
    }
    let path = dir.join("proof.json");
    fs::write(&path, serde_json::to_string_pretty(&out)?)
        .with_context(|| format!("writing {}", path.display()))
}

/// A look as the bundle names it, its pictures at `looks/<folder>/<path>`, with each
/// picture's path in the bundle and its file on this machine.
pub(crate) fn bundled(look: &Look) -> (Look, Vec<(String, PathBuf)>) {
    let mut out = look.clone();
    let mut files = Vec::new();
    let folder = look
        .dir
        .file_name()
        .map_or_else(|| look.check.clone(), |f| f.to_string_lossy().to_string());
    for item in &mut out.items {
        for rel in [&mut item.before, &mut item.after].into_iter().flatten() {
            let to = format!("looks/{folder}/{rel}");
            files.push((to.clone(), look.dir.join(&*rel)));
            *rel = to;
        }
    }
    // The folder of this machine means nothing in the bundle.
    out.dir = PathBuf::new();
    (out, files)
}

fn copy(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::copy(from, to)
        .map(drop)
        .with_context(|| format!("copying {} to {}", from.display(), to.display()))
}
