//! The core loop through the CLI: config, run and result store, journal, plan, stages and
//! claims, status, overview. Every test drives a real git repository.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::{Repo, runs};

/// `tests` passes unless `src/broken` exists; `lint` checks a value in `src/`.
/// Each check appends a line to `.runs/<name>` so tests can see real runs. `.runs/` is ignored.
const CONFIG: &str = r#"
[check.tests]
run = "echo run >> .runs/tests; test ! -e src/broken"
stage = "working"
inputs = ["src/**"]

[check.docs]
run = "echo run >> .runs/docs"
stage = "working"
inputs = ["docs/**"]

[check.screens]
run = "echo run >> .runs/screens; echo shot > \"$FEARLEZZ_OUT/home.txt\"; echo '{\"kind\":\"screen\",\"name\":\"home\",\"file\":\"home.txt\"}' > \"$FEARLEZZ_OUT/proof.jsonl\""
stage = "look"
inputs = ["src/**"]

[check.e2e]
run = "echo run >> .runs/e2e"
stage = "merge"
inputs = ["src/**"]
"#;

fn repo() -> Repo {
    let repo = Repo::new(
        CONFIG,
        &[
            (".gitignore", ".runs/\n"),
            (".runs/.keep", ""),
            ("src/lib.rs", "fn a() {}\n"),
            ("docs/readme.md", "docs\n"),
        ],
    );
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    repo.branch("feat/x");
    repo
}

// Config

#[test]
fn config_rejects_a_check_with_an_unknown_stage() {
    let repo = Repo::new(
        "[check.a]\nrun = \"true\"\nstage = \"someday\"\ninputs = [\"**\"]\n",
        &[],
    );
    let out = repo.fz(&["--plan"]);
    assert!(!out.ok);
    assert!(out.stderr.contains("someday"), "stderr: {}", out.stderr);
}

#[test]
fn projects_can_add_their_own_stages() {
    let repo = Repo::new(
        "[stages]\norder = [\"working\", \"qa\", \"merge\"]\n\n[check.a]\nrun = \"true\"\nstage = \"qa\"\ninputs = [\"**\"]\n",
        &[("a.txt", "a")],
    );
    repo.branch("feat/qa");
    repo.write("a.txt", "b");
    let out = repo.fz(&["--plan"]);
    assert!(out.ok, "{}", out.stderr);
    assert!(out.stdout.contains("qa"), "{}", out.stdout);
}

// Plan

#[test]
fn plan_lists_only_checks_whose_inputs_the_change_touched() {
    let repo = repo();
    let out = repo.fz(&["--plan"]);
    assert!(out.ok, "{}", out.stderr);
    assert!(
        !out.stdout.contains("tests"),
        "nothing changed yet: {}",
        out.stdout
    );

    repo.write("src/lib.rs", "fn b() {}\n");
    let out = repo.fz(&["--plan"]);
    assert!(out.stdout.contains("tests"), "{}", out.stdout);
    assert!(out.stdout.contains("screens"), "{}", out.stdout);
    assert!(out.stdout.contains("e2e"), "{}", out.stdout);
    assert!(
        !out.stdout.contains("docs"),
        "docs untouched: {}",
        out.stdout
    );
}

#[test]
fn since_refuses_odd_input_instead_of_crashing() {
    let out = repo().fz(&["--since", "2ü"]);
    assert!(!out.ok);
    assert!(out.stderr.contains("is not like 2h"), "{}", out.stderr);
}

#[test]
fn an_unreadable_journal_line_is_skipped_with_a_warning() {
    let repo = repo();
    repo.fz(&["say", "note", "first"]);
    let journals = repo.path(".git/fearlezz/journals");
    let dir = std::fs::read_dir(&journals)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let events = dir.join("events.jsonl");
    let mut text = std::fs::read_to_string(&events).unwrap();
    text.push_str(
        "{\"at\":\"2026-01-01T00:00:00Z\",\"event\":{\"type\":\"from_the_future\"}}\n{\"at\":",
    );
    std::fs::write(&events, text).unwrap();
    let out = repo.fz(&["--all"]);
    assert!(out.ok, "{}", out.stderr);
    assert!(
        out.stderr.contains("skipped 2 unreadable entries"),
        "{}",
        out.stderr
    );
}

