//! Generated code is up to date: the files a check's `generates` names must come out of its
//! command the same as the tree had them. The command runs in place, as a generator does, so
//! the regenerated files stay and committing them is the whole fix.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::Result;

use fearlezz_proof::why::Generated;

use crate::config::Check;
use crate::git::Git;

/// The name of the diff a run writes into its folder when its generated files differ.
pub(crate) const DIFF: &str = "generated.diff";

/// The generated files as the tree has them before the command runs, by path from the root.
pub(crate) struct Snapshot(BTreeMap<String, Vec<u8>>);

/// The files below `root` that `check` generates, among `files`, those git tracks or would
/// track.
fn read(files: &[String], root: &Path, check: &Check) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut generated = BTreeMap::new();
    for path in files.iter().filter(|p| check.generated(p)) {
        generated.insert(path.clone(), fs::read(root.join(path))?);
    }
    Ok(generated)
}

impl Snapshot {
    /// The files `check` generates in the worktree at `root`, as they are now.
    pub(crate) fn take(root: &Path, check: &Check) -> Result<Snapshot> {
        read(&Git::discover(root)?.files()?, root, check).map(Snapshot)
    }

    /// How the files `check` generates came out of its command compared with this snapshot,
    /// with `files` those git tracks or would track now: why they fail it, if they do, and the
    /// diff of those that differ.
    pub(crate) fn compare(
        &self,
        files: &[String],
        root: &Path,
        check: &Check,
    ) -> Result<(Option<Generated>, String)> {
        let now = read(files, root, check)?;
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
            Some(Generated::Stale(stale))
        } else if now.is_empty() {
            Some(Generated::NamesNoFile(check.generates.patterns().to_vec()))
        } else {
            None
        };
        Ok((failed, diff))
    }
}
