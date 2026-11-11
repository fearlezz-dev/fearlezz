//! The look at a change: stills and films compared with the same check run on the base.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::{Repo, runs};

const RECORD: &str = r#"set -e
echo run >> "$COUNTER"
still() {
  ffmpeg -v error -nostdin -f lavfi -i "color=c=$2:s=64x48" -frames:v 1 "$FEARLEZZ_OUT/$1.png"
  echo "{\"kind\":\"still\",\"file\":\"$1.png\"}" >> "$FEARLEZZ_OUT/proof.jsonl"
}
still home "$(cat src/color.txt)"
still about white
if [ -e src/extra ]; then still extra green; fi
"#;

const CONFIG: &str = r#"
[check.screens]
run = "sh record.sh"
stage = "look"
inputs = ["src/**", "record.sh"]
env = ["COUNTER"]
"#;

fn repo() -> (Repo, String) {
    let repo = Repo::new(
        CONFIG,
        &[
            (".gitignore", ".runs/\n"),
            ("record.sh", RECORD),
            ("src/color.txt", "red"),
        ],
    );
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    let counter = repo.path(".runs/screens").to_string_lossy().to_string();
    repo.branch("feat/blue");
    repo.write("src/color.txt", "blue");
    repo.write("src/extra", "");
    repo.fz(&["run", "--stage", "look", "--no-run"]);
    (repo, counter)
}

#[test]
fn run_compares_stills_with_the_base_and_writes_a_report() {
    let (repo, counter) = repo();
    let out = repo.fz_env(&["run"], &[("COUNTER", &counter)]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("look screens: 1 changed, 1 new, 1 same"),
        "{}",
        out.stdout
    );
    let report = out
        .lines()
        .iter()
        .find_map(|l| l.trim().strip_prefix("report: "))
        .map(std::path::PathBuf::from)
        .expect("a report line");
    let html = std::fs::read_to_string(&report).unwrap();
    assert!(html.contains("home") && html.contains("extra"), "{html}");
    assert!(
        html.contains("class=\"mark\""),
        "the changed area is outlined"
    );
    // The check ran once here and once on the base.
    assert_eq!(runs(std::path::Path::new(&counter)), 2);
}

#[test]
fn the_base_run_and_the_comparison_are_reused() {
    let (repo, counter) = repo();
    repo.fz_env(&["run"], &[("COUNTER", &counter)]);
    let again = repo.fz_env(&["run"], &[("COUNTER", &counter)]);
    assert!(
        again.stdout.contains("look screens: 1 changed"),
        "{}",
        again.stdout
    );
    assert_eq!(runs(std::path::Path::new(&counter)), 2);
}

#[test]
fn status_and_next_point_at_the_report() {
    let (repo, counter) = repo();
    repo.fz_env(&["run"], &[("COUNTER", &counter)]);
    let status = repo.fz_env(&[], &[("COUNTER", &counter)]);
    assert!(
        status.stdout.contains("look screens: 1 changed"),
        "{}",
        status.stdout
    );
    assert!(
        status.last_line().starts_with("next: look at screens"),
        "{}",
        status.stdout
    );
    assert!(
        status.last_line().contains("fearlezz say approve screens"),
        "{}",
        status.stdout
    );
    assert!(
        status.stdout.contains("needs the developer's approval"),
        "{}",
        status.stdout
    );
}

#[test]
fn a_check_that_fails_on_the_base_says_there_is_no_before() {
    let repo = Repo::new(
        CONFIG,
        &[
            (".gitignore", ".runs/\n"),
            ("record.sh", "exit 3\n"),
            ("src/color.txt", "red"),
        ],
    );
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    let counter = repo.path(".runs/screens").to_string_lossy().to_string();
    repo.branch("feat/fix-recorder");
    repo.write("record.sh", RECORD);
    repo.fz(&["run", "--stage", "look", "--no-run"]);
    let out = repo.fz_env(&["run"], &[("COUNTER", &counter)]);
    assert!(out.ok, "{}", out.stderr);
    assert!(
        out.stdout
            .contains("no before (the check fails on the base commit"),
        "{}",
        out.stdout
    );
    // Without a before every picture is new, so nobody merges them unseen.
    assert!(out.stdout.contains("2 new"), "{}", out.stdout);
    let status = repo.fz_env(&[], &[("COUNTER", &counter)]);
    assert!(
        status.last_line().starts_with("next: look at screens"),
        "{}",
        status.stdout
    );
}

