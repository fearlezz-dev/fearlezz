//! The look at a change: stills and films compared with the same check run on the base.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use std::path::PathBuf;

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

/// The before and after files a run or status names for picture `name`, when it names them.
fn files_of(out: &common::Output, name: &str) -> (Option<PathBuf>, Option<PathBuf>) {
    let lines = out.lines();
    let at = lines
        .iter()
        .position(|l| l.trim_start().starts_with(&format!("{name}:")))
        .unwrap_or_else(|| panic!("no line for {name}: {}", out.stdout));
    let mut files = (None, None);
    for line in lines.iter().skip(at + 1) {
        let line = line.trim();
        if let Some(path) = line.strip_prefix("before: ") {
            files.0 = Some(PathBuf::from(path));
        } else if let Some(path) = line.strip_prefix("after: ") {
            files.1 = Some(PathBuf::from(path));
        } else {
            break;
        }
    }
    files
}

#[test]
fn run_compares_stills_with_the_base_and_names_their_files() {
    let (repo, counter) = repo();
    let out = repo.fz_env(&["run"], &[("COUNTER", &counter)]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("look screens: 1 changed, 1 new, 1 same"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("home.png: changed"), "{}", out.stdout);
    let (before, after) = files_of(&out, "home.png");
    let (before, after) = (before.expect("a before"), after.expect("an after"));
    assert!(before.is_file() && after.is_file(), "{}", out.stdout);
    assert_ne!(
        std::fs::read(&before).unwrap(),
        std::fs::read(&after).unwrap()
    );
    let (none, extra) = files_of(&out, "extra.png");
    assert!(
        none.is_none(),
        "a new picture has no before: {}",
        out.stdout
    );
    assert!(extra.unwrap().is_file(), "{}", out.stdout);
    // Pictures that stay the same are counted, not listed.
    assert!(!out.stdout.contains("about.png:"), "{}", out.stdout);
    // Only the host has a page.
    assert!(!out.stdout.contains("report"), "{}", out.stdout);
    assert!(
        !after
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("index.html")
            .exists()
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
fn a_broken_look_needs_a_new_comparison_instead_of_reading_ok() {
    let (repo, counter) = repo();
    let out = repo.fz_env(&["run"], &[("COUNTER", &counter)]);
    let after = files_of(&out, "home.png").1.expect("an after");
    // A look written in part, as a crash while writing it leaves it.
    let look = after.parent().unwrap().parent().unwrap().join("look.json");
    std::fs::write(&look, "{\"check\":").unwrap();
    let status = repo.fz_env(&[], &[("COUNTER", &counter)]);
    assert!(
        status
            .stdout
            .contains("not compared with this base commit yet"),
        "{}",
        status.stdout
    );
    let again = repo.fz_env(&["run"], &[("COUNTER", &counter)]);
    assert!(
        again.stdout.contains("look screens: 1 changed"),
        "{}",
        again.stdout
    );
}

#[test]
fn status_names_the_changed_pictures_with_their_files() {
    let (repo, counter) = repo();
    repo.fz_env(&["run"], &[("COUNTER", &counter)]);
    let status = repo.fz_env(&[], &[("COUNTER", &counter)]);
    assert!(
        status
            .stdout
            .contains("look screens: 1 changed, 1 new, 1 same"),
        "{}",
        status.stdout
    );
    let (before, after) = files_of(&status, "home.png");
    assert!(before.unwrap().is_file(), "{}", status.stdout);
    assert!(after.unwrap().is_file(), "{}", status.stdout);
    assert!(!status.stdout.contains("approv"), "{}", status.stdout);
    assert_eq!(
        status.last_line(),
        "next: when ready: fearlezz run --stage merge",
        "{}",
        status.stdout
    );
}

#[test]
fn the_final_stage_is_ready_with_changed_pictures_and_names_them() {
    let (repo, counter) = repo();
    repo.commit("blue");
    ran(&repo, &counter, &["run", "--stage", "merge", "--no-run"]);
    let out = ran(&repo, &counter, &["run"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("2 pictures changed (screens): a person reviews them on the pull request"),
        "{}",
        out.stdout
    );
    assert_eq!(
        out.last_line(),
        "next: all proof is in: ready to merge",
        "{}",
        out.stdout
    );
    let status = ran(&repo, &counter, &[]);
    assert!(
        status
            .stdout
            .contains("2 pictures changed (screens): a person reviews them on the pull request"),
        "{}",
        status.stdout
    );
    assert_eq!(status.last_line(), "next: all proof is in: ready to merge");
}

#[test]
fn say_approve_is_gone() {
    let (repo, counter) = repo();
    ran(&repo, &counter, &["run"]);
    for args in [
        vec!["say", "approve", "screens", "--said", "looks good"],
        vec!["say", "approve", "screens", "--github"],
    ] {
        assert!(!ran(&repo, &counter, &args).ok, "{args:?}");
    }
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
    let (before, after) = files_of(&status, "home.png");
    assert!(before.is_none(), "{}", status.stdout);
    assert!(after.unwrap().is_file(), "{}", status.stdout);
}

fn ran(repo: &Repo, counter: &str, args: &[&str]) -> common::Output {
    repo.fz_env(args, &[("COUNTER", counter)])
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
fn flaky_pictures_are_shown_apart() {
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
    assert!(
        out.stdout.contains("home.png (flaky): changed"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("pictures changed"), "{}", out.stdout);
    assert_eq!(
        out.last_line(),
        "next: all proof is in: ready to merge",
        "{}",
        out.stdout
    );
}

/// Stands in for Playwright: `toHaveScreenshot` writes each still under the snapshot path the
/// project's config points into `$FEARLEZZ_OUT`, and the JUnit reporter counts the tests.
const FAKE_PLAYWRIGHT: &str = r#"#!/bin/sh
set -e
echo "$*" >> "$FEARLEZZ_OUT/args"
mkdir -p "$FEARLEZZ_OUT/stills/desktop"
ffmpeg -v error -nostdin -f lavfi -i "color=c=$(cat src/color.css):s=64x48" -frames:v 1 "$FEARLEZZ_OUT/stills/desktop/home.png"
# The junit reporter's outputFile, as the piece's playwright.config lines set it.
echo '<testsuites><testsuite tests="1"><testcase name="home"/></testsuite></testsuites>' > "$FEARLEZZ_OUT/junit.xml"
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
    // A plain tool command: the reporters live in playwright.config, not in shell syntax.
    let result: serde_json::Value = serde_json::from_str(&status.stdout).unwrap();
    let args = result["dir"]
        .as_str()
        .map(|dir| std::fs::read_to_string(std::path::Path::new(dir).join("args")).unwrap());
    assert_eq!(
        args.as_deref(),
        Some("test --update-snapshots=all\n"),
        "{}",
        status.stdout
    );
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

/// The stills check of `stills_can_be_named_by_a_glob_in_the_proof_folder`, with `stills`.
fn stills_config(stills: &str) -> String {
    format!(
        "\n[check.screens]\nrun = \"sh record.sh\"\nstage = \"look\"\ninputs = [\"src/**\", \
         \"record.sh\"]\n{stills}\n"
    )
}

/// A repository whose check writes stills named by `stills/**/*.png`, on a branch.
fn stills_repo() -> Repo {
    let repo = Repo::new(
        &stills_config("stills = \"stills/**/*.png\""),
        &[("record.sh", WRITE_FILES), ("src/color.txt", "red")],
    );
    repo.branch("feat/change");
    repo
}

/// `fearlezz.toml` with `stills` as given, as `Repo::new` writes it.
fn write_config(repo: &Repo, stills: &str) {
    repo.write(
        "fearlezz.toml",
        &format!(
            "{}\n[settings.skill]\nagents = []\n\n[settings.hooks]\nagents = []\n",
            stills_config(stills)
        ),
    );
}

#[test]
fn changing_only_the_stills_glob_runs_the_check_again() {
    let repo = stills_repo();
    repo.write("src/color.txt", "blue");
    repo.fz(&["run", "--stage", "look", "--no-run"]);
    let first = repo.fz(&["run"]);
    assert!(first.ok, "{}\n{}", first.stdout, first.stderr);
    write_config(&repo, "stills = \"stills/desktop/*.png\"");
    let out = repo.fz(&["run"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains(", 0 reused"), "{}", out.stdout);
    let status = repo.fz(&["--check", "screens"]);
    assert!(
        !status.stdout.contains("\"id\": \"mobile/home\""),
        "{}",
        status.stdout
    );
}

#[test]
fn a_change_that_removes_every_picture_shows_them_gone() {
    let repo = stills_repo();
    repo.write("record.sh", "true\n");
    repo.fz(&["run", "--stage", "look", "--no-run"]);
    let out = repo.fz(&["run"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("look screens: 2 gone"),
        "{}",
        out.stdout
    );
    let status = repo.fz(&[]);
    let (before, after) = files_of(&status, "desktop/home");
    assert!(before.unwrap().is_file(), "{}", status.stdout);
    assert!(after.is_none(), "{}", status.stdout);
}

#[test]
fn a_check_without_pictures_whose_base_cannot_run_says_nothing_of_a_look() {
    let repo = Repo::new(
        &format!(
            "{}\n[env]\nprepare = \"test ! -e src/broken\"\n",
            stills_config("stills = \"stills/**/*.png\"")
        ),
        &[
            ("record.sh", "true\n"),
            ("src/color.txt", "red"),
            ("src/broken", "\n"),
        ],
    );
    repo.branch("feat/change");
    repo.remove("src/broken");
    repo.write("src/color.txt", "blue");
    repo.fz(&["run", "--stage", "look", "--no-run"]);
    let out = repo.fz(&["run"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(!out.stdout.contains("look screens"), "{}", out.stdout);
}

#[test]
fn a_check_without_pictures_on_the_same_inputs_as_the_base_says_nothing_of_a_look() {
    let repo = Repo::new(
        &stills_config("stills = \"stills/**/*.png\""),
        &[("record.sh", "true\n"), ("src/color.txt", "red")],
    );
    repo.branch("feat/change");
    repo.write("notes.txt", "\n");
    let out = repo.fz(&["run", "screens"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(!out.stdout.contains("look screens"), "{}", out.stdout);
}

#[test]
fn a_rebase_onto_a_new_base_compares_the_pictures_again() {
    let repo = stills_repo();
    repo.write("src/color.txt", "blue");
    repo.fz(&["run", "--stage", "look", "--no-run"]);
    let first = repo.fz(&["run"]);
    assert!(first.ok, "{}\n{}", first.stdout, first.stderr);
    repo.commit("feat: blue");
    // The base branch moves on without touching what the check reads.
    repo.git(&["switch", "-q", "main"]);
    repo.write("README", "\n");
    repo.commit("Add a readme");
    repo.git(&["switch", "-q", "feat/change"]);
    repo.git(&["rebase", "-q", "main"]);
    let status = repo.fz(&[]);
    assert!(
        status.last_line().starts_with("next: fearlezz run"),
        "{}",
        status.stdout
    );
    let again = repo.fz(&["run"]);
    assert!(again.ok, "{}\n{}", again.stdout, again.stderr);
    assert!(
        again.stdout.contains("look screens: 1 changed"),
        "{}",
        again.stdout
    );
}

#[test]
fn dropping_the_stills_glob_weakens_the_proof() {
    let repo = stills_repo();
    write_config(&repo, "");
    repo.commit("drop the stills");
    let out = repo.fz(&["run", "--ci"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("it no longer takes stills by stills/**/*.png"),
        "{}",
        out.stdout
    );
}

#[test]
fn narrowing_the_stills_glob_weakens_the_proof() {
    let repo = stills_repo();
    write_config(&repo, "stills = \"stills/desktop/*.png\"");
    repo.commit("fewer stills");
    let out = repo.fz(&["run", "--ci"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains(
            "it takes stills by another glob (stills/**/*.png, now stills/desktop/*.png)"
        ),
        "{}",
        out.stdout
    );
}
