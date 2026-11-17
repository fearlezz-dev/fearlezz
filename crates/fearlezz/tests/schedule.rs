//! Scheduled checks: too slow, too costly or needing real infrastructure to run per change, so
//! a change never needs them; `fearlezz run --ci --scheduled` proves them on a schedule.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;

const CONFIG: &str = r#"
[check.unit]
run = 'echo unit >> ran'
stage = "working"

[check.full]
run = 'echo full >> ran'
stage = "working"
schedule = true
"#;

/// A repository with `CONFIG` and a change on a branch, so every check of a change is due.
fn repo(config: &str) -> Repo {
    let repo = Repo::new(config, &[(".gitignore", "ran\n"), ("src/lib.txt", "a")]);
    repo.branch("feat/x");
    repo.write("src/lib.txt", "b");
    repo.commit("change");
    repo
}

fn ran(repo: &Repo) -> String {
    std::fs::read_to_string(repo.path("ran")).unwrap_or_default()
}

#[test]
fn a_change_never_needs_a_scheduled_check() {
    let repo = repo(CONFIG);
    let out = repo.fz(&["run", "--stage", "merge"]);
    assert!(out.ok, "{}", out.stdout);
    assert_eq!(ran(&repo), "unit\n");
    assert!(out.last_line().contains("ready to merge"), "{}", out.stdout);
}

#[test]
fn proving_a_commit_leaves_scheduled_checks_out() {
    let repo = repo(CONFIG);
    let out = repo.fz(&["run", "--ci"]);
    assert!(out.ok, "{}", out.stdout);
    assert_eq!(ran(&repo), "unit\n");
}

#[test]
fn a_scheduled_run_proves_only_the_scheduled_checks() {
    let repo = repo(CONFIG);
    let out = repo.fz(&["run", "--ci", "--scheduled"]);
    assert!(out.ok, "{}", out.stdout);
    assert_eq!(ran(&repo), "full\n");
}

#[test]
fn a_scheduled_run_fails_when_a_scheduled_check_fails() {
    let repo = repo(&CONFIG.replace("echo full >> ran", "exit 1"));
    let out = repo.fz(&["run", "--ci", "--scheduled"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stdout.contains("fail full"), "{}", out.stdout);
}

#[test]
fn the_plan_names_the_scheduled_checks() {
    let repo = repo(CONFIG);
    let out = repo.fz(&["--plan"]);
    assert!(
        out.lines()
            .iter()
            .any(|l| l.contains("scheduled") && l.contains("full") && !l.contains("unit")),
        "{}",
        out.stdout
    );
    assert!(out.last_line().starts_with("next:"), "{}", out.stdout);
}

#[test]
fn naming_a_scheduled_check_runs_it() {
    let repo = repo(CONFIG);
    let out = repo.fz(&["run", "full"]);
    assert!(out.ok, "{}", out.stdout);
    assert_eq!(ran(&repo), "full\n");
}

#[test]
fn a_scheduled_run_without_scheduled_checks_proves_nothing_and_fails() {
    let repo = repo(&CONFIG.replace("schedule = true\n", ""));
    let out = repo.fz(&["run", "--ci", "--scheduled"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stderr.contains("no check has schedule = true"),
        "{}\n{}",
        out.stdout,
        out.stderr
    );
}

#[test]
fn a_proof_split_over_jobs_leaves_scheduled_checks_out_of_its_legs() {
    let repo = repo(CONFIG);
    let out = repo.fz(&["run", "--ci", "--plan"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("unit"), "{}", out.stdout);
    assert!(!out.stdout.contains("full"), "{}", out.stdout);
}

#[test]
fn a_scheduled_run_proves_in_one_job() {
    let repo = repo(CONFIG);
    for split in [&["--plan"][..], &["--leg", "default"], &["--gate", "legs"]] {
        let mut args = vec!["run", "--ci", "--scheduled"];
        args.extend(split);
        let out = repo.fz(&args);
        assert!(!out.ok, "{args:?}: {}", out.stdout);
        assert!(
            out.stderr.contains("cannot be used with"),
            "{args:?}: {}",
            out.stderr
        );
    }
}