fn ran(repo: &Repo, counter: &str, args: &[&str]) -> common::Output {
    repo.fz_env(args, &[("COUNTER", counter)])
}

#[test]
fn approving_needs_the_developers_words() {
    let (repo, counter) = repo();
    ran(&repo, &counter, &["run"]);
    assert!(!ran(&repo, &counter, &["say", "approve", "screens"]).ok);
    let empty = ran(
        &repo,
        &counter,
        &["say", "approve", "screens", "--said", "  "],
    );
    assert!(!empty.ok);
    assert!(empty.stderr.contains("words"), "{}", empty.stderr);
}

#[test]
fn merge_waits_for_approval_of_changed_pictures() {
    let (repo, counter) = repo();
    ran(&repo, &counter, &["run", "--stage", "merge", "--no-run"]);
    let out = ran(&repo, &counter, &["run"]);
    assert!(!out.ok, "merge is not proven yet: {}", out.stdout);
    assert!(
        out.last_line()
            .contains("fearlezz say approve screens --said"),
        "{}",
        out.stdout
    );
}

#[test]
fn an_approval_survives_the_fresh_run_at_merge() {
    let (repo, counter) = repo();
    ran(&repo, &counter, &["run"]);
    let ok = ran(
        &repo,
        &counter,
        &["say", "approve", "screens", "--said", "looks good"],
    );
    assert!(ok.ok, "{}", ok.stderr);
    assert!(ok.stdout.contains("approved screens"), "{}", ok.stdout);
    let status = ran(&repo, &counter, &[]);
    assert!(status.stdout.contains("approved"), "{}", status.stdout);
    repo.commit("the work");
    ran(&repo, &counter, &["run", "--stage", "merge", "--no-run"]);
    let out = ran(&repo, &counter, &["run"]);
    assert_eq!(
        out.last_line(),
        "next: all proof is in: ready to merge",
        "{}",
        out.stdout
    );
}

#[test]
fn changing_the_pictures_again_drops_the_approval() {
    let (repo, counter) = repo();
    ran(&repo, &counter, &["run"]);
    ran(
        &repo,
        &counter,
        &["say", "approve", "screens", "--said", "looks good"],
    );
    repo.write("src/color.txt", "green");
    let out = ran(&repo, &counter, &["run"]);
    assert!(
        out.last_line().contains("fearlezz say approve screens"),
        "{}",
        out.stdout
    );
}

#[test]
fn there_is_nothing_to_approve_without_changed_pictures() {
    let (repo, counter) = repo();
    let out = ran(
        &repo,
        &counter,
        &["say", "approve", "screens", "--said", "looks good"],
    );
    assert!(!out.ok);
    assert!(out.stderr.contains("nothing to approve"), "{}", out.stderr);
}

// Approvals given on the pull request

/// Stands in for the GitHub CLI: pull request 7 of the current branch, whose comments are the
/// JSON in `.runs/comments.json`, filtered with the real jq as `gh --jq` would.
const FAKE_GH: &str = r#"#!/bin/sh
case "$1 $2" in
  "pr view") echo 7 ;;
  "api repos/{owner}/{repo}/issues/7/comments")
    while [ "$#" -gt 0 ]; do
      if [ "$1" = --jq ]; then jq -c "$2" "$GH_COMMENTS"; exit; fi
      shift
    done
    cat "$GH_COMMENTS" ;;
  *) echo "unexpected gh $*" >&2; exit 1 ;;
esac
"#;

/// The comments on the pull request: one approval by `author` (of `kind`, `Bot` or `User`) for
/// `head`, covering `screens`.
fn comments(head: &str, author: &str, kind: &str) -> String {
    let hidden = serde_json::json!({
        "head": head,
        "fingerprint": "f00d",
        "checks": ["screens"],
        "by": "lead",
        "said": "ship it",
    });
    serde_json::json!([
        { "user": { "login": "lead", "type": "User" }, "body": "/fearlezz approve ship it",
          "html_url": "https://github.com/o/r/pull/7#issuecomment-1" },
        { "user": { "login": author, "type": kind },
          "body": format!("@lead approved the pictures.\n<!-- fearlezz-approved {hidden} -->"),
          "html_url": "https://github.com/o/r/pull/7#issuecomment-2" },
    ])
    .to_string()
}

