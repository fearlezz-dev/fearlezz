//! Checks that need secrets: a check names them, Fearlezz takes each from its own env or else
//! fetches it from SecretSpec, hands only those to the check, and their values never show in
//! what Fearlezz keeps or prints. The tests use SecretSpec's `env` provider, so SecretSpec's
//! values come from Fearlezz's own env too.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;

const SECRETSPEC: &str = r#"[project]
name = "demo"
revision = "1.0"

[profiles.default]
EVAL_API_KEY = { description = "the model eval's key", required = true }
"#;

const VALUE: &str = "sk-test-0123456789";

/// A repository with `checks` and a change on a branch, so every check is due.
fn repo(checks: &str) -> Repo {
    let repo = Repo::new(
        checks,
        &[("secretspec.toml", SECRETSPEC), ("src/lib.txt", "a")],
    );
    repo.branch("feat/eval");
    repo.write("src/lib.txt", "b");
    repo
}

/// `fearlezz run` with SecretSpec reading the key from Fearlezz's env, when `key` is given.
fn run(repo: &Repo, key: Option<&str>) -> common::Output {
    let mut env = vec![("SECRETSPEC_PROVIDER", "env")];
    if let Some(key) = key {
        env.push(("EVAL_API_KEY", key));
    }
    repo.fz_env(&["run", "--stage", "working"], &env)
}

#[test]
fn a_check_gets_only_the_secrets_it_names() {
    let repo = repo(&format!(
        r#"
[check.eval]
run = 'test "$EVAL_API_KEY" = "{VALUE}"'
stage = "working"
secrets = ["EVAL_API_KEY"]

[check.unit]
run = 'test -z "$EVAL_API_KEY"'
stage = "working"
"#
    ));
    let out = run(&repo, Some(VALUE));
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("2 passed"), "{}", out.stdout);
}

#[test]
fn a_secret_never_shows_in_what_fearlezz_keeps_or_prints() {
    let repo = repo(
        r#"
[check.eval]
run = 'echo "key is $EVAL_API_KEY"; echo "$EVAL_API_KEY" > "$FEARLEZZ_OUT/note.txt"; exit 1'
stage = "working"
secrets = ["EVAL_API_KEY"]
"#,
    );
    let out = run(&repo, Some(VALUE));
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stdout.contains("key is ***"), "{}", out.stdout);
    assert!(!out.stdout.contains(VALUE), "{}", out.stdout);
    assert!(!out.stderr.contains(VALUE), "{}", out.stderr);
    never_kept(&repo);
}

/// Fails when any file Fearlezz keeps for `repo` holds the secret's value.
fn never_kept(repo: &Repo) {
    let mut kept = vec![repo.cache.clone()];
    while let Some(path) = kept.pop() {
        if path.is_dir() {
            kept.extend(std::fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
        } else {
            let bytes = std::fs::read(&path).unwrap();
            assert!(
                !String::from_utf8_lossy(&bytes).contains(VALUE),
                "{} holds the secret",
                path.display()
            );
        }
    }
}

#[test]
fn a_secret_set_in_fearlezz_env_reaches_the_check_without_secretspec() {
    let repo = repo(&format!(
        r#"
[check.eval]
run = 'echo "key is $EVAL_API_KEY"; test "$EVAL_API_KEY" = "{VALUE}"'
stage = "working"
secrets = ["EVAL_API_KEY"]
"#
    ));
    let out = repo.fz_env(
        &["run", "--stage", "working"],
        &[("PATH", "/usr/bin:/bin"), ("EVAL_API_KEY", VALUE)],
    );
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    never_kept(&repo);
}

#[test]
fn a_secret_that_cannot_be_fetched_fails_the_check_with_the_fix() {
    let repo = repo(
        r#"
[check.eval]
run = 'touch ran'
stage = "working"
secrets = ["EVAL_API_KEY"]
"#,
    );
    let out = run(&repo, None);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("fail eval: SecretSpec could not give it EVAL_API_KEY"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("Secret 'EVAL_API_KEY' not found"),
        "SecretSpec's own reason: {}",
        out.stdout
    );
    assert!(
        out.lines()
            .iter()
            .any(|l| l.trim_start().starts_with("fix:") && l.contains("secretspec.toml")),
        "{}",
        out.stdout
    );
    assert!(!repo.path("ran").exists(), "the command must not run");
}

#[test]
fn a_secret_that_could_not_be_fetched_never_makes_the_check_unstable() {
    let repo = repo(&format!(
        r#"
[check.eval]
run = 'test "$EVAL_API_KEY" = "{VALUE}"'
stage = "working"
secrets = ["EVAL_API_KEY"]
"#
    ));
    assert!(!run(&repo, None).ok);
    let out = run(&repo, Some(VALUE));
    assert!(out.ok, "{}", out.stdout);
    let status = repo.fz(&[]);
    assert!(!status.stdout.contains("unstable"), "{}", status.stdout);
}

#[test]
fn without_secretspec_the_check_names_it_with_how_to_install_it() {
    let repo = repo(
        r#"
[check.eval]
run = 'touch ran'
stage = "working"
secrets = ["EVAL_API_KEY"]
"#,
    );
    let out = repo.fz_env(
        &["run", "--stage", "working"],
        &[("PATH", "/usr/bin:/bin"), ("SECRETSPEC_PROVIDER", "env")],
    );
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("fail eval: secretspec was not found"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("pkgs.secretspec"), "{}", out.stdout);
    assert!(!repo.path("ran").exists(), "the command must not run");
}

#[test]
fn a_secret_may_not_replace_a_variable_every_check_gets() {
    let repo = repo(
        r#"
[check.eval]
run = 'true'
stage = "working"
secrets = ["PATH"]
"#,
    );
    let out = run(&repo, Some(VALUE));
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stderr
            .contains("check eval: the secret PATH would replace a variable every check gets"),
        "{}",
        out.stderr
    );
}
