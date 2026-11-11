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
    format!("{config}\n[settings.skill]\nagents = []\n\n[settings.hooks]\nagents = []\n")
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

/// A branch off `main` with a committed change the checks read.
fn branch_with_change() -> Repo {
    let repo = repo_with(&format!("{NO_PROOF}{CHECKS}"));
    repo.branch("feat/x");
    repo.write("src/lib.rs", "fn b() {}\n");
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
    repo.fz(&["say", "note", "started"]);
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

#[test]
fn the_job_summary_shows_the_end_of_a_failing_output() {
    let repo = repo_with(&format!(
        "{NO_PROOF}[check.tests]\nrun = \"echo boom-output; test ! -e src/broken\"\nstage = \"working\"\ninputs = \
         [\"src/**\"]\n"
    ));
    repo.branch("feat/x");
    repo.write("src/broken", "x\n");
    repo.commit("break");
    let summary = repo.path(".runs/summary.md");
    let out = repo.fz_env(
        &["run", "--ci"],
        &[("GITHUB_STEP_SUMMARY", summary.to_str().unwrap())],
    );
    assert!(!out.ok, "{}", out.stdout);
    let text = std::fs::read_to_string(&summary)
        .unwrap_or_else(|_| panic!("{}\n{}", out.stdout, out.stderr));
    assert!(text.contains("boom-output"), "{text}");
}

// The status comment on the pull request

/// The variables GitHub Actions sets, so the comment can link the run.
const GITHUB: [(&str, &str); 3] = [
    ("GITHUB_SERVER_URL", "https://github.com"),
    ("GITHUB_REPOSITORY", "owner/repo"),
    ("GITHUB_RUN_ID", "42"),
];

fn comment(repo: &Repo) -> (common::Output, String) {
    let path = repo.path(".runs/comment.md");
    let out = repo.fz_env(
        &["run", "--ci", "--comment", path.to_str().unwrap()],
        &GITHUB,
    );
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{}", out.stdout));
    (out, text)
}

#[test]
fn the_status_comment_says_proven_and_lists_the_checks() {
    let repo = branch_with_change();
    let (out, text) = comment(&repo);
    assert!(out.ok, "{}", out.stdout);
    assert!(text.starts_with("<!-- fearlezz -->"), "{text}");
    assert!(text.contains("**Proven**"), "{text}");
    assert!(text.contains("| tests | passed |"), "{text}");
    assert!(
        text.contains("https://github.com/owner/repo/actions/runs/42"),
        "{text}"
    );
    assert!(!text.contains("| what |"), "{text}");
}

#[test]
fn the_status_comment_names_each_blocker_and_what_to_do() {
    let repo = branch_with_change();
    repo.write("src/broken", "x\n");
    repo.write("README.md", "changed\n");
    repo.commit("break");
    with_checks(
        &repo,
        &CHECKS.replace("[check.e2e]\n", "[check.e2e]\noff = true\n"),
    );
    let (out, text) = comment(&repo);
    assert!(!out.ok, "{}", out.stdout);
    assert!(text.contains("**Not proven**"), "{text}");
    assert!(text.contains("| what | why | what to do |"), "{text}");
    assert!(text.contains("| check `tests` failed |"), "{text}");
    assert!(text.contains("| `README.md` has no proof |"), "{text}");
    assert!(text.contains("add it to a check's inputs"), "{text}");
    assert!(text.contains("| check `e2e` is weaker |"), "{text}");
    assert!(text.contains("it is gone"), "{text}");
}

#[test]
fn the_status_comment_needs_ci() {
    let repo = branch_with_change();
    assert!(!repo.fz(&["run", "--comment", "x.md"]).ok);
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

// Lints, rules and env are part of the floor

/// A findings check with lint levels, an env list, and two rules.
const LINTED: &str = r#"
[check.lint]
run = "touch \"$FEARLEZZ_OUT/findings.jsonl\""
stage = "working"
inputs = ["src/**"]
env = ["HOME"]
findings = { read = "jsonl", on = "everywhere", lints = ["ruff"] }

[lints.ruff]
E501 = "gate"
F401 = "flag"

[rule.no-secrets]
level = "gate"
message = "keep secrets out"
files = ["src/**"]
skip = ["src/vendor/**"]
changed = ["secrets/**"]

[rule.snake]
level = "flag"
files = ["src/**"]
name = "snake_case"
"#;

/// A branch whose config turns `LINTED` into `edit(LINTED)`, with the tool files synced on both
/// sides, and what CI says about it.
fn ci_after(edit: impl Fn(&str) -> String) -> common::Output {
    let repo = repo_with(&format!("{NO_PROOF}{CHECKS}{LINTED}"));
    repo.fz(&["setup", "sync"]);
    repo.commit("sync");
    repo.branch("feat/x");
    repo.write("src/lib.rs", "fn b() {}\n");
    repo.commit("change");
    repo.write(
        "fearlezz.toml",
        &full(&format!("{NO_PROOF}{CHECKS}{}", edit(LINTED))),
    );
    repo.fz(&["setup", "sync"]);
    repo.commit("config");
    ci(&repo)
}

fn assert_weakens(out: &common::Output, line: &str) {
    assert!(!out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains(line), "{}\n{}", out.stdout, out.stderr);
}

#[test]
fn lowering_a_lint_is_red_and_named() {
    let out = ci_after(|c| c.replace("E501 = \"gate\"", "E501 = \"flag\""));
    assert_weakens(&out, "weakens lints.ruff.E501: it goes from gate to flag");
}

#[test]
fn calling_more_pictures_flaky_is_red_and_named() {
    let out = ci_after(|c| c.replace("env = [\"HOME\"]", "env = [\"HOME\"]\nflaky = [\"home\"]"));
    assert_weakens(&out, "weakens lint: it calls more pictures flaky (+home)");
}

#[test]
fn unsetting_a_lint_is_red_and_named() {
    let out = ci_after(|c| c.replace("F401 = \"flag\"\n", ""));
    assert_weakens(&out, "weakens lints.ruff.F401: it is no longer set");
}

#[test]
fn a_check_that_drops_its_lints_table_is_red() {
    let out = ci_after(|c| c.replace(", lints = [\"ruff\"]", ""));
    assert_weakens(
        &out,
        "weakens lint: its findings no longer get the levels of [lints.ruff]",
    );
}

#[test]
fn findings_that_count_only_on_changed_lines_again_are_red() {
    let out = ci_after(|c| c.replace("on = \"everywhere\", ", ""));
    assert_weakens(
        &out,
        "weakens lint: its findings count only on changed lines",
    );
}

#[test]
fn adding_an_env_variable_is_red() {
    let out = ci_after(|c| c.replace("env = [\"HOME\"]", "env = [\"HOME\", \"SKIP_SLOW\"]"));
    assert_weakens(
        &out,
        "weakens lint: it sees other environment variables (+SKIP_SLOW)",
    );
}

#[test]
fn removing_an_env_variable_is_red() {
    let out = ci_after(|c| c.replace("env = [\"HOME\"]", "env = []"));
    assert_weakens(
        &out,
        "weakens lint: it sees other environment variables (-HOME)",
    );
}

#[test]
fn turning_a_rule_off_is_red() {
    let out = ci_after(|c| c.replace("[rule.no-secrets]\n", "[rule.no-secrets]\noff = true\n"));
    assert_weakens(&out, "weakens rule.no-secrets: it is gone");
}

#[test]
fn lowering_a_rule_is_red() {
    let out = ci_after(|c| c.replacen("level = \"gate\"", "level = \"hint\"", 1));
    assert_weakens(
        &out,
        "weakens rule.no-secrets: its level goes from gate to hint",
    );
}

#[test]
fn a_rule_covering_fewer_files_is_red() {
    let out = ci_after(|c| c.replace("skip = [\"src/vendor/**\"]", "skip = [\"src/**\"]"));
    assert_weakens(
        &out,
        "weakens rule.no-secrets: it no longer covers src/lib.rs",
    );
}

#[test]
fn a_rule_that_checks_something_else_is_red() {
    let out = ci_after(|c| c.replace("changed = [\"secrets/**\"]", "changed = [\"keys/**\"]"));
    assert_weakens(&out, "weakens rule.no-secrets: it checks something else");
}

#[test]
fn raising_levels_and_rewording_a_rule_is_fine() {
    let out = ci_after(|c| {
        c.replace("F401 = \"flag\"", "F401 = \"gate\"")
            .replace("keep secrets out", "never commit secrets")
            .replace("level = \"flag\"", "level = \"gate\"")
    });
    assert!(out.ok, "{}", out.stdout);
    assert!(!out.stdout.contains("weakens"), "{}", out.stdout);
}

// Settings and the environment are part of the floor

/// Tool settings, agent settings, and an environment.
const TUNED: &str = r#"
[env]
prepare = "true"
inputs = ["env.nix"]
up = "true"
down = "true"
env = ["HOME"]

[settings.clippy]
allow-unwrap-in-tests = false

[settings.ruff]
line-length = 100

[settings.claude]
permissions = { allow = ["Bash(ls)"] }
"#;

/// A branch whose config turns `TUNED` into `edit(TUNED)`, with the tool files synced on both
/// sides, and what CI says about it.
fn ci_after_tuning(edit: impl Fn(&str) -> String) -> common::Output {
    let repo = repo_with(&format!("{NO_PROOF}{CHECKS}{TUNED}"));
    repo.fz(&["setup", "sync"]);
    repo.commit("sync");
    repo.branch("feat/x");
    repo.write("src/lib.rs", "fn b() {}\n");
    repo.commit("change");
    repo.write(
        "fearlezz.toml",
        &full(&format!("{NO_PROOF}{CHECKS}{}", edit(TUNED))),
    );
    repo.fz(&["setup", "sync"]);
    repo.commit("tune");
    ci(&repo)
}

/// A synced repository with `TUNED`, and a branch whose only change is `fearlezz.toml` turned
/// into `edit(TUNED)`, left unsynced.
fn config_only_branch(edit: impl Fn(&str) -> String) -> Repo {
    let repo = repo_with(&format!("{NO_PROOF}{CHECKS}{TUNED}"));
    repo.fz(&["setup", "sync"]);
    repo.commit("sync");
    repo.branch("feat/x");
    repo.write(
        "fearlezz.toml",
        &full(&format!("{NO_PROOF}{CHECKS}{}", edit(TUNED))),
    );
    repo.commit("tune");
    repo
}

#[test]
fn settings_changed_without_a_sync_are_caught() {
    let repo = config_only_branch(|c| c.replace("Bash(ls)", "Bash(eza)"));
    let out = ci(&repo);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("differs from the settings in fearlezz.toml"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_change_to_the_config_alone_runs_every_check() {
    let repo = config_only_branch(|c| {
        format!("{c}\n[check.extra]\nrun = \"true\"\nstage = \"working\"\ninputs = [\"lib/**\"]\n")
    });
    let out = ci(&repo);
    assert!(out.ok, "{}", out.stdout);
    for check in ["tests", "e2e", "docs"] {
        assert_eq!(
            runs(&repo.path(&format!(".runs/{check}"))),
            1,
            "{check}: {}",
            out.stdout
        );
    }
    assert!(out.stdout.contains("passed extra"), "{}", out.stdout);
}

#[test]
fn adding_a_tool_setting_is_red_and_named() {
    let out = ci_after_tuning(|c| c.replace("line-length = 100", "line-length = 100\nfix = true"));
    assert_weakens(&out, "weakens settings.ruff.fix: it is added (true)");
}

#[test]
fn changing_a_tool_setting_is_red_and_named() {
    let out = ci_after_tuning(|c| {
        c.replace(
            "allow-unwrap-in-tests = false",
            "allow-unwrap-in-tests = true",
        )
    });
    assert_weakens(
        &out,
        "weakens settings.clippy.allow-unwrap-in-tests: it changes from false to true",
    );
}

#[test]
fn removing_a_tool_setting_is_red_and_named() {
    let out = ci_after_tuning(|c| c.replace("line-length = 100\n", ""));
    assert_weakens(
        &out,
        "weakens settings.ruff.line-length: it is removed (was 100)",
    );
}

#[test]
fn agent_settings_may_change() {
    let out = ci_after_tuning(|c| c.replace("Bash(ls)", "Bash(eza)"));
    assert!(out.ok, "{}", out.stdout);
    assert!(!out.stdout.contains("weakens"), "{}", out.stdout);
}

#[test]
fn changing_an_env_command_is_red_and_named() {
    let out = ci_after_tuning(|c| c.replace("prepare = \"true\"", "prepare = \"echo skip\""));
    assert_weakens(
        &out,
        "weakens env.prepare: it changes from true to echo skip",
    );
}

#[test]
fn adding_a_shell_is_red_and_named() {
    let out =
        ci_after_tuning(|c| c.replace("prepare = \"true\"", "prepare = \"true\"\nshell = \"env\""));
    assert_weakens(&out, "weakens env.shell: it is added (env)");
}

#[test]
fn removing_a_service_command_is_red_and_named() {
    let out = ci_after_tuning(|c| c.replace("down = \"true\"\n", ""));
    assert_weakens(&out, "weakens env.down: it is removed (was true)");
}

#[test]
fn changing_the_env_variables_is_red_and_named() {
    let out = ci_after_tuning(|c| c.replace("env = [\"HOME\"]", "env = [\"CI\"]"));
    assert_weakens(&out, "weakens env.env: it changes (+CI, -HOME)");
}

#[test]
fn narrowing_the_env_inputs_is_red_but_widening_is_fine() {
    let out = ci_after_tuning(|c| c.replace("inputs = [\"env.nix\"]", "inputs = []"));
    assert_weakens(&out, "weakens env.inputs: it no longer has env.nix");
    let out = ci_after_tuning(|c| {
        c.replace(
            "inputs = [\"env.nix\"]",
            "inputs = [\"env.nix\", \"env.lock\"]",
        )
    });
    assert!(out.ok, "{}", out.stdout);
    assert!(!out.stdout.contains("weakens"), "{}", out.stdout);
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
fn the_work_is_never_ready_while_a_changed_file_is_unproven() {
    for with_proven_change in [false, true] {
        let repo = repo_with(&format!("{NO_PROOF}{CHECKS}"));
        repo.branch("feat/x");
        repo.write("README.md", "changed\n");
        if with_proven_change {
            repo.write("src/lib.rs", "fn b() {}\n");
        }
        repo.commit("readme");
        assert!(!repo.fz(&["run", "--stage", "merge"]).ok);
        let out = repo.fz(&[]);
        assert!(!out.stdout.contains("ready to merge"), "{}", out.stdout);
        assert!(
            out.last_line()
                .contains("takes away proof the base branch requires"),
            "{}",
            out.stdout
        );
    }
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
        out.stdout
            .contains("weakens no_proof: it newly says *.md needs no proof"),
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

// The proof is what CI ran, not what the change says about itself

#[test]
fn a_code_change_needs_no_goal_or_claims() {
    let repo = repo_with(&format!("{NO_PROOF}{CHECKS}"));
    repo.branch("feat/x");
    repo.write("src/lib.rs", "fn b() {}\n");
    repo.commit("change");
    let out = ci(&repo);
    assert!(out.ok, "{}", out.stdout);
    assert!(!out.stdout.contains("claim"), "{}", out.stdout);
}

// The agent finds out at its own final run

#[test]
fn the_final_run_of_the_work_reports_a_weakening_too() {
    let repo = branch_with_change();
    with_checks(&repo, &CHECKS.replace("test ! -e src/broken", "true"));
    let out = repo.fz(&["run", "--stage", "merge", "--reason", "done"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stdout.contains("weakens tests"), "{}", out.stdout);
    assert!(!out.stdout.contains("ready to merge"), "{}", out.stdout);
}

#[test]
fn the_status_comment_names_the_pull_requests_head_not_the_merge_commit() {
    let repo = branch_with_change();
    let head = repo.git(&["rev-parse", "HEAD"]).trim().to_string();
    let short = repo
        .git(&["rev-parse", "--short", "HEAD"])
        .trim()
        .to_string();
    // GitHub checks out a merge of the head into the base branch.
    repo.git(&["checkout", "-q", "--detach", "main"]);
    repo.write("src/other.rs", "fn c() {}\n");
    repo.commit("base moved on");
    repo.git(&["merge", "-q", "--no-ff", "--no-edit", "feat/x"]);
    let merge = repo
        .git(&["rev-parse", "--short", "HEAD"])
        .trim()
        .to_string();
    let event = repo.path(".runs/event.json");
    std::fs::write(
        &event,
        format!(r#"{{"pull_request": {{"head": {{"sha": "{head}"}}}}}}"#),
    )
    .unwrap();
    let path = repo.path(".runs/comment.md");
    let mut env = GITHUB.to_vec();
    env.push(("GITHUB_EVENT_PATH", event.to_str().unwrap()));
    let out = repo.fz_env(&["run", "--ci", "--comment", path.to_str().unwrap()], &env);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{}", out.stdout));
    assert!(
        text.contains(&format!("commit {short} (merged with main as {merge})")),
        "{text}"
    );
}