/// Runs `fearlezz` with the fake GitHub CLI answering for `comments`.
fn with_gh(repo: &Repo, counter: &str, comments: &str, args: &[&str]) -> common::Output {
    repo.write(".runs/bin/gh", FAKE_GH);
    let gh = repo.path(".runs/bin/gh");
    let mut perms = std::fs::metadata(&gh).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut perms, 0o755);
    std::fs::set_permissions(&gh, perms).unwrap();
    repo.write(".runs/comments.json", comments);
    let path = format!(
        "{}:{}",
        repo.path(".runs/bin").display(),
        std::env::var("PATH").unwrap()
    );
    let file = repo.path(".runs/comments.json");
    repo.fz_env(
        args,
        &[
            ("COUNTER", counter),
            ("PATH", &path),
            ("GH_COMMENTS", file.to_str().unwrap()),
        ],
    )
}

/// The work run and committed, so its head is what the pull request shows.
fn committed() -> (Repo, String, String) {
    let (repo, counter) = repo();
    ran(&repo, &counter, &["run"]);
    repo.commit("the work");
    let head = repo.git(&["rev-parse", "HEAD"]).trim().to_string();
    (repo, counter, head)
}

#[test]
fn the_next_line_names_approving_on_the_pull_request() {
    let (repo, counter, _) = committed();
    let status = ran(&repo, &counter, &[]);
    assert!(
        status
            .last_line()
            .contains("fearlezz say approve screens --github"),
        "{}",
        status.stdout
    );
}

#[test]
fn an_approval_on_the_pull_request_counts_with_who_gave_it() {
    let (repo, counter, head) = committed();
    let out = with_gh(
        &repo,
        &counter,
        &comments(&head, "github-actions[bot]", "Bot"),
        &["say", "approve", "screens", "--github"],
    );
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("approved screens on GitHub by @lead (\"ship it\")"),
        "{}",
        out.stdout
    );
    let status = ran(&repo, &counter, &[]);
    assert!(
        status
            .stdout
            .contains("approved on GitHub by @lead (\"ship it\")"),
        "{}",
        status.stdout
    );
    ran(&repo, &counter, &["run", "--stage", "merge", "--no-run"]);
    let out = ran(&repo, &counter, &["run"]);
    assert_eq!(
        out.last_line(),
        "next: all proof is in: ready to merge",
        "{}",
        out.stdout
    );
}

#[test]
fn an_approval_for_another_commit_does_not_count() {
    let (repo, counter, _) = committed();
    let other = "0".repeat(40);
    let out = with_gh(
        &repo,
        &counter,
        &comments(&other, "github-actions[bot]", "Bot"),
        &["say", "approve", "screens", "--github"],
    );
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stderr.contains("0000000"), "{}", out.stderr);
    assert!(out.stderr.contains("push"), "{}", out.stderr);
}

#[test]
fn an_approval_line_a_person_wrote_does_not_count() {
    let (repo, counter, head) = committed();
    let out = with_gh(
        &repo,
        &counter,
        &comments(&head, "lead", "User"),
        &["say", "approve", "screens", "--github"],
    );
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stderr.contains("/fearlezz approve"), "{}", out.stderr);
}

#[test]
fn approving_on_the_pull_request_needs_the_work_committed() {
    let (repo, counter) = repo();
    ran(&repo, &counter, &["run"]);
    let head = repo.git(&["rev-parse", "HEAD"]).trim().to_string();
    let out = with_gh(
        &repo,
        &counter,
        &comments(&head, "github-actions[bot]", "Bot"),
        &["say", "approve", "screens", "--github"],
    );
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stderr.contains("commit"), "{}", out.stderr);
}

#[test]
fn a_base_another_command_uses_is_not_removed() {
    let (repo, counter) = repo();
    repo.commit("blue");
    let base = |repo: &Repo| repo.git(&["merge-base", "HEAD", "main"])[..12].to_string();
    let run = |repo: &Repo| {
        let out = repo.fz_env(&["run"], &[("COUNTER", &counter)]);
        assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    };
    // Moves the base: a new commit on main, merged into the branch.
    let advance = |repo: &Repo, n: usize| {
        repo.git(&["checkout", "--quiet", "main"]);
        repo.write(&format!("src/n{n}.txt"), "x");
        repo.commit("more");
        repo.git(&["checkout", "--quiet", "feat/blue"]);
        repo.git(&["merge", "--quiet", "--no-edit", "main"]);
    };
    run(&repo);
    let first = base(&repo);
    let held = repo.path(&format!(".git/fearlezz/base-holds/{first}"));
    let hold = std::fs::File::open(&held).unwrap();
    hold.lock_shared().unwrap();

    // Three newer bases would push it out, but another command still holds it.
    for n in 0..3 {
        advance(&repo, n);
        run(&repo);
    }
    let dir = repo.path(&format!(".git/fearlezz/base/{first}"));
    assert!(dir.exists(), "a held base was removed");

    drop(hold);
    advance(&repo, 3);
    run(&repo);
    assert!(
        !dir.exists(),
        "a base nobody holds stays after three newer ones"
    );
}

