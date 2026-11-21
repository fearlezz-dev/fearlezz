//! A run as it streams to the proof host, and as the host shows it live: what CI sends while
//! its checks run (protocol version 2 of `ee/README.md`), and what a page of the work receives.
//! The core writes the first half, the host reads it and writes the second, and the page reads
//! the second through TypeScript types generated from these.
//!
//! A run opens with its planned checks, each check says when it starts and when it ends (its
//! entry in the proof, with the files it made), and done closes it with `proof.json`. Files go
//! by content: each call that names files answers which the host lacks.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::floor::Weakening;
use crate::look::Look;
use crate::notes::Note;
use crate::review::{Accept, Job, Review, Verdict};
use crate::{Check, Proof, Pull};

/// The content keys of a look's pictures (`review::key`), by picture name.
pub type Keys = BTreeMap<String, String>;

/// The files a call names, by their path in the proof bundle.
pub type Files = BTreeMap<String, File>;

/// One file, by content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct File {
    /// Its SHA-256, 64 lowercase hex digits: the name the host keeps it under.
    pub sha256: String,
    /// Its size in bytes.
    pub size: u64,
}

/// Where a run runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// A CI job, which proves itself with its OIDC token.
    Ci {
        /// The job's id in its workflow (`GITHUB_JOB`), the same for every leg of a matrix.
        job: String,
        /// The workflow run's page.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_url: Option<String>,
        /// The workflow run (`GITHUB_RUN_ID`). With the attempt and the runner it names the
        /// one job a run is, a leg of a matrix among its siblings, as GitHub's `workflow_job`
        /// webhook names it.
        run_id: u64,
        /// The workflow run's attempt (`GITHUB_RUN_ATTEMPT`), 1 until it is run again.
        run_attempt: u32,
        /// The runner that took the job (`RUNNER_NAME`).
        runner_name: String,
        /// The leg it runs, for a run Fearlezz split over several jobs (`linux`, `macos/ui`):
        /// the leg it counts as in review, without waiting for GitHub to name its job.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        leg: Option<String>,
        /// Every leg the run's plan made, this one among them: the review waits for each.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        planned: Vec<String>,
    },
    /// A developer's machine, signed in to the host: unproven, and never reviewed, though a
    /// person may leave notes on its pictures.
    Local {
        /// Who ran it: the login of the CLI's sign in, which the host sets from its token.
        #[serde(default)]
        login: String,
    },
}

/// Opens a run: what CI, or a signed in CLI, sends before any check runs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Open {
    /// `owner/name` on GitHub.
    pub repository: String,
    /// The branch the work is on: a pull request's head branch, or the pushed branch.
    pub branch: String,
    /// The pull request the run proves, when it proves one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull: Option<Pull>,
    /// The full commit people know: the pull request's head, or the pushed commit.
    pub head: String,
    /// The base branch.
    pub base: String,
    /// Where it runs.
    pub source: Source,
    /// The checks it will run, in order.
    pub checks: Vec<String>,
}

/// What opening a run answers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Opened {
    /// The run's id, for the calls that follow.
    pub run: String,
    /// The host's page of the work, where the run shows live.
    pub url: String,
}

/// One check of a run starting or ending.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// The check starts.
    Started {
        /// Its id.
        check: String,
    },
    /// The check ended, or could not run.
    Finished {
        /// Its entry in the proof; `output` names its output among `files`.
        check: Box<Check>,
        /// The files it made, its pictures among them.
        #[serde(default)]
        files: Files,
        /// Its pictures compared with the base commit, for a check that takes pictures; their
        /// paths are among `files`. `done` carries every look again, as the final word.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        look: Option<Box<Look>>,
    },
}

/// The files the host lacks, by SHA-256: send each with its bytes before done.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Missing {
    /// Their SHA-256s.
    pub missing: Vec<String>,
}

/// What a CI run's change weakens, asked before the run judges it: whether an admin accepted
/// exactly that of the run's head.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Weakened {
    /// What it weakens.
    pub weakened: Vec<Weakening>,
}

/// What asking about the weakenings answers: the admin's accept of exactly those of the run's
/// head, or none.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Accepted {
    /// The accept.
    pub accepted: Option<Accept>,
}

