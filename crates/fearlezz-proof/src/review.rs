//! Review on the proof host: a person's verdict on each picture, an admin's accept of a head's
//! weakenings, and where review of a head stands. The host derives a head's review from its CI
//! runs, the work's verdicts and the head's accept, shows it on the work's page and writes it to
//! GitHub as its check run and pull request comment, so the page and GitHub say one thing.
//!
//! A verdict follows the picture, not the run: it is kept by check, picture name and content, so
//! a picture that did not change keeps its verdict across pushes and runs of the same work.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::events::{Run, RunCheck, RunState, Source};
use crate::floor::Weakening;
use crate::look::Item;
use crate::media::Marks;
use crate::notes::{Note, NoteState};
use crate::{Proof, Ran, short};

/// What a person may do on a repository, as GitHub's collaborator permission says it. Which
/// act needs which role is the host's one rule (`Act` in its `say.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Read, or nothing.
    #[serde(rename = "read")]
    Less,
    /// Write or maintain.
    Write,
    /// Admin.
    Admin,
}

impl Role {
    /// The role GitHub's `permission` field names: `admin`, `write`, or anything less. It reads
    /// back what `word` wrote.
    pub fn of(permission: &str) -> Role {
        match permission {
            "admin" => Role::Admin,
            "write" => Role::Write,
            _ => Role::Less,
        }
    }

    /// The role's word, as GitHub says it: `admin`, `write` or `read`.
    pub fn word(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Write => "write",
            Role::Less => "read",
        }
    }
}

/// Who looks at a work's page, as the host answers `GET /api/v2/viewer/<owner>/<name>`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(untagged)]
pub enum Viewer {
    /// A signed in person, and what GitHub lets them do on the repository.
    Signed {
        /// Their GitHub login.
        login: String,
        /// Their role on the repository.
        role: Role,
    },
    /// Nobody signed in.
    Out {
        /// Always null.
        login: (),
    },
}

/// A person's word that one picture looks right, as its content was when they gave it. What
/// should change is never a verdict: it is an open note on the picture.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "PictureVerdict"))]
pub struct Verdict {
    /// The check that took the picture.
    pub check: String,
    /// The picture's name in its look.
    pub picture: String,
    /// What the picture showed, as [`key`] names it.
    pub key: String,
    /// Their GitHub login.
    pub by: String,
    /// When, in seconds since the epoch.
    pub at: f64,
    /// The run they gave it on.
    pub run: String,
    /// The commit that run proved.
    pub head: String,
}

/// An admin's accept of exactly the weakenings of one head their proof showed. A CI run of that
/// head asks for it before it judges what it weakens, and passes when it covers that.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Accept {
    /// The commit it accepts the weakenings of.
    pub head: String,
    /// What it accepts, as the proof it was given on showed it.
    pub weakened: Vec<Weakening>,
    /// Their GitHub login.
    pub by: String,
    /// When, in seconds since the epoch.
    pub at: f64,
}

impl Accept {
    /// Whether it accepts exactly `weakened` at `head`: nothing more, nothing less. It compares
    /// each part and the ways it weakened, in any order, never the part as it was and is now:
    /// that TOML is how a reader sees the change, and rendering it otherwise accepts nothing new.
    pub fn covers(&self, head: &str, weakened: &[Weakening]) -> bool {
        let sorted = |w: &[Weakening]| {
            let mut w: Vec<_> = w
                .iter()
                .map(|w| {
                    let mut hows = w.hows.clone();
                    hows.sort();
                    hows.dedup();
                    (w.part.clone(), hows)
                })
                .collect();
            w.sort();
            w.dedup();
            w
        };
        self.head == head && sorted(&self.weakened) == sorted(weakened)
    }
}

/// Where review of a head stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "ReviewState"))]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// No CI run of the head has opened yet.
    Ci,
    /// A CI run of the head is running.
    Running,
    /// The proof of the head failed: a check, or what it weakens while no admin accepted it.
    Failed,
    /// A CI job of the head has no proof: every run of it ended without one.
    Cancelled,
    /// An open note says what should change: the agent has notes to act on.
    NeedsChange,
    /// A picture waits for a look.
    Waits,
    /// Every picture to look at looks right and every weakening is accepted.
    Approved,
}

