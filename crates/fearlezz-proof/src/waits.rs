//! What waits for a person: each obligation a proof leaves, who settles it and how, and what a
//! settling covers. Proofs, the status comment's hidden line and the proof host derive the same
//! list, so the wording and the rule for who may settle what live here once.

use std::fmt;

use serde::{Deserialize, Serialize};

/// The two things a person says by comment: approving changed pictures, which needs write
/// access, and accepting a weakened proof, which needs admin rights.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// `/fearlezz approve <words>`, for changed pictures or a fork's head.
    Approve,
    /// `/fearlezz accept <words>`, for a weakened proof.
    Accept,
}

/// What a person may do on a repository, as GitHub's collaborator permission says it.
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

impl Kind {
    /// Both kinds, in the order what waits is told.
    pub const ALL: [Kind; 2] = [Kind::Approve, Kind::Accept];

    /// The kind whose command word is `word`.
    pub fn of(word: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.word() == word)
    }

    /// The command's word: `/fearlezz <word> <words>`.
    pub fn word(self) -> &'static str {
        match self {
            Kind::Approve => "approve",
            Kind::Accept => "accept",
        }
    }

    /// What giving it is called: an approval, an accept.
    pub fn noun(self) -> &'static str {
        match self {
            Kind::Approve => "approval",
            Kind::Accept => "accept",
        }
    }

    /// Having given it: approved, accepted.
    pub fn past(self) -> &'static str {
        match self {
            Kind::Approve => "approved",
            Kind::Accept => "accepted",
        }
    }

    /// The role giving it needs.
    pub fn needs(self) -> Role {
        match self {
            Kind::Approve => Role::Write,
            Kind::Accept => Role::Admin,
        }
    }

    /// Whether a person with `role` may give it.
    pub fn allows(self, role: Role) -> bool {
        role >= self.needs()
    }

    /// What the role it needs is called, before a repository: `write access to`.
    pub fn needs_on(self) -> &'static str {
        match self {
            Kind::Approve => "write access to",
            Kind::Accept => "admin rights on",
        }
    }

    /// What giving it does, after "can": `approve pictures`.
    pub fn act(self) -> &'static str {
        match self {
            Kind::Approve => "approve pictures",
            Kind::Accept => "accept a weakened proof",
        }
    }

    /// What its words say, as a placeholder: `what you think of the pictures`.
    pub fn hint(self) -> &'static str {
        match self {
            Kind::Approve => "what you think of the pictures",
            Kind::Accept => "why the weaker proof is right",
        }
    }

    /// Who comments what, in a sentence: `` `/fearlezz approve <words>` ``.
    pub fn asks(self) -> &'static str {
        match self {
            Kind::Approve => "`/fearlezz approve <words>`",
            Kind::Accept => "an admin's `/fearlezz accept <words>`",
        }
    }

    /// What the proof showed for it to be given: `its pictures`.
    pub fn shown(self) -> &'static str {
        match self {
            Kind::Approve => "its pictures",
            Kind::Accept => "what it weakens",
        }
    }

    /// What a run does that needs it again: `shows other pictures`.
    pub fn renewed_by(self) -> &'static str {
        match self {
            Kind::Approve => "shows other pictures",
            Kind::Accept => "weakens something else",
        }
    }

    /// When nothing waits for it: `no changed pictures wait for approval`.
    pub fn nothing(self) -> &'static str {
        match self {
            Kind::Approve => "no changed pictures wait for approval",
            Kind::Accept => "no weakened proof waits for an accept",
        }
    }

    /// What it settles, as a status comment's headline names it when it alone waits.
    fn headline(self) -> &'static str {
        match self {
            Kind::Approve => "Pictures need approval",
            Kind::Accept => "Weakened proof needs an admin",
        }
    }
}

/// What a change weakens about the proof the base branch requires.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Weakening {
    /// What weakened.
    #[serde(flatten)]
    pub part: Part,
    /// How it weakened, such as `it runs another command`.
    pub how: String,
}

/// A part of the config that can weaken.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
}

/// How a weakening gets past where nothing can wait for an admin's accept.
pub const PAST: &str = "when it must change, ask a person with admin rights: on a pull request \
                        from this repository on GitHub they accept it with /fearlezz accept \
                        <words> after reading why, elsewhere they merge past CI";

impl Weakening {
    /// What weakened, for a reader: ``check `tests` is weaker``.
    pub fn what(&self) -> String {
        match &self.part {
            Part::Check(id) => format!("check `{id}` is weaker"),
            Part::Setting(path) => format!("`{path}` is weaker"),
        }
    }

