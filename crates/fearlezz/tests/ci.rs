//! Proving a commit for CI with `fearlezz run --ci`: what the change needs, fresh, with no
//! journal, against the rules of the base branch, so a change cannot weaken its own proof.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::{Repo, runs};

/// `tests` fails while `src/broken` exists. Each check appends a line to `.runs/<name>`.
const CHECKS: &str = r#"
[check.tests]
run = "echo run >> .runs/tests; test ! -e src/broken"
stage = "working"
inputs = ["src/**"]

[check.e2e]
run = "echo run >> .runs/e2e"
stage = "merge"
inputs = ["src/**"]

[check.docs]
run = "echo run >> .runs/docs; test ! -e docs/broken"
stage = "working"
inputs = ["docs/**"]
"#;

const NO_PROOF: &str = "no_proof = [\"docs/**\"]\n";

const NEXT: &str =
    "next: this proved the commit, not the work; fearlezz shows where the work stands";

/// `fearlezz.toml` as `Repo::new` writes it, so a branch can change it.
fn full(config: &str) -> String {
    format!("{config}\n[settings.skill]\nagents = []\n")
}

fn repo_with(config: &str) -> Repo {
    let repo = Repo::new(
        config,
        &[
            (".gitignore", ".runs/\n"),
            ("src/lib.rs", "fn a() {}\n"),
            ("docs/readme.md", "docs\n"),
            ("README.md", "readme\n"),
        ],
    );
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    repo
}

/// Writes the work file with a goal and one claim proven by `check`.
fn declare(repo: &Repo, check: &str) {
    repo.fz(&["say", "start", "--kind", "feature", "--goal", "the goal"]);
    repo.fz(&["say", "claim", "works", "--check", check]);
}

/// A branch off `main` with a committed change the checks read, and its work file.
fn branch_with_change() -> Repo {
    let repo = repo_with(&format!("{NO_PROOF}{CHECKS}"));
    repo.branch("feat/x");
    repo.write("src/lib.rs", "fn b() {}\n");
    declare(&repo, "tests");
    repo.commit("change");
    repo
}

/// The branch changes `fearlezz.toml` to `checks`, keeping the rest.
fn with_checks(repo: &Repo, checks: &str) {
    repo.write("fearlezz.toml", &full(&format!("{NO_PROOF}{checks}")));
    repo.commit("config");
}

fn ci(repo: &Repo) -> common::Output {
    repo.fz(&["run", "--ci"])
}

// What runs

#[test]
fn ci_runs_what_the_change_needs_fresh_and_writes_no_journal() {
    let repo = branch_with_change();
    for round in 1..=2 {
        let out = repo.fz_env(&["run", "--ci"], &[("FEARLEZZ_JOURNAL", "ci/1")]);
        assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
        assert_eq!(out.last_line(), NEXT);
        assert!(!out.stdout.contains("ready to merge"), "{}", out.stdout);
        for check in ["tests", "e2e"] {
            assert_eq!(
                runs(&repo.path(&format!(".runs/{check}"))),
                round,
                "{check}"
            );
        }
    }
    assert!(!repo.path(".git/fearlezz/journals/ci%2F1").exists());
    assert!(!repo.path(".git/fearlezz/journals/ci").exists());
}

#[test]
fn ci_on_the_base_branch_runs_every_check() {
    let repo = repo_with(&format!("{NO_PROOF}{CHECKS}"));
    let out = ci(&repo);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert_eq!(runs(&repo.path(".runs/tests")), 1, "{}", out.stdout);
    assert_eq!(runs(&repo.path(".runs/e2e")), 1);
}

#[test]
fn ci_fails_when_a_check_fails() {
    let repo = branch_with_change();
    repo.write("src/broken", "x\n");
    repo.commit("break");
    let out = ci(&repo);
    assert!(!out.ok, "{}", out.stdout);
    assert_eq!(out.last_line(), NEXT);
}

#[test]
fn ci_leaves_the_journal_of_the_branch_alone() {
    let repo = branch_with_change();
    repo.fz(&["say", "start", "--kind", "feature", "--goal", "the goal"]);
    assert!(ci(&repo).ok);
    let status = repo.fz(&[]);
    assert!(
        status.stdout.contains("stage exploring"),
        "{}",
        status.stdout
    );
    assert!(
        status.stdout.contains("tests never run"),
        "{}",
        status.stdout
    );
}

#[test]
fn ci_does_not_combine_with_a_stage_or_named_checks() {
    let repo = branch_with_change();
    assert!(!repo.fz(&["run", "--ci", "--stage", "merge"]).ok);
    assert!(!repo.fz(&["run", "--ci", "tests"]).ok);
}

// What it shows