/// Stands in for Playwright: `toHaveScreenshot` writes each still under the snapshot path the
/// project's config points into `$FEARLEZZ_OUT`, and the JUnit reporter counts the tests.
const FAKE_PLAYWRIGHT: &str = r#"#!/bin/sh
set -e
echo "$*" >> "$FEARLEZZ_OUT/args"
mkdir -p "$FEARLEZZ_OUT/stills/desktop"
ffmpeg -v error -nostdin -f lavfi -i "color=c=$(cat src/color.css):s=64x48" -frames:v 1 "$FEARLEZZ_OUT/stills/desktop/home.png"
echo '<testsuites><testsuite tests="1"><testcase name="home"/></testsuite></testsuites>' > "$PLAYWRIGHT_JUNIT_OUTPUT_FILE"
"#;

#[test]
fn the_playwright_piece_compares_the_stills_playwright_writes() {
    use std::os::unix::fs::PermissionsExt as _;
    let repo = Repo::new(
        "use = [\"playwright\"]\n",
        &[
            (".gitignore", "node_modules/\n"),
            ("src/color.css", "red"),
            ("package-lock.json", "{}"),
        ],
    );
    // Installed in the main checkout only, as npm does; the piece clones it into the base.
    repo.write("node_modules/.bin/playwright", FAKE_PLAYWRIGHT);
    std::fs::set_permissions(
        repo.path("node_modules/.bin/playwright"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    repo.branch("feat/blue");
    repo.write("src/color.css", "blue");
    repo.fz(&["run", "--stage", "look", "--no-run"]);
    let out = repo.fz(&["run"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("look playwright: 1 changed"),
        "{}",
        out.stdout
    );
    let status = repo.fz(&["--check", "playwright"]);
    assert!(status.stdout.contains("\"tests\": 1"), "{}", status.stdout);
    assert!(
        status.stdout.contains("\"id\": \"desktop/home\""),
        "{}",
        status.stdout
    );
}

/// A tool that writes its pictures into a folder (Playwright's `toHaveScreenshot` with its
/// snapshot path in `$FEARLEZZ_OUT`) needs no `proof.jsonl`: `stills` names them by glob.
const WRITE_FILES: &str = r#"set -e
still() {
  mkdir -p "$(dirname "$FEARLEZZ_OUT/stills/$1")"
  ffmpeg -v error -nostdin -f lavfi -i "color=c=$2:s=64x48" -frames:v 1 "$FEARLEZZ_OUT/stills/$1.png"
}
still desktop/home "$(cat src/color.txt)"
still mobile/home white
if [ -e src/extra ]; then still desktop/extra green; fi
"#;

#[test]
fn stills_can_be_named_by_a_glob_in_the_proof_folder() {
    let repo = Repo::new(
        r#"
[check.screens]
run = "sh record.sh"
stage = "look"
inputs = ["src/**", "record.sh"]
stills = "stills/**/*.png"
"#,
        &[("record.sh", WRITE_FILES), ("src/color.txt", "red")],
    );
    repo.branch("feat/blue");
    repo.write("src/color.txt", "blue");
    repo.write("src/extra", "");
    repo.fz(&["run", "--stage", "look", "--no-run"]);
    let out = repo.fz(&["run"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("look screens: 1 changed, 1 new, 1 same"),
        "{}",
        out.stdout
    );
    let status = repo.fz(&["--check", "screens"]);
    // Named by the path below the glob's fixed part, without the extension.
    for id in ["desktop/home", "mobile/home", "desktop/extra"] {
        assert!(
            status.stdout.contains(&format!("\"id\": \"{id}\"")),
            "{id}: {}",
            status.stdout
        );
    }
}