/// Where one picture stands in review.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "PictureStanding"))]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    /// Not looked at yet: no open note on it, and no looks right verdict on its content.
    Waits,
    /// An open note on it says what should change.
    NeedsChange,
    /// A person said its content looks right.
    LooksRight,
}

/// One picture of the head's runs a person looks at: changed, new, gone, timing or unreadable,
/// and not flaky.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "ReviewPicture"))]
pub struct Picture {
    /// The check that took it.
    pub check: String,
    /// Its name in its look.
    pub name: String,
    /// What it shows, as [`key`] names it.
    pub key: String,
    /// Where it stands: an open note on it outweighs a looks right verdict.
    pub standing: Standing,
    /// The looks right verdict on it, when someone gave one on this content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Verdict>,
    /// What a person said about what it showed before it changed, so it shows for a look
    /// again; none when nothing was said about other content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed_since: Option<ChangedSince>,
}

/// What a person said about what a picture showed before: it changed since. The verdict and
/// the notes stay as they were; a person says looks right again, or resolves the notes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct ChangedSince {
    /// The newest looks right verdict on what it showed before, when none is on what it shows
    /// now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Verdict>,
    /// The ids of its open notes made on what it showed before.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// Where review of one head stands, and what waits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Review {
    /// The commit it is of.
    pub head: String,
    /// Where it stands.
    pub state: State,
    /// Every picture to look at, in the order of its runs and looks.
    pub pictures: Vec<Picture>,
    /// What the head weakens, as its proofs show it: it fails the proof until an admin accepts
    /// it, and the accept runs the proof job again.
    pub weakened: Vec<Weakening>,
    /// The accept of exactly those weakenings, once an admin gave it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accept: Option<Accept>,
    /// The work's open notes on pictures the head asks a person to look at, oldest first: they
    /// follow the work, not the head, and one whose picture the head does not show lapses.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Note>,
    /// Where the preview deploy of the head lives, as its proof names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    /// What never reached the host, when that is why the review is cancelled.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<Missing>,
    /// Where it stands in one line, as [`Review::title_for`] says it to each role; empty in a
    /// review stored before the host sent it, until [`Review::titled`].
    #[serde(default)]
    pub title: Title,
}

/// Where review of a head stands in one line, said to a person of each role, so a page shows
/// the line its viewer can act on without wording it again.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "ReviewTitle"))]
pub struct Title {
    /// To someone who may only read: signed out, through a share link, or read access. GitHub's
    /// check run and comment say this one.
    pub read: String,
    /// To someone with write access, who gives verdicts.
    pub write: String,
    /// To an admin, who also accepts weakenings.
    pub admin: String,
}

/// A CI job that waits for a runner, before Fearlezz in it opens a run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Job {
    /// Its name, as its workflow names it.
    pub name: String,
    /// The commit it will prove.
    pub head: String,
    /// Its page on GitHub.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Since when it waits, in seconds since the epoch.
    pub since: f64,
}

/// What a picture or text shows, as a verdict follows it: the SHA-256 of its own file (its
/// after), from `sha`, the SHA-256 of a file of the run by its path; a gone one is the one it
/// was (`gone:<before>`). Only the picture a person saw counts, so a base drawn again keeps it;
/// whether another picture looks the same as it is what a run's CLI compares ([`Since`]).
/// None while a file it names is not known.
pub fn key(item: &Item, sha: impl Fn(&str) -> Option<String>) -> Option<String> {
    match (&item.after, &item.before) {
        (Some(after), _) => sha(after),
        (None, Some(before)) => Some(format!("gone:{}", sha(before)?)),
        (None, None) => None,
    }
}

/// What a run's CLI found comparing each picture of a check with pictures a person said
/// something about ([`approved`]): by picture name, the key of each and whether the picture
/// looks the same as it now, by the one comparison the CLI and the page share (a film that only
/// changed its timing does not).
pub type Since = BTreeMap<String, BTreeMap<String, bool>>;

