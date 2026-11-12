//! Proving a commit for CI with `fearlezz run --ci`: what the change needs, fresh, with no
//! journal, against the rules of the base branch, so a change cannot weaken its own proof.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::github::FakeGitHub;
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
fn reading_findings_another_way_is_red() {
    let out = ci_after(|c| c.replace("read = \"jsonl\"", "read = \"sarif\""));
    assert_weakens(
        &out,
        "weakens lint: its findings are read another way (jsonl, now sarif)",
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

// Changed pictures on the pull request

/// Records two stills, `home` in the color `src/color.txt` names and `about` in white.
const RECORD: &str = r#"set -e
still() {
  ffmpeg -v error -nostdin -f lavfi -i "color=c=$2:s=64x48" -frames:v 1 "$FEARLEZZ_OUT/$1.png"
  echo "{\"kind\":\"still\",\"file\":\"$1.png\"}" >> "$FEARLEZZ_OUT/proof.jsonl"
}
still home "$(cat src/color.txt)"
still about white
"#;

const SCREENS: &str = r#"
[check.screens]
run = "sh record.sh"
stage = "look"
inputs = ["src/**", "record.sh"]
"#;

/// A branch whose `home` still turns from red to `color`.
fn screens_branch(color: &str) -> Repo {
    let repo = screens_main();
    repo.write("src/color.txt", color);
    repo.commit("color");
    repo
}

/// The stills on `main`, and a branch `feat/look` off it with nothing changed yet.
fn screens_main() -> Repo {
    let repo = Repo::new(
        SCREENS,
        &[
            (".gitignore", ".runs/\n"),
            ("record.sh", RECORD),
            ("src/color.txt", "red"),
            ("src/other.txt", "a"),
        ],
    );
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    repo.branch("feat/look");
    repo
}

const PICTURES_URL: &str = "https://github.com/owner/repo/raw/fearlezz-pictures/pr-7/42-1";

/// `fearlezz run --ci` with the comment written to a file and the pictures copied into a
/// folder served at `url`, as on a CI other than GitHub Actions.
fn ci_with_pictures(repo: &Repo, url: Option<&str>) -> (common::Output, String) {
    let comment = repo.path(".runs/comment.md");
    let dir = repo.path(".runs/pictures");
    let mut args = vec![
        "run",
        "--ci",
        "--comment",
        comment.to_str().unwrap(),
        "--pictures",
        dir.to_str().unwrap(),
    ];
    if let Some(url) = url {
        args.extend(["--pictures-url", url]);
    }
    let out = repo.fz_env(&args, &GITHUB);
    let text = std::fs::read_to_string(&comment)
        .unwrap_or_else(|_| panic!("{}\n{}", out.stdout, out.stderr));
    (out, text)
}

/// The JSON in the hidden line an approval checks.
fn waiting(text: &str) -> serde_json::Value {
    let line = text
        .lines()
        .find_map(|l| l.strip_prefix("<!-- fearlezz-pictures "))
        .unwrap_or_else(|| panic!("no hidden line in {text}"));
    serde_json::from_str(line.strip_suffix(" -->").unwrap()).unwrap()
}

#[test]
fn changed_pictures_wait_for_a_person_without_failing_the_proof() {
    let repo = screens_branch("blue");
    let (out, text) = ci_with_pictures(&repo, Some(PICTURES_URL));
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(text.contains("**Pictures wait**"), "{text}");
    let shown: Vec<&str> = text.lines().filter(|l| !l.starts_with("<!--")).collect();
    assert!(!shown.iter().any(|l| l.contains("Proven")), "{text}");
    assert!(text.contains("`/fearlezz approve "), "{text}");
    let head = repo.git(&["rev-parse", "HEAD"]);
    let w = waiting(&text);
    assert_eq!(w["head"], head.trim(), "{text}");
    assert_eq!(w["checks"], serde_json::json!(["screens"]), "{text}");
    assert!(
        w["approved"].as_str().unwrap().starts_with("**Proven**"),
        "{text}"
    );
}

#[test]
fn the_comment_shows_before_and_after_from_the_pictures_folder() {
    let repo = screens_branch("blue");
    let (out, text) = ci_with_pictures(&repo, Some(PICTURES_URL));
    assert!(out.ok, "{}", out.stdout);
    let sources: Vec<&str> = text
        .split("src=\"")
        .skip(1)
        .map(|s| s.split('"').next().unwrap())
        .collect();
    assert_eq!(sources.len(), 2, "before and after of home only: {text}");
    for src in sources {
        let rel = src
            .strip_prefix(&format!("{PICTURES_URL}/"))
            .unwrap_or_else(|| panic!("{src}"));
        assert!(
            repo.path(".runs/pictures").join(rel).is_file(),
            "{rel} is not in the pictures folder"
        );
    }
    assert!(text.contains("**home.png** changed"), "{text}");
    assert!(
        !text.contains("**about.png**"),
        "same pictures are not shown: {text}"
    );
}

#[test]
fn the_comment_marks_flaky_pictures() {
    let record = RECORD.replace("still about white", "still about \"$(cat src/about.txt)\"");
    let repo = Repo::new(
        &format!("{SCREENS}flaky = [\"about.png\"]\n"),
        &[
            (".gitignore", ".runs/\n"),
            ("record.sh", &record),
            ("src/color.txt", "red"),
            ("src/about.txt", "white"),
        ],
    );
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    repo.branch("feat/look");
    repo.write("src/color.txt", "blue");
    repo.write("src/about.txt", "black");
    repo.commit("colors");
    let (out, text) = ci_with_pictures(&repo, None);
    assert!(out.ok, "{}", out.stdout);
    assert!(text.contains("**home.png** changed:"), "{text}");
    assert!(
        text.contains("**about.png** changed (flaky, never blocks)"),
        "{text}"
    );
}

#[test]
fn without_a_pictures_url_the_comment_names_the_artifact() {
    let repo = screens_branch("blue");
    let (out, text) = ci_with_pictures(&repo, None);
    assert!(out.ok, "{}", out.stdout);
    assert!(!text.contains("<img"), "{text}");
    assert!(text.contains("**home.png** changed"), "{text}");
    assert!(text.contains("`fearlezz-pictures` artifact"), "{text}");
}

#[test]
fn the_fingerprint_follows_the_inputs_of_the_pictures() {
    let repo = screens_branch("blue");
    let fingerprint = |repo: &Repo| {
        let (out, text) = ci_with_pictures(repo, None);
        assert!(text.contains("fearlezz-pictures"), "{}", out.stdout);
        waiting(&text)["fingerprint"].clone()
    };
    let first = fingerprint(&repo);
    let again = fingerprint(&repo);
    assert_eq!(first, again);
    // Gray is far from blue; green would look the same as red to the comparison.
    repo.write("src/color.txt", "gray");
    repo.commit("gray");
    let other = fingerprint(&repo);
    assert_ne!(first, other);
}

#[test]
fn pictures_that_stay_the_same_leave_the_commit_proven() {
    let repo = screens_main();
    repo.write("src/other.txt", "b");
    repo.commit("other");
    let (out, text) = ci_with_pictures(&repo, Some(PICTURES_URL));
    assert!(out.ok, "{}", out.stdout);
    assert!(text.contains("**Proven**"), "{text}");
    assert!(!text.contains("fearlezz-pictures"), "{text}");
}

#[test]
fn a_failing_check_stays_not_proven_with_pictures_waiting() {
    let repo = screens_branch("blue");
    repo.write(
        "fearlezz.toml",
        &full(&format!(
            "{SCREENS}\n[check.unit]\nrun = \"false\"\nstage = \"working\"\ninputs = [\"src/**\"]\n"
        )),
    );
    repo.commit("unit");
    let (out, text) = ci_with_pictures(&repo, None);
    assert!(!out.ok, "{}", out.stdout);
    assert!(text.contains("**Not proven**"), "{text}");
    assert!(!text.contains("**Proven**:"), "{text}");
    assert!(text.contains("**home.png** changed"), "{text}");
}

#[test]
fn the_tools_fearlezz_needs_name_ffmpeg_only_for_pictures() {
    let repo = screens_branch("blue");
    let out = repo.fz(&["setup", "needs"]);
    assert!(out.ok, "{}", out.stderr);
    assert_eq!(out.stdout, "ffmpeg\n");
    let plain = branch_with_change();
    let out = plain.fz(&["setup", "needs"]);
    assert!(out.ok, "{}", out.stderr);
    assert_eq!(out.stdout, "");
}

#[test]
fn films_and_recordings_outside_the_look_stage_need_ffmpeg_too() {
    for field in [
        "films = \"films/*.webm\"",
        "record = { film = \"home\", seconds = 1 }",
    ] {
        let repo = Repo::new(
            &format!("[check.ui]\nrun = \"true\"\nstage = \"working\"\n{field}\n"),
            &[],
        );
        let out = repo.fz(&["setup", "needs"]);
        assert!(out.ok, "{}", out.stderr);
        assert_eq!(out.stdout, "ffmpeg\n", "{field}");
    }
}

// On GitHub Actions: what the event asks, against a fake GitHub API

const BRANCH: &str = "fearlezz-pictures";

/// A workflow's run of `fearlezz run --ci` for `event` (written to the event file), in `dir`,
/// talking to the fake API.
fn on_actions(
    gh: &FakeGitHub,
    dir: &std::path::Path,
    name: &str,
    event: &serde_json::Value,
    run: &str,
) -> common::Output {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("event.json");
    std::fs::write(&file, event.to_string()).unwrap();
    let temp_dir = temp.path().to_str().unwrap();
    common::fz_at_env(
        dir,
        &["run", "--ci"],
        &[
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_EVENT_NAME", name),
            ("GITHUB_EVENT_PATH", file.to_str().unwrap()),
            ("GITHUB_TOKEN", "token"),
            ("GITHUB_API_URL", &gh.url),
            ("GITHUB_SERVER_URL", "https://github.com"),
            ("GITHUB_REPOSITORY", "owner/repo"),
            ("GITHUB_RUN_ID", run),
            ("GITHUB_RUN_ATTEMPT", "1"),
            ("RUNNER_TEMP", temp_dir),
        ],
    )
}

/// The event of pull request 7 at `head`, opened from `from` (this repository or a fork).
fn pull_request(head: &str, from: &str) -> serde_json::Value {
    serde_json::json!({
        "action": "synchronize",
        "number": 7,
        "pull_request": {
            "number": 7,
            "head": { "sha": head, "repo": { "full_name": from } },
        },
        "repository": { "full_name": "owner/repo" },
    })
}

fn head_of(repo: &Repo) -> String {
    repo.git(&["rev-parse", "HEAD"]).trim().to_string()
}

/// The status comment Fearlezz wrote on pull request 7.
fn status_comment(gh: &FakeGitHub) -> common::github::Comment {
    let mine: Vec<_> = gh
        .state()
        .comments
        .iter()
        .filter(|c| c.pr == 7 && c.body.starts_with("<!-- fearlezz -->"))
        .cloned()
        .collect();
    assert_eq!(mine.len(), 1, "one status comment: {mine:?}");
    mine[0].clone()
}

/// A branch with changed pictures, proven on pull request 7 of the fake.
fn proven_on_actions() -> (FakeGitHub, Repo, String) {
    let gh = FakeGitHub::start();
    let repo = screens_branch("blue");
    let head = head_of(&repo);
    gh.state().pulls.insert(7, (head.clone(), "open".into()));
    let event = pull_request(&head, "owner/repo");
    let out = on_actions(&gh, &repo.root, "pull_request", &event, "42");
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    (gh, repo, head)
}

#[test]
fn on_a_pull_request_ci_publishes_the_pictures_comments_and_waits() {
    let (gh, _repo, head) = proven_on_actions();
    let files = gh.state().branch_files(BRANCH);
    assert_eq!(files.len(), 2, "before and after of home: {files:?}");
    assert!(
        files.iter().all(|f| f.starts_with("pr-7/42-1/")),
        "{files:?}"
    );
    let comment = status_comment(&gh);
    assert!(
        comment.body.contains("**Pictures wait**"),
        "{}",
        comment.body
    );
    for file in &files {
        let src = format!("https://github.com/owner/repo/raw/{BRANCH}/{file}");
        assert!(comment.body.contains(&src), "{src} in {}", comment.body);
    }
    let status = gh.state().status(&head, "fearlezz/pictures").unwrap();
    assert_eq!(status.state, "pending");
    assert_eq!(status.target, comment.url());
}

#[test]
fn a_new_run_replaces_the_comment_and_the_pictures() {
    let (gh, repo, head) = proven_on_actions();
    let event = pull_request(&head, "owner/repo");
    let out = on_actions(&gh, &repo.root, "pull_request", &event, "43");
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    status_comment(&gh);
    let files = gh.state().branch_files(BRANCH);
    assert!(
        files.iter().all(|f| f.starts_with("pr-7/43-1/")),
        "{files:?}"
    );
    assert_eq!(gh.state().branch_parents(BRANCH).len(), 1);
}

#[test]
fn pictures_that_stay_the_same_set_the_status_green() {
    let gh = FakeGitHub::start();
    let repo = screens_main();
    repo.write("src/other.txt", "b");
    repo.commit("other");
    let head = head_of(&repo);
    let event = pull_request(&head, "owner/repo");
    let out = on_actions(&gh, &repo.root, "pull_request", &event, "42");
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(status_comment(&gh).body.contains("**Proven**"));
    let status = gh.state().status(&head, "fearlezz/pictures").unwrap();
    assert_eq!(status.state, "success");
    assert_eq!(gh.state().branch_files(BRANCH), Vec::<String>::new());
}

#[test]
fn a_fork_keeps_its_pictures_in_the_artifact() {
    let gh = FakeGitHub::start();
    let repo = screens_branch("blue");
    let head = head_of(&repo);
    let event = pull_request(&head, "someone/repo");
    let out = on_actions(&gh, &repo.root, "pull_request", &event, "42");
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert_eq!(gh.state().branch_files(BRANCH), Vec::<String>::new());
    let comment = status_comment(&gh);
    assert!(!comment.body.contains("<img"), "{}", comment.body);
    assert!(
        comment.body.contains("`fearlezz-pictures` artifact"),
        "{}",
        comment.body
    );
    let status = gh.state().status(&head, "fearlezz/pictures").unwrap();
    assert_eq!(status.state, "pending");
}

/// The event of a comment on pull request 7 by `login`.
fn comment_event(gh: &FakeGitHub, login: &str, body: &str) -> serde_json::Value {
    let c = gh.state().add_comment(7, login, "User", body);
    serde_json::json!({
        "action": "created",
        "issue": { "number": 7, "pull_request": { "url": "https://api.github.com/x" } },
        "comment": { "id": c.id, "body": body, "html_url": c.url(),
                     "user": { "login": login, "type": "User" } },
        "repository": { "full_name": "owner/repo" },
    })
}

/// Answers a comment in a folder with no checkout, as the pictures workflow runs, and returns
/// the last comment on the pull request.
fn answer(gh: &FakeGitHub, event: &serde_json::Value) -> String {
    let empty = tempfile::tempdir().unwrap();
    let out = on_actions(gh, empty.path(), "issue_comment", event, "50");
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    gh.state().comments.last().unwrap().body.clone()
}

#[test]
fn a_person_with_write_access_approves_the_pictures_by_comment() {
    let (gh, _repo, head) = proven_on_actions();
    gh.state().permissions.insert("lead".into(), "write".into());
    let event = comment_event(&gh, "lead", "/fearlezz approve the blue home is right");
    let asked = event["comment"]["html_url"].as_str().unwrap().to_string();
    let reply = answer(&gh, &event);
    assert!(reply.contains("> the blue home is right"), "{reply}");
    assert!(reply.contains("<!-- fearlezz-approved "), "{reply}");
    let status = gh.state().status(&head, "fearlezz/pictures").unwrap();
    assert_eq!(status.state, "success");
    assert_eq!(status.target, asked);
    let comment = status_comment(&gh);
    assert!(comment.body.contains("**Proven**"), "{}", comment.body);
    assert!(comment.body.contains("@lead approved"), "{}", comment.body);
}

#[test]
fn approving_needs_write_access() {
    let (gh, _repo, head) = proven_on_actions();
    gh.state()
        .permissions
        .insert("visitor".into(), "read".into());
    let reply = answer(
        &gh,
        &comment_event(&gh, "visitor", "/fearlezz approve fine"),
    );
    assert!(reply.contains("needs write access"), "{reply}");
    let status = gh.state().status(&head, "fearlezz/pictures").unwrap();
    assert_eq!(status.state, "pending");
}

#[test]
fn approving_needs_words() {
    let (gh, _repo, head) = proven_on_actions();
    gh.state().permissions.insert("lead".into(), "admin".into());
    let reply = answer(&gh, &comment_event(&gh, "lead", "/fearlezz approve"));
    assert!(reply.contains("your own words"), "{reply}");
    let status = gh.state().status(&head, "fearlezz/pictures").unwrap();
    assert_eq!(status.state, "pending");
}

#[test]
fn an_approval_for_an_older_head_is_refused() {
    let (gh, _repo, head) = proven_on_actions();
    gh.state().permissions.insert("lead".into(), "write".into());
    gh.state().pulls.insert(7, ("f".repeat(40), "open".into()));
    let reply = answer(&gh, &comment_event(&gh, "lead", "/fearlezz approve ok"));
    assert!(
        reply.contains("nothing to approve for fffffff yet"),
        "{reply}"
    );
    let status = gh.state().status(&head, "fearlezz/pictures").unwrap();
    assert_eq!(status.state, "pending");
}

#[test]
fn other_comments_are_left_alone() {
    let (gh, _repo, _) = proven_on_actions();
    let before = gh.state().comments.len();
    answer(
        &gh,
        &comment_event(&gh, "lead", "/fearlezz approved? not yet"),
    );
    assert_eq!(gh.state().comments.len(), before + 1, "no reply");
}

/// The event of pull request 7 closing.
fn closed() -> serde_json::Value {
    let mut event = pull_request(&"a".repeat(40), "owner/repo");
    event["action"] = "closed".into();
    event
}

#[test]
fn closing_a_pull_request_removes_its_pictures_and_squashes() {
    let gh = FakeGitHub::start();
    gh.state()
        .put(BRANCH, &[("pr-7/1-1/a.png", "a"), ("pr-8/1-1/b.png", "b")]);
    gh.state().put(BRANCH, &[("pr-8/2-1/c.png", "c")]);
    let empty = tempfile::tempdir().unwrap();
    let out = on_actions(&gh, empty.path(), "pull_request", &closed(), "60");
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let files = gh.state().branch_files(BRANCH);
    assert_eq!(files, ["pr-8/1-1/b.png", "pr-8/2-1/c.png"]);
    assert!(gh.state().branch_parents(BRANCH).is_empty(), "one commit");
}

#[test]
fn closing_the_last_pull_request_with_pictures_removes_the_branch() {
    let gh = FakeGitHub::start();
    gh.state().put(BRANCH, &[("pr-7/1-1/a.png", "a")]);
    let empty = tempfile::tempdir().unwrap();
    let out = on_actions(&gh, empty.path(), "pull_request", &closed(), "60");
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(!gh.state().refs.contains_key(&format!("heads/{BRANCH}")));
}

#[test]
fn the_schedule_removes_the_pictures_of_closed_pull_requests() {
    let gh = FakeGitHub::start();
    gh.state()
        .put(BRANCH, &[("pr-7/1-1/a.png", "a"), ("pr-8/1-1/b.png", "b")]);
    gh.state().pulls.insert(7, ("a".repeat(40), "open".into()));
    gh.state()
        .pulls
        .insert(8, ("b".repeat(40), "closed".into()));
    let empty = tempfile::tempdir().unwrap();
    let event = serde_json::json!({ "schedule": "0 4 * * 1" });
    let out = on_actions(&gh, empty.path(), "schedule", &event, "70");
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert_eq!(gh.state().branch_files(BRANCH), ["pr-7/1-1/a.png"]);
    assert!(gh.state().branch_parents(BRANCH).is_empty(), "one commit");
}
