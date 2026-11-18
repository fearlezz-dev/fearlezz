//! The floor of a proof: what a change weakens about the proof its base branch requires, as
//! typed weakenings, and how one gets past where it blocks.

use std::fmt;

use serde::{Deserialize, Serialize};

/// What a change weakens about one part of the proof the base branch requires: every way it
/// weakened, and the part as it was and is now, once.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Weakening {
    /// What weakened.
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(flatten))]
    pub part: Part,
    /// Each way it weakened, such as `it runs another command`, in the order found.
    pub hows: Vec<String>,
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

    /// Every way it weakened, for a reader: `it runs another command; it is gone`.
    pub fn why(&self) -> String {
        self.hows.join("; ")
    }
}

/// ``check `tests` is weaker: it runs another command; it moves to a later stage than working``.
impl fmt::Display for Weakening {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.what(), self.why())
    }
}

/// One way a part of the config weakened, rendered into a [`Weakening`]'s `hows` by its
/// `Display`, the one place each shared shape (a value replaced or lowered, a setting changed,
/// a list's names) is worded. A way of a single part, said whole, is [`Change::Is`].
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Change {
    /// A way with no value to name, said whole: `it is gone`, `it runs another command`.
    Is(String),
    /// A value replaced by a weaker one: `it runs in another folder (src, now the root)`.
    Replaced {
        /// What happened, such as `it runs in another folder`.
        how: String,
        /// The value on the base branch, as a reader sees it.
        from: String,
        /// The value in the change.
        to: String,
    },
    /// A level lowered: `it goes from gate to flag`.
    Lowered {
        /// What lowers, such as `it goes` or `its level goes`.
        how: String,
        /// The level on the base branch.
        from: String,
        /// The level in the change.
        to: String,
    },
    /// A setting that came, which Fearlezz cannot tell the direction of: `it is added (true)`.
    Added(String),
    /// A setting that went: `it is removed (was true)`.
    Removed(String),
    /// A setting whose value changed: `it changes from 80 to 100`.
    Changed {
        /// The value on the base branch.
        from: String,
        /// The value in the change.
        to: String,
    },
    /// Names a list gained, lost or put in another order: `it gets other secrets (+TOKEN)`.
    Names {
        /// What the list does now, such as `it gets other secrets`.
        how: String,
        /// Each name added or dropped, or the new order; [`Change::names`] builds it only when
        /// there is one.
        names: Vec<Name>,
    },
    /// A `no_proof` glob that lets more files go without proof.
    Exempts(Name),
}

/// How a list differs from the base branch's, one name at a time.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Name {
    /// A name or glob the list adds: `+TOKEN`.
    Added(String),
    /// A name or glob the list drops: `-TOKEN`.
    Dropped(String),
    /// The same names in an order that means more.
    Reordered,
}

impl Change {
    /// A setting that came, went or changed from `from` to `to`, or nothing when it stayed.
    pub fn set(from: Option<String>, to: Option<String>) -> Option<Change> {
        match (from, to) {
            (Some(from), Some(to)) if from != to => Some(Change::Changed { from, to }),
            (None, Some(to)) => Some(Change::Added(to)),
            (Some(from), None) => Some(Change::Removed(from)),
            _ => None,
        }
    }

    /// A list that does `how` by `names`, or nothing when no name differs.
    pub fn names(how: impl Into<String>, names: Vec<Name>) -> Option<Change> {
        (!names.is_empty()).then(|| Change::Names {
            how: how.into(),
            names,
        })
    }

    /// A value replaced: `how (from, now to)`.
    pub fn replaced(
        how: impl Into<String>,
        from: impl fmt::Display,
        to: impl fmt::Display,
    ) -> Self {
        Change::Replaced {
            how: how.into(),
            from: from.to_string(),
            to: to.to_string(),
        }
    }
}

/// A way said whole.
impl From<String> for Change {
    fn from(how: String) -> Self {
        Change::Is(how)
    }
}

impl From<&str> for Change {
    fn from(how: &str) -> Self {
        Change::Is(how.to_string())
    }
}

/// The names `head` adds and `base` had that `head` drops, the added first, each in its list's
/// order.
pub fn names_diff(base: &[String], head: &[String]) -> Vec<Name> {
    let added = head
        .iter()
        .filter(|n| !base.contains(n))
        .cloned()
        .map(Name::Added);
    let dropped = base
        .iter()
        .filter(|n| !head.contains(n))
        .cloned()
        .map(Name::Dropped);
    added.chain(dropped).collect()
}

impl fmt::Display for Change {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Change::Is(how) => f.write_str(how),
            Change::Replaced { how, from, to } => write!(f, "{how} ({from}, now {to})"),
            Change::Lowered { how, from, to } => write!(f, "{how} from {from} to {to}"),
            Change::Added(to) => write!(f, "it is added ({to})"),
            Change::Removed(from) => write!(f, "it is removed (was {from})"),
            Change::Changed { from, to } => write!(f, "it changes from {from} to {to}"),
            Change::Names { how, names } => {
                let names: Vec<String> = names.iter().map(Name::to_string).collect();
                write!(f, "{how} ({})", names.join(", "))
            }
            Change::Exempts(Name::Added(glob)) => write!(f, "it newly says {glob} needs no proof"),
            Change::Exempts(Name::Dropped(glob)) => write!(
                f,
                "it no longer takes {} back from needing no proof",
                glob.trim_start_matches('!')
            ),
            Change::Exempts(Name::Reordered) => {
                f.write_str("it reorders its globs so more files need no proof")
            }
        }
    }
}

/// `+NAME`, `-NAME` or `reordered`.
impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Name::Added(name) => write!(f, "+{name}"),
            Name::Dropped(name) => write!(f, "-{name}"),
            Name::Reordered => f.write_str("reordered"),
        }
    }
}