/// The pictures a person said something about, which a run compares its own with: by check
/// and picture name, the key of the newest looks right on it and of the picture each open
/// note was made on, each with what its check said happened in that film (its marks), which
/// a film is timed from.
pub type Approved = BTreeMap<String, BTreeMap<String, BTreeMap<String, Option<Marks>>>>;

/// The pictures of `runs` (every run known) a person said something about, which a new run
/// compares its own with: the newest looks right on each picture, and the picture each open
/// note was made on.
pub fn approved(runs: &[ProvenRun], verdicts: &[Verdict], notes: &[Note]) -> Approved {
    let mut approved = Approved::new();
    let mut newest: BTreeMap<(&str, &str), &Verdict> = BTreeMap::new();
    for v in verdicts {
        let at = newest.entry((&v.check, &v.picture)).or_insert(v);
        if v.at > at.at {
            *at = v;
        }
    }
    let mut add = |check: &str, name: &str, key: &str, run: &str| {
        let marks = item_in(runs, run, check, name).and_then(|i| i.marks.clone());
        let known = approved
            .entry(check.to_owned())
            .or_default()
            .entry(name.to_owned())
            .or_default()
            .entry(key.to_owned())
            .or_default();
        if known.is_none() {
            *known = marks;
        }
    };
    for ((check, name), v) in newest {
        add(check, name, &v.key, &v.run);
    }
    for n in notes.iter().filter(|n| n.state == NoteState::Open) {
        if let Some(key) = key_in(runs, &n.run, &n.check, &n.picture) {
            add(&n.check, &n.picture, key, &n.run);
        }
    }
    approved
}

/// Picture `name` of `check` as run `id` showed it, when that run is among `runs`.
fn item_in<'a>(runs: &'a [ProvenRun], id: &str, check: &str, name: &str) -> Option<&'a Item> {
    let (run, _) = runs.iter().find(|(run, _)| run.id == id)?;
    let check = run.checks.iter().find(|c| c.id == check)?;
    check.look.as_ref()?.items.iter().find(|i| i.name == name)
}

/// Whether picture `name` of `check`, now `key`, looks as the picture of key `then` did: the
/// same file, or one its run's CLI found the same.
fn alike(check: &RunCheck, name: &str, key: &str, then: &str) -> bool {
    then == key
        || check
            .since
            .get(name)
            .and_then(|s| s.get(then))
            .copied()
            .unwrap_or(false)
}

/// The content key `check` gave picture `name` in run `id`, when that run is among `runs`.
fn key_in<'a>(runs: &'a [ProvenRun], id: &str, check: &str, name: &str) -> Option<&'a String> {
    let (run, _) = runs.iter().find(|(run, _)| run.id == id)?;
    run.checks.iter().find(|c| c.id == check)?.keys.get(name)
}

/// What was said about another look of picture `name` of `check` (`run_check` its check in the
/// run shown), whose content is now `key`: the newest looks right verdict on a picture that
/// does not look alike (unless one is on a picture alike), and the open notes made on a run
/// whose picture does not look alike. A note whose run is not known says nothing.
fn changed_since(
    runs: &[ProvenRun],
    (run_check, name, key): (&RunCheck, &str, &str),
    verdicts: &[Verdict],
    verdict: Option<&Verdict>,
    open: &[&Note],
) -> Option<ChangedSince> {
    let check = run_check.id.as_str();
    let verdict = match verdict {
        Some(_) => None,
        None => verdicts
            .iter()
            .filter(|v| v.check == check && v.picture == name)
            .max_by(|a, b| a.at.total_cmp(&b.at))
            .filter(|v| !alike(run_check, name, key, &v.key))
            .cloned(),
    };
    let notes: Vec<String> = open
        .iter()
        .filter(|n| n.check == check && n.picture == name)
        .filter(|n| {
            key_in(runs, &n.run, check, name).is_some_and(|then| !alike(run_check, name, key, then))
        })
        .map(|n| n.id.clone())
        .collect();
    (verdict.is_some() || !notes.is_empty()).then_some(ChangedSince { verdict, notes })
}

