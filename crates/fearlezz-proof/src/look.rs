//! The look at a change: every still and film a check records, compared with the same check
//! run on the base commit.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::media::Comparison;

/// How a picture differs from its before.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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

/// One still or film of a look.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    /// Its path below the check's media folder.
    pub name: String,
    /// `still` or `film`.
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
    /// Named by the check's `flaky`: compared and shown, but never blocks or waits for
    /// approval.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flaky: bool,
}

/// One check's pictures compared with its run on the base commit.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Look {
    /// The check that took the pictures.
    pub check: String,
    /// The base commit it was compared with.
    pub base_commit: String,
    /// How the check did on the base commit.
    pub base_result: String,
    /// Why there is no before (no base branch, the check fails on the base, the look
    /// failed). Then every picture counts as new and needs approval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_before: Option<String>,
    /// Something about the comparison a person should know, such as a base run that passed
    /// without taking any pictures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Every picture.
    pub items: Vec<Item>,
    /// The report page on this machine; empty in a proof bundle.
    #[serde(default, skip_serializing_if = "no_path")]
    pub report: PathBuf,
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

    /// Items a person should look at: anything but same, and never a flaky one.
    pub fn to_look_at(&self) -> usize {
        self.items
            .iter()
            .filter(|i| i.change != Change::Same && !i.flaky)
            .count()
    }

    /// The folder it is saved in.
    pub fn dir(&self) -> &Path {
        self.report.parent().unwrap_or(&self.report)
    }
}
