//! Why a check failed: read once from its run, then said in the terminal and in the proof.

use fearlezz_proof as proof;

use crate::env::NoPreview;
use crate::store::RunResult;
use crate::{findings, generated, process, run};

/// Why a run failed, the first that applies: what kept its command from running, then what
/// its output showed, then its exit.
pub(crate) enum Why {
    /// SecretSpec could not give it this secret; its command never ran.
    Secret(String),
    /// Its preview could not be deployed; its command never ran.
    Preview(NoPreview),
    /// The shell could not find this tool.
    MissingTool(String),
    /// It reported a finding that blocks, such as this one.
    Gate(String),
    /// Its generated files do not come out as the tree has them.
    Generated(generated::Failed),
    /// It ran no tests; what its readers said about it.
    NoTests(Vec<String>),
    /// Its command passed, but what the check reads (a coverage report) is missing or wrong.
    Unread(String),
    /// Its command failed with this exit code, or a signal.
    Exit(Option<i32>),
}

impl Why {
    /// Why the failed run `r` failed.
    pub(crate) fn of(r: &RunResult) -> Why {
        if let Some(name) = &r.missing_secret {
            return Why::Secret(name.clone());
        }
        if let Some(why) = &r.no_preview {
            return Why::Preview(why.clone());
        }
        if let Some(tool) = r
            .not_found()
            .then(|| run::missing_command(&r.log))
            .flatten()
        {
            return Why::MissingTool(tool);
        }
        if let Some(found) = r.findings.iter().find(|x| x.level == findings::Level::Gate) {
            return Why::Gate(found.to_string());
        }
        if let Some(failed) = &r.generated {
            return Why::Generated(failed.clone());
        }
        if r.tests == Some(0) && r.exit_code == Some(0) {
            return Why::NoTests(r.proof_errors.clone());
        }
        if let (Some(0), Some(why)) = (r.exit_code, r.proof_errors.first()) {
            return Why::Unread(why.clone());
        }
        Why::Exit(r.exit_code)
    }

    /// The line after `fail <check>` in the terminal, which lists findings and the end of the
    /// log below it: `: <why>`, or ` (exit <code>)`.
    pub(crate) fn line(&self) -> String {
        match self {
            Why::Gate(_) => ": blocking findings".into(),
            Why::Exit(code) => format!(" (exit {})", code_word(*code)),
            Why::NoTests(_) => ": it ran no tests, so it proves nothing".into(),
            _ => format!(": {}", self.why()),
        }
    }

    /// What to do, in the terminal of the worktree the run is in; None when the findings or
    /// the log below say it.
    pub(crate) fn fix_here(&self, check: &str) -> Option<String> {
        match self {
            Why::Gate(_) | Why::Exit(_) => None,
            Why::Generated(failed) => Some(failed.fix(check, true)),
            _ => Some(self.fix(check)),
        }
    }

    /// The proof's item for check `check`.
    pub(crate) fn item(&self, check: &str) -> proof::Item {
        proof::Item {
            what: format!("check `{check}` failed"),
            why: self.why(),
            fix: self.fix(check),
        }
    }

    fn why(&self) -> String {
        match self {
            Why::Secret(name) => format!("SecretSpec could not give it {name}"),
            Why::Preview(why) => preview(why).0,
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

    fn fix(&self, check: &str) -> String {
        match self {
            Why::Secret(name) => secret_fix(name),
            Why::Preview(why) => preview(why).1,
            Why::MissingTool(tool) => install_fix(tool),
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

/// An exit code as the terminal says it.
fn code_word(code: Option<i32>) -> String {
    code.map_or_else(|| "signal".to_string(), |c| c.to_string())
}

/// How to make a missing tool reachable for checks.
fn install_fix(tool: &str) -> String {
    let nix = if run::NIX_TOOLS.contains(&tool) {
        format!(" (in devenv.nix: add `pkgs.{tool}` to packages)")
    } else {
        String::new()
    };
    format!(
        "install {tool} where checks find it: the project's dev dependencies ({}), its devenv{nix}, \
         or PATH",
        process::TOOL_DIRS.join(", ")
    )
}

/// How to let SecretSpec give a check the secret `name`.
fn secret_fix(name: &str) -> String {
    format!(
        "declare {name} in secretspec.toml and store it with its provider (1Password: \
         `secretspec set {name}`); sign in to the provider locally (`op signin`), and give CI \
         its token (1Password: the OP_SERVICE_ACCOUNT_TOKEN secret in the job's env)"
    )
}

/// Why a check's preview could not be deployed, and what to do about it.
fn preview(why: &NoPreview) -> (String, String) {
    let by_hand = "run `[env] preview` by hand at the checkout root, with FEARLEZZ_PREVIEW_NAME set, \
                   and read its log (above)";
    match why {
        NoPreview::Failed { code } => (
            format!("the preview failed (exit {})", code_word(*code)),
            format!("make it deploy: {by_hand}"),
        ),
        NoPreview::NoUrl => (
            "the preview printed no URL".to_string(),
            format!(
                "make [env] preview print the URL of its deploy (the last http:// or https:// URL \
                 it prints is taken): {by_hand}"
            ),
        ),
        NoPreview::Secret { name } => (
            format!("SecretSpec could not give the preview {name}"),
            secret_fix(name),
        ),
        NoPreview::NoSecretSpec => (
            "secretspec was not found for the preview's secrets".to_string(),
            install_fix("secretspec"),
        ),
        NoPreview::Unset => (
            "its checkout's [env] has no preview command".to_string(),
            "add a preview command to [env] in fearlezz.toml".to_string(),
        ),
    }
}