/// Each picture or text of the live runs a person looks at, once per content, with its
/// verdict, where it stands and what was said about it before it changed (`runs` are every run
/// known, to find what an open note was made on).
fn looked_at(
    live: &[&ProvenRun],
    runs: &[ProvenRun],
    verdicts: &[Verdict],
    open: &[&Note],
) -> Vec<Picture> {
    let mut pictures: Vec<Picture> = Vec::new();
    for (run, _) in live {
        for check in &run.checks {
            let Some(look) = &check.look else { continue };
            for item in look.items.iter().filter(|i| i.to_look_at()) {
                let Some(key) = check.keys.get(&item.name) else {
                    continue;
                };
                let seen = pictures
                    .iter()
                    .any(|p| p.check == check.id && p.name == item.name && &p.key == key);
                if seen {
                    continue;
                }
                // The newest looks right on a picture that looks alike.
                let verdict = verdicts
                    .iter()
                    .filter(|v| v.check == check.id && v.picture == item.name)
                    .filter(|v| alike(check, &item.name, key, &v.key))
                    .max_by(|a, b| a.at.total_cmp(&b.at))
                    .cloned();
                let noted = open
                    .iter()
                    .any(|n| n.check == check.id && n.picture == item.name);
                let standing = match (noted, &verdict) {
                    (true, _) => Standing::NeedsChange,
                    (false, Some(_)) => Standing::LooksRight,
                    (false, None) => Standing::Waits,
                };
                let changed_since = changed_since(
                    runs,
                    (check, &item.name, key),
                    verdicts,
                    verdict.as_ref(),
                    open,
                );
                pictures.push(Picture {
                    check: check.id.clone(),
                    name: item.name.clone(),
                    key: key.clone(),
                    standing,
                    verdict,
                    changed_since,
                });
            }
        }
    }
    pictures
}

/// What GitHub Actions told the host of a head's CI, through its webhooks.
#[derive(Clone, Debug, Default)]
pub struct Actions {
    /// The display name GitHub gives each run's job, by run id (see [`Leg`]).
    pub legs: BTreeMap<String, String>,
    /// The workflow run attempts of the head that completed, as (`run_id`, `run_attempt`): a
    /// leg their plan made that opened no run by then never will.
    pub ended: BTreeSet<(u64, u32)>,
    /// Those of them that are runs of a workflow the repository proves with: one that ended
    /// while no run of the head reached the host ended without its proof.
    pub proof_ended: BTreeSet<(u64, u32)>,
    /// The names of the jobs of each attempt, as GitHub's `workflow_job` webhook gave them,
    /// which name the job of a leg that never reached the host.
    pub jobs: BTreeMap<(u64, u32), Vec<String>>,
}

/// What never reached the host though the head's CI ended, which the review names and the
/// host runs again once.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "ReviewMissing"))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Missing {
    /// A leg the plan of attempt `run_attempt` of workflow run `run_id` made that opened no
    /// run before the attempt completed, with its job's name when one job of the attempt
    /// names the leg.
    Leg {
        /// The leg's id, as the plan names it (`macos`).
        leg: String,
        /// The display name of its job (`leg macos`), when one job of the attempt names it.
        job: Option<String>,
        /// The workflow run (`GITHUB_RUN_ID`).
        run_id: u64,
        /// Its attempt that completed.
        run_attempt: u32,
    },
    /// The newest attempt of the proof workflow's run of the head, which completed with no run
    /// of the head reaching the host.
    Workflow {
        /// The workflow run (`GITHUB_RUN_ID`).
        run_id: u64,
        /// Its newest attempt that completed.
        run_attempt: u32,
    },
}

/// Whether job `name` names leg `leg`: the leg alone, or one word of it (`leg macos`,
/// `leg (macos, macos-latest)`).
pub fn names_leg(name: &str, leg: &str) -> bool {
    name == leg
        || name
            .split(|c: char| c.is_whitespace() || "(),".contains(c))
            .any(|word| word == leg)
}