    /// What to do where it waits for an admin's accept, below the proof `base` requires.
    pub fn accept_fix(base: &str) -> String {
        format!(
            "keep the proof {base} requires; when it must change, a person with admin rights \
             reads why and accepts it with `/fearlezz accept <words>`"
        )
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

/// One thing that waits for a person before a pull request's head counts as proven.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Obligation {
    /// The changed pictures of a check, for an approval.
    Pictures(String),
    /// A fork's head, whose run writes nothing: its approval covers the whole head and names
    /// it, so it cannot count for a later push.
    Head(String),
    /// A weakening, for an admin's accept. One accept covers every weakening of the head.
    Weakened(Weakening),
}

impl Obligation {
    /// Who settles it, and how.
    pub fn kind(&self) -> Kind {
        match self {
            Obligation::Pictures(_) | Obligation::Head(_) => Kind::Approve,
            Obligation::Weakened(_) => Kind::Accept,
        }
    }
}

/// Everything that waits for a person on one head, in the order it was found. Derived from a
/// proof, the status comment's hidden line, or a fork's head; never stored on its own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Obligations(Vec<Obligation>);

impl FromIterator<Obligation> for Obligations {
    fn from_iter<I: IntoIterator<Item = Obligation>>(iter: I) -> Obligations {
        Obligations(iter.into_iter().collect())
    }
}

impl Obligations {
    /// Whether nothing waits.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Every obligation.
    pub fn iter(&self) -> impl Iterator<Item = &Obligation> {
        self.0.iter()
    }

    /// The obligations `kind` settles.
    pub fn of(&self, kind: Kind) -> impl Iterator<Item = &Obligation> {
        self.0.iter().filter(move |o| o.kind() == kind)
    }

    /// The kinds that settle something here, approve first.
    pub fn kinds(&self) -> Vec<Kind> {
        Kind::ALL
            .into_iter()
            .filter(|k| self.of(*k).next().is_some())
            .collect()
    }

    /// The checks whose changed pictures wait.
    pub fn checks(&self) -> Vec<&str> {
        self.0
            .iter()
            .filter_map(|o| match o {
                Obligation::Pictures(check) => Some(check.as_str()),
                Obligation::Head(_) | Obligation::Weakened(_) => None,
            })
            .collect()
    }

    /// The weakenings that wait for an accept.
    pub fn weakenings(&self) -> Vec<&Weakening> {
        self.0
            .iter()
            .filter_map(|o| match o {
                Obligation::Weakened(w) => Some(w),
                Obligation::Pictures(_) | Obligation::Head(_) => None,
            })
            .collect()
    }

    /// The fork's head that waits, when this is a fork's.
    pub fn fork_head(&self) -> Option<&str> {
        self.0.iter().find_map(|o| match o {
            Obligation::Head(head) => Some(head.as_str()),
            Obligation::Pictures(_) | Obligation::Weakened(_) => None,
        })
    }

    /// What `kind` covers here, as a reply names it: ``the changed pictures of `ui` ``, this
    /// fork's head, or the weakened proof.
    pub fn covers(&self, kind: Kind) -> String {
        match kind {
            Kind::Approve if self.fork_head().is_some() => "this fork's head".into(),
            Kind::Approve => {
                let checks: Vec<String> = self.checks().iter().map(|c| format!("`{c}`")).collect();
                format!("the changed pictures of {}", checks.join(", "))
            }
            Kind::Accept => "the weakened proof".into(),
        }
    }

    /// What `kind` still waits for, as a status description names it.
    fn waiting(&self, kind: Kind) -> String {
        match kind {
            Kind::Approve => match self.fork_head() {
                Some(head) => format!(
                    "this fork's head waits for /fearlezz approve {} <words>",
                    short(head)
                ),
                None => format!(
                    "changed pictures of {} wait for /fearlezz approve",
                    self.checks().join(", ")
                ),
            },
            Kind::Accept => {
                let mut names: Vec<&str> =
                    self.weakenings().iter().map(|w| w.part.name()).collect();
                names.dedup();
                format!(
                    "weakened {} waits for an admin's /fearlezz accept",
                    names.join(", ")
                )
            }
        }
    }

    /// What still waits, as a status description names it, of the kinds `settled` does not
    /// call settled.
    pub fn still(&self, settled: impl Fn(Kind) -> bool) -> Vec<String> {
        self.kinds()
            .into_iter()
            .filter(|k| !settled(*k))
            .map(|k| self.waiting(k))
            .collect()
    }

    /// The headline's word for what waits, and what each kind needs, or None when nothing
    /// waits: `Pictures need approval`, ``the changed pictures of `ui` need a person's
    /// approval``.
    pub fn headline(&self) -> Option<(&'static str, Vec<String>)> {
        let kinds = self.kinds();
        let word = match kinds.as_slice() {
            [] => return None,
            [one] => one.headline(),
            _ => "Needs approval",
        };
        let needs = kinds
            .iter()
            .map(|k| match k {
                Kind::Approve => format!("{} need a person's approval", self.covers(*k)),
                Kind::Accept => format!("{} needs an admin's accept", self.covers(*k)),
            })
            .collect();
        Some((word, needs))
    }
}

/// The first seven characters of a commit, as people read it.
pub fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}
