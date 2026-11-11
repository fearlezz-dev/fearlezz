//! Red before green: CI runs a check on the base commit with the change's test files laid
//! over it, to show those tests fail without the change. A fix whose tests pass there too
//! proves nothing about the fix, so a `fix:` commit is held to it.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;

/// Every shell script in `tests/` must pass. The test files are `tests/**`.
const CHECKS: &str = r#"
[check.tests]
run = "for t in tests/*.sh; do sh \"$t\" || exit 1; done"
stage = "working"
inputs = ["src/**", "tests/**"]
test_files = ["tests/**"]
"#;

/// A repository whose `src/value` is 1, with one old test that passes either way.
fn repo() -> Repo {
    let repo = Repo::new(
        CHECKS,
        &[
            (".gitignore", ".runs/\n"),
            ("src/value", "1\n"),
            ("tests/old.sh", "test -e src/value\n"),
        ],
    );
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    repo.branch("change");
    repo
}

/// `src/value` becomes 2, with a new test file and a commit message.
fn change(repo: &Repo, test: &str, message: &str) {
    repo.write("src/value", "2\n");
    repo.write("tests/new.sh", test);
    repo.commit(message);
}

/// A test that shows the change: it fails while the value is 1.
const SHOWS_IT: &str = "test \"$(cat src/value)\" = 2\n";
/// A test that passes on the base commit too.
const PASSES_ANYWAY: &str = "test -e src/value\n";

fn ci(repo: &Repo) -> (common::Output, String) {
    let path = repo.path(".runs/comment.md");
    let out = repo.fz(&["run", "--ci", "--comment", path.to_str().unwrap()]);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{}", out.stdout));
    (out, text)
}

#[test]
fn a_fix_whose_new_test_fails_on_the_base_commit_is_red_before_green() {
    let repo = repo();
    change(&repo, SHOWS_IT, "fix: the value is two");
    let (out, text) = ci(&repo);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("red before green tests: tests/new.sh fails on the base commit"),
        "{}",
        out.stdout
    );
    assert!(text.contains("**Proven**"), "{text}");
    assert!(
        text.contains("Red before green: `tests` fails on the base commit"),
        "{text}"
    );
}

#[test]
fn a_fix_whose_new_test_passes_on_the_base_commit_is_not_proven() {
    let repo = repo();
    change(&repo, PASSES_ANYWAY, "fix: the value is two");
    let (out, text) = ci(&repo);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains(
            "not red before green tests: tests/new.sh passes on the base commit too, so it \
             does not show the fix"
        ),
        "{}",
        out.stdout
    );
    assert!(text.contains("**Not proven**"), "{text}");
    assert!(
        text.contains("| the tests of `tests` do not show the fix |"),
        "{text}"
    );
}

#[test]
fn a_feature_whose_new_test_passes_on_the_base_commit_is_only_shown() {
    let repo = repo();
    change(&repo, PASSES_ANYWAY, "feat: a second value");
    let (out, text) = ci(&repo);
    assert!(out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("not red before green tests"),
        "{}",
        out.stdout
    );
    assert!(text.contains("**Proven**"), "{text}");
}

#[test]
fn a_change_without_test_files_runs_nothing_on_the_base_commit() {
    let repo = repo();
    repo.write("src/value", "2\n");
    repo.commit("fix: the value is two");
    let (out, _) = ci(&repo);
    assert!(out.ok, "{}", out.stdout);
    assert!(!out.stdout.contains("red before green"), "{}", out.stdout);
}

#[test]
fn the_pull_request_title_decides_whether_it_is_a_fix() {
    let repo = repo();
    change(&repo, PASSES_ANYWAY, "a plain sentence");
    let event = repo.path(".runs/event.json");
    std::fs::write(
        &event,
        r#"{"pull_request": {"title": "fix: the value is two"}}"#,
    )
    .unwrap();
    let out = repo.fz_env(
        &["run", "--ci"],
        &[("GITHUB_EVENT_PATH", event.to_str().unwrap())],
    );
    assert!(!out.ok, "{}", out.stdout);
}

#[test]
fn changing_which_files_are_tests_weakens_the_proof() {
    let repo = repo();
    repo.write(
        "fearlezz.toml",
        &format!(
            "{}\n[settings.skill]\nagents = []\n\n[settings.hooks]\nagents = []\n",
            CHECKS.replace(
                "test_files = [\"tests/**\"]",
                "test_files = [\"tests/none/**\"]"
            )
        ),
    );
    repo.commit("config");
    let (out, text) = ci(&repo);
    assert!(!out.ok, "{}", out.stdout);
    assert!(text.contains("it names other test files"), "{text}");
}

// Naming the tests