/// The review of `head` from the work's runs (newest first, each with its proof once done),
/// what GitHub Actions said of their jobs (`actions`), its verdicts, its notes and the head's
/// accept. Only CI runs of the head count, the newest of each leg that did not end without its
/// proof: a workflow run again replaces the legs it ran, every other leg keeps its newest run,
/// and a run again that is cancelled replaces nothing, so the older run's failure and pictures
/// stay in review. A leg whose every run was cancelled has no proof, which keeps the review
/// from approving; so has a leg the newest attempt of a workflow run planned that opened no
/// run before every such attempt ended.
pub fn review(
    head: &str,
    runs: &[(Run, Option<Proof>)],
    actions: &Actions,
    verdicts: &[Verdict],
    notes: &[Note],
    accept: Option<&Accept>,
) -> Review {
    // Each leg, with its newest run that did not end without its proof, if it has one.
    let mut seen: Vec<(Leg<'_>, Option<&ProvenRun>)> = Vec::new();
    for entry in runs {
        let Some(leg) = Leg::of(&entry.0, head, &actions.legs) else {
            continue;
        };
        let counts = (entry.0.state != RunState::Cancelled).then_some(entry);
        match seen.iter_mut().find(|(l, _)| *l == leg) {
            Some((_, chosen)) => *chosen = chosen.or(counts),
            None => seen.push((leg, counts)),
        }
    }
    // The plans that count: the newest attempt of each workflow run of the head, since running
    // it again plans anew and may leave out a leg whose pass it reuses.
    let mut plans: Vec<(u64, u32, &[String])> = Vec::new();
    for (run, _) in runs.iter().filter(|(run, _)| run.head == head) {
        let Source::Ci {
            planned,
            run_id,
            run_attempt,
            ..
        } = &run.source
        else {
            continue;
        };
        match plans.iter_mut().find(|(id, ..)| id == run_id) {
            Some(plan) if plan.1 < *run_attempt => *plan = (*run_id, *run_attempt, planned),
            Some(_) => {}
            None => plans.push((*run_id, *run_attempt, planned)),
        }
    }
    // A leg a plan made that has not opened a run is still to come while any attempt whose
    // plan made it runs, and has no proof once every such attempt ended.
    let mut missing: Vec<(&str, bool, (u64, u32))> = Vec::new();
    for (run_id, run_attempt, planned) in plans {
        let ended = actions.ended.contains(&(run_id, run_attempt));
        for leg in planned {
            if seen.iter().any(|(l, _)| *l == Leg::Planned(leg)) {
                continue;
            }
            match missing.iter_mut().find(|(l, ..)| l == leg) {
                Some((_, all_ended, attempt)) => {
                    *all_ended &= ended;
                    *attempt = (*attempt).max((run_id, run_attempt));
                }
                None => missing.push((leg, ended, (run_id, run_attempt))),
            }
        }
    }
    let to_come = missing.iter().any(|(_, ended, _)| !ended);
    let unproven = !missing.is_empty() || seen.iter().any(|(_, chosen)| chosen.is_none());
    let live: Vec<&ProvenRun> = seen.iter().filter_map(|(_, chosen)| *chosen).collect();

    let open: Vec<&Note> = notes.iter().filter(|n| n.is_open()).collect();
    let pictures = looked_at(&live, runs, verdicts, &open);
    // An open note holds review only on a picture the head asks a person to look at: one on a
    // picture renamed, removed or no longer changed lapses, since nothing shows it.
    let open: Vec<&Note> = open
        .into_iter()
        .filter(|n| {
            pictures
                .iter()
                .any(|p| p.check == n.check && p.name == n.picture)
        })
        .collect();
    let mut weakened: Vec<Weakening> = Vec::new();
    for proof in live.iter().filter_map(|(_, proof)| proof.as_ref()) {
        for w in &proof.weakened {
            if !weakened.contains(w) {
                weakened.push(w.clone());
            }
        }
    }
    // A proof is a CI run's word, and the preview becomes a link on the page and in the
    // pull request's comment, so only a plain web address is taken.
    let preview = live
        .iter()
        .find_map(|(_, proof)| proof.as_ref()?.preview.clone())
        .filter(|url| web_address(url));
    let accept = accept
        .filter(|a| !weakened.is_empty() && a.covers(head, &weakened))
        .cloned();

    let failed = live.iter().any(|(run, proof)| {
        run.state == RunState::Done
            && proof
                .as_ref()
                .is_none_or(|p| failed(p, head, accept.as_ref()))
    });
    // The proof workflow's newest attempt of the head that ended, while no run reached the host.
    let workflow = actions.proof_ended.iter().max().copied();
    let state = if seen.is_empty() && workflow.is_none() {
        State::Ci
    } else if seen.is_empty() {
        State::Cancelled
    } else if to_come || live.iter().any(|(run, _)| run.state == RunState::Running) {
        State::Running
    } else if failed {
        State::Failed
    } else if unproven {
        State::Cancelled
    } else if !open.is_empty() {
        State::NeedsChange
    } else if pictures.iter().any(|p| p.standing == Standing::Waits) {
        State::Waits
    } else {
        State::Approved
    };
    let missing = match (state, workflow) {
        (State::Cancelled, Some((run_id, run_attempt))) if seen.is_empty() => {
            vec![Missing::Workflow {
                run_id,
                run_attempt,
            }]
        }
        (State::Cancelled, _) => missing
            .into_iter()
            .map(|(leg, _, (run_id, run_attempt))| {
                let names = actions.jobs.get(&(run_id, run_attempt));
                let mut naming = names
                    .into_iter()
                    .flatten()
                    .filter(|name| names_leg(name, leg));
                let job = match (naming.next(), naming.next()) {
                    (Some(name), None) => Some(name.clone()),
                    _ => None,
                };
                Missing::Leg {
                    leg: leg.to_owned(),
                    job,
                    run_id,
                    run_attempt,
                }
            })
            .collect(),
        _ => Vec::new(),
    };
    Review {
        head: head.to_owned(),
        state,
        pictures,
        weakened,
        accept,
        notes: open.into_iter().cloned().collect(),
        preview,
        missing,
        title: Title::default(),
    }
    .titled()
}

/// Whether the proof of a done run of `head` failed, as the host judges it: something to fix, a
/// failed check, or what it weakens unless `accept`, the host's own accept of the head, covers
/// it. The accept a proof names is the run's word, so the host never takes it alone.
pub fn failed(proof: &Proof, head: &str, accept: Option<&Accept>) -> bool {
    let weakened =
        !proof.weakened.is_empty() && accept.is_none_or(|a| !a.covers(head, &proof.weakened));
    proof.to_fix() > 0 || weakened || proof.checks.iter().any(|c| c.outcome == Ran::Failed)
}

/// Whether `url` is a plain `http://` or `https://` address with a host: nothing that runs
/// script, breaks a line or reads as Markdown.
fn web_address(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    else {
        return false;
    };
    !rest.is_empty()
        && !rest.starts_with('/')
        && !rest
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "\"'<>()[]{}`\\".contains(c))
}

