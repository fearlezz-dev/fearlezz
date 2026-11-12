//! The look at a change: stills and films compared with the same check run on the base.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::github::{Comment, FakeGitHub, PERSON_TOKEN, Status};
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

/// Pull request 7 on the fake GitHub at `head`, with a `/fearlezz approve ship it` comment by
/// `lead`, who has write access, and the `fearlezz/pictures` status on `status_on` set by
/// `creator` (login and type) to success, linked to that comment.
fn approved(head: &str, status_on: &str, creator: (&str, &str)) -> (FakeGitHub, Comment) {
    let gh = FakeGitHub::start();
    let mut state = gh.state();
    state.pulls.insert(7, (head.into(), "open".into()));
    state.permissions.insert("lead".into(), "write".into());
    let comment = state.add_comment(7, "lead", "User", "/fearlezz approve ship it");
    state.statuses.push(Status {
        sha: status_on.into(),
        state: "success".into(),
        context: "fearlezz/pictures".into(),
        description: "approved by lead".into(),
        target: comment.url(),
        creator: creator.0.into(),
        creator_kind: creator.1.into(),
    });
    drop(state);
    (gh, comment)
}

const ACTIONS: (&str, &str) = ("github-actions[bot]", "Bot");

/// Runs `fearlezz` on the developer's machine, with their token, against the fake GitHub.
fn with_gh(repo: &Repo, counter: &str, gh: &FakeGitHub, args: &[&str]) -> common::Output {
    repo.fz_env(
        args,
        &[
            ("COUNTER", counter),
            ("GITHUB_API_URL", &gh.url),
            ("GITHUB_TOKEN", PERSON_TOKEN),
        ],
    )
}

/// The work run and committed, so its head is what the pull request shows, with the
/// repository on GitHub as its `origin`.
fn committed() -> (Repo, String, String) {
    let (repo, counter) = repo();
    ran(&repo, &counter, &["run"]);
    repo.commit("the work");
    repo.git(&["remote", "add", "origin", "git@github.com:owner/repo.git"]);
    let head = repo.git(&["rev-parse", "HEAD"]).trim().to_string();
    (repo, counter, head)
}

const APPROVE: [&str; 4] = ["say", "approve", "screens", "--github"];

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
    let (gh, _) = approved(&head, &head, ACTIONS);
    let out = with_gh(&repo, &counter, &gh, &APPROVE);
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
fn an_approval_set_by_an_app_token_counts() {
    let (repo, counter, head) = committed();
    let (gh, _) = approved(&head, &head, ("proof-app[bot]", "Bot"));
    let out = with_gh(&repo, &counter, &gh, &APPROVE);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
}

#[test]
fn an_approval_for_another_commit_does_not_count() {
    let (repo, counter, _) = committed();
    let other = "0".repeat(40);
    let (gh, _) = approved(&other, &other, ACTIONS);
    let out = with_gh(&repo, &counter, &gh, &APPROVE);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stderr.contains("push"), "{}", out.stderr);
}

#[test]
fn an_approval_line_another_bot_wrote_does_not_count() {
    let (repo, counter, head) = committed();
    let gh = FakeGitHub::start();
    gh.state().pulls.insert(7, (head.clone(), "open".into()));
    let hidden = serde_json::json!({
        "head": head, "checks": ["screens"], "by": "lead", "said": "ship it",
    });
    gh.state().add_comment(
        7,
        "helper[bot]",
        "Bot",
        &format!("@lead approved the pictures.\n<!-- fearlezz-approved {hidden} -->"),
    );
    let out = with_gh(&repo, &counter, &gh, &APPROVE);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stderr.contains("/fearlezz approve"), "{}", out.stderr);
}

#[test]
fn a_status_a_persons_account_set_does_not_count() {
    let (repo, counter, head) = committed();
    let (gh, _) = approved(&head, &head, ("dev", "User"));
    let out = with_gh(&repo, &counter, &gh, &APPROVE);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stderr.contains("/fearlezz approve"), "{}", out.stderr);
}

#[test]
fn a_deleted_approval_does_not_count() {
    let (repo, counter, head) = committed();
    let (gh, comment) = approved(&head, &head, ACTIONS);
    gh.state().comments.retain(|c| c.id != comment.id);
    let out = with_gh(&repo, &counter, &gh, &APPROVE);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stderr.contains("deleted"), "{}", out.stderr);
}

#[test]
fn an_approval_by_someone_without_write_access_does_not_count() {
    let (repo, counter, head) = committed();
    let (gh, _) = approved(&head, &head, ACTIONS);
    gh.state().permissions.insert("lead".into(), "read".into());
    let out = with_gh(&repo, &counter, &gh, &APPROVE);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stderr.contains("write access"), "{}", out.stderr);
}

