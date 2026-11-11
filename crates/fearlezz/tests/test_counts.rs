//! Test counts: a check with a `tests` reader says how many tests it ran, and a run of zero
//! tests fails, so a test command that proves nothing cannot pass.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;

/// A test runner that prints what `cargo test` prints, for the test files in `src/`: one
/// `test result` line per file, with as many passed tests as the file has lines.
const CARGO: &str = r#"
[check.tests]
run = '''
for f in src/*.txt; do
  n=$(wc -l < "$f" | tr -d ' ')
  echo "test result: ok. $n passed; 0 failed; 0 ignored; 0 measured; 0 filtered out"
done
'''
stage = "working"
inputs = ["src/**"]
tests = "cargo"
"#;

/// A test runner that writes a JUnit report: one test case per line of `src/a.txt`, skipped
/// when the line says skip. No report while `src/no-report` exists.
const JUNIT: &str = r#"
[check.tests]
run = '''
test -e src/no-report && exit 0
{
  echo '<testsuites><testsuite name="all">'
  while read -r line; do
    case "$line" in
      skip*) echo "<testcase name=\"$line\"><skipped/></testcase>" ;;
      *) echo "<testcase name=\"$line\"/>" ;;
    esac
  done < src/a.txt
  echo '</testsuite></testsuites>'
} > "$FEARLEZZ_OUT/junit.xml"
'''
stage = "working"
inputs = ["src/**"]
tests = "junit"
"#;

fn repo(config: &str, files: &[(&str, &str)]) -> Repo {
    let repo = Repo::new(config, files);
    repo.branch("feat/x");
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    repo
}

/// The line `fearlezz run --ci` shows for the tests check.
fn ci_line(repo: &Repo) -> (bool, String, String) {
    let out = repo.fz(&["run", "--ci"]);
    let line = out
        .lines()
        .into_iter()
        .find(|l| l.contains(" tests  "))
        .unwrap_or_else(|| panic!("{}", out.stdout))
        .to_string();
    (out.ok, line, out.stdout)
}

#[test]
fn ci_shows_how_many_tests_the_cargo_output_counts() {
    let repo = repo(
        CARGO,
        &[("src/a.txt", "1\n2\n3\n"), ("src/b.txt", "1\n2\n")],
    );
    let (ok, line, out) = ci_line(&repo);
    assert!(ok, "{out}");
    assert!(line.starts_with("passed tests"), "{line}");
    assert!(line.contains("  5 tests  "), "{line}");
}

#[test]
fn the_job_summary_shows_the_count() {
    let repo = repo(CARGO, &[("src/a.txt", "1\n2\n3\n")]);
    let summary = repo.path("summary.md");
    let out = repo.fz_env(
        &["run", "--ci"],
        &[("GITHUB_STEP_SUMMARY", summary.to_str().unwrap())],
    );
    assert!(out.ok, "{}", out.stdout);
    let text = std::fs::read_to_string(&summary).unwrap();
    assert!(text.contains("| check | result | tests | time |"), "{text}");
    assert!(text.contains("| tests | passed | 3 |"), "{text}");
}

#[test]
fn zero_tests_fails_the_check() {
    let repo = repo(CARGO, &[("src/a.txt", "")]);
    let out = repo.fz(&["run", "tests"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("fail tests: it ran no tests, so it proves nothing"),
        "{}",
        out.stdout
    );
}

#[test]
fn output_without_a_test_result_fails_the_check() {
    let repo = repo(
        &CARGO.replace("echo \"test result", "echo \"all good"),
        &[("src/a.txt", "1\n")],
    );
    let out = repo.fz(&["run", "tests"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("no test result line in its output"),
        "{}",
        out.stdout
    );
}

#[test]
fn junit_counts_the_test_cases_that_ran() {
    let repo = repo(JUNIT, &[("src/a.txt", "one\ntwo\nskip three\n")]);
    let (ok, line, out) = ci_line(&repo);
    assert!(ok, "{out}");
    assert!(line.contains("  2 tests  "), "{line}");
}

#[test]
fn junit_without_a_report_fails_and_says_where_it_looked() {
    let repo = repo(JUNIT, &[("src/a.txt", "one\n"), ("src/no-report", "")]);
    let out = repo.fz(&["run", "tests"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("no $FEARLEZZ_OUT/junit.xml"),
        "{}",
        out.stdout
    );
}

#[test]
fn an_unknown_reader_is_an_error() {
    let repo = Repo::new(
        &CARGO.replace("\"cargo\"", "\"mocha\""),
        &[("src/a.txt", "1\n")],
    );
    let out = repo.fz(&["run", "tests"]);
    assert!(!out.ok);
    assert!(out.stderr.contains("mocha"), "{}", out.stderr);
}

#[test]
fn dropping_the_reader_weakens_the_floor() {
    let repo = Repo::new(CARGO, &[("src/a.txt", "1\n")]);
    repo.branch("feat/x");
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    repo.write("fearlezz.toml", &config.replace("tests = \"cargo\"\n", ""));
    repo.commit("drop the reader");
    let out = repo.fz(&["run", "--ci"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("weakens tests: it no longer counts its tests"),
        "{}",
        out.stdout
    );
}
