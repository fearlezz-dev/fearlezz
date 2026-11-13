//! What one proving run showed: the bundle model lives in `fearlezz-proof`, shared with the
//! proof host. This adds what only the machine that ran the checks knows: each check's entry
//! from its run, and writing the bundle (`proof.json` with outputs, media and the page).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

pub(crate) use fearlezz_proof::{Check, Item, Proof, Pull, RED_NEXT, Ran, Red, Verdict};

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
        run: check.run.trim().to_string(),
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

/// The entry of a check that could not run here, and why.
pub(crate) fn not_run(check: &crate::config::Check, stage: &str, why: &str) -> Check {
    Check {
        id: check.id.clone(),
        run: check.run.trim().to_string(),
        stage: stage.to_string(),
        outcome: Ran::NotRun,
        seconds: 0.0,
        exit_code: None,
        tests: None,
        cases: Vec::new(),
        findings: Vec::new(),
        findings_elsewhere: 0,
        output: None,
        failure: None,
        not_run: Some(why.to_string()),
        unstable: false,
        red: None,
        log: None,
    }
}

/// Writes `proof.json` into `dir` with every check's output and every picture's media beside
/// it, named relative to `dir`, and the page that shows it as `index.html`.
pub(crate) fn write(proof: &Proof, dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut out = proof.clone();
    for check in &mut out.checks {
        let Some(log) = &check.log else { continue };
        let rel = format!("outputs/{}.log", check.id);
        copy(log, &dir.join(&rel))?;
        check.output = Some(rel);
    }
    for look in &mut out.looks {
        let from = look.dir().to_path_buf();
        let folder = from
            .file_name()
            .map_or_else(|| look.check.clone(), |f| f.to_string_lossy().to_string());
        for item in &mut look.items {
            for rel in [&mut item.before, &mut item.after].into_iter().flatten() {
                let to = format!("looks/{folder}/{rel}");
                copy(&from.join(&*rel), &dir.join(&to))?;
                *rel = to;
            }
        }
        // The report page of this machine means nothing in the bundle.
        look.report = PathBuf::new();
    }
    let path = dir.join("proof.json");
    fs::write(&path, serde_json::to_string_pretty(&out)?)
        .with_context(|| format!("writing {}", path.display()))?;
    crate::report::write_proof(dir, &out)
}

fn copy(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::copy(from, to)
        .map(drop)
        .with_context(|| format!("copying {} to {}", from.display(), to.display()))
}
