//! Agent hooks: how Fearlezz tells an agent, while it works, that it skipped the proof. Each
//! agent runs `fearlezz setup hook <agent> <event>` and reads the answer in its own format.
//! The hooks are guardrails, never the boundary: CI is. So a hook that cannot judge says
//! nothing, and lets the agent go on.

use std::path::PathBuf;

use clap::ValueEnum;
use serde_json::{Value, json};

use crate::config::Config;

/// An agent whose hooks Fearlezz writes and answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum Agent {
    Claude,
    Codex,
    Cursor,
}

pub(crate) const AGENTS: [(&str, Agent); 3] = [
    ("claude", Agent::Claude),
    ("codex", Agent::Codex),
    ("cursor", Agent::Cursor),
];

/// The moments Fearlezz hooks into.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum Event {
    SessionStart,
    BeforeShell,
    AfterShell,
    Stop,
}

impl Agent {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
            Agent::Cursor => "cursor",
        }
    }

    /// The hooks this agent reads, as its settings file has them. Every command ends in
    /// `|| true`, so a Fearlezz without this subcommand never blocks the agent.
    pub(crate) fn settings(self) -> Value {
        let command = |event: &str| format!("fearlezz setup hook {} {event} || true", self.name());
        match self {
            Agent::Claude | Agent::Codex => {
                let handler =
                    |event: &str| json!([{ "type": "command", "command": command(event) }]);
                json!({ "hooks": {
                    "SessionStart": [{ "hooks": handler("session-start") }],
                    "PreToolUse": [{ "matcher": "Bash", "hooks": handler("before-shell") }],
                    "PostToolUse": [{ "matcher": "Bash", "hooks": handler("after-shell") }],
                    "Stop": [{ "hooks": handler("stop") }],
                }})
            }
            // Cursor's afterShellExecution has no answer that reaches the agent.
            Agent::Cursor => json!({
                "version": 1,
                "hooks": {
                    "sessionStart": [{ "command": command("session-start") }],
                    "beforeShellExecution": [{ "command": command("before-shell") }],
                    "stop": [{ "command": command("stop"), "loop_limit": 1 }],
                },
            }),
        }
    }
}

/// What the agent's hook input says, in the terms Fearlezz judges.
pub(crate) struct Input {
    /// The project folder, when the input names one.
    pub dir: Option<PathBuf>,
    pub command: Option<String>,
    /// The agent is already going on because a stop hook asked it to.
    pub again: bool,
}

impl Input {
    pub(crate) fn parse(agent: Agent, text: &str) -> Input {
        let v: Value = serde_json::from_str(text).unwrap_or(Value::Null);
        let text = |at: &str| v.pointer(at).and_then(Value::as_str).map(String::from);
        match agent {
            Agent::Claude | Agent::Codex => Input {
                dir: text("/cwd").map(PathBuf::from),
                command: text("/tool_input/command"),
                again: v
                    .pointer("/stop_hook_active")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            },
            Agent::Cursor => Input {
                dir: text("/workspace_roots/0")
                    .or_else(|| std::env::var("CURSOR_PROJECT_DIR").ok())
                    .map(PathBuf::from),
                command: text("/command"),
                again: v
                    .pointer("/loop_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    > 0,
            },
        }
    }
}

/// What Fearlezz knows about the work, for judging a hook.
pub(crate) struct Work {
    pub name: String,
    pub declared: Declared,
    pub ready: bool,
    pub waiting: bool,
    /// The change touches files a check reads, outside `no_proof`.
    pub needs_proof: bool,
    pub now: String,
    pub next: String,
}

/// How much the work has said about itself.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Declared {
    Nothing,
    Goal,
    GoalAndClaims,
}

pub(crate) enum Answer {
    Nothing,
    /// Something the agent should know.
    Context(String),
    /// The command must not run.
    Deny(String),
    /// The agent must not stop yet.
    KeepGoing(String),
}

const SAY_START: &str = "fearlezz say start --kind bug|feature|refactor|other --goal \"<the goal \
                         in the developer's words>\"";

