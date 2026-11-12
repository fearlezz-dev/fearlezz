//! Agent hooks: `fearlezz setup sync` writes them into each agent's own settings, and
//! `fearlezz setup hook <agent> <event>` answers them, so an agent finds out it skipped the
//! proof while it works: at the start of a session, after running a check's command by hand,
//! before opening or merging a pull request, and when it wants to stop.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;

/// `tests` passes; `unit` starts with a command an agent would run by hand, and passes here
/// without a Cargo project.
const CONFIG: &str = r#"
[check.tests]
run = "true"
stage = "working"

[check.unit]
run = "cargo test --quiet || true"
stage = "working"

[settings.skill]
agents = []
"#;

fn repo(config: &str) -> Repo {
    Repo::with_skill(config, &[("src/lib.rs", "fn a() {}\n")])
}

/// A branch whose code changed, with nothing proven yet.
fn changed() -> Repo {
    let repo = repo(&format!(
        "{CONFIG}\n[settings.hooks]\nagents = [\"claude\"]\n"
    ));
    repo.branch("feat/x");
    repo.fz(&["setup", "sync"]);
    repo.write("src/lib.rs", "fn b() {}\n");
    repo.commit("change");
    repo
}

fn shell(command: &str) -> String {
    serde_json::json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": { "command": command },
    })
    .to_string()
}

fn hook(repo: &Repo, agent: &str, event: &str, input: &str) -> serde_json::Value {
    let out = repo.fz_input(&["setup", "hook", agent, event], input);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    if out.stdout.trim().is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_str(&out.stdout).unwrap_or_else(|e| panic!("{e}: {}", out.stdout))
    }
}

fn denied(answer: &serde_json::Value) -> bool {
    answer["hookSpecificOutput"]["permissionDecision"] == "deny"
}

