//! Coverage: a test check that reads its coverage report finds each line the change touched
//! that no test ran, judged like any finding. Old untested code never counts. How strict it is
//! follows the project's coverage: under the target (80%) a change must test that share of its
//! lines or it blocks; at or above it more tests add little, so untested lines are only hints.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;

/// A test runner that measures `src/*.txt` as lcov: a line holding `miss` ran in no test, a
/// line holding `skip` is not code, every other line ran. It fails while `src/fail` exists.
const LCOV: &str = r#"
[check.tests]
run = '''
for f in src/*.txt; do
  echo "SF:$f"
  n=0
  while IFS= read -r line; do
    n=$((n + 1))
    case "$line" in
      *miss*) echo "DA:$n,0" ;;
      *skip*) ;;
      *) echo "DA:$n,3" ;;
    esac
  done < "$f"
  echo "end_of_record"
done > "$FEARLEZZ_OUT/lcov.info"
echo "test result: ok. 1 passed; 0 failed"
test ! -e src/fail
'''
stage = "working"
inputs = ["src/**"]
tests = { count = 'test result: \w+\. (\d+) passed; (\d+) failed' }
coverage = "lcov"
"#;

/// A project at 67%: two of its three lines ran.
const OLD: &str = "one\nmiss from long ago\nthree\n";

/// The same runner, with a fixed level for every uncovered line, for tests of what is read.
fn flagged() -> String {
    format!("{LCOV}\n[lints.coverage]\nuncovered = \"flag\"\n")
}

fn feature(config: &str, files: &[(&str, &str)]) -> Repo {
    let repo = Repo::new(config, files);
    repo.branch("feat/x");
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    repo
}