#[test]
fn a_deleted_file_counts_as_touched() {
    let repo = repo();
    repo.remove("docs/readme.md");
    let out = repo.fz(&["--plan"]);
    assert!(out.stdout.contains("docs"), "{}", out.stdout);
}

#[test]
fn every_command_ends_with_a_next_line() {
    let repo = repo();
    repo.write("src/lib.rs", "fn b() {}\n");
    // Every command that prints for people; `result`, `compare --json` and errors are for
    // programs or say what to do themselves. `update` is covered in tests/presets.rs.
    for args in [
        vec!["--plan"],
        vec!["run"],
        vec![],
        vec!["--all"],
        vec!["say", "note", "hi"],
        vec!["say", "claim", "it works"],
        vec!["run", "--stage", "working", "--no-run"],
        vec!["--since", "1h"],
        vec!["setup", "sync"],
        vec!["setup", "down"],
        vec!["--page"],
        vec!["say", "ask", "ready?"],
    ] {
        let out = repo.fz(&args);
        assert!(out.ok, "{args:?}: {}", out.stderr);
        assert!(
            out.last_line().starts_with("next: "),
            "{args:?}: {}",
            out.stdout
        );
    }
}

// Run and result store

#[test]
fn run_runs_needed_checks_of_the_current_stage_and_earlier() {
    let repo = repo();
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    repo.write("src/lib.rs", "fn b() {}\n");
    let out = repo.fz(&["run"]);
    assert!(out.ok, "{}", out.stderr);
    assert_eq!(runs(&repo.path(".runs/tests")), 1);
    assert_eq!(runs(&repo.path(".runs/screens")), 0, "look comes later");
    assert_eq!(runs(&repo.path(".runs/docs")), 0, "docs not touched");
}

#[test]
fn unchanged_inputs_reuse_the_earlier_result() {
    let repo = repo();
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    repo.write("src/lib.rs", "fn b() {}\n");
    repo.fz(&["run"]);
    let out = repo.fz(&["run"]);
    assert_eq!(runs(&repo.path(".runs/tests")), 1);
    assert!(out.stdout.contains("1 reused"), "{}", out.stdout);

    repo.write("docs/readme.md", "other\n");
    repo.fz(&["run", "tests"]);
    assert_eq!(
        runs(&repo.path(".runs/tests")),
        1,
        "docs are not an input of tests"
    );

    repo.write("src/lib.rs", "fn c() {}\n");
    repo.fz(&["run", "tests"]);
    assert_eq!(runs(&repo.path(".runs/tests")), 2);
}

#[test]
fn reuse_survives_a_commit() {
    let repo = repo();
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    repo.write("src/lib.rs", "fn b() {}\n");
    repo.fz(&["run", "tests"]);
    repo.commit("work");
    repo.fz(&["run", "tests"]);
    assert_eq!(runs(&repo.path(".runs/tests")), 1);
}

#[test]
fn a_check_only_sees_the_env_vars_it_declares() {
    let config = r#"
[check.env]
run = "echo \"seen=$SECRET_THING declared=$MODE\" > \"$FEARLEZZ_OUT/env.txt\"; echo '{\"kind\":\"text\",\"file\":\"env.txt\"}' > \"$FEARLEZZ_OUT/proof.jsonl\""
stage = "working"
inputs = ["**"]
env = ["MODE"]
"#;
    let repo = Repo::new(config, &[("a.txt", "a")]);
    repo.branch("feat/env");
    repo.write("a.txt", "b");
    let out = repo.fz_env(
        &["run", "env"],
        &[("SECRET_THING", "leak"), ("MODE", "fast")],
    );
    assert!(out.ok, "{}", out.stderr);
    let result: serde_json::Value =
        serde_json::from_str(&repo.fz(&["--check", "env"]).stdout).unwrap();
    let dir = std::path::Path::new(result["dir"].as_str().unwrap());
    let seen = std::fs::read_to_string(dir.join("env.txt")).unwrap();
    assert_eq!(seen.trim(), "seen= declared=fast");
}

