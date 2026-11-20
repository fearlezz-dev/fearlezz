//! `[commits]`: the built in `commits` check, which holds a change to Conventional Commits
//! (`type(scope)!: words`). The change's title is judged when it has one (the pull request's,
//! in CI or from `gh` locally; see [`crate::title`]), since with a squash merge it is the
//! commit; without one every commit since the base commit is. Merge commits,
//! the `Revert "..."` commits git writes and the `fixup!` and `squash!` commits an autosquash
//! folds away are left alone.

use std::path::Path;

use anyhow::Result;
use serde::Deserialize;

use crate::findings::{Finding, Level};

/// The id of the check `[commits]` forms.
pub(crate) const CHECK: &str = "commits";

/// The types of the Conventional Commits specification and of the Angular convention it
/// grew from, which release tools (release-please, convco) know.
const TYPES: [&str; 11] = [
    "feat", "fix", "docs", "style", "refactor", "perf", "test", "build", "ci", "chore", "revert",
];

/// The subjects `git commit --fixup` and `--squash` write, which `rebase --autosquash` folds
/// into the commit they name.
const AUTOSQUASH: [&str; 3] = ["fixup! ", "squash! ", "amend! "];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawCommits {
    types: Option<Vec<String>>,
}

#[derive(Clone, Debug)]
pub(crate) struct Commits {
    types: Vec<String>,
    /// The base branch, whose merge base starts the commits judged locally.
    base: String,
}

/// One message judged: the pull request title, or a commit by its short id.
struct Message {
    commit: Option<String>,
    subject: String,
}

impl Commits {
    pub(crate) fn parse(raw: RawCommits, base: &str) -> Commits {
        Commits {
            types: raw
                .types
                .unwrap_or_else(|| TYPES.iter().map(|t| (*t).to_string()).collect()),
            base: base.to_string(),
        }
    }

    /// A stable text of what it checks, part of its inputs.
    pub(crate) fn fingerprint(&self) -> String {
        format!("types {:?}", self.types)
    }

    /// The messages judged, as one text: they are inputs too, since rewording a commit keeps
    /// its tree.
    pub(crate) fn messages_text(&self, root: &Path) -> Result<String> {
        use std::fmt::Write as _;
        let mut text = String::new();
        for m in self.messages(root)? {
            let what = m.commit.as_deref().unwrap_or("title");
            writeln!(text, "{what}\n{}", m.subject)?;
        }
        Ok(text)
    }

    fn messages(&self, root: &Path) -> Result<Vec<Message>> {
        let git = crate::git::Git::discover(root)?;
        if let Some(crate::title::Title(title)) = crate::title::of(&git) {
            return Ok(vec![Message {
                commit: None,
                subject: title,
            }]);
        }
        let base = git.base_commit(&self.base)?;
        Ok(git
            .commits_since(&base)?
            .into_iter()
            .filter(|(_, subject)| !AUTOSQUASH.iter().any(|p| subject.starts_with(p)))
            .map(|(id, subject)| Message {
                commit: Some(id),
                subject,
            })
            .collect())
    }

    /// A gate finding for each message that is no conventional commit.
    pub(crate) fn check(&self, root: &Path) -> Result<Vec<Finding>> {
        let types: Vec<&str> = self.types.iter().map(String::as_str).collect();
        Ok(self
            .messages(root)?
            .into_iter()
            .filter(|m| !conventional(&m.subject, &types))
            .map(|m| {
                let (message, fix) = match &m.commit {
                    Some(id) => (
                        format!(
                            "commit \"{}\" ({id}) is not a conventional commit",
                            m.subject
                        ),
                        "reword the commit",
                    ),
                    None => (
                        format!(
                            "pull request title \"{}\" is not a conventional commit",
                            m.subject
                        ),
                        "rename the pull request",
                    ),
                };
                Finding {
                    fix: Some(format!(
                        "{fix} as `<type>(<scope>)!: <what it does>`, such as `feat: ...` or \
                         `fix: ...`; types are {}",
                        types.join(", ")
                    )),
                    why: Some(
                        "release notes and versions are made from the commit messages".into(),
                    ),
                    anywhere: true,
                    ..Finding::new("", 0, Level::Gate, CHECK, message)
                }
            })
            .collect())
    }
}

/// Whether `subject` is `type(scope)!: words` with a known type, or a revert git wrote.
fn conventional(subject: &str, types: &[&str]) -> bool {
    if subject.starts_with("Revert \"") {
        return true;
    }
    let Some((head, words)) = subject.split_once(": ") else {
        return false;
    };
    let head = head.strip_suffix('!').unwrap_or(head);
    let kind = match head.split_once('(') {
        Some((kind, scope)) => {
            let Some(scope) = scope.strip_suffix(')') else {
                return false;
            };
            if scope.is_empty() || scope.contains(['(', ')']) {
                return false;
            }
            kind
        }
        None => head,
    };
    types.contains(&kind) && !words.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::{TYPES, conventional};

    #[test]
    fn a_conventional_subject_has_a_known_type_an_optional_scope_and_words() {
        for good in [
            "feat: a",
            "fix(parser): b",
            "feat!: c",
            "refactor(core)!: d",
            "Revert \"feat: a\"",
        ] {
            assert!(conventional(good, &TYPES), "{good}");
        }
        for bad in [
            "Update a",
            "feature: a",
            "feat:a",
            "feat: ",
            "feat(): a",
            "feat(a: b",
            "Feat: a",
        ] {
            assert!(!conventional(bad, &TYPES), "{bad}");
        }
    }
}
