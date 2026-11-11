//! A journal: the append-only record of one piece of work, one per branch. Everything shown
//! about the work (stage, what is stale, what is next) is derived from it.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::store::Outcome;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Entry {
    pub at: Timestamp,
    #[serde(flatten)]
    pub event: Event,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum Event {
    Ran {
        check: String,
        key: String,
        result: String,
        outcome: Outcome,
        tree: String,
        reused: bool,
    },
    Stage {
        to: String,
        reason: Option<String>,
    },
    Wait {
        question: String,
    },
    /// The developer agreed with the changed pictures of a check, for these inputs.
    Approve {
        check: String,
        key: String,
        said: String,
    },
    Note {
        text: String,
    },
    /// An event of a feature since removed (the kind, goal and claims of the work), kept
    /// readable so older journals still load.
    #[serde(other)]
    Retired,
}

pub(crate) struct Journal {
    pub name: String,
    /// Where events are appended; `None` keeps them in memory only.
    file: Option<PathBuf>,
    pub entries: Vec<Entry>,
}

fn journals_dir(common: &Path) -> PathBuf {
    common.join("fearlezz").join("journals")
}

/// Branch names hold `/`; folder names cannot.
fn folder(name: &str) -> String {
    name.replace('%', "%25").replace('/', "%2F")
}

fn unfolder(folder: &str) -> String {
    folder.replace("%2F", "/").replace("%25", "%")
}

impl Journal {
    /// Reads a journal. A line this version cannot read (written by a newer Fearlezz in
    /// another worktree, or cut short by a crash) is skipped with a warning, so one bad line
    /// never hides every other journal.
    pub(crate) fn open(common: &Path, name: &str) -> Result<Journal> {
        let file = journals_dir(common).join(folder(name)).join("events.jsonl");
        let text = match fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", file.display())),
        };
        let mut entries = Vec::new();
        let mut skipped = 0;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            match serde_json::from_str(line) {
                Ok(entry) => entries.push(entry),
                Err(_) => skipped += 1,
            }
        }
        if skipped > 0 {
            eprintln!(
                "warning: skipped {skipped} unreadable entries in journal {name} ({}); a newer \
                 Fearlezz may have written them",
                file.display()
            );
        }
        Ok(Journal {
            name: name.to_string(),
            file: Some(file),
            entries,
        })
    }

    /// A journal that lives only as long as this command: for proving a commit, which says
    /// nothing about any piece of work.
    pub(crate) fn in_memory(name: &str) -> Journal {
        Journal {
            name: name.to_string(),
            file: None,
            entries: Vec::new(),
        }
    }

    pub(crate) fn is_in_memory(&self) -> bool {
        self.file.is_none()
    }

    pub(crate) fn all(common: &Path) -> Result<Vec<Journal>> {
        let Ok(dirs) = fs::read_dir(journals_dir(common)) else {
            return Ok(Vec::new());
        };
        let mut journals = Vec::new();
        for dir in dirs.filter_map(std::result::Result::ok) {
            let name = unfolder(&dir.file_name().to_string_lossy());
            let journal = Journal::open(common, &name)?;
            if !journal.entries.is_empty() {
                journals.push(journal);
            }
        }
        Ok(journals)
    }

    pub(crate) fn append(&mut self, at: Timestamp, event: Event) -> Result<()> {
        let entry = Entry { at, event };
        if let Some(file) = &self.file {
            if let Some(dir) = file.parent() {
                fs::create_dir_all(dir)?;
            }
            let mut line = serde_json::to_string(&entry)?;
            line.push('\n');
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(file)?
                .write_all(line.as_bytes())?;
        }
        self.entries.push(entry);
        Ok(())
    }
}
