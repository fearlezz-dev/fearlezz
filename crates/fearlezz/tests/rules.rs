//! Rules: `[rule.<name>]` conventions that tests cannot express, judged as the `rules` check
//! on the lines a change touched.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;

fn feature(config: &str, files: &[(&str, &str)]) -> Repo {
    let repo = Repo::new(config, files);
    repo.branch("feat/x");
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    repo
}

const FROZEN: &str = r#"
[rule.engine-frozen]
level = "gate"
changed = ["engine/**"]
message = "engine/ comes from upstream; change it there."
"#;

#[test]
fn a_change_to_a_frozen_path_gates_and_other_changes_pass() {
    let repo = feature(
        FROZEN,
        &[
            ("engine/core.rs", "fn a() {}\n"),
            ("src/main.rs", "fn main() {}\n"),
        ],
    );
    repo.write("src/main.rs", "fn main() { a(); }\n");
    assert!(repo.fz(&["run", "rules"]).ok);

    repo.write("engine/core.rs", "fn a() { b(); }\n");
    let out = repo.fz(&["run", "rules"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("gate engine/core.rs:1  engine-frozen"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("change it there"), "{}", out.stdout);
}

#[test]
fn deleting_a_frozen_file_counts_as_touching_it() {
    let repo = feature(FROZEN, &[("engine/core.rs", "fn a() {}\n")]);
    repo.remove("engine/core.rs");
    let out = repo.fz(&["run", "rules"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stdout.contains("engine/core.rs"), "{}", out.stdout);
}

const NAMES: &str = r#"
[rule.screen-names]
level = "flag"
files = "lib/screens/**/*.dart"
name = "snake_case"
"#;

#[test]
fn a_new_file_with_the_wrong_name_is_flagged_but_old_ones_wait() {
    let repo = feature(NAMES, &[("lib/screens/OldScreen.dart", "class A {}\n")]);
    repo.write("lib/screens/homeScreen.dart", "class B {}\n");
    repo.write("lib/screens/settings_screen.dart", "class C {}\n");
    let out = repo.fz(&["run", "rules"]);
    // A flag is shown but does not fail the check.
    assert!(out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains(
            "flag lib/screens/homeScreen.dart:1  screen-names  homeScreen is not snake_case"
        ),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("settings_screen"), "{}", out.stdout);
    assert!(!out.stdout.contains("OldScreen"), "{}", out.stdout);
}

#[test]
fn a_new_widget_without_its_test_gets_a_hint() {
    let config = r#"
[rule.widget-has-test]
level = "gate"
files = "lib/widgets/*.dart"
needs = "test/widgets/{stem}_test.dart"
fix = "add a widget test"
"#;
    let repo = feature(config, &[]);
    repo.write("lib/widgets/button.dart", "class Button {}\n");
    let out = repo.fz(&["run", "rules"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("lib/widgets/button.dart needs test/widgets/button_test.dart"),
        "{}",
        out.stdout
    );

    repo.write("test/widgets/button_test.dart", "void main() {}\n");
    assert!(repo.fz(&["run", "rules"]).ok);
}

#[test]
fn deleting_a_companion_file_fails_its_needs_rule() {
    let config = r#"
[rule.widget-has-test]
level = "gate"
files = "lib/widgets/*.dart"
needs = "test/widgets/{stem}_test.dart"
"#;
    let repo = feature(
        config,
        &[
            ("lib/widgets/button.dart", "class Button {}\n"),
            ("test/widgets/button_test.dart", "void main() {}\n"),
        ],
    );
    assert!(repo.fz(&["run", "rules"]).ok);
    repo.remove("test/widgets/button_test.dart");
    let out = repo.fz(&["run", "rules"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("needs test/widgets/button_test.dart"),
        "{}",
        out.stdout
    );
}

const FALLBACK: &str = r#"
[rule.silent-fallback]
level = "gate"
message = "A failure becomes a default value."
fix = "Return or propagate the error, or model the absent case in the type."
skip = ["**/tests/**"]

[rule.silent-fallback.rust]
any = ["$R.unwrap_or_default()", "$R.ok();"]

[rule.silent-fallback.python]
pattern = "$D.get($K) or $V"

[[rule.silent-fallback.test]]
rust = { bad = "fn f() { let x = read().unwrap_or_default(); }", good = "fn f() -> R { let x = read()?; }" }
"#;

#[test]
fn a_code_rule_matches_new_code_in_each_language_and_old_code_waits() {
    let old = "fn old() {\n    let a = load().unwrap_or_default();\n}\n";
    let repo = feature(FALLBACK, &[("src/old.rs", old)]);
    repo.write(
        "src/new.rs",
        "fn new() {\n    let b = read().unwrap_or_default();\n}\n",
    );
    repo.write(
        "tools/x.py",
        "import os\n\nport = os.environ.get(\"PORT\") or 80\n",
    );
    repo.write(
        "tests/t.rs",
        "fn t() { let c = x().unwrap_or_default(); }\n",
    );
    let out = repo.fz(&["run", "rules"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("gate src/new.rs:2  silent-fallback  A failure becomes a default value."),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("gate tools/x.py:3"), "{}", out.stdout);
    assert!(!out.stdout.contains("src/old.rs"), "{}", out.stdout);
    assert!(!out.stdout.contains("tests/t.rs"), "{}", out.stdout);
}

#[test]
fn a_rule_that_misses_its_bad_example_fails() {
    let broken = FALLBACK.replace("$R.unwrap_or_default()", "$R.unwrap_or_else()");
    let repo = feature(&broken, &[]);
    let out = repo.fz(&["run", "rules"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("rule silent-fallback misses its bad rust example 1"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_rule_says_what_it_checks_and_can_be_turned_off() {
    let repo = feature("[rule.empty]\nlevel = \"gate\"\n", &[]);
    let out = repo.fz(&[]);
    assert!(!out.ok);
    assert!(
        out.stderr.contains("rule empty: say what it checks"),
        "{}",
        out.stderr
    );

    // A piece's rule turned off by the project is gone, and so is the check it formed.
    repo.write("rules.toml", FROZEN);
    repo.write(
        "fearlezz.toml",
        "use = [\"./rules.toml\"]\n\n[rule.engine-frozen]\noff = true\n",
    );
    let out = repo.fz(&["run", "rules"]);
    assert!(!out.ok);
    assert!(out.stderr.contains("no check \"rules\""), "{}", out.stderr);
}