/// A run with its proof once done.
type ProvenRun = (Run, Option<Proof>);

/// The leg of a CI job a run proves: the job (`GITHUB_JOB`) with the display name GitHub gives
/// it, which tells the legs of a matrix apart (`proof (macos-latest)`) and stays the same when
/// a workflow runs again. Only GitHub knows that name; the host learns it from the
/// `workflow_job` webhook, by the run's workflow run, attempt and runner. Until it has, the run
/// is a leg of its own, so no newer run replaces it and its pictures never drop out of review.
#[derive(PartialEq)]
enum Leg<'a> {
    /// A leg of a proof Fearlezz split over several jobs, as the run names it.
    Planned(&'a str),
    Named {
        job: &'a str,
        name: &'a str,
    },
    Unknown {
        run: &'a str,
    },
}

impl<'a> Leg<'a> {
    /// The leg of `run` when it is a CI run of `head`.
    fn of(run: &'a Run, head: &str, legs: &'a BTreeMap<String, String>) -> Option<Leg<'a>> {
        let Source::Ci { job, leg, .. } = &run.source else {
            return None;
        };
        if run.head != head {
            return None;
        }
        Some(match (leg, legs.get(&run.id)) {
            (Some(leg), _) => Leg::Planned(leg),
            (None, Some(name)) => Leg::Named { job, name },
            (None, None) => Leg::Unknown { run: &run.id },
        })
    }
}

impl Review {
    /// The pictures that wait for a look.
    pub fn waiting(&self) -> impl Iterator<Item = &Picture> {
        self.standing(Standing::Waits)
    }

