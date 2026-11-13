//! The proof bundle of Fearlezz: what one proving run showed, as `proof.json` holds it, and
//! what in it waits for a person. `fearlezz run --ci --proof` writes it and the proof host
//! reads it, both through these types, so a page, the status comment and the host's approve
//! and accept read one model. It builds for wasm32 with serde alone.
//!
//! Each check carries its command, output, tests and findings, beside what the change weakens
//! about the proof, red before green, and every picture. What blocks, what waits and what to
//! read are derived, never stored.

pub mod findings;
pub mod look;
pub mod media;
pub mod waits;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::findings::Finding;
use crate::look::Look;
use crate::waits::{Obligation, Obligations, Weakening};

/// The version of `proof.json`; a reader refuses one it does not know.
pub const SCHEMA: u32 = 1;

/// At most this many test names on one line; the rest are counted.
const NAMES_SHOWN: usize = 5;

/// Everything one proving run showed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Proof {
    /// The version of `proof.json` it was written as.
    pub schema: u32,
    /// `owner/name` on GitHub.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// The pull request the run proves.
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
    pub weakened: Vec<Weakening>,
    /// Whether what it weakens waits for an admin's accept instead of keeping the commit from
    /// being proven: on a pull request whose run holds the `fearlezz/approval` status.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub weakenings_wait: bool,
    /// Every comparison of pictures with the base commit, unchanged ones too.
    pub looks: Vec<Look>,
}

/// The pull request a run proves: enough to know which one a page belongs to.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pull {
    /// Its number.
    pub number: u64,
    /// Its title, so a person knows which pull request a page belongs to.
    pub title: String,
    /// Its page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// How a check came out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ran {
    /// It passed.
    Passed,
    /// It failed.
    Failed,
    /// It could not run here, such as a recording without a display.
    NotRun,
}

impl Ran {
    /// The outcome as people read it.
    pub fn word(self) -> &'static str {
        match self {
            Ran::Passed => "passed",
            Ran::Failed => "failed",
            Ran::NotRun => "not run",
        }
    }
}

/// One check the run took up.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Check {
    /// Its id.
    pub id: String,
    /// The whole command.
    pub run: String,
    /// The stage it belongs to.
    pub stage: String,
    /// How it came out.
    pub outcome: Ran,
    /// How long it ran.
    pub seconds: f64,
    /// Its command's exit code, when it ended with one.
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
    /// What red before green showed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub red: Option<Red>,
    /// The output on this machine, until the bundle copies it.
    #[serde(skip)]
    pub log: Option<PathBuf>,
}

/// One test case a test report says ran.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Case {
    /// `classname::name`, or the name alone when there is no class.
    pub name: String,
    /// The `file` attribute, which some runners write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// It failed.
    pub failed: bool,
}

/// One thing that keeps a commit from being proven: what, why, and what to do about it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    /// What is wrong.
    pub what: String,
    /// Why.
    pub why: String,
    /// What to do about it.
    pub fix: String,
}

/// What running the check on the base commit with the change's test files showed.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Red {
    /// What it comes to.
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
    /// The check's output on the base commit, on this machine.
    #[serde(skip)]
    pub log: Option<PathBuf>,
}

/// What red before green comes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The tests fail on the base and pass here.
    Red,
    /// The tests do not build on the base, which counts as failing.
    Unbuilt,
    /// The base fails without the change's tests too, so failing there shows nothing.
    BaseFails,
    /// The tests pass on the base too.
    NotRed,
    /// The change adds the check, so the base has nothing to compare.
    New,
    /// The check could not run on the base.
    DidNotRun,
}

/// What a fix whose tests pass on the base commit does next.
pub const RED_NEXT: &str = "change a test so it fails without the fix; when the change fixes \
                            no bug, its pull request title does not start with fix: (nor, \
                            before there is one, any of its commits)";

/// What to do when red before green cannot run.
pub const DID_NOT_RUN_FIX: &str = "make the check run on the base commit";

impl Proof {
    /// The proof of `head` (people read it as `commit`) against the base branch `base`, before
    /// any check ran.
    pub fn new(commit: String, head: String, base: String) -> Proof {
        Proof {
            schema: SCHEMA,
            repository: None,
            pull: None,
            commit,
            head,
            base,
            run_url: None,
            checks: Vec::new(),
            weakened: Vec::new(),
            weakenings_wait: false,
            looks: Vec::new(),
        }
    }

    /// The proof of one look alone, for its own page.
    pub fn of_look(look: Look) -> Proof {
        Proof {
            looks: vec![look],
            ..Proof::new(String::new(), String::new(), String::new())
        }
    }

    /// The check `id`, to add what it showed.
    pub fn check_mut(&mut self, id: &str) -> Option<&mut Check> {
        self.checks.iter_mut().find(|c| c.id == id)
    }

    /// The checks that ran here, passed or failed.
    pub fn ran_here(&self) -> impl Iterator<Item = &Check> {
        self.checks.iter().filter(|c| c.outcome != Ran::NotRun)
    }

    /// What keeps the commit from being proven: failed checks, unstable ones, what the change
    /// weakens (unless it waits for an admin's accept), and fixes its tests do not prove, in
    /// that order.
    pub fn blockers(&self) -> Vec<Item> {
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
        items.extend(self.blocking_weakenings().map(|w| Item {
            what: w.what(),
            why: w.how.clone(),
            fix: Weakening::block_fix(&self.base),
        }));
        items.extend(
            self.checks
                .iter()
                .filter_map(|c| Some((c, c.red.as_ref().filter(|r| r.blocks)?)))
                .map(|(c, r)| r.blocker(&c.id)),
        );
        items
    }

