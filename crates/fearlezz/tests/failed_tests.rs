//! Failed and flaky tests: a test check names each test that failed, from its JUnit reports or
//! a `failed` pattern beside its `count`, in the CLI, the job summary and the proof bundle. A
//! runner that reruns a failed test says so in its report (`<flakyFailure>`, `<flakyError>`); a
//! test that passed only on a rerun fails the check like a failure, unless the check's `flaky_tests`
//! list names it, which shows it apart and weakens the floor.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;
use serde_json::Value;

/// A runner that prints what `cargo test --quiet` prints, failing each test named in
/// `src/failing.txt`.
const CARGO: &str = r#"
[check.tests]
run = '''
n=0
while read -r name; do
  [ -n "$name" ] || continue
  printf -- '---- %s stdout ----\n\nthread panicked\n\n' "$name"
  n=$((n + 1))
done < src/failing.txt
echo 'failures:'
sed 's/^/    /' src/failing.txt
echo "test result: ok. 3 passed; $n failed; 0 ignored; 0 measured; 0 filtered out"
test "$n" = 0
'''
stage = "working"
inputs = ["src/**"]
tests = { count = 'test result: \w+\. (\d+) passed; (\d+) failed', failed = '(?m)^---- (\S+) stdout ----' }
"#;

/// A runner that writes a JUnit report: one test case per line of `src/cases.txt`, as
/// `<name> <how>` where how is `pass`, `fail`, `flaky` (failed, then passed on a rerun) or
/// `rerun` (failed, and failed again on a rerun). It exits 0 whatever the cases say, as a
/// runner whose retries passed does.
const JUNIT: &str = r#"
[check.tests]
run = '''
{
  echo '<testsuites><testsuite name="all">'
  while read -r name how; do
    case "$how" in
      pass) echo "<testcase name=\"$name\"/>" ;;
      fail) echo "<testcase name=\"$name\"><failure message=\"no\"/></testcase>" ;;
      flaky) echo "<testcase name=\"$name\"><flakyFailure message=\"timeout\"/></testcase>" ;;
      rerun) echo "<testcase name=\"$name\"><failure/><rerunFailure/></testcase>" ;;
    esac
  done < src/cases.txt
  echo '</testsuite></testsuites>'
} > "$FEARLEZZ_OUT/junit.xml"
'''
stage = "working"
inputs = ["src/**"]
tests = "junit"
"#;

fn repo(config: &str, files: &[(&str, &str)]) -> Repo {
    let mut all = vec![(".gitignore", ".runs/\n")];
    all.extend_from_slice(files);
    let repo = Repo::new(config, &all);
    repo.branch("feat/x");
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    repo
}

