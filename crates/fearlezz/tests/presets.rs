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
fn biome_is_a_piece_of_its_own_beside_the_type_checker() {
    // The typescript piece is the type checker alone, so a project that lints and formats
    // with other tools runs no Biome.
    let repo = Repo::new("use = [\"typescript\"]\n", &[]);
    assert_eq!(checks(&repo), "settings, tsc");
    let repo = Repo::new("use = [\"biome\"]\n", &[]);
    assert_eq!(checks(&repo), "biome-format, biome-lint, settings");
}

#[test]
fn a_dart_package_gets_the_flutter_apps_analyzer_and_runs_dart_test() {
    let repo = Repo::new("use = [\"dart\"]\n", &[]);
    assert_eq!(
        checks(&repo),
        "dart-analyze, dart-format, dart-test, settings"
    );
    let out = repo.fz(&["setup", "sync"]);
    assert!(out.ok, "{}", out.stderr);
    let options = std::fs::read_to_string(repo.path("analysis_options.yaml")).unwrap();
    assert!(options.contains("prefer_double_quotes"), "{options}");
    // A package has no platform folders for Flutter to add.
    assert!(!options.contains("android/**"), "{options}");

    let repo = Repo::new("use = [\"flutter\"]\n", &[]);
    assert_eq!(
        checks(&repo),
        "dart-analyze, dart-format, flutter-test, settings"
    );
}

#[test]
fn the_flutter_piece_never_asks_for_the_types_it_forbids() {
    // omit_local_variable_types forbids the local types these two would ask for.
    let repo = Repo::new("use = [\"flutter\"]\n", &[]);
    let out = repo.fz(&["setup", "sync"]);
    assert!(out.ok, "{}", out.stderr);
    let options = std::fs::read_to_string(repo.path("analysis_options.yaml")).unwrap();
    for rule in [
        "specify_nonobvious_local_variable_types: false",
        "specify_nonobvious_property_types: false",
        "omit_local_variable_types: true",
    ] {
        assert!(options.contains(rule), "{rule}: {options}");
    }
}

#[test]
fn an_off_that_is_not_true_or_false_says_so() {
    let repo = Repo::new("use = [\"rust\"]\n\n[check.clippy]\noff = \"yes\"\n", &[]);
    let out = repo.fz(&["run", "clippy"]);
    assert!(!out.ok);
    assert!(
        out.stderr
            .contains("in `check.clippy`: off is string, expected true or false"),
        "{}",
        out.stderr
    );
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
    // The tuned command ran, not the piece's.
    let result: serde_json::Value =
        serde_json::from_str(&repo.fz(&["--check", "tests"]).stdout).unwrap();
    let dir = std::path::Path::new(result["dir"].as_str().unwrap());
    assert_eq!(
        std::fs::read_to_string(dir.join("proof.txt")).unwrap(),
        "tuned\n"
    );
}

