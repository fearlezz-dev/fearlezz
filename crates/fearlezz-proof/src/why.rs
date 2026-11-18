//! Why a check failed: known where its run fails, kept with the run, and said in the terminal
//! and as the proof's item.

use serde::{Deserialize, Serialize};

use crate::Item;

/// Why a run failed, the first that applies: what kept its command from running, then what
/// its output showed, then its exit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Why {
    /// SecretSpec could not give it this secret; its command never ran.
    Secret(String),
    /// Its preview could not be deployed; its command never ran.
    Preview(NoPreview),
    /// The shell could not find this tool.
    MissingTool(String),
    /// It reported a finding that blocks, such as this one.
    Gate(String),
    /// Its generated files do not come out as the tree has them.
    Generated(Generated),
    /// It ran no tests; what its readers said about it.
    NoTests(Vec<String>),
    /// Its command passed, but what the check reads (a coverage report) is missing or wrong.
    Unread(String),
    /// Its command failed with this exit code, or a signal.
    Exit(Option<i32>),
}

/// Why a checkout has no preview, so the checks that need it fail without running.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum NoPreview {
    /// The command failed.
    Failed {
        /// Its exit code, none when a signal stopped it.
        code: Option<i32>,
    },
    /// The command passed but printed no URL.
    NoUrl,
    /// SecretSpec could not give it a secret; its reason is in the log.
    Secret {
        /// The secret.
        name: String,
    },
    /// The shell found no `secretspec` to ask.
    NoSecretSpec,
    /// The checkout's `[env]` has no preview command, as on a base commit from before it.
    Unset,
}

/// Why a check's generated files fail it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Generated {
    /// Each file its command wrote otherwise than the tree had it, labeled for people
    /// (`gen/a.rs`, `gen/b.rs (new)`, `gen/c.rs (gone)`); the diff is in the run's folder.
    Stale(Vec<String>),
    /// Neither the tree nor the command has a file its globs (these) name, so it proves
    /// nothing.
    NamesNoFile(Vec<String>),
}

/// How to make a missing tool reachable, which only the CLI knows: where it looks for tools.
pub type Install = fn(&str) -> String;

impl Why {
    /// It never got to say anything about its inputs: SecretSpec could not give it a secret,
    /// its preview could not be deployed, or the shell could not find its command.
    pub fn proves_nothing(&self) -> bool {
        matches!(self, Why::Secret(_) | Why::Preview(_) | Why::MissingTool(_))
    }

    /// The line after `fail <check>` in the terminal, which lists findings and the end of the
    /// log below it: `: <why>`, or ` (exit <code>)`.
    pub fn line(&self) -> String {
        match self {
            Why::Gate(_) => ": blocking findings".into(),
            Why::Exit(code) => format!(" (exit {})", code_word(*code)),
            Why::NoTests(_) => ": it ran no tests, so it proves nothing".into(),
            Why::Secret(_)
            | Why::Preview(_)
            | Why::MissingTool(_)
            | Why::Generated(_)
            | Why::Unread(_) => format!(": {}", self.why()),
        }
    }

    /// What to do, in the terminal of the worktree the run is in; None when the findings or
    /// the log below say it.
    pub fn fix_here(&self, check: &str, install: Install) -> Option<String> {
        match self {
            Why::Gate(_) | Why::Exit(_) => None,
            Why::Generated(failed) => Some(failed.fix(check, true)),
            Why::Secret(_)
            | Why::Preview(_)
            | Why::MissingTool(_)
            | Why::NoTests(_)
            | Why::Unread(_) => Some(self.fix(check, install)),
        }
    }

    /// The proof's item for check `check`.
    pub fn item(&self, check: &str, install: Install) -> Item {
        Item {
            what: format!("check `{check}` failed"),
            why: self.why(),
            fix: self.fix(check, install),
        }
    }