#[test]
fn a_declared_env_var_is_an_input() {
    let config = "[check.env]\nrun = \"echo run >> .runs/env\"\nstage = \"working\"\ninputs = [\"a.txt\"]\nenv = [\"MODE\"]\n";
    let repo = Repo::new(config, &[(".gitignore", ".runs/\n"), ("a.txt", "a")]);
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    repo.branch("feat/env");
    repo.write("a.txt", "b");
    repo.fz_env(&["run", "env"], &[("MODE", "fast")]);
    repo.fz_env(&["run", "env"], &[("MODE", "fast")]);
    repo.fz_env(&["run", "env"], &[("MODE", "slow")]);
    assert_eq!(runs(&repo.path(".runs/env")), 2);
}

#[test]
fn proof_items_and_the_log_are_recorded_with_the_result() {
    let repo = repo();
    repo.write("src/lib.rs", "fn b() {}\n");
    let out = repo.fz(&["run", "screens"]);
    assert!(out.ok, "{}", out.stderr);
    let result = repo.fz(&["--check", "screens"]);
    let json: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
    assert_eq!(json["outcome"], "passed");
    assert_eq!(json["proofs"][0]["kind"], "screen");
    assert_eq!(json["exit_code"], 0);
    assert!(json["tree"].as_str().unwrap().len() >= 40);
    let dir = std::path::PathBuf::from(json["dir"].as_str().unwrap());
    assert_eq!(
        std::fs::read_to_string(dir.join("home.txt")).unwrap(),
        "shot\n"
    );
}

#[test]
fn a_failure_shows_its_tail_and_says_what_to_fix() {
    let repo = repo();
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    repo.write("src/broken", "x");
    let out = repo.fz(&["run"]);
    assert!(!out.ok, "a failed check fails the command");
    assert!(out.stdout.contains("fail tests"), "{}", out.stdout);
    assert!(out.last_line().contains("tests"), "{}", out.stdout);
    assert!(out.lines().len() <= 12, "{}", out.stdout);
}

#[test]
fn a_failure_is_never_hidden_by_an_earlier_pass_on_the_same_inputs() {
    // `flaky` reads `.runs/mode`, which is not an input: same inputs, different result.
    let config = "[check.flaky]\nrun = \"echo run >> .runs/flaky; test \\\"$(cat .runs/mode)\\\" = pass\"\nstage = \"working\"\ninputs = [\"a.txt\"]\n";
    let repo = Repo::new(config, &[(".gitignore", ".runs/\n"), ("a.txt", "a")]);
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    repo.branch("feat/flaky");
    repo.write("a.txt", "b");
    repo.write(".runs/mode", "pass");
    assert!(repo.fz(&["run", "flaky"]).ok);
    repo.write(".runs/mode", "fail");
    repo.fz(&["run", "flaky", "--fresh"]);
    repo.write(".runs/mode", "pass");
    let out = repo.fz(&["run", "flaky"]);
    assert_eq!(
        runs(&repo.path(".runs/flaky")),
        3,
        "after a failure the pass is not reused"
    );
    assert!(out.stdout.contains("unstable"), "{}", out.stdout);
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    let status = repo.fz(&[]);
    assert!(status.stdout.contains("unstable"), "{}", status.stdout);
    // Running again on the same inputs cannot clear it, so next says what can.
    assert!(
        status.last_line().starts_with("next: make flaky stable"),
        "{}",
        status.stdout
    );
}

