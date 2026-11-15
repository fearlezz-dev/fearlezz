//! Proving a commit for CI with `fearlezz run --ci`: every check, fresh, with no
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
fn ci_runs_every_check_fresh_and_writes_no_journal() {
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
fn ci_runs_every_check_whatever_its_inputs() {
    let repo = branch_with_change();
    let out = ci(&repo);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert_eq!(runs(&repo.path(".runs/docs")), 1, "{}", out.stdout);
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

// The job summary

/// The variables GitHub Actions sets, so the summary can link the run.
const GITHUB: [(&str, &str); 3] = [
    ("GITHUB_SERVER_URL", "https://github.com"),
    ("GITHUB_REPOSITORY", "owner/repo"),
    ("GITHUB_RUN_ID", "42"),
];

/// `fearlezz run --ci` with the variables GitHub Actions sets and `env`, and the job summary
/// it wrote.
fn summary_with(repo: &Repo, env: &[(&str, &str)]) -> (common::Output, String) {
    let path = repo.path(".runs/summary.md");
    // The summary is appended to, as on GitHub; each run here reads its own.
    if path.exists() {
        std::fs::remove_file(&path).unwrap();
    }
    let mut all = GITHUB.to_vec();
    all.push(("GITHUB_STEP_SUMMARY", path.to_str().unwrap()));
    all.extend_from_slice(env);
    let out = repo.fz_env(&["run", "--ci"], &all);
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{}\n{}", out.stdout, out.stderr));
    (out, text)
}

fn summary(repo: &Repo) -> (common::Output, String) {
    summary_with(repo, &[])
}

#[test]
fn the_job_summary_says_proven_and_lists_the_checks() {
    let repo = branch_with_change();
    let (out, text) = summary(&repo);
    assert!(out.ok, "{}", out.stdout);
    assert!(text.contains("**Proven**"), "{text}");
    assert!(text.contains("| tests | passed |"), "{text}");
    assert!(!text.contains("| what |"), "{text}");
}

#[test]
fn the_job_summary_names_each_blocker_and_what_to_do() {
    let repo = branch_with_change();
    repo.write("src/broken", "x\n");
    repo.commit("break");
    with_checks(
        &repo,
        &CHECKS.replace("[check.e2e]\n", "[check.e2e]\noff = true\n"),
    );
    let (out, text) = summary(&repo);
    assert!(!out.ok, "{}", out.stdout);
    assert!(text.contains("**Not proven**"), "{text}");
    assert!(text.contains("| what | why | what to do |"), "{text}");
    assert!(text.contains("| check `tests` failed |"), "{text}");
    assert!(text.contains("| check `e2e` is weaker |"), "{text}");
    assert!(text.contains("it is gone"), "{text}");
}

#[test]
fn the_status_comment_is_gone() {
    let repo = branch_with_change();
    assert!(!repo.fz(&["run", "--ci", "--comment", "x.md"]).ok);
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
fn narrowing_or_dropping_the_inputs_weakens_nothing() {
    for inputs in ["inputs = [\"src/other/**\"]\n", ""] {
        let repo = branch_with_change();
        with_checks(
            &repo,
            &CHECKS.replace(
                "inputs = [\"src/**\"]\n\n[check.e2e]",
                &format!("{inputs}\n[check.e2e]"),
            ),
        );
        let out = ci(&repo);
        assert!(out.ok, "{inputs}: {}", out.stdout);
        assert!(!out.stdout.contains("weakens"), "{inputs}: {}", out.stdout);
    }
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
    ci_between(LINTED, &edit(LINTED))
}

/// What CI says about a branch whose `LINTED` part of the config goes from `base` to `head`.
fn ci_between(base: &str, head: &str) -> common::Output {
    let repo = repo_with(&format!("{NO_PROOF}{CHECKS}{base}"));
    repo.fz(&["setup", "sync"]);
    repo.commit("sync");
    repo.branch("feat/x");
    repo.write("src/lib.rs", "fn b() {}\n");
    repo.commit("change");
    repo.write("fearlezz.toml", &full(&format!("{NO_PROOF}{CHECKS}{head}")));
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
fn dropping_a_flaky_negation_is_red_and_adding_one_is_fine() {
    let flaky = |list: &str| {
        LINTED.replace(
            "env = [\"HOME\"]",
            &format!("env = [\"HOME\"]\nflaky = {list}"),
        )
    };
    let out = ci_between(&flaky(r#"["*", "!home"]"#), &flaky(r#"["*"]"#));
    assert_weakens(&out, "weakens lint: it calls more pictures flaky (-!home)");

    for (base, head) in [
        (r#"["*"]"#, r#"["*", "!home"]"#),
        (r#"["home", "about"]"#, r#"["home", "!home/x", "about"]"#),
        (r#"["home", "about"]"#, r#"["about"]"#),
    ] {
        let out = ci_between(&flaky(base), &flaky(head));
        assert!(out.ok, "{base} to {head}: {}\n{}", out.stdout, out.stderr);
        assert!(
            !out.stdout.contains("flaky"),
            "{base} to {head}: {}",
            out.stdout
        );
    }
}

#[test]
fn reordering_flaky_so_a_negation_no_longer_holds_is_red() {
    let flaky = |list: &str| {
        LINTED.replace(
            "env = [\"HOME\"]",
            &format!("env = [\"HOME\"]\nflaky = {list}"),
        )
    };
    let out = ci_between(&flaky(r#"["*", "!home"]"#), &flaky(r#"["!home", "*"]"#));
    assert_weakens(
        &out,
        "weakens lint: it calls more pictures flaky (reordered)",
    );
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
fn changing_the_env_inputs_weakens_nothing() {
    for inputs in ["inputs = []", "inputs = [\"env.nix\", \"env.lock\"]"] {
        let out = ci_after_tuning(|c| c.replace("inputs = [\"env.nix\"]", inputs));
        assert!(out.ok, "{inputs}: {}", out.stdout);
        assert!(!out.stdout.contains("weakens"), "{inputs}: {}", out.stdout);
    }
}

// Proof is about behaviour, not files

#[test]
fn a_changed_file_no_check_lists_in_its_inputs_blocks_nothing() {
    let repo = repo_with(&format!("{NO_PROOF}{CHECKS}"));
    repo.branch("feat/x");
    repo.write("README.md", "changed\n");
    repo.commit("readme");
    let out = ci(&repo);
    assert!(out.ok, "{}", out.stdout);
    assert!(!out.stdout.contains("unproven"), "{}", out.stdout);
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
fn dropping_a_negation_from_no_proof_weakens_it_and_adding_one_does_not() {
    let narrowed = "no_proof = [\"docs/**\", \"!docs/api/**\"]\n";
    let repo = repo_with(&format!("{narrowed}{CHECKS}"));
    repo.branch("feat/x");
    repo.write("fearlezz.toml", &full(&format!("{NO_PROOF}{CHECKS}")));
    repo.commit("drop the negation");
    let out = ci(&repo);
    assert!(
        out.stdout.contains(
            "weakens no_proof: it no longer takes docs/api/** back from needing no proof"
        ),
        "{}",
        out.stdout
    );

    let repo = repo_with(&format!("{NO_PROOF}{CHECKS}"));
    repo.branch("feat/x");
    repo.write("fearlezz.toml", &full(&format!("{narrowed}{CHECKS}")));
    repo.commit("add a negation");
    let out = ci(&repo);
    assert!(!out.stdout.contains("weakens no_proof"), "{}", out.stdout);
}

#[test]
fn reordering_no_proof_so_a_negation_no_longer_holds_weakens_it() {
    let narrowed = "no_proof = [\"docs/**\", \"!docs/api/**\"]\n";
    let reordered = "no_proof = [\"!docs/api/**\", \"docs/**\"]\n";
    let repo = repo_with(&format!("{narrowed}{CHECKS}"));
    repo.branch("feat/x");
    repo.write("fearlezz.toml", &full(&format!("{reordered}{CHECKS}")));
    repo.commit("reorder");
    let out = ci(&repo);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("weakens no_proof: it reorders its globs so more files need no proof"),
        "{}",
        out.stdout
    );

    // A `!` glob put in anywhere only narrows, and so does dropping a positive glob.
    for (base, head) in [
        (
            r#"["docs/**", "*.md"]"#,
            r#"["docs/**", "!docs/api/**", "*.md"]"#,
        ),
        (r#"["docs/**", "*.md"]"#, r#"["*.md", "!docs/api/**"]"#),
        (
            r#"["!a/**", "docs/**", "*.md"]"#,
            r#"["!a/**", "*.md", "!b/**", "docs/**"]"#,
        ),
    ] {
        let repo = repo_with(&format!("no_proof = {base}\n{CHECKS}"));
        repo.branch("feat/x");
        repo.write(
            "fearlezz.toml",
            &full(&format!("no_proof = {head}\n{CHECKS}")),
        );
        repo.commit("narrow");
        let out = ci(&repo);
        assert!(
            !out.stdout.contains("weakens no_proof"),
            "{base} to {head}: {}",
            out.stdout
        );
    }

    // Positive globs side by side mean the same in any order.
    let both = "no_proof = [\"docs/**\", \"*.md\"]\n";
    let swapped = "no_proof = [\"*.md\", \"docs/**\"]\n";
    let repo = repo_with(&format!("{both}{CHECKS}"));
    repo.branch("feat/x");
    repo.write("fearlezz.toml", &full(&format!("{swapped}{CHECKS}")));
    repo.commit("swap");
    let out = ci(&repo);
    assert!(!out.stdout.contains("weakens no_proof"), "{}", out.stdout);
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
fn the_job_summary_names_the_pull_requests_head_not_the_merge_commit() {
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
    let (_, text) = summary_with(&repo, &[("GITHUB_EVENT_PATH", event.to_str().unwrap())]);
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

#[test]
fn changed_pictures_are_listed_and_neither_block_nor_wait() {
    let repo = screens_branch("blue");
    let (out, text) = summary(&repo);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(text.contains("**Proven**"), "{text}");
    assert!(text.contains("#### Changed pictures"), "{text}");
    assert!(
        text.contains("A person reviews them on the pull request."),
        "{text}"
    );
    assert!(!text.contains("/fearlezz"), "{text}");
    assert!(!text.contains("<!--"), "{text}");
    assert!(
        out.stdout.contains("look screens: 1 changed, 1 same"),
        "{}",
        out.stdout
    );
}

#[test]
fn the_job_summary_lists_the_changed_pictures() {
    let repo = screens_branch("blue");
    let (out, text) = summary(&repo);
    assert!(out.ok, "{}", out.stdout);
    assert!(!text.contains("<img"), "nothing serves them: {text}");
    assert!(text.contains("**home.png** changed"), "{text}");
    assert!(!text.contains("**about.png**"), "{text}");
}

#[test]
fn the_job_summary_marks_flaky_pictures() {
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
    let (out, text) = summary(&repo);
    assert!(out.ok, "{}", out.stdout);
    assert!(text.contains("**home.png** changed:"), "{text}");
    assert!(text.contains("**about.png** changed (flaky)"), "{text}");
}

#[test]
fn markdown_in_names_cannot_break_the_tables() {
    let repo = branch_with_change();
    with_checks(
        &repo,
        &CHECKS.replace(
            "[check.tests]\nrun = \"",
            "[check.tests]\nrun = \"echo '<b>|x' >/dev/null; ",
        ),
    );
    // A new command weakens the check, which is beside the point here.
    let (_, text) = summary(&repo);
    assert!(text.contains("| `echo '<b>\\|x' >/dev/null; "), "{text}");
}

#[test]
fn pictures_that_stay_the_same_leave_the_commit_proven() {
    let repo = screens_main();
    repo.write("src/other.txt", "b");
    repo.commit("other");
    let (out, text) = summary(&repo);
    assert!(out.ok, "{}", out.stdout);
    assert!(text.contains("**Proven**"), "{text}");
    assert!(!text.contains("Changed pictures"), "{text}");
}

#[test]
fn a_failing_check_stays_not_proven_with_changed_pictures() {
    let repo = screens_branch("blue");
    repo.write(
        "fearlezz.toml",
        &full(&format!(
            "{SCREENS}\n[check.unit]\nrun = \"false\"\nstage = \"working\"\ninputs = [\"src/**\"]\n"
        )),
    );
    repo.commit("unit");
    let (out, text) = summary(&repo);
    assert!(!out.ok, "{}", out.stdout);
    assert!(text.contains("**Not proven**"), "{text}");
    assert!(!text.contains("**Proven**:"), "{text}");
    assert!(text.contains("**home.png** changed"), "{text}");
}

#[test]
fn the_hidden_setup_needs_command_is_gone() {
    let repo = screens_branch("blue");
    assert!(!repo.fz(&["setup", "needs"]).ok);
    assert!(!repo.fz(&["run", "--ci", "--pictures", "x"]).ok);
}

// On GitHub Actions: CI proves and writes nothing to GitHub

/// A workflow's run of `fearlezz run --ci` for `event` (written to the event file), in `dir`,
/// with a workflow token and the fake API, and the job summary it wrote.
fn on_actions(
    gh: &FakeGitHub,
    dir: &std::path::Path,
    name: &str,
    event: &serde_json::Value,
) -> (common::Output, String) {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("event.json");
    std::fs::write(&file, event.to_string()).unwrap();
    let summary = temp.path().join("summary.md");
    let temp_dir = temp.path().to_str().unwrap();
    let graphql = format!("{}/graphql", gh.url);
    let out = common::fz_at_env(
        dir,
        &["run", "--ci"],
        &[
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_EVENT_NAME", name),
            ("GITHUB_EVENT_PATH", file.to_str().unwrap()),
            ("GITHUB_TOKEN", "token"),
            ("GITHUB_API_URL", &gh.url),
            ("GITHUB_GRAPHQL_URL", &graphql),
            ("GITHUB_SERVER_URL", "https://github.com"),
            ("GITHUB_REPOSITORY", "owner/repo"),
            ("GITHUB_RUN_ID", "42"),
            ("GITHUB_RUN_ATTEMPT", "1"),
            ("GITHUB_STEP_SUMMARY", summary.to_str().unwrap()),
            ("RUNNER_TEMP", temp_dir),
        ],
    );
    let text = std::fs::read_to_string(&summary).unwrap_or_default();
    (out, text)
}

/// The event of pull request 7 at `head`, opened from `from` (this repository or a fork).
fn pull_request(head: &str, from: &str) -> serde_json::Value {
    serde_json::json!({
        "action": "synchronize",
        "number": 7,
        "pull_request": {
            "number": 7,
            "title": "feat: a change",
            "head": { "sha": head, "repo": { "full_name": from } },
        },
        "repository": { "full_name": "owner/repo" },
    })
}

fn head_of(repo: &Repo) -> String {
    repo.git(&["rev-parse", "HEAD"]).trim().to_string()
}

#[test]
fn on_a_pull_request_ci_proves_and_writes_nothing_to_github() {
    let gh = FakeGitHub::start();
    let repo = screens_branch("blue");
    let event = pull_request(&head_of(&repo), "owner/repo");
    let (out, text) = on_actions(&gh, &repo.root, "pull_request", &event);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(text.contains("**Proven**"), "{text}");
    assert!(text.contains("**home.png** changed"), "{text}");
    assert!(gh.writes().is_empty(), "{:?}", gh.writes());
}

#[test]
fn a_weakening_fails_a_pull_request_and_writes_nothing_to_github() {
    for from in ["owner/repo", "someone/repo"] {
        let gh = FakeGitHub::start();
        let repo = branch_with_change();
        with_checks(&repo, &CHECKS.replace("test ! -e src/broken", "true"));
        let event = pull_request(&head_of(&repo), from);
        let (out, text) = on_actions(&gh, &repo.root, "pull_request", &event);
        assert!(!out.ok, "{from}: {}\n{}", out.stdout, out.stderr);
        assert!(
            out.stdout
                .contains("weakens tests: it runs another command"),
            "{from}: {}",
            out.stdout
        );
        assert!(text.contains("**Not proven**"), "{from}: {text}");
        assert!(text.contains("check `tests` is weaker"), "{from}: {text}");
        assert!(gh.writes().is_empty(), "{from}: {:?}", gh.writes());
    }
}

// A weakened proof fails everywhere

#[test]
fn a_weakening_still_fails_a_ci_run_outside_a_pull_request() {
    let repo = branch_with_change();
    with_checks(&repo, &CHECKS.replace("test ! -e src/broken", "true"));
    let (out, text) = summary(&repo);
    assert!(!out.ok, "{}", out.stdout);
    assert!(text.contains("**Not proven**"), "{text}");
    assert!(text.contains("merges past the required check"), "{text}");
}

// ffmpeg on the runner

/// A push to `main` proven on the fake, with no pull request.
fn pushed(repo: &Repo) -> common::Output {
    let gh = FakeGitHub::start();
    let event = serde_json::json!({ "ref": "refs/heads/main" });
    let (out, _) = on_actions(&gh, &repo.root, "push", &event);
    assert!(gh.writes().is_empty(), "{:?}", gh.writes());
    out
}

/// In CI the pull request title decides whether a change is a fix, so the proof runs again
/// when the title changes.
#[test]
fn the_proof_workflows_run_again_when_the_title_changes() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let types = "pull_request:\n    types: [opened, synchronize, reopened, edited]";
    let own = std::fs::read_to_string(root.join(".github/workflows/proof.yml")).unwrap();
    assert!(own.contains(&format!("  {types}")), "{own}");
    let readme = std::fs::read_to_string(root.join("README.md")).unwrap();
    let shown = readme
        .split("# .github/workflows/proof.yml")
        .nth(1)
        .and_then(|rest| rest.split("```").next())
        .unwrap();
    assert!(shown.contains(&format!("  {types}")), "{shown}");
    // A job skipped by a condition counts as passed for a required check, so an edit never
    // skips the proof: a description edit must not unblock a red pull request.
    assert!(!own.contains("changes.title"), "{own}");
    assert!(!shown.contains("changes.title"), "{shown}");
}

#[test]
fn checks_that_record_pictures_get_ffmpeg_on_the_runner() {
    for field in [
        "films = \"films/*.webm\"",
        "record = { film = \"home\", seconds = 1 }",
    ] {
        let repo = Repo::new(
            &format!("[check.ui]\nrun = \"true\"\nstage = \"working\"\n{field}\n"),
            &[],
        );
        let out = pushed(&repo);
        assert!(
            out.stdout.contains("ffmpeg for the pictures"),
            "{field}: {}",
            out.stdout
        );
    }
    let plain = Repo::new("[check.unit]\nrun = \"true\"\nstage = \"working\"\n", &[]);
    let out = pushed(&plain);
    assert!(!out.stdout.contains("ffmpeg"), "{}", out.stdout);
}