/// What Fearlezz tells the agent at `event`.
pub(crate) fn judge(config: &Config, work: &Work, event: Event, input: &Input) -> Answer {
    let command = input.command.as_deref().unwrap_or("");
    match event {
        Event::SessionStart => Answer::Context(format!(
            "This repository proves work with Fearlezz (see the fearlezz skill); only fearlezz run \
             counts as proof. Journal {}: {}. next: {}",
            work.name, work.now, work.next
        )),
        Event::BeforeShell
            if work.needs_proof && pushes(command) && work.declared == Declared::Nothing =>
        {
            Answer::Deny(format!(
                "Fearlezz: a push needs the goal and claims of this work first, so CI can prove \
                 them. Run {SAY_START}, then fearlezz say claim, and commit the work file."
            ))
        }
        Event::BeforeShell
            if work.needs_proof && pushes(command) && work.declared == Declared::Goal =>
        {
            Answer::Deny(
                "Fearlezz: a push needs the claims of this work first, so CI can prove them. Run \
                 fearlezz say claim \"<behavior>\" --check <check>, and commit the work file."
                    .into(),
            )
        }
        Event::BeforeShell if work.needs_proof && opens_pull_request(command) && !work.ready => {
            Answer::Deny(format!(
                "Fearlezz: a pull request that is not a draft needs the work ready to merge. Open \
                 it with --draft, or first: {}",
                work.next
            ))
        }
        Event::BeforeShell => Answer::Nothing,
        Event::AfterShell => match check_run_by_hand(config, command) {
            Some(id) => Answer::Context(format!(
                "Fearlezz: that ran the {id} check's command by hand. It is quick feedback, not \
                 proof; fearlezz run {id} records it in the journal."
            )),
            None => Answer::Nothing,
        },
        Event::Stop if input.again || work.waiting || work.ready || !work.needs_proof => {
            Answer::Nothing
        }
        Event::Stop => Answer::KeepGoing(format!(
            "Fearlezz: this work is not proven yet. next: {}. If only the developer can decide, \
             run fearlezz say ask \"<question>\" and stop.",
            work.next
        )),
    }
}

/// The answer as `agent` reads it on stdout; `None` prints nothing.
pub(crate) fn render(agent: Agent, event: Event, answer: Answer) -> Option<Value> {
    match agent {
        Agent::Claude | Agent::Codex => match answer {
            Answer::Nothing => None,
            Answer::Context(text) => {
                let name = match event {
                    Event::SessionStart => "SessionStart",
                    Event::BeforeShell => "PreToolUse",
                    Event::AfterShell => "PostToolUse",
                    Event::Stop => "Stop",
                };
                Some(json!({ "hookSpecificOutput": {
                    "hookEventName": name,
                    "additionalContext": text,
                }}))
            }
            Answer::Deny(text) => Some(json!({ "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": text,
            }})),
            Answer::KeepGoing(text) => Some(json!({ "decision": "block", "reason": text })),
        },
        // Cursor blocks a shell command whose hook prints no valid answer, so it always gets
        // one.
        Agent::Cursor => Some(match answer {
            Answer::Deny(text) => json!({
                "permission": "deny",
                "user_message": text,
                "agent_message": text,
            }),
            Answer::KeepGoing(text) => json!({ "followup_message": text }),
            Answer::Context(text) if event == Event::SessionStart => {
                json!({ "additional_context": text })
            }
            Answer::Nothing | Answer::Context(_) if event == Event::BeforeShell => {
                json!({ "permission": "allow" })
            }
            Answer::Nothing | Answer::Context(_) => json!({}),
        }),
    }
}

/// The simple commands of a shell line, each as its words without leading `NAME=value`.
fn commands(line: &str) -> Vec<Vec<&str>> {
    line.split(['\n', ';', '|', '&'])
        .map(|part| {
            part.split_whitespace()
                .skip_while(|w| w.contains('=') && !w.starts_with('-'))
                .collect::<Vec<_>>()
        })
        .filter(|words| !words.is_empty())
        .collect()
}

/// The words of a `git` command after its own options, such as `-C <dir>`.
fn git_args<'a>(words: &[&'a str]) -> Vec<&'a str> {
    let mut rest = Vec::new();
    let mut skip_value = false;
    for word in words.iter().skip(1) {
        if skip_value {
            skip_value = false;
        } else if rest.is_empty() && word.starts_with('-') {
            skip_value = matches!(*word, "-C" | "-c" | "--git-dir" | "--work-tree");
        } else {
            rest.push(*word);
        }
    }
    rest
}

fn pushes(line: &str) -> bool {
    commands(line)
        .iter()
        .any(|w| w.first() == Some(&"git") && git_args(w).first() == Some(&"push"))
}

/// `gh pr create` without `--draft`, or `gh pr ready` without `--undo`.
fn opens_pull_request(line: &str) -> bool {
    commands(line).iter().any(|w| match w.as_slice() {
        ["gh", "pr", "create", rest @ ..] => !rest.iter().any(|a| matches!(*a, "--draft" | "-d")),
        ["gh", "pr", "ready", rest @ ..] => !rest.contains(&"--undo"),
        _ => false,
    })
}

/// The check whose command, by its first two words, the line ran outside Fearlezz.
fn check_run_by_hand(config: &Config, line: &str) -> Option<String> {
    let commands = commands(line);
    if commands.iter().any(|w| w.first() == Some(&"fearlezz")) {
        return None;
    }
    config
        .checks
        .iter()
        .filter(|c| c.builtin.is_none())
        .find(|c| {
            let start: Vec<&str> = c.run.split_whitespace().take(2).collect();
            start.len() == 2 && commands.iter().any(|w| w.starts_with(&start))
        })
        .map(|c| c.id.clone())
}
