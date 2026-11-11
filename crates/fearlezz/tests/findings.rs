//! Findings: a check reports problems at file and line; only the lines a change touched count.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;

/// A linter that flags every line containing TODO (gate) or XXX (flag), anywhere in src.
const CONFIG: &str = r#"
[check.lint]
run = '''
grep -n -H -e TODO -e XXX src/*.txt | while IFS=: read -r file line text; do
  case "$text" in
    *TODO*) level=gate ;;
    *) level=flag ;;
  esac
  printf '{"file":"%s","line":%s,"level":"%s","rule":"no-markers","message":"marker left in","fix":"finish it or open an issue"}\n' "$file" "$line" "$level"
done > "$FEARLEZZ_OUT/findings.jsonl"
'''
stage = "working"
inputs = ["src/**"]
findings = "jsonl"
"#;

const OLD: &str = "one\nTODO from long ago\nthree\nfour\n";

fn feature() -> Repo {
    let repo = Repo::new(CONFIG, &[("src/a.txt", OLD)]);
    repo.branch("feat/x");
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    repo
}

#[test]
fn old_findings_do_not_block_new_work() {
    let repo = feature();
    repo.write("src/a.txt", &format!("{OLD}five\n"));
    let out = repo.fz(&["run", "lint"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("(1 more on lines this change did not touch)"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_gate_on_a_changed_line_fails_and_says_where() {
    let repo = feature();
    repo.write("src/a.txt", &format!("{OLD}TODO new one\n"));
    let out = repo.fz(&["run", "lint"]);
    assert!(!out.ok);
    assert!(
        out.stdout.contains("fail lint: blocking findings"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout
            .contains("gate src/a.txt:5  no-markers  marker left in"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("fix: finish it or open an issue"));
    let status = repo.fz(&[]);
    assert!(
        status.stdout.contains("findings lint:"),
        "{}",
        status.stdout
    );
}

#[test]
fn a_flag_passes_but_is_shown() {
    let repo = feature();
    repo.write("src/a.txt", &format!("{OLD}XXX look again\n"));
    let out = repo.fz(&["run", "lint"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("flag src/a.txt:5  no-markers"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_new_file_counts_as_changed_everywhere() {
    let repo = feature();
    repo.write("src/b.txt", "fine\nTODO in a new file\n");
    let out = repo.fz(&["run", "lint"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stdout.contains("gate src/b.txt:2"), "{}", out.stdout);
}

#[test]
fn everywhere_counts_old_findings_too() {
    let config = CONFIG.replace(
        "findings = \"jsonl\"",
        "findings = { read = \"jsonl\", on = \"everywhere\" }",
    );
    let repo = Repo::new(&config, &[("src/a.txt", OLD)]);
    repo.branch("feat/x");
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    repo.write("src/a.txt", &format!("{OLD}five\n"));
    let out = repo.fz(&["run", "lint"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stdout.contains("gate src/a.txt:2"), "{}", out.stdout);
}

#[test]
fn the_same_files_with_other_changed_lines_are_judged_again() {
    // main gains the TODO in a later commit; one branch starts after it, one before.
    let repo = Repo::new(CONFIG, &[("src/a.txt", "one\ntwo\nthree\nfour\n")]);
    let before = repo.git(&["rev-parse", "HEAD"]).trim().to_string();
    repo.write("src/a.txt", OLD);
    repo.commit("old todo");
    let same = format!("{OLD}five\n");

    repo.branch("feat/after");
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    repo.write("src/a.txt", &same);
    assert!(repo.fz(&["run", "lint"]).ok);
    repo.commit("five");

    // Identical file, but on this branch the TODO line is new: no reuse of the pass above.
    repo.git(&["switch", "-q", "-c", "feat/before", &before]);
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    repo.write("src/a.txt", &same);
    let out = repo.fz(&["run", "lint"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stdout.contains("gate src/a.txt:2"), "{}", out.stdout);
}

#[test]
fn a_tool_that_crashes_without_findings_fails() {
    let config = CONFIG.replace("grep -n", "exit 3; grep -n");
    let repo = Repo::new(&config, &[("src/a.txt", OLD)]);
    repo.branch("feat/x");
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    repo.write("src/a.txt", &format!("{OLD}five\n"));
    let out = repo.fz(&["run", "lint"]);
    assert!(!out.ok);
    assert!(out.stdout.contains("fail lint (exit 3)"), "{}", out.stdout);
}

#[test]
fn a_ci_checkout_with_only_the_remote_base_still_judges_the_change() {
    let repo = feature();
    // A pull request checkout: the base exists only as origin/main.
    repo.git(&["update-ref", "refs/remotes/origin/main", "main"]);
    repo.git(&["branch", "-D", "main"]);
    repo.write("src/a.txt", &format!("{OLD}TODO new one\n"));
    repo.commit("new marker");
    let out = repo.fz(&["run", "lint"]);
    assert!(!out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("gate src/a.txt:5"), "{}", out.stdout);
}

#[test]
fn a_missing_base_is_an_error_not_an_empty_change() {
    let repo = feature();
    repo.git(&["branch", "-D", "main"]);
    let out = repo.fz(&[]);
    assert!(!out.ok);
    assert!(
        out.stderr.contains("the base branch main is not here"),
        "{}",
        out.stderr
    );
}

#[test]
fn the_developers_diff_settings_do_not_hide_changed_lines() {
    let repo = feature();
    repo.git(&["config", "diff.noprefix", "true"]);
    repo.write("src/a.txt", &format!("{OLD}TODO new one\n"));
    let out = repo.fz(&["run", "lint"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stdout.contains("gate src/a.txt:5"), "{}", out.stdout);
}

#[test]
fn a_finding_named_from_another_folder_blocks_instead_of_hiding() {
    // The linter runs inside src/ and names files from there.
    let config = CONFIG
        .replace("grep -n -H", "cd src && grep -n -H")
        .replace("src/*.txt", "*.txt");
    let repo = Repo::new(&config, &[("src/a.txt", OLD)]);
    repo.branch("feat/x");
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    repo.write("src/a.txt", &format!("{OLD}XXX new\n"));
    let out = repo.fz(&["run", "lint"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("findings name a.txt, which are not files from the repository root"),
        "{}",
        out.stdout
    );
}