    /// The pictures that stand at `standing`.
    pub fn standing(&self, standing: Standing) -> impl Iterator<Item = &Picture> {
        self.pictures.iter().filter(move |p| p.standing == standing)
    }

    /// The pictures open notes are on, by check and name, once each in the order of the notes:
    /// what needs change, whether or not the head's runs show it to look at.
    pub fn noted(&self) -> Vec<(&str, &str)> {
        let mut noted: Vec<(&str, &str)> = Vec::new();
        for n in &self.notes {
            let at = (n.check.as_str(), n.picture.as_str());
            if !noted.contains(&at) {
                noted.push(at);
            }
        }
        noted
    }

    /// Whether the weakenings wait for an admin's accept.
    pub fn weakenings_wait(&self) -> bool {
        !self.weakened.is_empty() && self.accept.is_none()
    }

    /// The review with its [`Title`] derived from where it stands.
    pub fn titled(self) -> Review {
        let title = Title {
            read: self.title_for(Role::Less),
            write: self.title_for(Role::Write),
            admin: self.title_for(Role::Admin),
        };
        Review { title, ..self }
    }

    /// Where it stands in one line, as the check run's title and the comment say it.
    pub fn title(&self) -> String {
        self.title_for(Role::Less)
    }

    /// Where it stands in one line, said to a person of `role`: what waits for their look or
    /// their accept says so, when they may give it.
    pub fn title_for(&self, role: Role) -> String {
        let head = short(&self.head);
        match self.state {
            State::Ci => format!("Waiting for CI to prove {head}"),
            State::Running => format!("CI is proving {head}"),
            State::Failed if self.weakenings_wait() => {
                let whose = if role == Role::Admin {
                    "your"
                } else {
                    "an admin's"
                };
                format!("The proof of {head} failed: what it weakens waits for {whose} accept")
            }
            State::Failed => format!("The proof of {head} failed"),
            State::Cancelled if self.missing.is_empty() => {
                format!("A CI job of {head} ended without its proof: run it again")
            }
            State::Cancelled => self.missing_title(),
            State::NeedsChange => {
                let n = self.noted().len();
                let verb = if n == 1 { "needs" } else { "need" };
                format!("{} {verb} change at {head}", crate::plural(n, "picture"))
            }
            State::Waits => {
                let whose = if role >= Role::Write { "your" } else { "a" };
                match self.waiting().count() {
                    1 => format!("1 picture waits for {whose} look at {head}"),
                    n => format!("{n} pictures wait for {whose} look at {head}"),
                }
            }
            State::Approved if self.pictures.is_empty() && self.weakened.is_empty() => {
                format!("Proven at {head}, with nothing to review")
            }
            State::Approved => format!("Everything looks right at {head}"),
        }
    }

    /// What never reached the host in one line, and what to run again.
    fn missing_title(&self) -> String {
        let head = short(&self.head);
        let mut legs: Vec<&str> = Vec::new();
        let mut jobs: Vec<Option<&str>> = Vec::new();
        for missing in &self.missing {
            match missing {
                Missing::Workflow { .. } => {
                    return format!(
                        "The proof workflow of {head} ended without reaching the host: re-run it"
                    );
                }
                Missing::Leg { leg, job, .. } => {
                    legs.push(leg);
                    jobs.push(job.as_deref());
                }
            }
        }
        let quoted: Option<Vec<String>> = jobs
            .iter()
            .map(|job| job.map(|j| format!("\"{j}\"")))
            .collect();
        let (what, rerun) = match (legs.len(), quoted) {
            (1, Some(jobs)) => ("leg", format!("re-run job {}", jobs.join(", "))),
            (1, None) => ("leg", "re-run its job".to_owned()),
            (_, Some(jobs)) => ("legs", format!("re-run jobs {}", jobs.join(", "))),
            (_, None) => ("legs", "re-run their jobs".to_owned()),
        };
        format!(
            "{what} {} of {head} never reached the host: {rerun}",
            legs.join(", ")
        )
    }