#[test]
fn ci_shows_the_commit_and_each_check_with_its_command() {
    let repo = branch_with_change();
    let head = repo.git(&["rev-parse", "--short", "HEAD"]);
    let out = ci(&repo);
    assert!(
        out.stdout.contains(&format!("commit {}", head.trim())),
        "{}",
        out.stdout
    );
    let line = out
        .lines()
        .into_iter()
        .find(|l| l.contains("passed tests"))
        .unwrap_or_else(|| panic!("{}", out.stdout));
    assert!(line.contains("echo run >> .runs/tests"), "{line}");
    assert!(line.contains(" s "), "{line}");
}

#[test]
fn ci_writes_the_job_summary_when_asked() {
    let repo = branch_with_change();
    let summary = repo.path(".runs/summary.md");
    let out = repo.fz_env(
        &["run", "--ci"],
        &[("GITHUB_STEP_SUMMARY", summary.to_str().unwrap())],
    );
    assert!(out.ok, "{}", out.stdout);
    let text = std::fs::read_to_string(&summary).unwrap();
    assert!(text.contains("| tests | passed |"), "{text}");
    assert!(text.contains("| e2e | passed |"), "{text}");
}

// The base branch is the floor

#[test]
fn turning_a_check_off_is_red_and_named() {
    let repo = branch_with_change();
    with_checks(
        &repo,
        &CHECKS.replace("[check.e2e]\n", "[check.e2e]\noff = true\n"),
    );
    let out = ci(&repo);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("weakens e2e: it is gone"),
        "{}",
        out.stdout
    );
}

#[test]
fn narrowing_the_inputs_is_red_and_named() {
    let repo = branch_with_change();
    with_checks(
        &repo,
        &CHECKS.replace(
            "inputs = [\"src/**\"]\n\n[check.e2e]",
            "inputs = [\"src/other/**\"]\n\n[check.e2e]",
        ),
    );
    let out = ci(&repo);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("weakens tests: it no longer reads src/lib.rs"),
        "{}",
        out.stdout
    );
}

#[test]
fn changing_the_command_is_red_and_named() {
    let repo = branch_with_change();
    with_checks(&repo, &CHECKS.replace("test ! -e src/broken", "true"));
    let out = ci(&repo);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("weakens tests: it runs another command"),
        "{}",
        out.stdout
    );
}

#[test]
fn moving_a_check_to_a_later_stage_is_red_and_named() {
    let repo = branch_with_change();
    with_checks(
        &repo,
        &CHECKS.replacen("stage = \"working\"", "stage = \"look\"", 1),
    );
    let out = ci(&repo);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("weakens tests: it moves to a later stage"),
        "{}",
        out.stdout
    );
}

#[test]
fn adding_a_check_or_widening_its_inputs_is_fine() {
    let repo = branch_with_change();
    let more = CHECKS.replacen(
        "inputs = [\"src/**\"]",
        "inputs = [\"src/**\", \"lib/**\"]",
        1,
    );
    with_checks(
        &repo,
        &format!(
            "{more}\n[check.extra]\nrun = \"true\"\nstage = \"working\"\ninputs = [\"src/**\"]\n"
        ),
    );
    let out = ci(&repo);
    assert!(out.ok, "{}", out.stdout);
    assert!(!out.stdout.contains("weakens"), "{}", out.stdout);
}

#[test]
fn the_fix_line_names_the_admin_bypass() {
    let repo = branch_with_change();
    with_checks(&repo, &CHECKS.replace("test ! -e src/broken", "true"));
    let out = ci(&repo);
    assert!(out.stdout.contains("admin"), "{}", out.stdout);
}

// Every changed file is proven or declared

#[test]
fn a_changed_file_no_check_reads_is_red() {
    let repo = repo_with(&format!("{NO_PROOF}{CHECKS}"));
    repo.branch("feat/x");
    repo.write("README.md", "changed\n");
    repo.commit("readme");
    let out = ci(&repo);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stdout.contains("unproven README.md"), "{}", out.stdout);
}

#[test]
fn a_file_the_base_declares_as_needing_no_proof_is_fine() {
    let repo = repo_with(&format!("{NO_PROOF}{CHECKS}"));
    repo.branch("feat/x");
    repo.write("docs/readme.md", "changed\n");
    repo.commit("docs");
    let out = ci(&repo);
    assert!(out.ok, "{}", out.stdout);
}

#[test]
fn the_change_cannot_declare_its_own_files_as_needing_no_proof() {
    let repo = repo_with(&format!("{NO_PROOF}{CHECKS}"));
    repo.branch("feat/x");
    repo.write("README.md", "changed\n");
    repo.write(
        "fearlezz.toml",
        &full(&format!("no_proof = [\"docs/**\", \"*.md\"]\n{CHECKS}")),
    );
    repo.commit("readme");
    let out = ci(&repo);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("weakens no_proof: *.md"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("unproven README.md"), "{}", out.stdout);
}