/// Closes a run with its proof. Refused while a file of the run is missing.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Done {
    /// `proof.json`.
    pub proof: Box<Proof>,
    /// Every file of the proof bundle but `proof.json` itself.
    pub files: Files,
}

/// What closing a run answers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Closed {
    /// The host's page of the work.
    pub url: String,
}

/// A piece of work: one branch of one repository.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Work {
    /// `owner/name` as GitHub spells it.
    pub repository: String,
    /// Its branch.
    pub branch: String,
    /// Its pull request, once a run named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull: Option<Pull>,
}

/// Whether a run still runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// Opened and not closed.
    Running,
    /// Closed with its proof.
    Done,
    /// Its job ended without closing it.
    Cancelled,
}

/// One check of a run as the host knows it: planned, started, or ended with its entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct RunCheck {
    /// Its id.
    pub id: String,
    /// When it started, in seconds since the epoch by the host's clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started: Option<f64>,
    /// Its entry in the proof, once it ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<Check>,
    /// Its pictures compared with the base commit, once compared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<Look>,
    /// What each picture of its look shows, by name, as a verdict follows it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keys: Keys,
}

/// One run of the work, as a page shows it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Run {
    /// Its id.
    pub id: String,
    /// The commit it proves.
    pub head: String,
    /// The base branch.
    pub base: String,
    /// Where it runs.
    pub source: Source,
    /// Whether it still runs.
    pub state: RunState,
    /// When it opened, in seconds since the epoch.
    pub opened: f64,
    /// Its checks, planned ones first in their order, then any it ran besides.
    pub checks: Vec<RunCheck>,
}

/// Everything the host holds of a work, as a page needs it: the work and its runs, newest
/// first, as of event `seq`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Snapshot {
    /// The last event it includes; a page applies only later ones.
    pub seq: u64,
    /// The work.
    pub work: Work,
    /// Its runs, newest first.
    pub runs: Vec<Run>,
    /// Where review of its newest head stands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<Review>,
    /// Every verdict given on its pictures.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verdicts: Vec<Verdict>,
    /// Its CI jobs that wait for a runner.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waiting: Vec<Job>,
    /// Its open notes, oldest first, on whichever run they were made.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Note>,
}

/// What changed in a run.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    /// A run opened, with the work as it is now.
    Opened {
        /// The work.
        work: Work,
        /// The new run.
        run: Run,
    },
    /// A check started.
    Started {
        /// Its id.
        check: String,
        /// When, in seconds since the epoch.
        at: f64,
    },
    /// A check ended.
    Finished {
        /// Its entry.
        check: Check,
        /// Its pictures compared with the base commit, when it takes pictures.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        look: Option<Box<Look>>,
        /// What each picture of the look shows, as a verdict follows it.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        keys: Keys,
    },
    /// The run closed, with each check's final entry (red before green and instability are
    /// known only at the end) and every look.
    Done {
        /// Every check's entry.
        checks: Vec<Check>,
        /// Every look, each of the check named by its `check`.
        looks: Vec<Look>,
        /// What each picture of each look shows, by check.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        keys: BTreeMap<String, Keys>,
    },
    /// Its job ended without closing it.
    Cancelled {},
}

/// What changed in the work as a whole.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkChange {
    /// Someone gave a verdict on a picture; it replaces one on the same content. Saying it
    /// looks right settles what its open notes asked, so they are resolved with it.
    Verdict {
        /// The verdict.
        verdict: Verdict,
        /// The picture's notes the verdict resolved, as they are now.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        resolved: Vec<Note>,
    },
    /// Someone took back the verdict on a picture's content.
    Withdrawn {
        /// The check that took the picture.
        check: String,
        /// The picture's name.
        picture: String,
        /// The content the verdict was on.
        key: String,
    },
    /// Someone changed the words of notes: one, or a group linked by `link`.
    Edited {
        /// The notes as they are now.
        notes: Vec<Note>,
    },
    /// Where review of the newest head stands now.
    Review {
        /// The review, none while the work has no head.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        review: Option<Review>,
    },
    /// Someone made a note or resolved one; a resolved note leaves the page.
    Note {
        /// The note as it is now.
        note: Note,
    },
    /// The CI jobs that wait for a runner now.
    Waiting {
        /// Every one.
        waiting: Vec<Job>,
    },
}

