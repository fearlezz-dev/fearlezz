//! Review on the proof host: a person's verdict on each picture, an admin's accept of a head's
//! weakenings, and where review of a head stands. The host derives a head's review from its CI
//! runs, the work's verdicts and the head's accept, shows it on the work's page and writes it to
//! GitHub as its check run and pull request comment, so the page and GitHub say one thing.
//!
//! A verdict follows the picture, not the run: it is kept by check, picture name and content, so
//! a picture that did not change keeps its verdict across pushes and runs of the same work.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::events::{Run, RunState, Source};
use crate::floor::Weakening;
use crate::look::Item;
use crate::notes::Note;
use crate::{Proof, Ran, short};

/// What a person may do on a repository, as GitHub's collaborator permission says it: a
/// verdict, a note or a share link needs write access, an accept admin rights.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    /// Read, or nothing.
    Less,
    /// Write or maintain.
    Write,
    /// Admin.
    Admin,
}

impl Role {
    /// The role GitHub's `permission` field names: `admin`, `write`, or anything less.
    pub fn of(permission: &str) -> Role {
        match permission {
            "admin" => Role::Admin,
            "write" => Role::Write,
            _ => Role::Less,
        }
    }
}

/// What a person says of a picture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum VerdictKind {
    /// The picture looks right.
    LooksRight,
    /// It does not look right yet: what should change is in the open notes on it, which a
    /// person makes before saying this.
    NeedsChange,
}

impl VerdictKind {
    /// How it is stored and sent: `looks_right` or `needs_change`.
    pub fn word(self) -> &'static str {
        match self {
            VerdictKind::LooksRight => "looks_right",
            VerdictKind::NeedsChange => "needs_change",
        }
    }

    /// The kind a stored word names; anything else is the one before notes came.
    pub fn of(word: &str) -> VerdictKind {
        match word {
            "needs_change" => VerdictKind::NeedsChange,
            _ => VerdictKind::LooksRight,
        }
    }
}

/// A person's verdict on one picture, as its content was when they gave it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "PictureVerdict"))]
pub struct Verdict {
    /// The check that took the picture.
    pub check: String,
    /// The picture's name in its look.
    pub picture: String,
    /// What the picture showed, as [`key`] names it.
    pub key: String,
    /// What they said.
    pub kind: VerdictKind,
    /// Their GitHub login.
    pub by: String,
    /// When, in seconds since the epoch.
    pub at: f64,
    /// The run they gave it on.
    pub run: String,
    /// The commit that run proved.
    pub head: String,
}

/// An admin's accept of every weakening of one head.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Accept {
    /// The commit it accepts the weakenings of.
    pub head: String,
    /// Their GitHub login.
    pub by: String,
    /// When, in seconds since the epoch.
    pub at: f64,
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
    /// The proof of the head failed.
    Failed,
    /// Every CI run of the head ended without its proof.
    Cancelled,
    /// A person said a picture needs change: the agent has notes to act on.
    NeedsChange,
    /// A picture waits for a verdict, or a weakening for an admin's accept.
    Waits,
    /// Every picture to look at looks right and every weakening is accepted.
    Approved,
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
    /// The verdict on it, when someone gave one on this content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Verdict>,
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
    /// What the head weakens, waiting for an admin's accept.
    pub weakened: Vec<Weakening>,
    /// The accept of those weakenings, once an admin gave it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accept: Option<Accept>,
    /// The work's open notes, oldest first: they follow the work, not the head.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Note>,
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

/// What a picture shows, as a verdict follows it, from `sha`, the SHA-256 of a file of the run
/// by its path. A still is its before's and after's bytes (`still:<before>:<after>`, `-` for a
/// side it lacks). A compared film's bytes differ each time it is recorded, so it is which of
/// its stable states changed, as its comparison says (`film:<states before>:<states
/// after>:<pairs that differ>`). A film nothing compared (new, gone) is its bytes like a
/// still (`film:<before>:<after>`), so its verdict never carries over to another recording.
/// None while a file it names is not known.
pub fn key(item: &Item, sha: impl Fn(&str) -> Option<String>) -> Option<String> {
    let kind = if item.kind == "film" { "film" } else { "still" };
    if let (true, Some(c)) = (kind == "film", &item.comparison) {
        let changed: Vec<String> = c
            .pairs
            .iter()
            .filter(|p| p.px > 0)
            .map(|p| format!("{}-{}", p.before, p.after))
            .collect();
        return Some(format!(
            "film:{}:{}:{}",
            c.states_before,
            c.states_after,
            changed.join(",")
        ));
    }
    let side = |path: &Option<String>| match path {
        Some(path) => sha(path),
        None => Some("-".to_owned()),
    };
    Some(format!(
        "{kind}:{}:{}",
        side(&item.before)?,
        side(&item.after)?
    ))
}

