//! `use`: pieces of config pulled in by name, by path, or from a pinned repository.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;

/// The checks Fearlezz knows, as its error for an unknown check lists them.
fn checks(repo: &Repo) -> String {
    let out = repo.fz(&["run", "no-such-check"]);
    assert!(!out.ok);
    out.stderr
        .split("checks are ")
        .nth(1)
        .unwrap_or_else(|| panic!("{}", out.stderr))
        .trim()
        .to_string()
}

#[test]
fn a_built_in_piece_brings_its_checks() {
    let repo = Repo::new("use = [\"rust\"]\n", &[]);
    // The piece's lint levels bring the check that keeps Cargo.toml in line with them.
    assert_eq!(checks(&repo), "clippy, fmt, settings, tests");
}

#[test]
fn the_project_tunes_a_field_and_turns_a_check_off() {
    let config = r#"
use = ["rust"]

[check.clippy]
off = true

[check.tests]
run = "echo tuned > \"$FEARLEZZ_OUT/proof.txt\"; echo 'test result: ok. 1 passed; 0 failed'"
"#;
    let repo = Repo::new(config, &[("src/lib.rs", "")]);
    assert_eq!(checks(&repo), "fmt, settings, tests");
    repo.branch("feat/x");
    repo.write("src/lib.rs", "// changed\n");
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    let out = repo.fz(&["run", "tests"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    // The piece's inputs still apply: the tuned check ran because a .rs file changed.
    let result: serde_json::Value =
        serde_json::from_str(&repo.fz(&["--check", "tests"]).stdout).unwrap();
    let dir = std::path::Path::new(result["dir"].as_str().unwrap());
    assert_eq!(
        std::fs::read_to_string(dir.join("proof.txt")).unwrap(),
        "tuned\n"
    );
}

#[test]
fn a_local_piece_can_use_pieces_next_to_it() {
    let web = "use = [\"./base.toml\"]\n\n[check.screens]\nrun = \"true\"\nstage = \"look\"\n";
    let base = "[check.lint]\nrun = \"true\"\nstage = \"working\"\n";
    let repo = Repo::new(
        "use = [\"./proof/web.toml\"]\n",
        &[("proof/web.toml", web), ("proof/base.toml", base)],
    );
    assert_eq!(checks(&repo), "lint, screens");
}

#[test]
fn a_piece_that_uses_itself_is_refused() {
    let repo = Repo::new(
        "use = [\"./a.toml\"]\n",
        &[
            ("a.toml", "use = [\"./b.toml\"]\n"),
            ("b.toml", "use = [\"./a.toml\"]\n"),
        ],
    );
    let out = repo.fz(&[]);
    assert!(!out.ok);
    assert!(out.stderr.contains("uses itself"), "{}", out.stderr);
}

#[test]
fn an_unknown_piece_says_what_exists() {
    let repo = Repo::new("use = [\"rsut\"]\n", &[]);
    let out = repo.fz(&[]);
    assert!(!out.ok);
    assert!(
        out.stderr.contains(
            "built in are agents, devenv, eslint, flutter, flutter-look, go, kotlin, playwright, python, rust, safety, typescript"
        ),
        "{}",
        out.stderr
    );
}

/// A repository of pieces, like someone's shared setup, tagged v1.
fn shared() -> Repo {
    let web =
        "use = [\"../base/fearlezz.toml\"]\n\n[check.screens]\nrun = \"true\"\nstage = \"look\"\n";
    let base = "[check.lint]\nrun = \"true\"\nstage = \"working\"\n";
    let repo = Repo::new(
        "",
        &[("web/fearlezz.toml", web), ("base/fearlezz.toml", base)],
    );
    repo.git(&["tag", "v1"]);
    repo
}

fn git_piece(shared: &Repo, rev: &str) -> String {
    format!(
        "use = [\"git:file://{}//web/fearlezz.toml@{rev}\"]\n",
        shared.path("").display()
    )
}

#[test]
fn a_piece_from_a_repository_pinned_to_a_tag_is_fetched_once() {
    let shared = shared();
    let repo = Repo::new(&git_piece(&shared, "v1"), &[]);
    assert_eq!(checks(&repo), "lint, screens");
    // Fetched into the shared git folder: it keeps working without the source.
    drop(shared);
    assert_eq!(checks(&repo), "lint, screens");
}

#[test]
fn a_piece_pinned_to_a_commit_works_and_a_branch_is_refused() {
    let shared = shared();
    let commit = shared.git(&["rev-parse", "HEAD"]).trim().to_string();
    let pinned = Repo::new(&git_piece(&shared, &commit[..12]), &[]);
    assert_eq!(checks(&pinned), "lint, screens");

    let moving = Repo::new(&git_piece(&shared, "main"), &[]);
    let out = moving.fz(&[]);
    assert!(!out.ok);
    assert!(out.stderr.contains("branches move"), "{}", out.stderr);
}

/// Moves tag `v1` of `shared` to a new commit that adds a check.
fn move_tag(shared: &Repo) {
    shared.write("base/fearlezz.toml", "[check.lint]\nrun = \"true\"\nstage = \"working\"\n\n[check.audit]\nrun = \"true\"\nstage = \"working\"\n");
    shared.commit("audit");
    shared.git(&["tag", "-f", "v1"]);
}

#[test]
fn a_git_piece_is_locked_and_a_moved_tag_is_refused() {
    let shared = shared();
    let config = git_piece(&shared, "v1");
    let first = Repo::new(&config, &[]);
    assert_eq!(checks(&first), "lint, screens");
    let lock = std::fs::read_to_string(first.path("fearlezz.lock")).unwrap();
    let commit = shared.git(&["rev-parse", "v1"]).trim().to_string();
    assert!(lock.contains(&commit), "{lock}");

    // Someone moves the tag; a fresh clone with the committed lock must not take it silently.
    move_tag(&shared);
    let fresh = Repo::new(&config, &[("fearlezz.lock", &lock)]);
    let out = fresh.fz(&[]);
    assert!(!out.ok);
    assert!(out.stderr.contains("moved"), "{}", out.stderr);
    assert!(
        out.stderr.contains("fearlezz setup update"),
        "{}",
        out.stderr
    );

    // Accepting the move is an update; it says what changed.
    let out = fresh.fz(&["setup", "update"]);
    assert!(out.ok, "{}", out.stderr);
    assert!(
        out.stdout.contains("check.audit.run: added"),
        "{}",
        out.stdout
    );
    assert_eq!(checks(&fresh), "audit, lint, screens");
}

#[test]
fn update_moves_to_the_newest_tag_and_lists_the_changes() {
    let shared = shared();
    let repo = Repo::new(&git_piece(&shared, "v1"), &[]);
    assert_eq!(checks(&repo), "lint, screens");

    shared.write(
        "web/fearlezz.toml",
        "use = [\"../base/fearlezz.toml\"]\n\n[check.screens]\nrun = \"true\"\nstage = \"final\"\n",
    );
    shared.commit("screens at the end");
    shared.git(&["tag", "v1.10"]);
    shared.git(&["tag", "v1.9", "v1"]);

    let out = repo.fz(&["setup", "update"]);
    assert!(out.ok, "{}", out.stderr);
    assert!(out.stdout.contains("v1 → v1.10"), "{}", out.stdout);
    assert!(
        out.stdout
            .contains("check.screens.stage: \"look\" → \"final\""),
        "{}",
        out.stdout
    );
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    assert!(config.contains("@v1.10\""), "{config}");

    let again = repo.fz(&["setup", "update"]);
    assert!(
        again.stdout.contains("still the newest tag"),
        "{}",
        again.stdout
    );
    assert!(again.stdout.contains("no changes"), "{}", again.stdout);
    assert!(again.last_line().starts_with("next: "), "{}", again.stdout);
}

/// Moves `v1` of `shared` and returns a fresh clone of `config` with the lock from before.
fn moved_under(shared: &Repo, config: &str, files: &[(&str, &str)]) -> Repo {
    let first = Repo::new(config, files);
    assert!(first.fz(&[]).ok);
    let lock = std::fs::read_to_string(first.path("fearlezz.lock")).unwrap();
    move_tag(shared);
    let mut all = files.to_vec();
    all.push(("fearlezz.lock", &lock));
    let fresh = Repo::new(config, &all);
    assert!(!fresh.fz(&[]).ok);
    fresh
}

#[test]
fn update_accepts_a_moved_tag_of_a_piece_a_local_piece_names() {
    let shared = shared();
    let web = git_piece(&shared, "v1");
    let fresh = moved_under(
        &shared,
        "use = [\"./proof/web.toml\"]\n",
        &[("proof/web.toml", &web)],
    );
    let out = fresh.fz(&["setup", "update"]);
    assert!(out.ok, "{}", out.stderr);
    assert!(
        out.stdout.contains("now pins where its tag points"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("check.audit.run: added"),
        "{}",
        out.stdout
    );
    assert_eq!(checks(&fresh), "audit, lint, screens");
}

#[test]
fn update_accepts_a_moved_tag_of_a_piece_another_repository_names() {
    let shared = shared();
    let outer = Repo::new("", &[("fearlezz.toml", &git_piece(&shared, "v1"))]);
    outer.git(&["tag", "v1"]);
    let config = format!(
        "use = [\"git:file://{}//fearlezz.toml@v1\"]\n",
        outer.path("").display()
    );
    let fresh = moved_under(&shared, &config, &[]);
    let out = fresh.fz(&["setup", "update"]);
    assert!(out.ok, "{}", out.stderr);
    assert_eq!(checks(&fresh), "audit, lint, screens");
}

#[test]
fn update_works_when_the_old_pinned_commit_is_gone() {
    let shared = shared();
    let config = git_piece(&shared, "v1");
    let first = Repo::new(&config, &[]);
    assert!(first.fz(&[]).ok);
    let lock = std::fs::read_to_string(first.path("fearlezz.lock")).unwrap();
    // The release is amended and retagged, and the old commit is pruned.
    shared.write(
        "base/fearlezz.toml",
        "[check.lint]\nrun = \"false\"\nstage = \"working\"\n",
    );
    shared.git(&["add", "-A"]);
    shared.git(&["commit", "--quiet", "--amend", "--no-edit"]);
    shared.git(&["tag", "-f", "v1"]);
    shared.git(&["reflog", "expire", "--expire=now", "--all"]);
    shared.git(&["gc", "--quiet", "--prune=now"]);

    let fresh = Repo::new(&config, &[("fearlezz.lock", &lock)]);
    let out = fresh.fz(&["setup", "update"]);
    assert!(out.ok, "{}", out.stderr);
    assert!(out.stdout.contains("cannot be listed"), "{}", out.stdout);
    assert!(fresh.fz(&[]).ok);
}

#[test]
fn update_skips_prereleases_and_unrelated_tags() {
    let shared = shared();
    let repo = Repo::new(&git_piece(&shared, "v1"), &[]);
    assert!(repo.fz(&[]).ok);
    for tag in ["v1.1", "v1.1-rc1", "v2-rc1", "nightly", "release-9"] {
        shared.git(&["tag", tag]);
    }
    let out = repo.fz(&["setup", "update"]);
    assert!(out.ok, "{}", out.stderr);
    assert!(out.stdout.contains("v1 → v1.1\n"), "{}", out.stdout);
}

#[test]
fn ci_reads_the_lock_but_never_writes_it() {
    let shared = shared();
    let config = git_piece(&shared, "v1");
    let repo = Repo::new(&config, &[]);
    let out = repo.fz_env(&[], &[("CI", "true")]);
    assert!(!out.ok);
    assert!(
        out.stderr.contains("fearlezz.lock does not match"),
        "{}",
        out.stderr
    );
    assert!(!repo.path("fearlezz.lock").exists());

    // CI=false is a developer machine; with the lock written, CI reads it.
    assert!(repo.fz_env(&[], &[("CI", "false")]).ok);
    assert!(repo.fz_env(&[], &[("CI", "true")]).ok);
}

#[test]
fn worktrees_loading_a_new_piece_at_once_both_succeed() {
    let shared = shared();
    let repo = Repo::new(&git_piece(&shared, "v1"), &[]);
    let trees: Vec<_> = (0..3)
        .map(|i| repo.worktree(&format!("feat/{i}")))
        .collect();
    let results: Vec<common::Output> = std::thread::scope(|s| {
        let runs: Vec<_> = trees.iter().map(|wt| s.spawn(|| wt.fz(&[]))).collect();
        runs.into_iter().map(|r| r.join().unwrap()).collect()
    });
    for out in results {
        assert!(out.ok, "{}", out.stderr);
    }
}

#[test]
fn a_repository_url_cannot_pass_options_to_git() {
    let repo = Repo::new("use = [\"git:--upload-pack=touch x//f.toml@v1\"]\n", &[]);
    let out = repo.fz(&[]);
    assert!(!out.ok);
    assert!(out.stderr.contains("cannot start with -"), "{}", out.stderr);
}

#[test]
fn update_stays_in_the_major_and_says_when_a_bigger_one_is_out() {
    let shared = shared();
    let repo = Repo::new(&git_piece(&shared, "v1"), &[]);
    assert!(repo.fz(&[]).ok);
    for tag in ["v1.5", "v2.0", "v3.1"] {
        shared.git(&["tag", tag]);
    }
    let out = repo.fz(&["setup", "update"]);
    assert!(out.ok, "{}", out.stderr);
    assert!(out.stdout.contains("v1 → v1.5\n"), "{}", out.stdout);
    assert!(
        out.stdout.contains("v3.1 is out, a new major version"),
        "{}",
        out.stdout
    );
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    assert!(config.contains("@v1.5\""), "{config}");
    assert!(repo.fz(&["setup", "update"]).stdout.contains("v3.1 is out"));
}

/// The documented fake GitHub token from secret scanners' own test data. The AWS example key
/// is allowlisted by the scanner, so it cannot stand in here.
const FAKE_TOKEN: &str = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";

#[test]
fn the_safety_piece_gates_a_secret_on_a_changed_line_and_never_prints_it() {
    let repo = Repo::new(
        "use = [\"safety\"]\n",
        &[(
            "src/old.rs",
            &format!("const T: &str = \"{FAKE_TOKEN}\";\n"),
        )],
    );
    assert_eq!(checks(&repo), "secrets");
    repo.branch("feat/x");
    repo.fz(&["run", "--stage", "working", "--no-run"]);

    // A leak that was already committed does not block new work.
    repo.write("src/new.rs", "fn main() {}\n");
    let out = repo.fz(&["run", "secrets"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);

    // The same leak on a line this change adds does.
    repo.write(
        "src/new.rs",
        &format!("fn main() {{}}\nconst T: &str = \"{FAKE_TOKEN}\";\n"),
    );
    let out = repo.fz(&["run", "secrets"]);
    assert!(!out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("src/new.rs:2"), "{}", out.stdout);
    assert!(out.stdout.contains("github-pat"), "{}", out.stdout);
    assert!(!out.stdout.contains(FAKE_TOKEN), "{}", out.stdout);
}