    /// What waits and what is settled, in Markdown, as the check run's summary says it.
    pub fn summary(&self) -> String {
        let mut lines = vec![format!("{}.", self.title())];
        let legs: Vec<String> = self
            .missing
            .iter()
            .filter_map(|m| match m {
                Missing::Leg {
                    leg,
                    job,
                    run_id,
                    run_attempt,
                } => {
                    let job = job
                        .as_ref()
                        .map(|j| format!(", job \"{j}\""))
                        .unwrap_or_default();
                    Some(format!(
                        "- `{leg}`{job}, in attempt {run_attempt} of workflow run {run_id}"
                    ))
                }
                Missing::Workflow { .. } => None,
            })
            .collect();
        if !legs.is_empty() {
            lines.push(
                "\nLegs that never reached the host; the host runs each job again once:".into(),
            );
            lines.extend(legs);
        }
        if self
            .missing
            .iter()
            .any(|m| matches!(m, Missing::Workflow { .. }))
        {
            lines.push(
                "\nNo run of the head reached the host before the proof workflow ended; the \
                 host runs it again once."
                    .into(),
            );
        }
        let waiting: Vec<&Picture> = self.waiting().collect();
        if !waiting.is_empty() {
            lines.push("\nPictures that wait for a look:".into());
            lines.extend(
                waiting
                    .iter()
                    .map(|p| format!("- `{}` {}", p.check, p.name)),
            );
        }
        let changed: Vec<String> = self
            .pictures
            .iter()
            .filter_map(|p| {
                let since = p.changed_since.as_ref()?;
                let what = match (&since.verdict, since.notes.len()) {
                    (Some(v), 0) => format!("{} said it looks right", v.by),
                    (_, 1) => "its note".to_owned(),
                    (_, _) => "its notes".to_owned(),
                };
                Some(format!("- `{}` {}, changed since {what}", p.check, p.name))
            })
            .collect();
        if !changed.is_empty() {
            lines.push("\nPictures that changed since a person looked, to look at again:".into());
            lines.extend(changed);
        }
        if !self.notes.is_empty() {
            lines.push("\nPictures that need change, by their open notes:".into());
            lines.extend(self.notes.iter().map(|n| {
                let at = n.time.map(|t| format!(" at {t:.2} s")).unwrap_or_default();
                format!(
                    "- `{}` {}, {}{at}: {} ({})",
                    n.check,
                    n.picture,
                    n.side.label(),
                    quote(&n.words),
                    n.by
                )
            }));
        }
        let right: Vec<(&Picture, &Verdict)> = self
            .standing(Standing::LooksRight)
            .filter_map(|p| Some((p, p.verdict.as_ref()?)))
            .collect();
        if !right.is_empty() {
            lines.push("\nPictures that look right:".into());
            lines.extend(
                right
                    .iter()
                    .map(|(p, v)| format!("- `{}` {}, by {}", p.check, p.name, v.by)),
            );
        }
        if !self.weakened.is_empty() {
            lines.push(match &self.accept {
                Some(a) => format!("\nWhat it weakens, accepted by {}:", a.by),
                None => "\nWhat it weakens, which fails the proof until an admin accepts it on \
                         the work's page:"
                    .into(),
            });
            lines.extend(self.weakened.iter().map(|w| format!("- {w}")));
        }
        lines.join("\n") + "\n"
    }
}

/// A person's words on one line of Markdown, quoted, with every character Markdown gives a
/// meaning escaped, so they never become a link, a picture, a mention or markup of their own.
fn quote(words: &str) -> String {
    let mut line = String::new();
    for (i, word) in words.split_whitespace().enumerate() {
        if i > 0 {
            line.push(' ');
        }
        for c in word.chars() {
            if "\\`*_{}[]()#+-.!|<>~@&:".contains(c) {
                line.push('\\');
            }
            line.push(c);
        }
    }
    format!("\u{201c}{line}\u{201d}")
}

/// The content key of each picture of a look, by name, from the SHA-256s of the run's files
/// by path; a picture whose files are not all known has none yet.
pub fn keys(items: &[Item], files: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    items
        .iter()
        .filter_map(|item| {
            let key = key(item, |path| files.get(path).cloned())?;
            Some((item.name.clone(), key))
        })
        .collect()
}
