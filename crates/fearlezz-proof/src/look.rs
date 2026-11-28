//! The look at a change: every still, film and text a check takes, compared with the same
//! check run on the base commit.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::media::Comparison;

/// How a picture or text differs from its before.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "PictureChange"))]
#[serde(rename_all = "snake_case")]
pub enum Change {
    /// Other pictures.
    Changed,
    /// The same pictures at other times.
    Timing,
    /// No before.
    New,
    /// No after.
    Gone,
    /// One of the two could not be read.
    Unreadable,
    /// The same.
    Same,
}

impl Change {
    /// The change as people read it.
    pub fn label(self) -> &'static str {
        match self {
            Change::Changed => "changed",
            Change::Timing => "timing",
            Change::New => "new",
            Change::Gone => "gone",
            Change::Unreadable => "unreadable",
            Change::Same => "same",
        }
    }
}

/// One still, film or text of a look.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "Picture"))]
pub struct Item {
    /// Its path below the check's media folder.
    pub name: String,
    /// `still`, `film` or `text`.
    pub kind: String,
    /// How it differs from its before.
    pub change: Change,
    /// The comparison in one line.
    pub summary: String,
    /// The copy of the before inside the look folder, relative to it.
    pub before: Option<String>,
    /// The copy of the after inside the look folder, relative to it.
    pub after: Option<String>,
    /// What comparing them found.
    pub comparison: Option<Comparison>,
    /// Named by the check's `flaky`: compared and shown, but never waits for a person's look.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flaky: bool,
    /// A changed text's lines against its before, as the CLI compared them: kept lines folded
    /// to a count past a few on each side of a change. None for a still or film, and in a
    /// bundle from a CLI that did not write it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<Vec<DiffLine>>,
    /// The animated WebP copy of a film's after inside the look folder, relative to it: the one
    /// form of a film a page that allows only images (a README on GitHub) plays. The CLI makes it
    /// only on a CI run that a push to the default branch started, the only runs the host serves
    /// pictures of at a stable address; None
    /// otherwise, and for a still or text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webp: Option<String>,
    /// What the check said happened in its after film (marks and spans), timing it from them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marks: Option<crate::media::Marks>,
    /// What the check said happened in its before film, on the base commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_marks: Option<crate::media::Marks>,
}

/// One line of a text's diff.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DiffLine {
    /// A line both have.
    Same {
        /// The line, without its line break.
        text: String,
    },
    /// A line only the before has.
    Was {
        /// The line, without its line break.
        text: String,
    },
    /// A line only the after has.
    Now {
        /// The line, without its line break.
        text: String,
    },
    /// Kept lines left out, by how many.
    Fold {
        /// How many.
        count: u32,
    },
}

impl Item {
    /// Whether a person looks at it: anything but the same, and never a flaky one. A film
    /// that only changed its timing waits only past the timing budget
    /// ([`crate::media::TIMING_BUDGET`]).
    pub fn to_look_at(&self) -> bool {
        let on_time = self.change == Change::Timing
            && self
                .comparison
                .as_ref()
                .is_some_and(Comparison::within_budget);
        self.change != Change::Same && !on_time && !self.flaky
    }
}

/// `name` without its extension, and the extension: a short last part after a dot (four
/// characters at most), as the proof page reads a picture's name. A name without one is whole.
pub fn extension(name: &str) -> (&str, &str) {
    match name.rsplit_once('.') {
        Some((bare, ext)) if !bare.is_empty() && ext.len() <= 4 && !ext.contains('/') => {
            (bare, ext)
        }
        _ => (name, ""),
    }
}

/// The name a film's animated WebP copy is served at on the host: the film's name with `.webp`
/// in place of its extension, or added when it has none (`home.webm` and `home` are
/// `home.webp`). The CLI's showcase markup and the host's lookup both use this one rule.
pub fn webp_name(film: &str) -> String {
    format!("{}.webp", extension(film).0)
}

/// One check's pictures compared with its run on the base commit.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Look {
    /// The check that took the pictures.
    pub check: String,
    /// The base commit it was compared with.
    pub base_commit: String,
    /// How the check did on the base commit.
    pub base_result: String,
    /// Why there is no before (no base branch, the check fails on the base, the look
    /// failed). Then every picture counts as new, for a person to look at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_before: Option<String>,
    /// Something about the comparison a person should know, such as a base run that passed
    /// without taking any pictures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Every picture.
    pub items: Vec<Item>,
    /// The folder it is saved in on this machine, which its pictures are relative to; empty
    /// in a proof bundle, whose pictures are relative to the bundle.
    #[serde(default, skip_serializing_if = "no_path")]
    #[cfg_attr(feature = "ts", ts(skip))]
    pub dir: PathBuf,
}

fn no_path(path: &Path) -> bool {
    path.as_os_str().is_empty()
}

impl Look {
    /// For example `1 changed, 2 same, 1 new, 1 flaky`. Flaky pictures count apart.
    pub fn counts(&self) -> String {
        let mut changes: Vec<Change> = self
            .items
            .iter()
            .filter(|i| !i.flaky)
            .map(|i| i.change)
            .collect();
        changes.sort();
        let mut parts: Vec<String> = Vec::new();
        for c in changes.iter().copied() {
            let n = changes.iter().filter(|x| **x == c).count();
            let part = format!("{n} {}", c.label());
            if !parts.contains(&part) {
                parts.push(part);
            }
        }
        let flaky = self.items.iter().filter(|i| i.flaky).count();
        if flaky > 0 {
            parts.push(format!("{flaky} flaky"));
        }
        parts.join(", ")
    }

    /// How many of its pictures a person looks at (see [`Item::to_look_at`]).
    pub fn to_look_at(&self) -> usize {
        self.items.iter().filter(|i| i.to_look_at()).count()
    }
}