#[test]
fn another_worktree_reuses_results_but_keeps_its_own_journal() {
    let repo = repo();
    repo.write("src/lib.rs", "fn b() {}\n");
    repo.commit("work");
    repo.fz(&["run", "tests"]);
    repo.fz(&["say", "note", "first branch note"]);

    let other = repo.worktree("feat/y");
    let out = other.fz(&["run", "tests"]);
    assert!(out.stdout.contains("1 reused"), "{}", out.stdout);
    let status = other.fz(&[]);
    assert!(status.stdout.contains("feat/y"), "{}", status.stdout);
    assert!(
        !status.stdout.contains("first branch note"),
        "{}",
        status.stdout
    );
}

// Stages and claims

#[test]
fn the_final_stage_runs_every_check_fresh_once() {
    let repo = repo();
    repo.write("src/lib.rs", "fn b() {}\n");
    repo.fz(&["run", "--stage", "look", "--no-run"]);
    repo.fz(&["run"]);
    assert_eq!(runs(&repo.path(".runs/tests")), 1);

    repo.fz(&["run", "--stage", "merge", "--no-run"]);
    repo.fz(&["run"]);
    assert_eq!(runs(&repo.path(".runs/tests")), 2, "fresh at merge");
    assert_eq!(runs(&repo.path(".runs/e2e")), 1);

    repo.fz(&["run"]);
    assert_eq!(
        runs(&repo.path(".runs/tests")),
        2,
        "a fresh merge result is reused"
    );
}

#[test]
fn a_claim_is_proven_only_by_a_green_run_on_the_current_tree() {
    let repo = repo();
    repo.fz(&["say", "start", "--kind", "feature"]);
    repo.fz(&["say", "claim", "lib has b", "--check", "tests"]);
    repo.write("src/lib.rs", "fn b() {}\n");
    let plan = repo.fz(&["--plan"]);
    assert!(plan.stdout.contains("unproven"), "{}", plan.stdout);

    repo.fz(&["run", "tests"]);
    let plan = repo.fz(&["--plan"]);
    assert!(plan.stdout.contains("proven"), "{}", plan.stdout);
    assert!(!plan.stdout.contains("unproven"), "{}", plan.stdout);

    repo.write("docs/readme.md", "changed\n");
    let plan = repo.fz(&["--plan"]);
    assert!(
        plan.stdout.contains("unproven"),
        "tree changed: {}",
        plan.stdout
    );
}

#[test]
fn a_bug_claim_needs_red_then_green() {
    let repo = repo();
    repo.fz(&["say", "start", "--kind", "bug"]);
    repo.fz(&["say", "claim", "no broken file", "--check", "tests"]);
    repo.write("src/lib.rs", "fn b() {}\n");
    repo.fz(&["run", "tests"]);
    let plan = repo.fz(&["--plan"]);
    assert!(plan.stdout.contains("needs red first"), "{}", plan.stdout);

    repo.write("src/broken", "x");
    repo.fz(&["run", "tests"]);
    repo.remove("src/broken");
    repo.fz(&["run", "tests"]);
    let plan = repo.fz(&["--plan"]);
    assert!(plan.stdout.contains("proven"), "{}", plan.stdout);
    assert!(!plan.stdout.contains("needs red first"), "{}", plan.stdout);
}

#[test]
fn going_back_a_stage_is_allowed_and_keeps_the_reason() {
    let repo = repo();
    repo.fz(&["run", "--stage", "look", "--no-run"]);
    let out = repo.fz(&[
        "run",
        "--stage",
        "working",
        "--reason",
        "screens look wrong",
        "--no-run",
    ]);
    assert!(out.ok, "{}", out.stderr);
    let status = repo.fz(&[]);
    assert!(status.stdout.contains("working"), "{}", status.stdout);
    assert!(
        status.stdout.contains("screens look wrong"),
        "{}",
        status.stdout
    );
}

// Status