#[test]
fn changed_lines_no_test_ran_are_found_in_runs() {
    let repo = feature(&flagged(), &[("src/a.txt", OLD)]);
    repo.write(
        "src/a.txt",
        &format!("{OLD}four\nmiss one\nskip\nmiss two\nfive\nmiss three\n"),
    );
    let out = repo.fz(&["run", "tests"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("flag src/a.txt:5  uncovered  changed lines 5 to 7 ran in no test"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout
            .contains("flag src/a.txt:9  uncovered  this changed line ran in no test"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("src/a.txt:2"), "{}", out.stdout);
}

#[test]
fn tested_changes_have_no_coverage_findings() {
    let repo = feature(LCOV, &[("src/a.txt", OLD)]);
    repo.write("src/a.txt", &format!("{OLD}four\n"));
    let out = repo.fz(&["run", "tests"]);
    assert!(out.ok, "{}", out.stdout);
    assert!(!out.stdout.contains("uncovered"), "{}", out.stdout);
}

#[test]
fn a_project_can_make_uncovered_code_block() {
    let config = format!("{LCOV}\n[lints.coverage]\nuncovered = \"gate\"\n");
    let repo = feature(&config, &[("src/a.txt", OLD)]);
    repo.write("src/a.txt", &format!("{OLD}miss new\n"));
    let out = repo.fz(&["run", "tests"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("gate src/a.txt:4  uncovered"),
        "{}",
        out.stdout
    );
}

#[test]
fn coverage_findings_never_excuse_a_failing_test_run() {
    let repo = feature(LCOV, &[("src/a.txt", OLD)]);
    repo.write("src/a.txt", &format!("{OLD}miss new\n"));
    repo.write("src/fail", "");
    let out = repo.fz(&["run", "tests"]);
    assert!(!out.ok, "{}", out.stdout);
}

#[test]
fn a_run_that_writes_no_report_fails() {
    let config = LCOV.replace("$FEARLEZZ_OUT/lcov.info", "/dev/null");
    let repo = feature(&config, &[("src/a.txt", OLD)]);
    repo.write("src/a.txt", &format!("{OLD}four\n"));
    let out = repo.fz(&["run", "tests"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("no lcov report matching $FEARLEZZ_OUT/lcov.info written by this run"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_report_left_by_an_earlier_run_is_not_read() {
    let config = LCOV
        .replace("$FEARLEZZ_OUT/lcov.info", "coverage/lcov.info")
        .replace("done > \"coverage", "done > \"${COVER:-coverage}")
        .replace(
            "coverage = \"lcov\"",
            "coverage = { lcov = \"coverage/lcov.info\" }",
        )
        .replace(
            "stage = \"working\"",
            "stage = \"working\"\nenv = [\"COVER\"]",
        );
    let config = format!("{config}\n[lints.coverage]\nuncovered = \"flag\"\n");
    let repo = feature(&config, &[("src/a.txt", OLD), ("coverage/.keep", "")]);
    repo.write("src/a.txt", &format!("{OLD}miss new\n"));
    let first = repo.fz(&["run", "tests"]);
    assert!(first.ok, "{}", first.stdout);
    assert!(first.stdout.contains("uncovered"), "{}", first.stdout);
    repo.write("src/a.txt", &format!("{OLD}miss newer\n"));
    let out = repo.fz_env(&["run", "tests"], &[("COVER", "/dev/null#")]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("no lcov report matching coverage/lcov.info"),
        "{}",
        out.stdout
    );
}

#[test]
fn reading_coverage_another_way_runs_the_check_again() {
    let repo = feature(LCOV, &[("src/a.txt", OLD)]);
    repo.write("src/a.txt", &format!("{OLD}four\n"));
    assert!(repo.fz(&["run", "tests"]).ok);
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    repo.write(
        "fearlezz.toml",
        &config.replace(
            "coverage = \"lcov\"",
            "coverage = { lcov = \"$FEARLEZZ_OUT/*.info\" }",
        ),
    );
    let out = repo.fz(&["run", "tests"]);
    assert!(!out.stdout.contains("1 reused"), "{}", out.stdout);
}

#[test]
fn a_report_from_a_project_folder_names_files_from_there() {
    let config = flagged()
        .replace("for f in src/*.txt", "for f in lib/*.txt")
        .replace("test ! -e src/fail", "true")
        .replace(
            "inputs = [\"src/**\"]",
            "inputs = [\"app/**\"]\ndir = \"app\"",
        );
    let repo = feature(&config, &[("app/lib/a.txt", OLD)]);
    repo.write("app/lib/a.txt", &format!("{OLD}miss new\n"));
    let out = repo.fz(&["run", "tests"]);
    assert!(out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("flag app/lib/a.txt:4  uncovered"),
        "{}",
        out.stdout
    );
}

/// `go test -coverprofile`: blocks by import path, from the module go.mod names.
#[test]
fn a_go_coverprofile_names_files_by_import_path() {
    let config = r#"
[check.go-test]
run = '''
cat > "$FEARLEZZ_OUT/cover.out" <<'EOF'
mode: set
example.com/m/pkg/a.go:1.1,4.2 2 1
example.com/m/pkg/a.go:5.1,6.9 1 0
example.com/m/pkg/a.go:6.9,7.2 1 1
EOF
echo "--- PASS: TestA"
'''
stage = "working"
tests = { count = '(?m)^\s*--- (?:PASS|FAIL):' }
coverage = "go"

[lints.coverage]
uncovered = "flag"
"#;
    let old = "package pkg\n\nfunc A() {\n}\n";
    let repo = feature(
        config,
        &[
            ("go.mod", "module example.com/m\n\ngo 1.22\n"),
            ("pkg/a.go", old),
        ],
    );
    repo.write("pkg/a.go", &format!("{old}func B() {{\n  b()\n}}\n"));
    let out = repo.fz(&["run", "go-test"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("flag pkg/a.go:5  uncovered  this changed line ran in no test"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("pkg/a.go:6"), "{}", out.stdout);
}

/// A runner that writes JaCoCo XML, as Kover does: `com/x/Foo.kt` with line 3 covered, line 4
/// missed and line 5 partly covered.
const JACOCO: &str = r#"
[check.gradle-test]
run = '''
cat > "$FEARLEZZ_OUT/jacoco.xml" <<'EOF'
<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<!DOCTYPE report PUBLIC "-//JACOCO//DTD Report 1.1//EN" "report.dtd">
<report name="app">
  <package name="com/x">
    <sourcefile name="Foo.kt">
      <line nr="3" mi="0" ci="4" mb="0" cb="0"/>
      <line nr="4" mi="2" ci="0" mb="0" cb="0"/>
      <line nr="5" mi="1" ci="1" mb="1" cb="1"/>
    </sourcefile>
  </package>
</report>
EOF
echo "--- PASS: FooTest"
'''
stage = "working"
tests = { count = '(?m)^\s*--- (?:PASS|FAIL):' }
coverage = "jacoco"

[lints.coverage]
uncovered = "flag"
"#;

/// JaCoCo names source files by package, found under any source folder.
#[test]
fn a_jacoco_report_finds_files_by_package() {
    let config = JACOCO;
    let file = "app/src/main/kotlin/com/x/Foo.kt";
    let repo = feature(config, &[(file, "package com.x\n\n")]);
    repo.write(
        file,
        "package com.x\n\nfun a() = 1\nfun b() = 2\nfun c() = 3\n",
    );
    let out = repo.fz(&["run", "gradle-test"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains(&format!(
            "flag {file}:4  uncovered  this changed line ran in no test"
        )),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains(&format!("{file}:5")), "{}", out.stdout);
}

#[test]
fn removing_coverage_weakens_the_proof() {
    let repo = Repo::new(LCOV, &[("src/a.txt", OLD)]);
    repo.branch("feat/x");
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    repo.write(
        "fearlezz.toml",
        &config.replace("coverage = \"lcov\"\n", ""),
    );
    repo.commit("drop coverage");
    let out = repo.fz(&["run", "--ci"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("weakens tests: it no longer measures coverage"),
        "{}",
        out.stdout
    );
}

/// The JaCoCo runner above, for a change that touches only a test file.
#[test]
fn a_jacoco_report_of_files_the_change_did_not_touch_is_fine() {
    let file = "app/src/main/kotlin/com/x/Foo.kt";
    let repo = feature(
        JACOCO,
        &[(
            file,
            "package com.x\n\nfun a() = 1\nfun b() = 2\nfun c() = 3\n",
        )],
    );
    repo.write("app/src/test/kotlin/com/x/FooTest.kt", "package com.x\n");
    let out = repo.fz(&["run", "gradle-test"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(!out.stdout.contains("uncovered"), "{}", out.stdout);
}

/// Kotlin may leave the common root package out of its folders.
#[test]
fn a_jacoco_file_outside_its_package_folder_is_found_by_name() {
    let file = "app/src/main/kotlin/Foo.kt";
    let repo = feature(JACOCO, &[(file, "package com.x\n\n")]);
    repo.write(
        file,
        "package com.x\n\nfun a() = 1\nfun b() = 2\nfun c() = 3\n",
    );
    let out = repo.fz(&["run", "gradle-test"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains(&format!("flag {file}:4  uncovered")),
        "{}",
        out.stdout
    );
}

#[test]
fn lowering_the_coverage_level_weakens_the_proof() {
    let config = format!("{LCOV}\n[lints.coverage]\nuncovered = \"gate\"\n");
    let repo = Repo::new(&config, &[("src/a.txt", OLD)]);
    repo.branch("feat/x");
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    repo.write(
        "fearlezz.toml",
        &config.replace("uncovered = \"gate\"", "uncovered = \"flag\""),
    );
    repo.commit("lower coverage");
    let out = repo.fz(&["run", "--ci"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stdout.contains("weakens"), "{}", out.stdout);
    assert!(out.stdout.contains("uncovered"), "{}", out.stdout);
}

#[test]
fn a_project_turns_off_the_coverage_a_piece_measures() {
    let config = r#"
use = ["go"]

[check.go-test]
run = "echo '--- PASS: TestA'"
coverage = false

[check.golangci-lint]
off = true

[check.gofmt]
off = true
"#;
    let repo = feature(config, &[("a.go", "package a\n")]);
    repo.write("a.go", "package a\n\nfunc A() {}\n");
    let out = repo.fz(&["run", "go-test"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
}

#[test]
fn under_the_target_a_mostly_untested_change_blocks() {
    let repo = feature(LCOV, &[("src/a.txt", OLD)]);
    repo.write("src/a.txt", &format!("{OLD}four\nmiss one\nmiss two\n"));
    let out = repo.fz(&["run", "tests"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("gate src/a.txt:5  uncovered  changed lines 5 to 6 ran in no test"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains(
            "why: the project is at 50% and this change tests 1 of its 3 lines; under 80%, a \
             change must test 80% of its lines"
        ),
        "{}",
        out.stdout
    );
}

#[test]
fn under_the_target_a_mostly_tested_change_is_flagged() {
    // Two of seven lines ran; with the change, eleven of seventeen.
    let low = format!("{OLD}{}", "miss old\n".repeat(4));
    let repo = feature(LCOV, &[("src/a.txt", &low)]);
    let tested = "ran\n".repeat(9);
    repo.write("src/a.txt", &format!("{low}{tested}miss new\n"));
    let out = repo.fz(&["run", "tests"]);
    assert!(out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("flag src/a.txt:17  uncovered"),
        "{}",
        out.stdout
    );
}

#[test]
fn at_the_target_untested_lines_are_only_hints() {
    let high = format!("{}miss from long ago\n", "ran\n".repeat(20));
    let repo = feature(LCOV, &[("src/a.txt", &high)]);
    repo.write("src/a.txt", &format!("{high}miss new\nmiss two\n"));
    let out = repo.fz(&["run", "tests"]);
    assert!(out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("hint src/a.txt:22  uncovered"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout
            .contains("why: the project is at 86%; at or above 80% more tests add little"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_check_sets_its_own_target() {
    let config = LCOV.replace(
        "coverage = \"lcov\"",
        "coverage = { read = \"lcov\", target = 50 }",
    );
    let repo = feature(&config, &[("src/a.txt", OLD)]);
    repo.write("src/a.txt", &format!("{OLD}miss new\n"));
    let out = repo.fz(&["run", "tests"]);
    assert!(out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("hint src/a.txt:4  uncovered"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_changed_test_file_is_not_measured() {
    let config = LCOV.replace(
        "coverage = \"lcov\"",
        "coverage = \"lcov\"\ntest_files = [\"src/*_test.txt\"]",
    );
    let repo = feature(&config, &[("src/a.txt", OLD), ("src/a_test.txt", "ran\n")]);
    repo.write("src/a_test.txt", "ran\nmiss helper\n");
    let out = repo.fz(&["run", "tests"]);
    assert!(out.ok, "{}", out.stdout);
    assert!(!out.stdout.contains("uncovered"), "{}", out.stdout);
}

#[test]
fn reading_coverage_another_way_weakens_the_proof() {
    let repo = Repo::new(LCOV, &[("src/a.txt", OLD)]);
    repo.branch("feat/x");
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    repo.write(
        "fearlezz.toml",
        &config.replace("coverage = \"lcov\"", "coverage = \"jacoco\""),
    );
    repo.commit("read jacoco");
    let out = repo.fz(&["run", "--ci"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("weakens tests: it reads its coverage as jacoco instead of lcov"),
        "{}",
        out.stdout
    );
}

#[test]
fn lowering_the_target_weakens_the_proof() {
    let repo = Repo::new(LCOV, &[("src/a.txt", OLD)]);
    repo.branch("feat/x");
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    repo.write(
        "fearlezz.toml",
        &config.replace(
            "coverage = \"lcov\"",
            "coverage = { read = \"lcov\", target = 60 }",
        ),
    );
    repo.commit("lower the target");
    let out = repo.fz(&["run", "--ci"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("weakens tests: its coverage target is lower (80%, now 60%)"),
        "{}",
        out.stdout
    );
}

#[test]
fn ci_shows_what_was_measured() {
    let repo = Repo::new(LCOV, &[("src/a.txt", OLD)]);
    repo.branch("feat/x");
    repo.write("src/a.txt", &format!("{OLD}four\nmiss one\nmiss two\n"));
    repo.commit("feat: more");
    let out = repo.fz(&["run", "--ci"]);
    assert!(
        out.stdout
            .contains("  1 test  coverage 50%, 1 of 3 changed lines  "),
        "{}",
        out.stdout
    );
}

#[test]
fn naming_test_files_for_the_first_time_weakens_coverage() {
    let repo = Repo::new(LCOV, &[("src/a.txt", OLD)]);
    repo.branch("feat/x");
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    repo.write(
        "fearlezz.toml",
        &config.replace(
            "coverage = \"lcov\"",
            "coverage = \"lcov\"\ntest_files = [\"src/**\"]",
        ),
    );
    repo.commit("name test files");
    let out = repo.fz(&["run", "--ci"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("weakens tests: it names other test files"),
        "{}",
        out.stdout
    );
}

#[test]
fn naming_other_test_files_runs_the_check_again() {
    let repo = feature(LCOV, &[("src/a.txt", OLD)]);
    repo.write("src/a.txt", &format!("{OLD}four\n"));
    assert!(repo.fz(&["run", "tests"]).ok);
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    repo.write(
        "fearlezz.toml",
        &config.replace(
            "coverage = \"lcov\"",
            "coverage = \"lcov\"\ntest_files = [\"src/*_test.txt\"]",
        ),
    );
    let out = repo.fz(&["run", "tests"]);
    assert!(!out.stdout.contains("1 reused"), "{}", out.stdout);
}

#[test]
fn a_target_without_a_reader_says_to_name_one() {
    let config = LCOV.replace("coverage = \"lcov\"", "coverage = { target = 70 }");
    let repo = Repo::new(&config, &[("src/a.txt", OLD)]);
    let out = repo.fz(&["run", "tests"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stderr
            .contains("names no reader; write read = \"lcov\""),
        "{}",
        out.stderr
    );
}

/// Go marks every line of a block, comments and blank lines too.
#[test]
fn a_go_block_does_not_flag_its_comments_or_blank_lines() {
    let config = r#"
[check.go-test]
run = '''
printf 'mode: set\nexample.com/m/a.go:3.11,8.2 2 0\n' > "$FEARLEZZ_OUT/cover.out"
echo "--- PASS: TestA"
'''
stage = "working"
tests = { count = '(?m)^\s*--- (?:PASS|FAIL):' }
coverage = "go"

[lints.coverage]
uncovered = "flag"
"#;
    let repo = feature(
        config,
        &[
            ("go.mod", "module example.com/m\n"),
            ("a.go", "package a\n"),
        ],
    );
    repo.write(
        "a.go",
        "package a\n\nfunc A() {\n\t// why\n\n\ta()\n\tb()\n}\n",
    );
    let out = repo.fz(&["run", "go-test"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("flag a.go:3  uncovered  changed lines 3 to 8 ran in no test"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("tests 0 of its 4 lines"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_jacoco_file_is_not_matched_by_name_to_another_package() {
    // The report also names a file the repository has, so it is read at all.
    let config = JACOCO.replace(
        "  </package>\n</report>",
        "  </package>\n  <package name=\"util\">\n    <sourcefile name=\"Bar.kt\">\n      \
         <line nr=\"1\" mi=\"0\" ci=\"1\" mb=\"0\" cb=\"0\"/>\n    </sourcefile>\n  \
         </package>\n</report>",
    );
    let file = "app/src/main/kotlin/util/Foo.kt";
    let repo = feature(
        &config,
        &[
            (file, "package util\n\n"),
            ("app/src/main/kotlin/util/Bar.kt", "package util\n"),
        ],
    );
    repo.write(
        file,
        "package util\n\nfun a() = 1\nfun b() = 2\nfun c() = 3\n",
    );
    let out = repo.fz(&["run", "gradle-test"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(!out.stdout.contains("uncovered"), "{}", out.stdout);
}

/// A JaCoCo file the repository does not keep (generated code) counts toward nothing, as it
/// does in an lcov report.
#[test]
fn a_jacoco_file_the_repository_does_not_keep_counts_toward_nothing() {
    let config = JACOCO.replace(
        "  </package>\n</report>",
        "  </package>\n  <package name=\"com/x/gen\">\n    <sourcefile name=\"Gen.kt\">\n      \
         <line nr=\"1\" mi=\"5\" ci=\"0\" mb=\"0\" cb=\"0\"/>\n      \
         <line nr=\"2\" mi=\"5\" ci=\"0\" mb=\"0\" cb=\"0\"/>\n      \
         <line nr=\"3\" mi=\"5\" ci=\"0\" mb=\"0\" cb=\"0\"/>\n    </sourcefile>\n  \
         </package>\n</report>",
    );
    let file = "app/src/main/kotlin/com/x/Foo.kt";
    let repo = Repo::new(&config, &[(file, "package com.x\n\n")]);
    repo.branch("feat/x");
    repo.write(
        file,
        "package com.x\n\nfun a() = 1\nfun b() = 2\nfun c() = 3\n",
    );
    repo.commit("feat: foo");
    let out = repo.fz(&["run", "--ci"]);
    assert!(
        out.stdout.contains("coverage 66%, 2 of 3 changed lines"),
        "{}",
        out.stdout
    );
}

/// A JaCoCo file that is no single file of the repository counts toward nothing, and the
/// check's log says which, so a report naming files from elsewhere is not lost unseen.
#[test]
fn a_jacoco_file_no_repository_file_is_named_in_the_log() {
    let config = JACOCO.replace(
        "  </package>\n</report>",
        "  </package>\n  <package name=\"com/x/gen\">\n    <sourcefile name=\"Gen.kt\">\n      \
         <line nr=\"1\" mi=\"5\" ci=\"0\" mb=\"0\" cb=\"0\"/>\n    </sourcefile>\n  \
         </package>\n</report>",
    );
    let file = "app/src/main/kotlin/com/x/Foo.kt";
    let repo = feature(&config, &[(file, "package com.x\n\n")]);
    repo.write(
        file,
        "package com.x\n\nfun a() = 1\nfun b() = 2\nfun c() = 3\n",
    );
    let out = repo.fz(&["run", "gradle-test"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let logs = logs(&repo.path(".git/fearlezz/results"));
    assert!(
        logs.contains(
            "coverage: the jacoco report names com/x/gen/Gen.kt, which is no single file of the \
             repository's code, so it counts toward nothing"
        ),
        "{logs}"
    );
}

/// Every check log stored below `dir`, one after another.
fn logs(dir: &std::path::Path) -> String {
    let mut text = String::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            text.push_str(&logs(&path));
        } else if path.file_name().is_some_and(|n| n == "output.log") {
            text.push_str(&std::fs::read_to_string(&path).unwrap_or_default());
        }
    }
    text
}

/// When git cannot list the project's files, nothing would count, so coverage would pass
/// unmeasured; the check fails instead.
#[test]
fn coverage_fails_when_git_cannot_list_the_files() {
    let config = flagged().replace(
        "test ! -e src/fail\n",
        "test ! -e src/fail\n[ -e src/break ] && echo broken > \"$(git rev-parse --git-dir)/index\"\ntrue\n",
    );
    let repo = feature(&config, &[("src/a.txt", OLD)]);
    repo.write("src/a.txt", &format!("{OLD}miss one\n"));
    repo.write("src/break", "");
    let out = repo.fz(&["run", "tests"]);
    assert!(!out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        format!("{}{}", out.stdout, out.stderr).contains("could not list the repository's files"),
        "{}\n{}",
        out.stdout,
        out.stderr
    );
}