#[test]
fn approving_on_the_pull_request_needs_the_work_committed() {
    let (repo, counter) = repo();
    ran(&repo, &counter, &["run"]);
    let head = repo.git(&["rev-parse", "HEAD"]).trim().to_string();
    let (gh, _) = approved(&head, &head, ACTIONS);
    let out = with_gh(&repo, &counter, &gh, &APPROVE);
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
    let bases = repo.bases();
    let held = bases.join("holds").join(&first);
    let hold = std::fs::File::open(&held).unwrap();
    hold.lock_shared().unwrap();

    // Three newer bases would push it out, but another command still holds it.
    for n in 0..3 {
        advance(&repo, n);
        run(&repo);
    }
    let dir = bases.join("checkouts").join(&first);
    assert!(dir.exists(), "a held base was removed");

    drop(hold);
    advance(&repo, 3);
    run(&repo);
    assert!(
        !dir.exists(),
        "a base nobody holds stays after three newer ones"
    );
}

#[test]
fn the_base_checkout_lives_outside_the_repository() {
    // Tools refuse files below `.git` (Vite's dev server answers 403 there), so a base
    // checkout there would take no pictures.
    let (repo, counter) = repo();
    repo.write(
        "record.sh",
        &format!("case \"$PWD\" in */.git/*) exit 9;; esac\n{RECORD}"),
    );
    repo.commit("guard");
    repo.git(&["checkout", "--quiet", "main"]);
    repo.write(
        "record.sh",
        &format!("case \"$PWD\" in */.git/*) exit 9;; esac\n{RECORD}"),
    );
    repo.commit("guard");
    repo.git(&["checkout", "--quiet", "feat/blue"]);
    repo.git(&["merge", "--quiet", "--no-edit", "main"]);
    let out = repo.fz_env(&["run"], &[("COUNTER", &counter)]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("look screens: 1 changed, 1 new, 1 same"),
        "{}",
        out.stdout
    );
    let checkouts: Vec<_> = std::fs::read_dir(repo.bases().join("checkouts"))
        .unwrap()
        .collect();
    assert_eq!(checkouts.len(), 1, "one base checkout in the user cache");
    assert!(!repo.path(".git/fearlezz/base").exists());
}

#[test]
fn a_base_that_passed_without_pictures_is_called_suspicious() {
    let repo = Repo::new(
        CONFIG,
        &[
            (".gitignore", ".runs/\n"),
            ("record.sh", "echo run >> \"$COUNTER\"\n"),
            ("src/color.txt", "red"),
        ],
    );
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    let counter = repo.path(".runs/screens").to_string_lossy().to_string();
    repo.branch("feat/record");
    repo.write("record.sh", RECORD);
    repo.fz(&["run", "--stage", "look", "--no-run"]);
    let out = repo.fz_env(&["run"], &[("COUNTER", &counter)]);
    assert!(out.ok, "{}", out.stderr);
    assert!(out.stdout.contains("look screens: 2 new"), "{}", out.stdout);
    assert!(
        out.stdout
            .contains("the base run passed without any pictures"),
        "{}",
        out.stdout
    );
}

/// Films a tool writes into a folder, such as Playwright videos.
const WRITE_FILMS: &str = r#"set -e
mkdir -p "$FEARLEZZ_OUT/films/home"
ffmpeg -v error -nostdin -f lavfi -i "color=c=white:s=64x48:r=10:d=1" -vf "drawbox=x=$(cat src/x.txt):y=10:w=20:h=20:color=blue:t=fill" "$FEARLEZZ_OUT/films/home/video.gif"
"#;