#[test]
fn the_config_files_are_judged_by_the_floor_not_by_a_check() {
    let repo = branch_with_change();
    with_checks(
        &repo,
        &format!(
            "{CHECKS}\n[check.extra]\nrun = \"true\"\nstage = \"working\"\ninputs = [\"src/**\"]\n"
        ),
    );
    let out = ci(&repo);
    assert!(out.ok, "{}", out.stdout);
    assert!(
        !out.stdout.contains("unproven fearlezz.toml"),
        "{}",
        out.stdout
    );
}

// The work file: what the change claims, proven again

#[test]
fn a_code_change_without_a_work_file_is_red() {
    let repo = repo_with(&format!("{NO_PROOF}{CHECKS}"));
    repo.branch("feat/x");
    repo.write("src/lib.rs", "fn b() {}\n");
    repo.commit("change");
    let out = ci(&repo);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stdout.contains("no goal or claims"), "{}", out.stdout);
}

#[test]
fn a_change_with_only_files_needing_no_proof_needs_no_work_file() {
    let repo = repo_with(&format!("{NO_PROOF}{CHECKS}"));
    repo.branch("feat/x");
    repo.write("docs/readme.md", "changed\n");
    repo.commit("docs");
    let out = ci(&repo);
    assert!(out.ok, "{}", out.stdout);
}

#[test]
fn each_claim_is_proven_again_in_ci() {
    let repo = branch_with_change();
    let out = ci(&repo);
    assert!(out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("proven c1 by tests: works"),
        "{}",
        out.stdout
    );
}

#[test]
fn the_check_of_a_claim_runs_even_when_the_change_does_not_need_it() {
    let repo = repo_with(&format!("{NO_PROOF}{CHECKS}"));
    repo.branch("feat/x");
    repo.write("src/lib.rs", "fn b() {}\n");
    declare(&repo, "docs");
    repo.commit("change");
    let out = ci(&repo);
    assert!(out.ok, "{}", out.stdout);
    assert_eq!(runs(&repo.path(".runs/docs")), 1, "{}", out.stdout);
    assert!(
        out.stdout.contains("proven c1 by docs: works"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_claim_whose_check_fails_is_red() {
    let repo = repo_with(&format!("{NO_PROOF}{CHECKS}"));
    repo.branch("feat/x");
    repo.write("src/lib.rs", "fn b() {}\n");
    repo.write("docs/broken", "x\n");
    declare(&repo, "docs");
    repo.commit("change");
    let out = ci(&repo);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("unproven c1: docs failed"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_claim_without_a_check_is_red() {
    let repo = repo_with(&format!("{NO_PROOF}{CHECKS}"));
    repo.branch("feat/x");
    repo.write("src/lib.rs", "fn b() {}\n");
    repo.fz(&["say", "start", "--kind", "feature", "--goal", "the goal"]);
    repo.fz(&["say", "claim", "no check named"]);
    repo.commit("change");
    let out = ci(&repo);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("unproven c1: no check proves it"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_work_file_without_claims_is_red() {
    let repo = repo_with(&format!("{NO_PROOF}{CHECKS}"));
    repo.branch("feat/x");
    repo.write("src/lib.rs", "fn b() {}\n");
    repo.fz(&["say", "start", "--kind", "feature", "--goal", "the goal"]);
    repo.commit("change");
    let out = ci(&repo);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("no claims in .fearlezz/work/feat/x.toml"),
        "{}",
        out.stdout
    );
}

#[test]
fn the_job_summary_shows_the_claims() {
    let repo = branch_with_change();
    let summary = repo.path(".runs/summary.md");
    repo.fz_env(
        &["run", "--ci"],
        &[("GITHUB_STEP_SUMMARY", summary.to_str().unwrap())],
    );
    let text = std::fs::read_to_string(&summary).unwrap();
    assert!(text.contains("proven c1 by tests: works"), "{text}");
}

// The agent finds out at its own final run

#[test]
fn the_final_run_of_the_work_reports_a_weakening_too() {
    let repo = branch_with_change();
    with_checks(&repo, &CHECKS.replace("test ! -e src/broken", "true"));
    repo.fz(&["say", "start", "--kind", "feature", "--goal", "the goal"]);
    repo.fz(&["say", "claim", "works", "--check", "tests"]);
    let out = repo.fz(&["run", "--stage", "merge", "--reason", "done"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stdout.contains("weakens tests"), "{}", out.stdout);
    assert!(!out.stdout.contains("ready to merge"), "{}", out.stdout);
}