#[test]
fn status_is_short_and_shows_the_last_note() {
    let repo = repo();
    repo.write("src/lib.rs", "fn b() {}\n");
    repo.fz(&["run"]);
    repo.fz(&["say", "note", "waiting on the server fix"]);
    let out = repo.fz(&[]);
    assert!(out.lines().len() <= 12, "{}", out.stdout);
    assert!(
        out.stdout.contains("waiting on the server fix"),
        "{}",
        out.stdout
    );
}

#[test]
fn status_says_why_a_check_is_stale() {
    let repo = repo();
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    repo.write("src/lib.rs", "fn b() {}\n");
    repo.fz(&["run"]);
    repo.write("src/lib.rs", "fn c() {}\n");
    let out = repo.fz(&[]);
    assert!(out.stdout.contains("stale"), "{}", out.stdout);
    assert!(out.stdout.contains("src/lib.rs"), "{}", out.stdout);
}

#[test]
fn waiting_for_the_developer_puts_the_journal_first() {
    let repo = repo();
    repo.write("src/lib.rs", "fn b() {}\n");
    repo.fz(&["run"]);
    let other = repo.worktree("feat/waiting");
    other.fz(&["say", "ask", "Is the flash too long?"]);
    let all = repo.fz(&["--all"]);
    let first = all.lines()[0];
    assert!(
        first.contains("needs you") && first.contains("feat/waiting"),
        "{}",
        all.stdout
    );
    assert!(
        all.stdout.contains("Is the flash too long?"),
        "{}",
        all.stdout
    );
    let own = other.fz(&[]);
    assert!(own.last_line().contains("waiting"), "{}", own.stdout);
}

#[test]
fn since_shows_what_happened_after_a_moment() {
    let repo = repo();
    repo.fz_env(
        &["say", "note", "old note"],
        &[("FEARLEZZ_NOW", "2026-11-10T08:00:00Z")],
    );
    repo.fz_env(
        &["say", "note", "new note"],
        &[("FEARLEZZ_NOW", "2026-11-10T12:00:00Z")],
    );
    let out = repo.fz_env(
        &["--since", "2h"],
        &[("FEARLEZZ_NOW", "2026-11-10T13:00:00Z")],
    );
    assert!(out.ok, "{}", out.stderr);
    assert!(out.stdout.contains("new note"), "{}", out.stdout);
    assert!(!out.stdout.contains("old note"), "{}", out.stdout);
}

// Overview

#[test]
fn overview_writes_one_page_with_every_journal() {
    let repo = repo();
    repo.fz(&["say", "note", "main work"]);
    let other = repo.worktree("feat/waiting");
    other.fz(&["say", "ask", "Pick a variant"]);
    let out = repo.fz(&["--page"]);
    assert!(out.ok, "{}", out.stderr);
    let path = out
        .lines()
        .iter()
        .find_map(|l| l.strip_prefix("wrote "))
        .unwrap()
        .to_string();
    let html = std::fs::read_to_string(path).unwrap();
    assert!(html.contains("feat/x") && html.contains("feat/waiting"));
    assert!(html.contains("Needs you"));
    assert!(html.contains("Pick a variant"));
}

#[test]
fn a_detached_head_needs_a_journal_name() {
    let repo = repo();
    repo.git(&["checkout", "-q", "--detach"]);
    let out = repo.fz(&[]);
    assert!(!out.ok);
    assert!(out.stderr.contains("detached"), "{}", out.stderr);
    let out = repo.fz_env(
        &["say", "note", "from ci"],
        &[("FEARLEZZ_JOURNAL", "ci/42")],
    );
    assert!(out.ok, "{}", out.stderr);
    let all = repo.fz(&["--all"]);
    assert!(all.stdout.contains("ci/42"), "{}", all.stdout);
}