#[test]
fn films_can_be_named_by_a_glob_in_the_proof_folder() {
    let repo = Repo::new(
        r#"
[check.screens]
run = "sh record.sh"
stage = "look"
inputs = ["src/**", "record.sh"]
films = "films/**/*.gif"
"#,
        &[("record.sh", WRITE_FILMS), ("src/x.txt", "5")],
    );
    repo.branch("feat/move");
    repo.write("src/x.txt", "35");
    repo.fz(&["run", "--stage", "look", "--no-run"]);
    let out = repo.fz(&["run"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("look screens: 1 changed"),
        "{}",
        out.stdout
    );
    let status = repo.fz(&["--check", "screens"]);
    assert!(
        status.stdout.contains("\"id\": \"home/video\"")
            && status.stdout.contains("\"kind\": \"film\""),
        "{}",
        status.stdout
    );
}

#[test]
fn flaky_pictures_are_shown_but_never_wait_for_approval() {
    let repo = Repo::new(
        &format!("{CONFIG}flaky = [\"home.png\"]\n"),
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
    repo.commit("blue");
    ran(&repo, &counter, &["run", "--stage", "merge", "--no-run"]);
    let out = ran(&repo, &counter, &["run"]);
    assert!(
        out.stdout.contains("look screens: 1 same, 1 flaky"),
        "{}",
        out.stdout
    );
    assert_eq!(
        out.last_line(),
        "next: all proof is in: ready to merge",
        "{}",
        out.stdout
    );
    let report = out
        .lines()
        .iter()
        .find_map(|l| l.trim().strip_prefix("report: "))
        .map(std::path::PathBuf::from)
        .expect("a report line");
    let html = std::fs::read_to_string(report).unwrap();
    assert!(html.contains("Flaky"), "the report says which are flaky");
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

/// The report page a run printed.
fn report_of(out: &common::Output) -> String {
    let path = out
        .lines()
        .iter()
        .find_map(|l| l.trim().strip_prefix("report: "))
        .map(std::path::PathBuf::from)
        .expect("a report line");
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn the_report_is_one_page_to_review_and_approve_from() {
    let (repo, counter) = repo();
    let html = report_of(&ran(&repo, &counter, &["run"]));
    // Counts that filter, one per kind of change there is.
    for kind in ["changed", "new", "same"] {
        assert!(
            html.contains(&format!("data-filter=\"{kind}\"")),
            "{kind}: {html}"
        );
    }
    assert!(html.contains("data-name=\"home.png\" data-change=\"changed\""));
    // Ways to compare a before with its after.
    for mode in ["side", "slider", "onion", "diff", "blink"] {
        assert!(html.contains(&format!("data-mode=\"{mode}\"")), "{mode}");
    }
    // A decision per picture, and approval built from the developer's words.
    for word in ["Looks right", "Not right", "Flaky"] {
        assert!(html.contains(word), "{word}");
    }
    assert!(html.contains("fearlezz say approve screens --said"));
}

#[test]
fn the_report_needs_nothing_from_the_network_and_keeps_to_the_house_style() {
    let (repo, counter) = repo();
    let html = report_of(&ran(&repo, &counter, &["run"]));
    assert!(!html.contains("http://") && !html.contains("https://"));
    assert!(html.contains("prefers-reduced-motion"));
    assert!(html.contains("prefers-color-scheme: dark"));
    // Borders go all the way round or not at all.
    for side in ["border-left", "border-right", "border-top", "border-bottom"] {
        assert!(!html.contains(side), "{side}");
    }
}

#[test]
fn the_report_groups_by_the_parts_of_a_picture_name() {
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
    repo.fz(&["run", "--stage", "look", "--no-run"]);
    let html = report_of(&repo.fz(&["run"]));
    assert!(html.contains("data-group=\"viewport\""), "{html}");
    assert!(html.contains("data-viewport=\"desktop\""), "{html}");
    assert!(html.contains("data-viewport=\"mobile\""), "{html}");
}

/// A film of a box that appears after a second, at a place the source names.
const RECORD_FILM: &str = r#"set -e
ffmpeg -v error -nostdin -f lavfi -i "color=c=white:s=160x120:r=10:d=2" \
  -vf "drawbox=x=$(cat src/x.txt):y=30:w=50:h=40:color=blue:t=fill:enable='gte(t,1)'" \
  -pix_fmt yuv420p "$FEARLEZZ_OUT/flow.mp4"
echo '{"kind":"film","file":"flow.mp4"}' >> "$FEARLEZZ_OUT/proof.jsonl"
"#;

#[test]
fn a_changed_film_plays_both_sides_on_one_clock_with_lanes_of_where_they_differ() {
    let repo = Repo::new(
        r#"
[check.flow]
run = "sh record.sh"
stage = "look"
inputs = ["src/**", "record.sh"]
"#,
        &[("record.sh", RECORD_FILM), ("src/x.txt", "20")],
    );
    repo.branch("feat/move");
    repo.write("src/x.txt", "90");
    repo.fz(&["run", "--stage", "look", "--no-run"]);
    let out = repo.fz(&["run"]);
    assert!(
        out.stdout.contains("look flow: 1 changed"),
        "{}",
        out.stdout
    );
    let html = report_of(&out);
    assert!(html.contains("class=\"film\""), "{html}");
    assert_eq!(html.matches("<video").count(), 2, "{html}");
    assert!(html.contains("class=\"lane before\""), "{html}");
    assert!(
        html.contains("class=\"diff\""),
        "a differing stretch: {html}"
    );
    assert!(html.contains("data-seek="), "a jump to the change: {html}");
}
