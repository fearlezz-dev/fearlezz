//! The work file: what a piece of work declares, committed with the change so CI can read it.
//! It holds the goal and the claims from the journal, never results, so it never goes stale and
//! never names its own commit. `fearlezz say` writes it; CI proves each claim again.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::journal::{Event, Journal, Kind};

/// The folder of work files in the repository, one per journal.
pub(crate) const DIR: &str = ".fearlezz/work";

const HEADER: &str = "# Written by fearlezz say from the journal of this work. CI proves each \
                      claim again by running its check.\n\n";

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkFile {
    pub kind: Option<Kind>,
    pub goal: Option<String>,
    #[serde(default)]
    pub claim: BTreeMap<String, Claim>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Claim {
    pub text: String,
    pub check: Option<String>,
}

/// Where the work file of the journal called `name` lives, from the repository root.
pub(crate) fn path(name: &str) -> String {
    format!("{DIR}/{name}.toml")
}

pub(crate) fn is_work_file(path: &str) -> bool {
    path.starts_with(DIR)
        && Path::new(path)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("toml"))
}

impl WorkFile {
    /// What the journal declares: its latest kind and goal, and every claim with its check.
    pub(crate) fn of(journal: &Journal) -> WorkFile {
        let mut file = WorkFile::default();
        for entry in &journal.entries {
            match &entry.event {
                Event::Started { kind, goal } => {
                    file.kind = Some(*kind);
                    file.goal.clone_from(goal);
                }
                Event::Claim { id, text, check } => {
                    file.claim.insert(
                        id.clone(),
                        Claim {
                            text: text.clone(),
                            check: check.clone(),
                        },
                    );
                }
                Event::Link { claim, check } => {
                    if let Some(c) = file.claim.get_mut(claim) {
                        c.check = Some(check.clone());
                    }
                }
                Event::Ran { .. }
                | Event::Stage { .. }
                | Event::Wait { .. }
                | Event::Approve { .. }
                | Event::Note { .. } => {}
            }
        }
        file
    }

    fn text(&self) -> Result<String> {
        Ok(format!("{HEADER}{}", toml::to_string(self)?))
    }

    pub(crate) fn read(root: &Path, path: &str) -> Result<WorkFile> {
        let text =
            std::fs::read_to_string(root.join(path)).with_context(|| format!("reading {path}"))?;
        toml::from_str(&text).with_context(|| format!("reading {path}"))
    }
}

/// Whether the work file in `root` says what the journal declares.
pub(crate) fn current(root: &Path, journal: &Journal) -> Result<bool> {
    let want = WorkFile::of(journal).text()?;
    Ok(std::fs::read_to_string(root.join(path(&journal.name))).is_ok_and(|have| have == want))
}

/// Writes the work file of a journal that declared anything, when it differs. Returns its path
/// when written.
pub(crate) fn write(root: &Path, journal: &Journal) -> Result<Option<String>> {
    let file = WorkFile::of(journal);
    if journal.is_in_memory() || file == WorkFile::default() || current(root, journal)? {
        return Ok(None);
    }
    let rel = path(&journal.name);
    let full = root.join(&rel);
    if let Some(dir) = full.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&full, file.text()?)?;
    Ok(Some(rel))
}

/// What the work files a change touches declare, and whether CI proved it.
pub(crate) struct Declared {
    files: Vec<(String, WorkFile)>,
    /// The change touches a file a check reads, so it must declare a goal and claims.
    needs_claims: bool,
}

impl Declared {
    /// The work files among `changed` that still exist.
    pub(crate) fn in_change(
        root: &Path,
        changed: &BTreeSet<String>,
        needs_claims: bool,
    ) -> Result<Declared> {
        let files = changed
            .iter()
            .filter(|p| is_work_file(p) && root.join(p).exists())
            .map(|p| Ok((p.clone(), WorkFile::read(root, p)?)))
            .collect::<Result<_>>()?;
        Ok(Declared {
            files,
            needs_claims,
        })
    }

    /// The checks the claims name, which must run whether or not the change needs them.
    pub(crate) fn checks(&self) -> BTreeSet<String> {
        self.files
            .iter()
            .flat_map(|(_, f)| f.claim.values().filter_map(|c| c.check.clone()))
            .collect()
    }

    /// One line per finding, and whether all is proven, given the checks that ran and passed
    /// here.
    pub(crate) fn judge(
        &self,
        ran: &BTreeSet<String>,
        passed: &BTreeSet<String>,
    ) -> (Vec<String>, bool) {
        let mut lines = Vec::new();
        let mut proven = true;
        if self.files.is_empty() && self.needs_claims {
            lines.push(format!(
                "no goal or claims: a change the checks read needs its work file in {DIR}, \
                 written by fearlezz say start and fearlezz say claim"
            ));
            proven = false;
        }
        for (path, file) in &self.files {
            if let Some(goal) = &file.goal {
                lines.push(format!("work {path}: {goal}"));
            } else {
                lines.push(format!("no goal in {path}"));
                proven = false;
            }
            if file.claim.is_empty() {
                lines.push(format!("no claims in {path}"));
                proven = false;
            }
            for (id, claim) in &file.claim {
                let line = match &claim.check {
                    None => format!("unproven {id}: no check proves it"),
                    Some(check) if passed.contains(check) => {
                        format!("proven {id} by {check}: {}", claim.text)
                    }
                    Some(check) if ran.contains(check) => {
                        format!("unproven {id}: {check} failed here")
                    }
                    Some(check) => format!("unproven {id}: {check} did not run here"),
                };
                proven &= line.starts_with("proven");
                lines.push(line);
            }
        }
        (lines, proven)
    }
}
