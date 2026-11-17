//! Commit messages: the `commits` piece turns on the built in `commits` check, which holds a
//! change to Conventional Commits. In CI the pull request title is judged, since with a squash
//! merge it is the commit; locally every commit since the base is.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;

/// A repository on the `commits` piece, on a branch with one conventional commit.
fn repo() -> Repo {
    let repo = Repo::new("use = [\"commits\"]\n", &[("src/a.txt", "1\n")]);
    repo.branch("feat/x");
    repo.write("src/a.txt", "2\n");
    repo.commit("feat: a is two");
    repo
}

#[test]
fn every_commit_since_the_base_is_a_conventional_commit() {
    let repo = repo();
    let out = repo.fz(&["run", "commits"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);

    repo.write("src/a.txt", "3\n");
    repo.commit("Update a");
    let out = repo.fz(&["run", "commits"]);
    assert!(!out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("commit \"Update a\" ("),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("feat: "), "{}", out.stdout);
}

#[test]
fn a_reworded_commit_is_judged_again() {
    // Rewording keeps the tree, so only the messages tell the two runs apart.
    let repo = repo();
    repo.write("src/a.txt", "3\n");
    repo.commit("Update a");
    assert!(!repo.fz(&["run", "commits"]).ok);
    repo.git(&["commit", "-q", "--amend", "-m", "fix: a is three"]);
    let out = repo.fz(&["run", "commits"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
}

#[test]
fn revert_commits_scopes_and_breaking_marks_pass() {
    let repo = repo();
    for subject in [
        "fix(parser)!: a breaking fix",
        "chore(deps): bump",
        "Revert \"feat: a is two\"",
    ] {
        repo.write("src/a.txt", &format!("{subject}\n"));
        repo.commit(subject);
    }
    let out = repo.fz(&["run", "commits"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);

    // An unknown type is no conventional commit.
    repo.write("src/a.txt", "4\n");
    repo.commit("feature: four");
    let out = repo.fz(&["run", "commits"]);
    assert!(!out.ok, "{}\n{}", out.stdout, out.stderr);
}

#[test]
fn in_ci_the_pull_request_title_is_the_commit() {
    // An interim commit with a plain subject is fine when the title is conventional, and a
    // plain title fails though every commit is conventional.
    let repo = repo();
    repo.write("src/a.txt", "3\n");
    repo.commit("review comments");
    let dir = tempfile::tempdir().unwrap();
    let event = dir.path().join("event.json");
    let ci = |title: &str| {
        std::fs::write(
            &event,
            format!(r#"{{"pull_request": {{"title": "{title}"}}}}"#),
        )
        .unwrap();
        repo.fz_env(
            &["run", "--ci"],
            &[("GITHUB_EVENT_PATH", event.to_str().unwrap())],
        )
    };
    let out = ci("feat: a is three");
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let out = ci("A is three");
    assert!(!out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("pull request title \"A is three\" is not a conventional commit"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_project_names_its_own_types() {
    let repo = Repo::new(
        "use = [\"commits\"]\n\n[commits]\ntypes = [\"feat\", \"fix\", \"wip\"]\n",
        &[("src/a.txt", "1\n")],
    );
    repo.branch("feat/x");
    repo.write("src/a.txt", "2\n");
    repo.commit("wip: halfway");
    assert!(repo.fz(&["run", "commits"]).ok);
    repo.write("src/a.txt", "3\n");
    repo.commit("chore: tidy");
    assert!(!repo.fz(&["run", "commits"]).ok);
}

#[test]
fn fixup_commits_an_autosquash_folds_away_are_left_alone() {
    let repo = repo();
    repo.write("src/a.txt", "3\n");
    repo.commit("fixup! feat: a is two");
    let out = repo.fz(&["run", "commits"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
}

#[test]
fn a_project_turns_off_the_check_a_piece_brings() {
    let repo = Repo::new(
        "use = [\"commits\"]\n\n[commits]\noff = true\n",
        &[("src/a.txt", "1\n")],
    );
    let out = repo.fz(&["run", "commits"]);
    assert!(!out.ok);
    assert!(
        out.stderr.contains("no check \"commits\""),
        "{}",
        out.stderr
    );
}