/// One message on a page's live connection: the snapshot first, then each event after it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Live {
    /// Everything as of now.
    Snapshot(Snapshot),
    /// One change of one run.
    Event {
        /// Its place in the work's events; each is one more than the one before.
        seq: u64,
        /// The run it changes.
        run: String,
        /// What changed.
        change: Box<Change>,
    },
    /// One change of the work as a whole.
    Work {
        /// Its place in the work's events.
        seq: u64,
        /// What changed.
        change: Box<WorkChange>,
    },
}

/// One work as its repository's page lists it: where its newest run stands and what waits on
/// it. The host sends it again whenever the work changes.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Listed {
    /// The work.
    pub work: Work,
    /// Its page's path on the host: its pull request's, or its branch's.
    pub path: String,
    /// Its last event: a listing of the work with a higher one replaces this.
    pub seq: u64,
    /// When it last changed, in seconds since the epoch.
    pub active: f64,
    /// Its newest run, none before its first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<Latest>,
    /// How many pictures of its head wait for a verdict.
    pub pictures: u32,
    /// How many notes on it are open.
    pub notes: u32,
    /// Whether what its head weakens waits for an admin's accept.
    pub weakened: bool,
    /// Where its newest run stands, as [`Listed::worded`] derives it.
    #[serde(default)]
    pub standing: Standing,
    /// Where its newest run stands, in a few words.
    #[serde(default)]
    pub word: String,
    /// What waits on it, each as a short phrase.
    #[serde(default)]
    pub waits: Vec<String>,
}

impl Listed {
    /// The listed work with where it stands and what waits on it derived from its newest run
    /// and its counts: pictures without a verdict, open notes, and a weakening for an admin.
    /// Passed checks with any of those open wait for a person, they are not done.
    pub fn worded(self) -> Listed {
        let mut waits = Vec::new();
        if self.pictures > 0 {
            waits.push(format!(
                "{} to look at",
                crate::plural(self.pictures, "picture")
            ));
        }
        if self.notes > 0 {
            waits.push(crate::plural(self.notes, "open note"));
        }
        if self.weakened {
            waits.push("a weakening to accept".to_owned());
        }
        let standing = match &self.latest {
            None => Standing::None,
            Some(l) if l.state == RunState::Running => Standing::Running,
            Some(l) if l.state == RunState::Cancelled => Standing::Cancelled,
            Some(l) if l.failed > 0 => Standing::Failed,
            Some(_) if !waits.is_empty() => Standing::Waits,
            Some(_) => Standing::Passed,
        };
        Listed {
            standing,
            word: standing.word().to_owned(),
            waits,
            ..self
        }
    }
}

/// Where a listed work's newest run stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "ListedStanding"))]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    /// It has no run yet.
    #[default]
    None,
    /// It runs.
    Running,
    /// Its checks passed and nothing waits on it.
    Passed,
    /// Its checks passed and something waits for a person.
    Waits,
    /// A check failed.
    Failed,
    /// It ended without its proof.
    Cancelled,
}

impl Standing {
    /// Where it stands, in a few words.
    pub fn word(self) -> &'static str {
        match self {
            Standing::None => "no run yet",
            Standing::Running => "running",
            Standing::Passed => "passed",
            Standing::Waits => "waits for a person",
            Standing::Failed => "failed",
            Standing::Cancelled => "stopped",
        }
    }
}

/// The newest run of a listed work.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Latest {
    /// The commit it proves.
    pub head: String,
    /// Where it runs: a local run is unproven.
    pub source: Source,
    /// Whether it still runs.
    pub state: RunState,
    /// How many of its checks failed so far.
    pub failed: u32,
}

/// One message on a repository page's live connection: every work first, newest activity
/// first, then each work again as it changes.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Listing {
    /// Every work of the repository as of now.
    Snapshot {
        /// `owner/name` as GitHub spells it.
        repository: String,
        /// Its works, newest activity first.
        works: Vec<Listed>,
    },
    /// One work as it is now.
    Work(Listed),
}

/// A live share link of a work, as the host lists it to a person with write access: who made
/// it and when, never the link itself.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Share {
    /// Its id, which revokes it.
    pub id: String,
    /// The GitHub login of who made it.
    pub made_by: String,
    /// When, in seconds since the epoch.
    pub made_at: f64,
}
