//! What one proving run showed, as one model: each check with its command, output, tests and
//! findings, what the change weakens about the proof, red before green, and every picture.
//! CI's status comment and check page are drawn from it, and `--proof` writes it as
//! `proof.json` beside the files it names (outputs and media), so a page can show all of it
//! without the machine that ran it. What blocks and what to read are derived, never stored.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::counts::Case;
use crate::findings::Finding;
use crate::floor::Item;
use crate::look::Look;
use crate::store::{Outcome, RunResult};

/// The version of `proof.json`; a reader refuses one it does not know.
pub(crate) const SCHEMA: u32 = 1;

/// At most this many test names on one line; the rest are counted.
const NAMES_SHOWN: usize = 5;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Proof {
    pub schema: u32,
    /// `owner/name` on GitHub.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull: Option<Pull>,
    /// The commit as people read it, with the merge it proved.
    pub commit: String,
    /// The full commit people know: the pull request's head, or HEAD. Approvals bind to it.
    pub head: String,
    /// The base branch.
    pub base: String,
    /// The CI run's page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_url: Option<String>,
    /// Every check the run took up, in the order it ran them.
    pub checks: Vec<Check>,
    /// What the change weakens about the proof, against the base branch's config.
    pub weakened: Vec<Item>,
    /// Every comparison of pictures with the base commit, unchanged ones too.
    pub looks: Vec<Look>,
}

/// The pull request a run proves: enough to know which one a page belongs to.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Pull {
    pub number: u64,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Ran {
    Passed,
    Failed,
    /// It could not run here, such as a recording without a display.
    NotRun,
}

impl Ran {
    pub(crate) fn word(self) -> &'static str {
        match self {
            Ran::Passed => "passed",
            Ran::Failed => "failed",
            Ran::NotRun => "not run",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Check {
    pub id: String,
    /// The whole command.
    pub run: String,
    pub stage: String,
    pub outcome: Ran,
    pub seconds: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// How many tests ran, for a check that counts them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tests: Option<u64>,
    /// Each test case, for a check whose reader names them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cases: Vec<Case>,
    /// The findings on lines the change touched, worst first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<Finding>,
    /// How many findings are on lines it did not touch.
    #[serde(default)]
    pub findings_elsewhere: usize,
    /// Everything it printed: a file in the bundle, relative to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Why it failed and what to do, for a failed check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<Item>,
    /// Why it did not run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_run: Option<String>,
    /// The same inputs gave both outcomes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unstable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub red: Option<Red>,
    /// The output on this machine, until the bundle copies it.
    #[serde(skip)]
    pub log: Option<PathBuf>,
}

/// What running the check on the base commit with the change's test files showed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Red {
    pub verdict: Verdict,
    /// The change's test files the check reads.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// The tests that failed on the base and pass here (red), or that pass on both (not red).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tests: Vec<String>,
    /// Why it did not run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// The change is a fix this does not prove.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub blocks: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Verdict {
    /// The tests fail on the base and pass here.
    Red,
    /// The tests do not build on the base, which counts as failing.
    Unbuilt,
    /// The base fails without the change's tests too, so failing there shows nothing.
    BaseFails,
    /// The tests pass on the base too.
    NotRed,
    /// The check could not run on the base.
    DidNotRun,
}

impl Proof {
    pub(crate) fn new(commit: String, head: String, base: String) -> Proof {
        let env = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let run_url = match (
            env("GITHUB_SERVER_URL"),
            env("GITHUB_REPOSITORY"),
            env("GITHUB_RUN_ID"),
        ) {
            (Some(server), Some(repo), Some(run)) => {
                Some(format!("{server}/{repo}/actions/runs/{run}"))
            }
            _ => None,
        };
        Proof {
            schema: SCHEMA,
            repository: env("GITHUB_REPOSITORY"),
            pull: pull(),
            commit,
            head,
            base,
            run_url,
            checks: Vec::new(),
            weakened: Vec::new(),
            looks: Vec::new(),
        }
    }

    /// Adds a check that ran, with why it failed when it did.
    pub(crate) fn ran(
        &mut self,
        check: &crate::config::Check,
        stage: &str,
        result: &RunResult,
        failure: Option<Item>,
    ) {
        #[expect(clippy::cast_precision_loss, reason = "a duration shown in seconds")]
        let seconds = result.duration_ms as f64 / 1000.0;
        self.checks.push(Check {
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
        });
    }

    /// Adds a check that could not run here, and why.
    pub(crate) fn not_run(&mut self, check: &crate::config::Check, stage: &str, why: &str) {
        self.checks.push(Check {
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
        });
    }

    pub(crate) fn check_mut(&mut self, id: &str) -> Option<&mut Check> {
        self.checks.iter_mut().find(|c| c.id == id)
    }

    /// The checks that ran here, passed or failed.
    pub(crate) fn ran_here(&self) -> impl Iterator<Item = &Check> {
        self.checks.iter().filter(|c| c.outcome != Ran::NotRun)
    }