#[test]
fn output_into_a_closed_pipe_ends_quietly() {
    let repo = repo();
    let fz = assert_cmd::cargo::cargo_bin("fearlezz");
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "'{}' --plan | head -c 1 >/dev/null; '{}' --help | head -c 1 >/dev/null",
            fz.display(),
            fz.display()
        ))
        .current_dir(&repo.root)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("panicked"), "{stderr}");
}

/// The result records stored for each set of inputs, by key.
fn stored(repo: &Repo) -> std::collections::BTreeMap<String, usize> {
    let root = repo.path(".git/fearlezz/results");
    std::fs::read_dir(root)
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| {
            let n = std::fs::read_dir(e.path())
                .unwrap()
                .flatten()
                .filter(|f| f.path().extension().is_some_and(|x| x == "json"))
                .count();
            (e.file_name().to_string_lossy().to_string(), n)
        })
        .collect()
}

#[test]
fn old_results_are_cleaned_up_but_what_is_read_stays() {
    let config = "[check.c]\nrun = \"test \\\"$(cat .runs/mode)\\\" = pass\"\nstage = \"working\"\ninputs = [\"a.txt\"]\n";
    let repo = Repo::new(config, &[(".gitignore", ".runs/\n"), ("a.txt", "a")]);
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    repo.branch("feat/x");
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    let day =
        |n: i64| (jiff::Timestamp::now() + jiff::SignedDuration::from_hours(24 * n)).to_string();
    let run = |mode: &str, at: &str| {
        repo.write(".runs/mode", mode);
        repo.fz_env(&["run", "c", "--fresh"], &[("FEARLEZZ_NOW", at)]);
    };
    for mode in ["pass", "fail", "pass", "pass"] {
        run(mode, &day(0));
    }
    assert_eq!(stored(&repo).values().copied().collect::<Vec<_>>(), [4]);

    // A day later the cleanup runs: the latest result, which is the latest pass, and the
    // latest fail stay, so the check is still known to be unstable.
    run("pass", &day(2));
    assert_eq!(stored(&repo).values().copied().collect::<Vec<_>>(), [2]);
    assert!(repo.fz(&[]).stdout.contains("unstable"));

    // Other inputs nobody ran for 30 days go; the ones the journal points at stay.
    let first = stored(&repo).into_keys().next().unwrap();
    repo.write("a.txt", "b");
    run("pass", &day(2));
    repo.write("a.txt", "a");
    run("pass", &day(40));
    let keys: Vec<String> = stored(&repo).into_keys().collect();
    assert_eq!(keys, [first]);
}

// The command line

#[test]
fn fearlezz_alone_says_where_the_work_stands_and_teaches_the_first_step() {
    let repo = repo();
    let out = repo.fz(&[]);
    assert!(out.ok, "{}", out.stderr);
    assert!(
        out.last_line()
            .starts_with("next: say what this work is: fearlezz say start"),
        "{}",
        out.stdout
    );
}

#[test]
fn run_with_a_stage_moves_and_runs_and_no_run_only_moves() {
    let repo = repo();
    repo.write("src/lib.rs", "fn b() {}\n");
    let out = repo.fz(&["run", "--stage", "working", "--no-run"]);
    assert!(
        out.stdout.contains("stage exploring -> working"),
        "{}",
        out.stdout
    );
    assert_eq!(runs(&repo.path(".runs/tests")), 0);
    assert!(out.last_line().starts_with("next: "), "{}", out.stdout);

    let out = repo.fz(&["run", "--stage", "look"]);
    assert!(out.ok, "{}", out.stderr);
    assert!(
        out.stdout.contains("stage working -> look"),
        "{}",
        out.stdout
    );
    assert_eq!(runs(&repo.path(".runs/tests")), 1);
}

#[test]
fn views_of_the_work_do_not_combine_and_no_run_needs_a_stage() {
    let repo = repo();
    assert!(!repo.fz(&["--all", "--plan"]).ok);
    assert!(!repo.fz(&["run", "--no-run"]).ok);
    assert!(!repo.fz(&["--all", "run"]).ok);
}