/// The review of `head` from the work's runs (newest first, each with its proof once done),
/// its verdicts, its notes and the head's accept. Only CI runs of the head count, the newest of
/// each job.
pub fn review(
    head: &str,
    runs: &[(Run, Option<Proof>)],
    verdicts: &[Verdict],
    notes: &[Note],
    accept: Option<&Accept>,
) -> Review {
    let mut jobs: Vec<&str> = Vec::new();
    let mut counted: Vec<&(Run, Option<Proof>)> = Vec::new();
    for entry in runs {
        let Source::Ci { job, .. } = &entry.0.source else {
            continue;
        };
        if entry.0.head != head || jobs.contains(&job.as_str()) {
            continue;
        }
        jobs.push(job);
        counted.push(entry);
    }
    let live: Vec<&(Run, Option<Proof>)> = counted
        .iter()
        .copied()
        .filter(|(run, _)| run.state != RunState::Cancelled)
        .collect();

    let mut pictures: Vec<Picture> = Vec::new();
    for (run, _) in &live {
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
                let verdict = verdicts
                    .iter()
                    .find(|v| v.check == check.id && v.picture == item.name && &v.key == key)
                    .cloned();
                pictures.push(Picture {
                    check: check.id.clone(),
                    name: item.name.clone(),
                    key: key.clone(),
                    verdict,
                });
            }
        }
    }
    let mut weakened: Vec<Weakening> = Vec::new();
    for (_, proof) in &live {
        let Some(proof) = proof.as_ref().filter(|p| p.weakenings_wait) else {
            continue;
        };
        for w in &proof.weakened {
            if !weakened.contains(w) {
                weakened.push(w.clone());
            }
        }
    }
    let accept = accept
        .filter(|a| a.head == head && !weakened.is_empty())
        .cloned();

    let failed = live.iter().any(|(run, proof)| {
        run.state == RunState::Done
            && proof.as_ref().is_none_or(|p| {
                !p.blockers().is_empty() || p.checks.iter().any(|c| c.outcome == Ran::Failed)
            })
    });
    let state = if counted.is_empty() {
        State::Ci
    } else if live.iter().any(|(run, _)| run.state == RunState::Running) {
        State::Running
    } else if live.is_empty() {
        State::Cancelled
    } else if failed {
        State::Failed
    } else if pictures.iter().any(Picture::needs_change) {
        State::NeedsChange
    } else if pictures.iter().any(|p| p.verdict.is_none())
        || (!weakened.is_empty() && accept.is_none())
    {
        State::Waits
    } else {
        State::Approved
    };
    Review {
        head: head.to_owned(),
        state,
        pictures,
        weakened,
        accept,
        notes: notes.iter().filter(|n| n.is_open()).cloned().collect(),
    }
}

impl Picture {
    /// Whether a person said its content needs change.
    pub fn needs_change(&self) -> bool {
        self.verdict
            .as_ref()
            .is_some_and(|v| v.kind == VerdictKind::NeedsChange)
    }

    /// Whether a person said its content looks right.
    pub fn looks_right(&self) -> bool {
        self.verdict
            .as_ref()
            .is_some_and(|v| v.kind == VerdictKind::LooksRight)
    }
}

/// `1 picture` or `3 pictures`.
fn pictures(n: usize) -> String {
    if n == 1 {
        "1 picture".into()
    } else {
        format!("{n} pictures")
    }
}

impl Review {
    /// The pictures that wait for a verdict.
    pub fn waiting(&self) -> impl Iterator<Item = &Picture> {
        self.pictures.iter().filter(|p| p.verdict.is_none())
    }

    /// Whether the weakenings wait for an admin's accept.
    pub fn weakenings_wait(&self) -> bool {
        !self.weakened.is_empty() && self.accept.is_none()
    }

    /// Where it stands in one line, as the check run's title and the comment say it.
    pub fn title(&self) -> String {
        let head = short(&self.head);
        match self.state {
            State::Ci => format!("Waiting for CI to prove {head}"),
            State::Running => format!("CI is proving {head}"),
            State::Failed => format!("The proof of {head} failed"),
            State::Cancelled => format!("The CI run of {head} ended without its proof"),
            State::NeedsChange => {
                let n = self.pictures.iter().filter(|p| p.needs_change()).count();
                let verb = if n == 1 { "needs" } else { "need" };
                format!("{} {verb} change at {head}", pictures(n))
            }
            State::Waits => {
                let waiting = self.waiting().count();
                match (waiting, self.weakenings_wait()) {
                    (0, _) => format!("A weakened proof waits for an admin's accept at {head}"),
                    (1, false) => format!("1 picture waits for a look at {head}"),
                    (n, false) => format!("{n} pictures wait for a look at {head}"),
                    (n, true) => format!(
                        "{} for a look and a weakened proof for an admin at {head}",
                        pictures(n)
                    ),
                }
            }
            State::Approved if self.pictures.is_empty() && self.weakened.is_empty() => {
                format!("Proven at {head}, with nothing to review")
            }
            State::Approved => format!("Everything looks right at {head}"),
        }
    }

    /// What waits and what is settled, in Markdown, as the check run's summary says it.
    pub fn summary(&self) -> String {
        let mut lines = vec![format!("{}.", self.title())];
        let waiting: Vec<&Picture> = self.waiting().collect();
        if !waiting.is_empty() {
            lines.push("\nPictures that wait for a look:".into());
            lines.extend(
                waiting
                    .iter()
                    .map(|p| format!("- `{}` {}", p.check, p.name)),
            );
        }
        let changes: Vec<(&Picture, &Verdict)> = self
            .pictures
            .iter()
            .filter(|p| p.needs_change())
            .filter_map(|p| Some((p, p.verdict.as_ref()?)))
            .collect();
        if !changes.is_empty() {
            lines.push("\nPictures that need change:".into());
            lines.extend(
                changes
                    .iter()
                    .map(|(p, v)| format!("- `{}` {}, by {}", p.check, p.name, v.by)),
            );
        }
        if !self.notes.is_empty() {
            lines.push("\nOpen notes:".into());
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
            .pictures
            .iter()
            .filter(|p| p.looks_right())
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
                None => "\nWhat it weakens, waiting for an admin's accept:".into(),
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