fn reason(answer: &serde_json::Value) -> String {
    answer["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

// Writing the hooks

#[test]
fn sync_writes_claude_code_hooks_by_default() {
    let repo = repo(CONFIG);
    let out = repo.fz(&["setup", "sync"]);
    assert!(out.ok, "{}", out.stderr);
    let text = std::fs::read_to_string(repo.path(".claude/settings.json")).unwrap();
    for part in [
        "\"SessionStart\"",
        "\"PreToolUse\"",
        "\"PostToolUse\"",
        "\"Stop\"",
        "fearlezz setup hook claude before-shell || true",
    ] {
        assert!(text.contains(part), "{part} in {text}");
    }
    assert!(!repo.path(".codex/hooks.json").exists());
    assert!(!repo.path(".cursor/hooks.json").exists());
}

#[test]
fn sync_writes_hooks_for_the_agents_the_project_uses() {
    let repo = repo(CONFIG);
    std::fs::create_dir_all(repo.path(".codex")).unwrap();
    std::fs::create_dir_all(repo.path(".cursor")).unwrap();
    repo.fz(&["setup", "sync"]);
    let codex = std::fs::read_to_string(repo.path(".codex/hooks.json")).unwrap();
    assert!(
        codex.contains("fearlezz setup hook codex stop || true"),
        "{codex}"
    );
    assert!(codex.contains("\"matcher\": \"Bash\""), "{codex}");
    let cursor = std::fs::read_to_string(repo.path(".cursor/hooks.json")).unwrap();
    assert!(cursor.contains("\"version\": 1"), "{cursor}");
    assert!(cursor.contains("\"beforeShellExecution\""), "{cursor}");
    assert!(
        cursor.contains("fearlezz setup hook cursor stop || true"),
        "{cursor}"
    );
}

#[test]
fn the_project_names_which_agents_get_hooks() {
    let repo = repo(&format!(
        "{CONFIG}\n[settings.hooks]\nagents = [\"cursor\"]\n"
    ));
    repo.fz(&["setup", "sync"]);
    assert!(repo.path(".cursor/hooks.json").exists());
    let claude = std::fs::read_to_string(repo.path(".claude/settings.json")).unwrap_or_default();
    assert!(!claude.contains("setup hook"), "{claude}");
}

#[test]
fn an_unknown_agent_is_an_error_naming_the_agents() {
    let repo = repo(&format!(
        "{CONFIG}\n[settings.hooks]\nagents = [\"cluade\"]\n"
    ));
    let out = repo.fz(&["setup", "sync"]);
    assert!(!out.ok);
    assert!(
        out.stderr.contains("claude, codex, cursor"),
        "{}",
        out.stderr
    );
}

#[test]
fn removed_hooks_are_drift_the_settings_check_finds() {
    let repo = repo(&format!(
        "{CONFIG}\n[settings.hooks]\nagents = [\"claude\"]\n"
    ));
    repo.fz(&["setup", "sync"]);
    repo.commit("hooks");
    repo.write(".claude/settings.json", "{}\n");
    let out = repo.fz(&["run", "settings"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains(".claude/settings.json"),
        "{}",
        out.stdout
    );
}

// At the start of a session

#[test]
fn a_session_starts_with_where_the_work_stands() {
    let repo = changed();
    let answer = hook(&repo, "claude", "session-start", "{}");
    let context = answer["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert_eq!(
        answer["hookSpecificOutput"]["hookEventName"],
        "SessionStart"
    );
    assert!(context.contains("next:"), "{context}");
    assert!(context.contains("fearlezz run"), "{context}");
}

// Before a pull request is opened or merged

#[test]
fn a_push_passes_since_ci_proves_it() {
    let repo = changed();
    let answer = hook(
        &repo,
        "claude",
        "before-shell",
        &shell("git push -u origin feat/x"),
    );
    assert!(answer.is_null(), "{answer}");
}

#[test]
fn a_command_hidden_in_a_longer_line_is_seen() {
    let repo = changed();
    let answer = hook(
        &repo,
        "claude",
        "before-shell",
        &shell("cargo fmt && gh pr create --fill"),
    );
    assert!(denied(&answer), "{answer}");
}

#[test]
fn merging_past_ci_is_left_to_a_person() {
    let repo = changed();
    repo.fz(&["run", "--stage", "merge", "--reason", "done"]);
    for command in ["gh pr merge 2 --admin --squash", "gh pr merge --admin"] {
        let answer = hook(&repo, "claude", "before-shell", &shell(command));
        assert!(denied(&answer), "{command}: {answer}");
        assert!(reason(&answer).contains("person"), "{answer}");
    }
    let plain = hook(
        &repo,
        "claude",
        "before-shell",
        &shell("gh pr merge 2 --squash"),
    );
    assert!(plain.is_null(), "{plain}");
}

#[test]
fn approving_pictures_on_the_pull_request_is_left_to_a_person() {
    let repo = changed();
    for command in [
        "gh pr comment 7 --body \"/fearlezz approve looks right\"",
        "gh api repos/o/r/issues/7/comments -f body='/fearlezz approve ok'",
        "curl -d '{\"body\":\"/fearlezz approve\"}' https://api.github.com/repos/o/r/issues/7/comments",
    ] {
        let answer = hook(&repo, "claude", "before-shell", &shell(command));
        assert!(denied(&answer), "{command}: {answer}");
        assert!(reason(&answer).contains("person"), "{answer}");
    }
    let search = hook(
        &repo,
        "claude",
        "before-shell",
        &shell("rg '/fearlezz approve' README.md"),
    );
    assert!(search.is_null(), "{search}");
}

#[test]
fn a_pull_request_needs_ready_unless_it_is_a_draft() {
    let repo = changed();
    let answer = hook(
        &repo,
        "claude",
        "before-shell",
        &shell("gh pr create --fill"),
    );
    assert!(denied(&answer), "{answer}");
    assert!(reason(&answer).contains("--draft"), "{answer}");

    let draft = hook(
        &repo,
        "claude",
        "before-shell",
        &shell("gh pr create --draft --fill"),
    );
    assert!(draft.is_null(), "{draft}");

    repo.fz(&["run", "--stage", "merge", "--reason", "done"]);
    let ready = hook(
        &repo,
        "claude",
        "before-shell",
        &shell("gh pr create --fill"),
    );
    assert!(ready.is_null(), "{ready}");
}

#[test]
fn other_commands_pass() {
    let repo = changed();
    let answer = hook(&repo, "claude", "before-shell", &shell("git status"));
    assert!(answer.is_null(), "{answer}");
}

#[test]
fn nothing_to_prove_means_nothing_to_refuse() {
    let repo = repo(&format!(
        "{CONFIG}\n[settings.hooks]\nagents = [\"claude\"]\n"
    ));
    let answer = hook(
        &repo,
        "claude",
        "before-shell",
        &shell("gh pr create --fill"),
    );
    assert!(answer.is_null(), "{answer}");
}

// After a check's command run by hand

#[test]
fn a_checks_command_run_by_hand_is_not_proof() {
    let repo = changed();
    let input = serde_json::json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": { "command": "cargo test --quiet -p app" },
    });
    let answer = hook(&repo, "claude", "after-shell", &input.to_string());
    assert_eq!(answer["hookSpecificOutput"]["hookEventName"], "PostToolUse");
    let context = answer["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(context.contains("not proof"), "{context}");
    assert!(context.contains("fearlezz run unit"), "{context}");
}

// When the agent wants to stop

#[test]
fn stopping_with_unproven_work_is_blocked_once() {
    let repo = changed();
    let answer = hook(&repo, "claude", "stop", "{\"stop_hook_active\": false}");
    assert_eq!(answer["decision"], "block", "{answer}");
    assert!(
        answer["reason"].as_str().unwrap().contains("say ask"),
        "{answer}"
    );

    let again = hook(&repo, "claude", "stop", "{\"stop_hook_active\": true}");
    assert!(again.is_null(), "{again}");
}

#[test]
fn waiting_for_the_developer_lets_the_agent_stop() {
    let repo = changed();
    repo.fz(&["say", "ask", "which way?"]);
    let answer = hook(&repo, "claude", "stop", "{\"stop_hook_active\": false}");
    assert!(answer.is_null(), "{answer}");
}

#[test]
fn proven_work_lets_the_agent_stop() {
    let repo = changed();
    repo.fz(&["run", "--stage", "merge", "--reason", "done"]);
    let answer = hook(&repo, "claude", "stop", "{\"stop_hook_active\": false}");
    assert!(answer.is_null(), "{answer}");
}

// Other agents and other places

#[test]
fn codex_answers_as_claude_code_does() {
    let repo = changed();
    let answer = hook(
        &repo,
        "codex",
        "before-shell",
        &shell("gh pr create --fill"),
    );
    assert!(denied(&answer), "{answer}");
}

#[test]
fn cursor_gets_its_own_answers() {
    let repo = changed();
    let deny = hook(
        &repo,
        "cursor",
        "before-shell",
        &serde_json::json!({ "command": "gh pr create --fill" }).to_string(),
    );
    assert_eq!(deny["permission"], "deny", "{deny}");
    assert!(
        deny["agent_message"].as_str().unwrap().contains("--draft"),
        "{deny}"
    );

    let allow = hook(
        &repo,
        "cursor",
        "before-shell",
        &serde_json::json!({ "command": "ls" }).to_string(),
    );
    assert_eq!(allow["permission"], "allow", "{allow}");

    let stop = hook(
        &repo,
        "cursor",
        "stop",
        "{\"status\": \"completed\", \"loop_count\": 0}",
    );
    assert!(stop["followup_message"].as_str().is_some(), "{stop}");
    let again = hook(
        &repo,
        "cursor",
        "stop",
        "{\"status\": \"completed\", \"loop_count\": 1}",
    );
    assert!(again.get("followup_message").is_none(), "{again}");

    let start = hook(&repo, "cursor", "session-start", "{}");
    assert!(start["additional_context"].as_str().is_some(), "{start}");
}

#[test]
fn the_hook_reads_the_project_from_its_input() {
    let repo = changed();
    let input = serde_json::json!({
        "cwd": repo.root.to_str().unwrap(),
        "tool_input": { "command": "gh pr create --fill" },
    });
    let answer = hook(&repo, "claude", "before-shell", &input.to_string());
    assert!(denied(&answer), "{answer}");
}

#[test]
fn outside_a_repository_a_hook_stays_quiet() {
    let dir = tempfile::tempdir().unwrap();
    let out = assert_cmd::Command::cargo_bin("fearlezz")
        .unwrap()
        .args(["setup", "hook", "claude", "stop"])
        .current_dir(dir.path())
        .write_stdin("{\"stop_hook_active\": false}")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}
