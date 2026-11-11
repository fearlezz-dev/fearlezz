//! Recording an app's window: a `record` table on a check, and a still on request. Recording
//! itself needs macOS with Screen Recording; these tests turn the recorder off
//! (`FEARLEZZ_RECORDER=off`) so they say the same on every machine.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;

const CONFIG: &str = r#"
[check.app-look]
run = "echo ran > .runs/app-look"
stage = "look"
inputs = ["app/**"]
record = { film = "home", seconds = 2 }

[check.unit]
run = "true"
stage = "working"
inputs = ["app/**"]
"#;

const OFF: (&str, &str) = ("FEARLEZZ_RECORDER", "off");

fn repo() -> Repo {
    let repo = Repo::new(
        CONFIG,
        &[(".gitignore", ".runs/\n"), ("app/main.txt", "v1")],
    );
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    repo.branch("feat/change");
    repo.write("app/main.txt", "v2");
    repo
}

#[test]
fn ci_does_not_run_a_check_that_cannot_record_there_and_says_so() {
    let repo = repo();
    repo.commit("change");
    let out = repo.fz_env(&["run", "--ci"], &[OFF]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("not run app-look: the recorder is off"),
        "{}",
        out.stdout
    );
    assert!(
        !repo.path(".runs/app-look").exists(),
        "its command never ran"
    );
    assert!(out.stdout.contains("1 not run"), "{}", out.stdout);
}

#[test]
fn locally_a_check_that_cannot_record_fails_with_the_one_time_fix() {
    let repo = repo();
    repo.fz(&["run", "--stage", "look", "--no-run"]);
    let out = repo.fz_env(&["run"], &[OFF]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("fail app-look: the recorder is off"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("  fix: "), "{}", out.stdout);
    assert!(
        !repo.path(".runs/app-look").exists(),
        "its command never ran"
    );
    // Nothing is stored, so once recording works the check is not called unstable.
    let again = repo.fz_env(&["run"], &[OFF]);
    assert!(!again.stdout.contains("unstable"), "{}", again.stdout);
    assert!(
        again.last_line().contains("permission") && again.last_line().contains("fearlezz run"),
        "{}",
        again.stdout
    );
    let status = repo.fz(&[]);
    assert!(
        status.last_line().contains("app-look"),
        "it is still needed: {}",
        status.stdout
    );
}

#[test]
fn a_still_on_request_says_what_to_fix_when_nothing_can_record() {
    let repo = repo();
    let out = repo.fz_env(&["--still", "Demo"], &[OFF]);
    assert!(!out.ok);
    assert!(out.stderr.contains("the recorder is off"), "{}", out.stderr);
}

#[test]
fn a_record_names_a_still_or_a_film() {
    let repo = Repo::new(
        "[check.look]\nrun = \"true\"\nstage = \"look\"\nrecord = { window = \"Demo\" }\n",
        &[],
    );
    let out = repo.fz(&[]);
    assert!(!out.ok);
    assert!(
        out.stderr.contains("record: name a still, a film or both"),
        "{}",
        out.stderr
    );
}

#[test]
fn dropping_the_record_of_a_check_weakens_the_proof() {
    let repo = repo();
    repo.write(
        "fearlezz.toml",
        &format!(
            "{}\n[settings.skill]\nagents = []\n\n[settings.hooks]\nagents = []\n",
            CONFIG.replace("record = { film = \"home\", seconds = 2 }\n", "")
        ),
    );
    repo.commit("change");
    let out = repo.fz_env(&["run", "--ci"], &[OFF]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("it no longer records its app's window"),
        "{}",
        out.stdout
    );
}