/// `fearlezz run --ci --proof` with the job summary, and what it printed, the summary and the
/// tests check's entry in `proof.json`.
fn prove(repo: &Repo) -> (common::Output, String, Value) {
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    let dir = repo.path(".runs/proof");
    let summary = repo.path(".runs/summary.md");
    let out = repo.fz_env(
        &["run", "--ci", "--proof", dir.to_str().unwrap()],
        &[("GITHUB_STEP_SUMMARY", summary.to_str().unwrap())],
    );
    let text = std::fs::read_to_string(dir.join("proof.json"))
        .unwrap_or_else(|_| panic!("no proof.json\n{}\n{}", out.stdout, out.stderr));
    let proof: Value = serde_json::from_str(&text).unwrap();
    let check = proof["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == "tests")
        .unwrap_or_else(|| panic!("no tests check\n{text}"))
        .clone();
    let summary = std::fs::read_to_string(&summary).unwrap_or_default();
    (out, summary, check)
}

/// The cases of a check's proof entry, as (name, failed, flaky).
fn cases(check: &Value) -> Vec<(String, bool, bool)> {
    check["cases"]
        .as_array()
        .map(|cases| {
            cases
                .iter()
                .map(|c| {
                    (
                        c["name"].as_str().unwrap().to_string(),
                        c["failed"].as_bool().unwrap(),
                        c["flaky"].as_bool().unwrap_or(false),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn the_cli_names_the_tests_a_failed_pattern_finds() {
    let repo = repo(
        CARGO,
        &[("src/failing.txt", "tests::two\ncore::sub::inner\n")],
    );
    let out = repo.fz(&["run", "tests"]);
    assert!(!out.ok, "{}", out.stdout);
    let lines = out.lines();
    assert!(lines.contains(&"  failed: tests::two"), "{}", out.stdout);
    assert!(
        lines.contains(&"  failed: core::sub::inner"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_test_named_twice_is_named_once() {
    let repo = repo(
        &CARGO.replace(
            "(?m)^---- (\\S+) stdout ----",
            "(?m)^(?:---- |    )(\\S+)(?: stdout ----)?$",
        ),
        &[("src/failing.txt", "tests::two\n")],
    );
    let out = repo.fz(&["run", "tests"]);
    let named = out
        .lines()
        .iter()
        .filter(|l| l.contains("failed: tests::two"))
        .count();
    assert_eq!(named, 1, "{}", out.stdout);
}

#[test]
fn many_failures_are_cut_short_with_how_many_more() {
    let names: String = (0..15).map(|i| format!("t{i}\n")).collect();
    let repo = repo(CARGO, &[("src/failing.txt", &names)]);
    let out = repo.fz(&["run", "tests"]);
    let named = out
        .lines()
        .iter()
        .filter(|l| l.starts_with("  failed: "))
        .count();
    assert_eq!(named, 10, "{}", out.stdout);
    assert!(out.stdout.contains("  and 5 more failed"), "{}", out.stdout);
}

#[test]
fn ci_names_the_failed_tests_in_the_summary_and_the_proof() {
    let repo = repo(CARGO, &[("src/failing.txt", "tests::two\n")]);
    let (out, summary, check) = prove(&repo);
    assert!(!out.ok, "{}", out.stdout);
    assert!(summary.contains("`tests::two` failed"), "{summary}");
    assert_eq!(cases(&check), [("tests::two".to_string(), true, false)]);
}

#[test]
fn a_failed_junit_case_is_named_and_fails_the_check_though_the_runner_exits_0() {
    let repo = repo(JUNIT, &[("src/cases.txt", "adds pass\nlogs_in fail\n")]);
    let out = repo.fz(&["run", "tests"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.lines().contains(&"  failed: logs_in"), "{}", out.stdout);
    assert!(!out.stdout.contains("failed: adds"), "{}", out.stdout);
}

#[test]
fn a_test_that_fails_its_rerun_too_is_a_failure_not_flaky() {
    let repo = repo(JUNIT, &[("src/cases.txt", "adds pass\nlogs_in rerun\n")]);
    let (out, _, check) = prove(&repo);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.lines().contains(&"  failed: logs_in"), "{}", out.stdout);
    assert!(
        cases(&check).contains(&("logs_in".to_string(), true, false)),
        "{check}"
    );
}

#[test]
fn a_test_that_passes_only_on_a_rerun_fails_the_check_and_says_how_to_list_it() {
    let repo = repo(JUNIT, &[("src/cases.txt", "adds pass\nlogs_in flaky\n")]);
    let (out, summary, check) = prove(&repo);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.lines()
            .contains(&"  flaky: logs_in (failed, then passed on a rerun)"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("flaky_tests = [\"logs_in\"]"),
        "{}",
        out.stdout
    );
    assert!(
        summary.contains("`logs_in` passed only on a rerun"),
        "{summary}"
    );
    assert!(
        cases(&check).contains(&("logs_in".to_string(), true, true)),
        "{check}"
    );
}

#[test]
fn a_flaky_test_the_check_lists_passes_and_shows_apart() {
    let config = JUNIT.replace(
        "tests = \"junit\"",
        "tests = \"junit\"\nflaky_tests = [\"logs_*\"]",
    );
    let repo = repo(&config, &[("src/cases.txt", "adds pass\nlogs_in flaky\n")]);
    let (out, summary, check) = prove(&repo);
    assert!(out.ok, "{}", out.stdout);
    assert!(
        out.lines()
            .contains(&"  flaky: logs_in (failed, then passed on a rerun; listed in flaky_tests)"),
        "{}",
        out.stdout
    );
    assert!(
        summary.contains("`logs_in` passed only on a rerun"),
        "{summary}"
    );
    assert!(
        cases(&check).contains(&("logs_in".to_string(), false, true)),
        "{check}"
    );
}

#[test]
fn listing_a_flaky_test_weakens_the_floor() {
    let repo = repo(JUNIT, &[("src/cases.txt", "adds pass\nlogs_in flaky\n")]);
    repo.write(
        "fearlezz.toml",
        &std::fs::read_to_string(repo.path("fearlezz.toml"))
            .unwrap()
            .replace(
                "tests = \"junit\"",
                "tests = \"junit\"\nflaky_tests = [\"logs_in\"]",
            ),
    );
    repo.commit("list the flaky test");
    let out = repo.fz(&["run", "--ci"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("weakens tests: it calls more tests flaky (+logs_in)"),
        "{}",
        out.stdout
    );
}

#[test]
fn removing_a_test_from_flaky_judges_it_again() {
    let config = JUNIT.replace(
        "tests = \"junit\"",
        "tests = \"junit\"\nflaky_tests = [\"logs_in\"]",
    );
    let repo = repo(&config, &[("src/cases.txt", "logs_in flaky\n")]);
    assert!(repo.fz(&["run", "tests"]).ok);
    repo.write(
        "fearlezz.toml",
        &std::fs::read_to_string(repo.path("fearlezz.toml"))
            .unwrap()
            .replace("flaky_tests = [\"logs_in\"]\n", ""),
    );
    let out = repo.fz(&["run", "tests"]);
    assert!(
        !out.ok,
        "a pass judged with the old list was reused\n{}",
        out.stdout
    );
}

#[test]
fn the_rust_piece_names_the_tests_cargo_says_failed() {
    let repo = repo(
        r#"use = ["rust"]

[check.tests]
run = "cat cargo.txt; exit 101"
"#,
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
            ),
            (
                "cargo.txt",
                "running 2 tests\ntests::two --- FAILED\n.\nfailures:\n\n---- tests::two stdout ----\n\n\
             thread 'tests::two' (1805252) panicked at src/lib.rs:3:47:\nassertion failed\n\n\n\
             failures:\n    tests::two\n\ntest result: FAILED. 1 passed; 1 failed; 0 ignored; \
             0 measured; 0 filtered out; finished in 0.00s\n\nrunning 1 test\nfailures:\n\n\
             ---- src/lib.rs - a (line 3) stdout ----\nboom\n\n",
            ),
        ],
    );
    let out = repo.fz(&["run", "tests"]);
    assert!(!out.ok, "{}", out.stdout);
    let named = out
        .lines()
        .iter()
        .filter(|l| l.starts_with("  failed: "))
        .count();
    assert_eq!(named, 2, "{}", out.stdout);
    assert!(
        out.lines().contains(&"  failed: src/lib.rs - a (line 3)"),
        "{}",
        out.stdout
    );
    assert!(
        out.lines().contains(&"  failed: tests::two"),
        "{}",
        out.stdout
    );
}

#[test]
fn the_go_piece_names_the_tests_and_subtests_go_says_failed() {
    let repo = repo(
        r#"use = ["go"]

[check.go-test]
run = "cat go.txt; exit 1"
"#,
        &[(
            "go.txt",
            "=== RUN   TestOk\n--- PASS: TestOk (0.00s)\n=== RUN   TestBad\n=== RUN   TestBad/sub_one\n\
             \x20   a_test.go:4: x\n--- FAIL: TestBad (0.00s)\n    --- FAIL: TestBad/sub_one (0.00s)\n\
             \x20   --- PASS: TestBad/fine (0.00s)\nFAIL\nFAIL\tgt\t0.407s\nFAIL\n",
        )],
    );
    let out = repo.fz(&["run", "go-test"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.lines().contains(&"  failed: TestBad"), "{}", out.stdout);
    assert!(
        out.lines().contains(&"  failed: TestBad/sub_one"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("failed: TestOk"), "{}", out.stdout);
}

#[test]
fn a_failed_pattern_without_a_count_is_an_error() {
    let repo = repo(
        &CARGO.replace(
            "tests = { count = 'test result: \\w+\\. (\\d+) passed; (\\d+) failed', ",
            "tests = { ",
        ),
        &[("src/failing.txt", "")],
    );
    let out = repo.fz(&["run", "tests"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stderr.contains("no such reader"), "{}", out.stderr);
}