/// The same tests, through a stand in runner that writes a JUnit report with one case per
/// script, and first "builds": every file a script names on a `# needs` line must exist.
const JUNIT: &str = r#"
[check.tests]
run = '''
for t in tests/*.sh; do
  for need in $(sed -n 's/^# needs //p' "$t"); do test -e "$need" || exit 1; done
done
r=0
{
  echo '<testsuite>'
  for t in tests/*.sh; do
    if sh "$t"; then echo "<testcase classname=\"$t\" name=\"works\"/>"
    else r=1; echo "<testcase classname=\"$t\" name=\"works\"><failure/></testcase>"; fi
  done
  echo '</testsuite>'
} > "$FEARLEZZ_OUT/junit.xml"
exit $r
'''
stage = "working"
inputs = ["src/**", "tests/**"]
test_files = ["tests/**"]
tests = "junit"
"#;

/// Like `repo`, with the JUnit runner.
fn junit_repo() -> Repo {
    let repo = Repo::new(
        JUNIT,
        &[
            (".gitignore", ".runs/\n"),
            ("src/value", "1\n"),
            ("tests/old.sh", "test -e src/value\n"),
        ],
    );
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    repo.branch("change");
    repo
}

#[test]
fn a_junit_check_names_the_tests_that_fail_on_the_base_commit() {
    let repo = junit_repo();
    change(&repo, SHOWS_IT, "fix: the value is two");
    let (out, text) = ci(&repo);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("red before green tests: tests/new.sh fails on the base commit"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout
            .contains("  failed on the base, passed here: tests/new.sh::works\n"),
        "{}",
        out.stdout
    );
    assert!(text.contains("`tests/new.sh::works`"), "{text}");
}

#[test]
fn a_junit_check_names_the_new_tests_that_pass_on_both() {
    let repo = junit_repo();
    change(&repo, PASSES_ANYWAY, "feat: a second value");
    let (out, _) = ci(&repo);
    assert!(out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("  passed on both: tests/new.sh::works\n"),
        "{}",
        out.stdout
    );
    assert!(
        !out.stdout.contains("tests/old.sh::works"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_test_that_does_not_build_on_the_base_commit_counts_as_red_and_says_so() {
    let repo = junit_repo();
    repo.write("src/helper", "\n");
    change(
        &repo,
        "# needs src/helper\ntest -e src/helper\n",
        "fix: add the helper",
    );
    let (out, _) = ci(&repo);
    assert!(out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains(
            "red before green tests: tests/new.sh does not build on the base commit, which \
             counts as failing, and passes here"
        ),
        "{}",
        out.stdout
    );
}

/// Puts a test on the base branch that already fails there, and the code that makes it pass
/// into the change.
fn break_the_base(repo: &Repo) {
    repo.git(&["switch", "-q", "main"]);
    repo.write("tests/broken.sh", "test -e src/fixed\n");
    repo.commit("a broken test");
    repo.git(&["switch", "-q", "change"]);
    repo.git(&["rebase", "-q", "main"]);
    repo.write("src/fixed", "\n");
}

#[test]
fn when_the_base_already_fails_failing_there_shows_nothing() {
    let repo = repo();
    break_the_base(&repo);
    change(&repo, SHOWS_IT, "fix: the value is two");
    let (out, text) = ci(&repo);
    assert!(out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains(
            "red before green tests: the base already fails without the change's test files, \
             so failing there shows nothing"
        ),
        "{}",
        out.stdout
    );
    assert!(text.contains("**Proven**"), "{text}");
}

#[test]
fn when_the_base_already_fails_a_junit_check_names_only_the_tests_the_change_makes_pass() {
    let repo = junit_repo();
    break_the_base(&repo);
    change(&repo, SHOWS_IT, "fix: the value is two");
    let (out, _) = ci(&repo);
    assert!(out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("  failed on the base, passed here: tests/new.sh::works\n"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("broken.sh::works"), "{}", out.stdout);
}

// Locally

const FINAL: &[&str] = &["run", "--stage", "merge", "--reason", "done"];

#[test]
fn the_final_stage_shows_red_before_green_locally() {
    let repo = repo();
    change(&repo, SHOWS_IT, "fix: the value is two");
    let out = repo.fz(FINAL);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("red before green tests: tests/new.sh fails on the base commit"),
        "{}",
        out.stdout
    );
}

#[test]
fn the_final_stage_holds_a_fix_whose_tests_pass_on_the_base_commit() {
    let repo = repo();
    change(&repo, PASSES_ANYWAY, "fix: the value is two");
    let out = repo.fz(FINAL);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("not red before green tests"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout
            .lines()
            .last()
            .is_some_and(|l| l.starts_with("next: change a test so it fails without the fix")),
        "{}",
        out.stdout
    );
}

#[test]
fn a_feature_whose_tests_pass_on_the_base_commit_is_only_shown_locally() {
    let repo = repo();
    change(&repo, PASSES_ANYWAY, "feat: a second value");
    let out = repo.fz(FINAL);
    assert!(out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("not red before green tests"),
        "{}",
        out.stdout
    );
}

#[test]
fn earlier_stages_do_not_run_the_base_commit() {
    let repo = repo();
    change(&repo, PASSES_ANYWAY, "fix: the value is two");
    let out = repo.fz(&["run", "--stage", "working", "--reason", "go"]);
    assert!(out.ok, "{}", out.stdout);
    assert!(!out.stdout.contains("red before green"), "{}", out.stdout);
}