    /// What the change weakens when nothing can wait for an admin's accept, so it blocks.
    fn blocking_weakenings(&self) -> impl Iterator<Item = &Weakening> {
        self.weakened.iter().filter(|_| !self.weakenings_wait)
    }

    /// How many blockers the change itself has to fix: all but what it weakens, which only a
    /// person settles (by accepting it where it waits, by asking an admin where it blocks).
    pub fn to_fix(&self) -> usize {
        self.blockers().len() - self.blocking_weakenings().count()
    }

    /// What a reader should know besides the blockers: checks that did not run here, then
    /// what red before green showed.
    pub fn notes(&self) -> Vec<String> {
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
    pub fn pictures_to_approve(&self) -> impl Iterator<Item = &Look> {
        self.looks.iter().filter(|l| l.to_look_at() > 0)
    }

    /// What waits for a person on its head: the changed pictures of each check, then what the
    /// change weakens where it waits for an admin's accept.
    pub fn obligations(&self) -> Obligations {
        let pictures = self
            .pictures_to_approve()
            .map(|l| Obligation::Pictures(l.check.clone()));
        let weakened = self
            .weakened
            .iter()
            .filter(|_| self.weakenings_wait)
            .cloned()
            .map(Obligation::Weakened);
        pictures.chain(weakened).collect()
    }
}

impl Red {
    /// `tests/a.sh` or `2 test files`, with the verbs agreeing.
    fn subject(&self) -> (String, &'static str, &'static str) {
        match self.files.as_slice() {
            [one] => (one.clone(), "fails", "passes"),
            many => (format!("{} test files", many.len()), "fail", "pass"),
        }
    }

    /// `does not build` or `do not build`, agreeing with the files.
    fn build(&self) -> &'static str {
        if self.files.len() == 1 {
            "does not build"
        } else {
            "do not build"
        }
    }

    /// What it showed for check `id`, in a sentence for a reader of the proof.
    pub fn note(&self, id: &str) -> String {
        let (what, _, passes) = self.subject();
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
            Verdict::Unbuilt => format!(
                "Red before green: the change's test files of `{id}` ({what}) {} on the base \
                 commit, which counts as failing, and {passes} on this one.",
                self.build()
            ),
            Verdict::BaseFails => format!(
                "Red before green shows nothing for `{id}`: it already fails on the base \
                 commit without the change's test files."
            ),
            Verdict::New => format!(
                "Red before green has nothing to compare for `{id}`: the check is new in this \
                 change."
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

    /// What it showed for check `id`, as the lines a run prints: the verdict, the tests it
    /// names, where the base run's output is, and what to do when it blocks.
    pub fn lines(&self, id: &str) -> Vec<String> {
        let (what, fails, passes) = self.subject();
        let mut lines = vec![match self.verdict {
            Verdict::Red => {
                format!(
                    "red before green {id}: {what} {fails} on the base commit and {passes} here"
                )
            }
            Verdict::Unbuilt => format!(
                "red before green {id}: {what} {} on the base commit, which counts as failing, \
                 and {passes} here",
                self.build()
            ),
            Verdict::BaseFails => format!(
                "red before green {id}: the base already fails without the change's test \
                 files, so failing there shows nothing"
            ),
            Verdict::New => format!(
                "red before green {id}: the check is new in this change, so the base commit \
                 has nothing to compare"
            ),
            Verdict::NotRed => {
                let it = if self.files.len() == 1 {
                    "it does"
                } else {
                    "they do"
                };
                format!(
                    "not red before green {id}: {what} {passes} on the base commit too, so {it} \
                     not show the fix"
                )
            }
            Verdict::DidNotRun => format!(
                "red before green {id} did not run: {}",
                self.why.as_deref().unwrap_or_default()
            ),
        }];
        if !self.tests.is_empty() {
            let lead = if self.verdict == Verdict::Red {
                "failed on the base, passed here"
            } else {
                "passed on both"
            };
            lines.push(format!("  {lead}: {}", test_names(&self.tests, "")));
        }
        if let Some(log) = &self.log {
            lines.push(format!("  log on the base: {}", log.display()));
        }
        if self.blocks {
            lines.push(format!("  fix: {}", self.fix()));
        }
        lines
    }

    /// What to do when it blocks.
    fn fix(&self) -> String {
        if self.verdict == Verdict::DidNotRun {
            format!("{DID_NOT_RUN_FIX}; a fix is not proven until its tests fail there")
        } else {
            RED_NEXT.into()
        }
    }

    fn blocker(&self, id: &str) -> Item {
        if self.verdict == Verdict::DidNotRun {
            return Item {
                what: format!("red before green did not run for `{id}`"),
                why: self.why.clone().unwrap_or_default(),
                fix: self.fix(),
            };
        }
        let (what, _, passes) = self.subject();
        Item {
            what: format!("the tests of `{id}` do not show the fix"),
            why: format!("{what} {passes} on the base commit too"),
            fix: self.fix(),
        }
    }
}

/// `a, b, c and 3 more`, each name wrapped in `quote`.
pub fn test_names(names: &[String], quote: &str) -> String {
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