#[test]
fn a_piece_check_reruns_after_a_change_to_any_file_outside_no_proof() {
    let config = r#"
use = ["rust"]
no_proof = ["*.md"]

[check.tests]
run = "echo run >> .runs/tests; echo 'test result: ok. 1 passed; 0 failed'"
"#;
    let repo = Repo::new(
        config,
        &[
            (".gitignore", ".runs/\n"),
            ("src/lib.rs", ""),
            ("data.json", "{}\n"),
            ("README.md", "readme\n"),
        ],
    );
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    repo.branch("feat/x");
    repo.write("data.json", "[]\n");
    let out = repo.fz(&["run", "tests"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert_eq!(common::runs(&repo.path(".runs/tests")), 1);
    repo.write("data.json", "[1]\n");
    repo.fz(&["run", "tests"]);
    assert_eq!(
        common::runs(&repo.path(".runs/tests")),
        2,
        "data.json is read"
    );
    repo.write("README.md", "other\n");
    repo.fz(&["run", "tests"]);
    assert_eq!(
        common::runs(&repo.path(".runs/tests")),
        2,
        "README.md needs no proof"
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
            "built in are actions, agents, betterleaks, biome, bun, commits, crates, dart, devenv, envtest, eslint, flutter, flutter-look, flux, github, go, go-race, helm, kotlin, kustomize, lua, next, nix, node-test, npm, opentofu, osv-scanner, oxlint, playwright, prettier, pub, python, qwik, react, rust, safety, solid, sops, svelte, terraform, trufflehog, typescript, vitest, vue, yamllint"
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
const FAKE_TOKEN: &str = "ghp_16C7e42F292c6912E7710c838347Ae178B4a"; // gitleaks:allow

/// A repository with a leak already committed, so a scanner has something old to ignore and
/// the test can add a new leak next to it.
fn leaky(piece: &str) -> Repo {
    let repo = Repo::new(
        &format!("use = [\"{piece}\"]\n"),
        &[(
            "src/old.rs",
            &format!("const T: &str = \"{FAKE_TOKEN}\";\n"),
        )],
    );
    repo.branch("feat/x");
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    repo
}

/// No file the checks stored (results, logs, reports) holds `secret`.
fn assert_nothing_stored_holds(repo: &Repo, secret: &str) {
    let mut stack = vec![repo.path(".git/fearlezz")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(text) = std::fs::read_to_string(&path) {
                assert!(
                    !text.contains(secret),
                    "{} holds the secret",
                    path.display()
                );
            }
        }
    }
}

/// Every secret scanner piece judges the same way: a leak that was already committed does not
/// block new work, the same leak on a line this change adds does, and the secret itself is
/// never printed or kept.
fn gates_a_leak_on_a_changed_line(piece: &str, check: &str, rule: &str) {
    let repo = leaky(piece);
    assert_eq!(checks(&repo), check);

    repo.write("src/new.rs", "fn main() {}\n");
    let out = repo.fz(&["run", check]);
    assert!(out.ok, "{piece}: {}\n{}", out.stdout, out.stderr);

    repo.write(
        "src/new.rs",
        &format!("fn main() {{}}\nconst T: &str = \"{FAKE_TOKEN}\";\n"),
    );
    let out = repo.fz(&["run", check]);
    assert!(!out.ok, "{piece}: {}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("src/new.rs:2"), "{}", out.stdout);
    assert!(out.stdout.contains(rule), "{}", out.stdout);
    assert!(
        out.stdout.contains("fix: remove the secret"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains(FAKE_TOKEN), "{}", out.stdout);
    assert_nothing_stored_holds(&repo, FAKE_TOKEN);
}

#[test]
fn the_safety_piece_gates_a_secret_on_a_changed_line_and_never_prints_it() {
    gates_a_leak_on_a_changed_line("safety", "gitleaks", "github-pat");
}

#[test]
fn betterleaks_is_a_scanner_to_pick_instead() {
    gates_a_leak_on_a_changed_line("betterleaks", "betterleaks", "github-pat");
}

#[test]
fn trufflehog_is_a_scanner_to_pick_instead() {
    gates_a_leak_on_a_changed_line("trufflehog", "trufflehog", "Github");
}

#[test]
fn scanners_combine_because_checks_are_named_for_their_tools() {
    let repo = Repo::new("use = [\"safety\", \"trufflehog\", \"osv-scanner\"]\n", &[]);
    assert_eq!(checks(&repo), "gitleaks, osv-scanner, trufflehog");
}

/// What osv-scanner 2.7.0 wrote for a Cargo.lock holding the vulnerable `time` 0.1.43, with
/// the scanned folder written as ROOT. It names the lockfile by an absolute file URI and no
/// line.
const OSV_SARIF: &str = include_str!("fixtures/osv-scanner.sarif");

#[test]
fn the_osv_scanner_piece_reads_what_osv_scanner_writes() {
    // The real scanner needs osv.dev; its report, from this checkout, stands in for it.
    let config = "use = [\"osv-scanner\"]\n\n[check.osv-scanner]\nrun = 'sed \"s|ROOT|$(pwd \
                  -P)|g\" osv.sarif > \"$FEARLEZZ_OUT/findings.sarif\"'\n";
    let repo = Repo::new(
        config,
        &[("osv.sarif", OSV_SARIF), ("Cargo.lock", "version = 3\n")],
    );
    repo.branch("feat/x");
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    repo.write("Cargo.lock", "version = 3\n\n# time 0.1.43\n");
    let out = repo.fz(&["run", "osv-scanner"]);
    // A vulnerable dependency is a flag: it may have no fix yet.
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.lines()
            .iter()
            .any(|l| l.contains("flag Cargo.lock") && l.contains("CVE-2020-26235")),
        "{}",
        out.stdout
    );
}

#[test]
fn a_missing_scanner_is_named_with_how_to_install_it() {
    let repo = leaky("safety");
    repo.write("src/new.rs", "fn main() {}\n");
    let out = repo.fz_env(&["run", "gitleaks"], &[("PATH", "/usr/bin:/bin")]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("gitleaks was not found"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("pkgs.gitleaks"), "{}", out.stdout);
}

/// A workflow with a template injection on line 8.
const WORKFLOW: &str = "name: ci\non: push\npermissions: {}\njobs:\n  a:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo ${{ github.event.head_commit.message }}\n";

#[test]
fn the_actions_piece_gates_what_zizmor_calls_an_error_on_a_changed_line() {
    let repo = Repo::new(
        "use = [\"actions\"]\n",
        &[(".github/workflows/old.yml", WORKFLOW)],
    );
    assert_eq!(checks(&repo), "zizmor");
    repo.branch("feat/x");
    repo.fz(&["run", "--stage", "working", "--no-run"]);

    // The old workflow is as bad, but this change did not touch it.
    repo.write(
        ".github/workflows/new.yml",
        "name: ok\non: push\npermissions: {}\njobs: {}\n",
    );
    let out = repo.fz(&["run", "zizmor"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);

    repo.write(".github/workflows/new.yml", WORKFLOW);
    let out = repo.fz(&["run", "zizmor"]);
    assert!(!out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains(".github/workflows/new.yml:8"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("zizmor/template-injection"),
        "{}",
        out.stdout
    );
}

#[test]
fn flutter_film_runs_its_entry_file_as_a_plain_command() {
    let preset: toml::Value = toml::from_str(include_str!("../presets/flutter-look.toml")).unwrap();
    let run = preset["check"]["flutter-film"]["run"].as_str().unwrap();
    assert_eq!(run, "flutter test integration_test/app_test.dart -d macos");
}

// The language pieces, run with the real tools devenv gives

/// A repository on `config` with `files`, synced, on a branch whose change is to come.
fn on_branch(config: &str, files: &[(&str, &str)]) -> Repo {
    let repo = Repo::new(config, files);
    assert!(repo.fz(&["setup", "sync"]).ok);
    repo.git(&["add", "-A"]);
    repo.git(&["commit", "-q", "--allow-empty", "-m", "synced"]);
    repo.branch("feat/x");
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    repo
}

#[test]
fn the_prettier_piece_fails_on_an_unformatted_file_and_leaves_fearlezz_files_alone() {
    // The agent skill and settings Fearlezz writes have the layout Fearlezz gives them.
    let repo = on_branch(
        "use = [\"prettier\"]\n\n[settings.claude.permissions]\nallow = [\"Bash(ls)\",    \"Bash(pwd)\"]\n",
        &[("src/a.ts", "export const a = 1;\n")],
    );
    assert_eq!(checks(&repo), "prettier, settings");
    repo.write(".claude/notes.json", "{\"a\":   1}\n");
    let out = repo.fz(&["run", "prettier"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);

    repo.write("src/a.ts", "export   const a = 1\n");
    let out = repo.fz(&["run", "prettier"]);
    assert!(!out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("src/a.ts"), "{}", out.stdout);
}

#[test]
fn the_lua_piece_fails_on_a_file_stylua_would_change() {
    let repo = on_branch("use = [\"lua\"]\n", &[("main.lua", "local a = 1\n")]);
    assert_eq!(checks(&repo), "stylua");
    let out = repo.fz(&["run", "stylua"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);

    repo.write("main.lua", "local   a=1\n");
    let out = repo.fz(&["run", "stylua"]);
    assert!(!out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("main.lua"), "{}", out.stdout);
}

#[test]
fn the_node_test_piece_counts_the_tests_node_ran() {
    let test = "import { test } from 'node:test';\nimport assert from 'node:assert';\n\
                test('one', () => assert.equal(1, 1));\ntest('two', () => assert.equal(2, 2));\n";
    let repo = on_branch(
        "use = [\"node-test\"]\n",
        &[
            ("package.json", "{ \"type\": \"module\" }\n"),
            ("test/a.test.js", test),
        ],
    );
    assert_eq!(checks(&repo), "node-test");
    repo.write("src/a.js", "export const a = 1;\n");
    repo.commit("feat: a");
    let out = repo.fz(&["run", "--ci"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.lines()
            .iter()
            .any(|l| l.contains("node-test") && l.contains("  2 tests  ")),
        "{}",
        out.stdout
    );
}

#[test]
fn the_go_race_piece_runs_the_tests_once_at_the_merge_stage() {
    let repo = Repo::new("use = [\"go\", \"go-race\"]\n", &[]);
    assert_eq!(
        checks(&repo),
        "go-race, go-test, gofmt, golangci-lint, settings"
    );
    let plan = repo.fz(&["--plan"]);
    assert!(plan.ok, "{}", plan.stderr);
}

/// A folder of stand ins for tools devenv does not give, each printing `output` to stdout and
/// exiting with `code`, and PATH with that folder first.
fn stand_ins(tools: &[(&str, &str, i32)]) -> (tempfile::TempDir, String) {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    for (tool, output, code) in tools {
        let path = dir.path().join(tool);
        std::fs::write(
            &path,
            format!("#!/bin/sh\ncat <<'OUT'\n{output}\nOUT\nexit {code}\n"),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!(
        "{}:{}",
        dir.path().display(),
        std::env::var("PATH").unwrap()
    );
    (dir, path)
}

/// What svelte-check 4.7 printed for a type error on line 2 of src/App.svelte and an image
/// without alt text on line 6 (its lines count from 0).
const SVELTE_CHECK: &str = r#"1794884543133 START "/repo"
1794884543135 {"type":"WARNING","filename":"src/App.svelte","start":{"line":5,"character":0},"end":{"line":5,"character":17},"message":"`<img>` element should have an alt attribute\nhttps://svelte.dev/e/a11y_missing_attribute","code":"a11y_missing_attribute","source":"svelte"}
1794884543135 {"type":"ERROR","filename":"src/App.svelte","start":{"line":1,"character":6},"end":{"line":1,"character":11},"message":"Type 'string' is not assignable to type 'number'.","code":2322,"source":"ts"}
1794884543135 COMPLETED 105 FILES 1 ERRORS 1 WARNINGS 1 FILES_WITH_PROBLEMS"#;

#[test]
fn the_svelte_piece_reads_svelte_check_and_its_type_errors_block_anywhere() {
    let app = "<script lang=\"ts\">\n  let count: number = 0;\n</script>\n\n<p>{count}</p>\n<img src=\"x.png\" alt=\"x\">\n";
    let repo = on_branch("use = [\"svelte\"]\n", &[("src/App.svelte", app)]);
    // svelte-check takes the place of tsc, with the typescript piece's strictness.
    assert_eq!(checks(&repo), "settings, svelte-check");
    let (_bin, path) = stand_ins(&[("svelte-kit", "", 0), ("svelte-check", SVELTE_CHECK, 1)]);

    // The change touched neither line, yet a type error blocks wherever it is.
    repo.write("src/other.ts", "export const a = 1;\n");
    let out = repo.fz_env(&["run", "svelte-check"], &[("PATH", &path)]);
    assert!(!out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("src/App.svelte:2"), "{}", out.stdout);
    assert!(!out.stdout.contains("src/App.svelte:6"), "{}", out.stdout);

    // Without the error, a warning on a line the change touched is shown and passes.
    let warning_only: String = SVELTE_CHECK
        .lines()
        .filter(|l| !l.contains("\"ERROR\""))
        .collect::<Vec<_>>()
        .join("\n");
    let (_bin, path) = stand_ins(&[("svelte-kit", "", 0), ("svelte-check", &warning_only, 0)]);
    repo.write("src/App.svelte", &app.replace(" alt=\"x\"", ""));
    let out = repo.fz_env(&["run", "svelte-check"], &[("PATH", &path)]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("src/App.svelte:6"), "{}", out.stdout);
    assert!(
        out.stdout.contains("a11y_missing_attribute"),
        "{}",
        out.stdout
    );
}

#[test]
fn the_oxlint_piece_writes_every_rule_and_gates_a_finding_on_a_changed_line() {
    let repo = on_branch(
        "use = [\"oxlint\", \"react\"]\n",
        &[("src/old.ts", "debugger;\n")],
    );
    assert_eq!(checks(&repo), "oxlint, settings");
    let config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(repo.path(".oxlintrc.json")).unwrap())
            .unwrap();
    assert_eq!(config["rules"]["eslint/no-debugger"], "error", "{config}");
    // The react piece's oxlint rules are written and their plugin turned on, and no Biome
    // file is written for its Biome rules, since Biome does not run.
    assert_eq!(config["rules"]["react/jsx-key"], "error", "{config}");
    assert!(
        config["plugins"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("react")),
        "{config}"
    );
    assert_eq!(config["categories"]["correctness"], "off", "{config}");
    assert!(!repo.path("biome.json").exists());

    // The old debugger is not this change's.
    repo.write("src/new.ts", "export const a = 1;\n");
    let out = repo.fz(&["run", "oxlint"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);

    repo.write("src/new.ts", "export const a = 1;\ndebugger;\n");
    let out = repo.fz(&["run", "oxlint"]);
    assert!(!out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("gate src/new.ts:2  eslint(no-debugger)"),
        "{}",
        out.stdout
    );
}

/// A folder with executable `scripts` (name, body), and PATH with that folder first.
fn scripts(scripts: &[(&str, &str)]) -> (tempfile::TempDir, String) {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    for (name, body) in scripts {
        let path = dir.path().join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!(
        "{}:{}",
        dir.path().display(),
        std::env::var("PATH").unwrap()
    );
    (dir, path)
}

#[test]
fn the_envtest_piece_gives_go_test_the_binaries_of_the_kubernetes_version_in_go_mod() {
    let go_mod = "module op\n\ngo 1.24\n\nrequire (\n\tk8s.io/api v0.31.2\n\tsigs.k8s.io/controller-runtime v0.19.0\n)\n";
    // The stand in go writes no coverage profile.
    let repo = on_branch(
        "use = [\"go\", \"envtest\"]\n\n[check.go-test]\ncoverage = false\n",
        &[("go.mod", go_mod)],
    );
    // setup-envtest prints the folder of the binaries it fetched for the version it is asked;
    // go passes only when the tests find them there.
    let (_bin, path) = scripts(&[
        (
            "setup-envtest",
            "[ \"$1 $2\" = \"use 1.31\" ] || { echo \"asked for $*\" >&2; exit 1; }\necho /assets/k8s/1.31.4",
        ),
        (
            "go",
            "[ \"$KUBEBUILDER_ASSETS\" = /assets/k8s/1.31.4 ] || { echo \"no assets: '$KUBEBUILDER_ASSETS'\"; exit 1; }\necho '--- PASS: TestReconcile (0.10s)'",
        ),
    ]);
    repo.write("controller.go", "package op\n");
    let out = repo.fz_env(&["run", "go-test"], &[("PATH", &path)]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);

    // Without setup-envtest the check fails and says how to get it.
    let (_bin, path) = scripts(&[("go", "echo '--- PASS: TestReconcile (0.10s)'")]);
    let path = path
        .split(':')
        .filter(|p| !std::path::Path::new(p).join("setup-envtest").exists())
        .collect::<Vec<_>>()
        .join(":");
    repo.write("controller.go", "package op\n\n// changed\n");
    let out = repo.fz_env(&["run", "go-test"], &[("PATH", &path)]);
    assert!(!out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("setup-envtest"), "{}", out.stdout);
}