    /// What keeps the commit from being proven: failed checks, unstable ones, what the change
    /// weakens, and fixes its tests do not prove, in that order.
    pub(crate) fn blockers(&self) -> Vec<Item> {
        let mut items: Vec<Item> = self
            .checks
            .iter()
            .filter_map(|c| c.failure.clone())
            .collect();
        items.extend(self.checks.iter().filter(|c| c.unstable).map(|c| Item {
            what: format!("check `{}` is unstable", c.id),
            why: "the same inputs gave passed and failed".into(),
            fix: "make it give one answer: a flaky test, or an input it does not declare".into(),
        }));
        items.extend(self.weakened.iter().cloned());
        items.extend(
            self.checks
                .iter()
                .filter_map(|c| Some((c, c.red.as_ref().filter(|r| r.blocks)?)))
                .map(|(c, r)| r.blocker(&c.id)),
        );
        items
    }

    /// What a reader should know besides the blockers: checks that did not run here, then
    /// what red before green showed.
    pub(crate) fn notes(&self) -> Vec<String> {
        let not_run = self.checks.iter().filter_map(|c| {
            c.not_run.as_ref().map(|why| {
                format!(
                    "Not run: `{}` records an app's window, and {why}. It is proven on a \
                     developer's machine.",
                    c.id
                )
            })
        });
        let red = self
            .checks
            .iter()
            .filter_map(|c| c.red.as_ref().map(|r| r.note(&c.id)));
        not_run.chain(red).collect()
    }

    /// Looks whose changed pictures wait for a person.
    pub(crate) fn waits(&self) -> Vec<Look> {
        self.looks
            .iter()
            .filter(|l| l.to_look_at() > 0)
            .cloned()
            .collect()
    }

    /// Writes `proof.json` into `dir` with every check's output and every picture's media
    /// beside it, named relative to `dir`.
    pub(crate) fn write(&self, dir: &Path) -> Result<()> {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let mut out = self.clone();
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
            .with_context(|| format!("writing {}", path.display()))
    }
}

impl Red {
    /// `tests/a.sh` or `2 test files`, with the verbs agreeing.
    pub(crate) fn subject(files: &[String]) -> (String, &'static str, &'static str) {
        match files {
            [one] => (one.clone(), "fails", "passes"),
            many => (format!("{} test files", many.len()), "fail", "pass"),
        }
    }

    fn note(&self, id: &str) -> String {
        let (what, _, passes) = Red::subject(&self.files);
        let names = |text: &mut String, lead: &str| {
            if !self.tests.is_empty() {
                text.push_str(lead);
                text.push_str(&test_names(&self.tests, "`"));
                text.push('.');
            }
        };
        match self.verdict {
            Verdict::Red => {
                let mut text = format!(
                    "Red before green: `{id}` fails on the base commit with the change's test \
                     files ({what}) and passes on this one."
                );
                names(&mut text, " Failed on the base, passed here: ");
                text
            }
            Verdict::Unbuilt => {
                let build = if self.files.len() == 1 {
                    "does not build"
                } else {
                    "do not build"
                };
                format!(
                    "Red before green: the change's test files of `{id}` ({what}) {build} on \
                     the base commit, which counts as failing, and {passes} on this one."
                )
            }
            Verdict::BaseFails => format!(
                "Red before green shows nothing for `{id}`: it already fails on the base \
                 commit without the change's test files."
            ),
            Verdict::NotRed => {
                let mut text = format!(
                    "Not red before green: `{id}` passes on the base commit with the change's \
                     test files ({what})."
                );
                names(&mut text, " Passed on both: ");
                text
            }
            Verdict::DidNotRun => format!(
                "Red before green did not run for `{id}`: {}.",
                self.why.as_deref().unwrap_or_default()
            ),
        }
    }

    fn blocker(&self, id: &str) -> Item {
        if self.verdict == Verdict::DidNotRun {
            return Item {
                what: format!("red before green did not run for `{id}`"),
                why: self.why.clone().unwrap_or_default(),
                fix: format!("{DID_NOT_RUN_FIX}; a fix is not proven until its tests fail there"),
            };
        }
        let (what, _, passes) = Red::subject(&self.files);
        Item {
            what: format!("the tests of `{id}` do not show the fix"),
            why: format!("{what} {passes} on the base commit too"),
            fix: crate::red::NEXT.into(),
        }
    }
}

/// What to do when red before green cannot run.
pub(crate) const DID_NOT_RUN_FIX: &str = "make the check run on the base commit";

/// `a, b, c and 3 more`, each name wrapped in `quote`.
pub(crate) fn test_names(names: &[String], quote: &str) -> String {
    let shown: Vec<String> = names
        .iter()
        .take(NAMES_SHOWN)
        .map(|n| format!("{quote}{n}{quote}"))
        .collect();
    let text = shown.join(", ");
    match names.len().checked_sub(NAMES_SHOWN) {
        Some(more) if more > 0 => format!("{text} and {more} more"),
        _ => text,
    }
}

/// The pull request GitHub Actions runs for, from its event file.
fn pull() -> Option<Pull> {
    let path = std::env::var_os("GITHUB_EVENT_PATH")?;
    let event: serde_json::Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    let pr = event.get("pull_request")?;
    Some(Pull {
        number: pr.get("number")?.as_u64()?,
        title: pr.get("title")?.as_str()?.to_string(),
        url: pr
            .get("html_url")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
    })
}

fn copy(from: &Path, to: &Path) -> Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::copy(from, to)
        .map(drop)
        .with_context(|| format!("copying {} to {}", from.display(), to.display()))
}
