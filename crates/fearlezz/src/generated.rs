//! Generated code is up to date: the files a check's `generates` names must come out of its
//! command the same as the tree had them. The command runs in place, as a generator does, so
//! the regenerated files stay and committing them is the whole fix.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::config::Check;
use crate::git::Git;

/// The name of the diff a run writes into its folder when its generated files differ.
pub(crate) const DIFF: &str = "generated.diff";

/// Why a check's generated files fail it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Failed {
    /// Each file its command wrote otherwise than the tree had it, labeled for people
    /// (`gen/a.rs`, `gen/b.rs (new)`, `gen/c.rs (gone)`); the diff is in the run's folder.
    Stale(Vec<String>),
    /// Neither the tree nor the command has a file its globs (these) name, so it proves
    /// nothing.
    NamesNoFile(Vec<String>),
}

impl Failed {
    /// Why it fails, in a few words.
    pub(crate) fn why(&self) -> String {
        match self {
            Failed::Stale(files) => {
                format!("generated files are out of date: {}", files.join(", "))
            }
            Failed::NamesNoFile(globs) => {
                format!("generates names no file git tracks ({})", globs.join(", "))
            }
        }
    }

    /// What to do about it, for check `check`; `here` when the regenerated files are in the
    /// worktree the person reads this in.
    pub(crate) fn fix(&self, check: &str, here: bool) -> String {
        match (self, here) {
            (Failed::Stale(_), true) => {
                format!("{check} regenerated them; commit them as they are now")
            }
            (Failed::Stale(_), false) => {
                format!("run fearlezz run {check} and commit the files it regenerates")
            }
            (Failed::NamesNoFile(_), _) => "name the files its command writes in generates".into(),
        }
    }
}

/// The generated files as the tree has them before the command runs, by path from the root.
pub(crate) struct Snapshot(BTreeMap<String, Vec<u8>>);

/// The files below `root` that `check` generates, among those git tracks or would track.
fn read(root: &Path, check: &Check) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut files = BTreeMap::new();
    for path in Git::discover(root)?.files()? {
        if check.generated(&path) {
            let bytes = fs::read(root.join(&path))?;
            files.insert(path, bytes);
        }
    }
    Ok(files)
}

impl Snapshot {
    /// The files `check` generates in the worktree at `root`, as they are now.
    pub(crate) fn take(root: &Path, check: &Check) -> Result<Snapshot> {
        read(root, check).map(Snapshot)
    }

    /// How the files `check` generates came out of its command compared with this snapshot:
    /// why they fail it, if they do, and the diff of those that differ.
    pub(crate) fn compare(&self, root: &Path, check: &Check) -> Result<(Option<Failed>, String)> {
        let now = read(root, check)?;
        let mut stale = Vec::new();
        let mut diff = String::new();
        let paths: std::collections::BTreeSet<&String> = self.0.keys().chain(now.keys()).collect();
        for path in paths {
            let (was, is) = (self.0.get(path), now.get(path));
            if was == is {
                continue;
            }
            stale.push(match (was, is) {
                (None, _) => format!("{path} (new)"),
                (_, None) => format!("{path} (gone)"),
                _ => path.clone(),
            });
            let text =
                |b: Option<&Vec<u8>>| b.map(|b| crate::text::readable(b)).unwrap_or_default();
            diff.push_str(&crate::text::unified(path, &text(was), &text(is)));
        }
        let failed = if !stale.is_empty() {
            Some(Failed::Stale(stale))
        } else if now.is_empty() {
            Some(Failed::NamesNoFile(check.generates.patterns().to_vec()))
        } else {
            None
        };
        Ok((failed, diff))
    }
}