    fn why(&self) -> String {
        match self {
            Why::Secret(name) => format!("SecretSpec could not give it {name}"),
            Why::Preview(why) => why.why(),
            Why::MissingTool(tool) => format!("{tool} was not found"),
            Why::Gate(found) => format!("blocking findings, such as {found}"),
            Why::Generated(failed) => failed.why(),
            Why::NoTests(said) => {
                let detail = said.first().map(|e| format!(": {e}")).unwrap_or_default();
                format!("it ran no tests, so it proves nothing{detail}")
            }
            Why::Unread(why) => why.clone(),
            Why::Exit(code) => format!(
                "its command ended with {}",
                code.map_or_else(|| "a signal".to_string(), |c| format!("exit code {c}"))
            ),
        }
    }

    fn fix(&self, check: &str, install: Install) -> String {
        match self {
            Why::Secret(name) => secret_fix(name),
            Why::Preview(why) => why.fix(install),
            Why::MissingTool(tool) => install(tool),
            Why::Gate(_) => "fix the findings; the check page lists them".into(),
            Why::Generated(failed) => failed.fix(check, false),
            Why::NoTests(_) => {
                "make it run its tests, or give it the tests reader its runner needs".into()
            }
            Why::Unread(_) => "make its command leave what the check reads".into(),
            Why::Exit(_) => {
                "fix what it reports; the check page shows the end of its output".into()
            }
        }
    }
}

impl NoPreview {
    fn why(&self) -> String {
        match self {
            NoPreview::Failed { code } => format!("the preview failed (exit {})", code_word(*code)),
            NoPreview::NoUrl => "the preview printed no URL".into(),
            NoPreview::Secret { name } => format!("SecretSpec could not give the preview {name}"),
            NoPreview::NoSecretSpec => "secretspec was not found for the preview's secrets".into(),
            NoPreview::Unset => "its checkout's [env] has no preview command".into(),
        }
    }

    fn fix(&self, install: Install) -> String {
        let by_hand = "run `[env] preview` by hand at the checkout root, with \
                       FEARLEZZ_PREVIEW_NAME set, and read its log (above)";
        match self {
            NoPreview::Failed { .. } => format!("make it deploy: {by_hand}"),
            NoPreview::NoUrl => format!(
                "make [env] preview print the URL of its deploy (the last http:// or https:// URL \
                 it prints is taken): {by_hand}"
            ),
            NoPreview::Secret { name } => secret_fix(name),
            NoPreview::NoSecretSpec => install("secretspec"),
            NoPreview::Unset => "add a preview command to [env] in fearlezz.toml".into(),
        }
    }
}

impl Generated {
    /// Why it fails, in a few words.
    fn why(&self) -> String {
        match self {
            Generated::Stale(files) => {
                format!("generated files are out of date: {}", files.join(", "))
            }
            Generated::NamesNoFile(globs) => {
                format!("generates names no file git tracks ({})", globs.join(", "))
            }
        }
    }

    /// What to do about it, for check `check`; `here` when the regenerated files are in the
    /// worktree the person reads this in.
    fn fix(&self, check: &str, here: bool) -> String {
        match (self, here) {
            (Generated::Stale(_), true) => {
                format!("{check} regenerated them; commit them as they are now")
            }
            (Generated::Stale(_), false) => {
                format!("run fearlezz run {check} and commit the files it regenerates")
            }
            (Generated::NamesNoFile(_), _) => {
                "name the files its command writes in generates".into()
            }
        }
    }
}

/// An exit code as the terminal says it.
fn code_word(code: Option<i32>) -> String {
    code.map_or_else(|| "signal".to_string(), |c| c.to_string())
}

/// How to let SecretSpec give a check the secret `name`.
fn secret_fix(name: &str) -> String {
    format!(
        "declare {name} in secretspec.toml and store it with its provider (1Password: \
         `secretspec set {name}`); sign in to the provider locally (`op signin`), and give CI \
         its token (1Password: the OP_SERVICE_ACCOUNT_TOKEN secret in the job's env)"
    )
}
