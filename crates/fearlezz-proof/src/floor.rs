//! The floor of a proof: what a change weakens about the proof its base branch requires, as
//! typed weakenings, and how one gets past where it blocks.

use std::fmt;

use serde::{Deserialize, Serialize};

/// What a change weakens about the proof the base branch requires.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Weakening {
    /// What weakened.
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(flatten))]
    pub part: Part,
    /// How it weakened, such as `it runs another command`.
    pub how: String,
    /// The part as the base branch's config has it, as TOML (`[check.tests]` with its fields,
    /// or `no_proof = [...]`), absent where it has no such part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub was: Option<String>,
    /// The part as the change's config has it, as TOML, absent where it is gone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub now: Option<String>,
}

/// A part of the config that can weaken.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Part {
    /// A check, by id.
    Check(String),
    /// Any other part, by its path in the config: `no_proof`, `lints.ruff.E501`,
    /// `rule.no-secrets`, `settings.ruff.fix`, `env.prepare`.
    Setting(String),
}

impl Part {
    /// Its id or path.
    pub fn name(&self) -> &str {
        match self {
            Part::Check(name) | Part::Setting(name) => name,
        }
    }

    /// Its path in the config: `check.<id>` for a check, the setting's own path otherwise.
    pub fn path(&self) -> String {
        match self {
            Part::Check(id) => format!("check.{id}"),
            Part::Setting(path) => path.clone(),
        }
    }
}

/// How a weakening gets past where nothing can wait for an admin's accept.
pub const PAST: &str = "when it must change, a person with admin rights reads why and merges \
                        past the required check";

impl Weakening {
    /// What weakened, for a reader: ``check `tests` is weaker``.
    pub fn what(&self) -> String {
        match &self.part {
            Part::Check(id) => format!("check `{id}` is weaker"),
            Part::Setting(path) => format!("`{path}` is weaker"),
        }
    }

    /// What to do where it blocks, below the proof `base` requires.
    pub fn block_fix(base: &str) -> String {
        format!("keep the proof {base} requires; {PAST}")
    }
}

/// ``check `tests` is weaker: it runs another command``.
impl fmt::Display for Weakening {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.what(), self.how)
    }
}
