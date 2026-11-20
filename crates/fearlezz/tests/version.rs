//! The lowest Fearlezz a config needs: `fearlezz = "0.5.1"` at the top of `fearlezz.toml`. An
//! older binary refuses every command that reads the config, naming what it needs, what it is
//! and where it runs from, before anything newer in the config can confuse it.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;

const CHECK: &str = r#"
[check.tests]
run = "true"
stage = "working"
"#;

const OURS: &str = env!("CARGO_PKG_VERSION");

#[test]
fn an_older_binary_refuses_a_config_that_needs_a_newer_one() {
    // A newer config also uses what this binary cannot read; the version is what it names.
    let repo = Repo::new(
        &format!("fearlezz = \"999.0.0\"\nuse = [\"a-piece-of-the-future\"]\n{CHECK}"),
        &[],
    );
    let exe = assert_cmd::cargo::cargo_bin("fearlezz");
    for args in [&[][..], &["run"], &["--plan"], &["setup", "sync"]] {
        let out = repo.fz(args);
        assert!(!out.ok, "{args:?}: {}", out.stdout);
        for said in [
            "needs Fearlezz 999.0.0 or newer",
            &format!("Fearlezz {OURS}"),
            &exe.display().to_string(),
            "install.sh",
        ] {
            assert!(out.stderr.contains(said), "{args:?} {said}: {}", out.stderr);
        }
        assert!(!out.stderr.contains("future"), "{args:?}: {}", out.stderr);
    }
}

#[test]
fn a_config_this_binary_meets_runs() {
    for pin in ["0.1.0", OURS, "0.1"] {
        let repo = Repo::new(&format!("fearlezz = \"{pin}\"\n{CHECK}"), &[]);
        let out = repo.fz(&["--plan"]);
        assert!(out.ok, "{pin}: {}{}", out.stdout, out.stderr);
    }
}

#[test]
fn a_piece_can_need_a_newer_fearlezz_too() {
    let repo = Repo::new(
        &format!("use = [\"./proof/piece.toml\"]\n{CHECK}"),
        &[("proof/piece.toml", "fearlezz = \"999.1.0\"\n")],
    );
    let out = repo.fz(&["--plan"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stderr.contains("needs Fearlezz 999.1.0 or newer"),
        "{}",
        out.stderr
    );
    assert!(out.stderr.contains("proof/piece.toml"), "{}", out.stderr);
}

#[test]
fn a_pin_that_is_no_version_says_so() {
    let repo = Repo::new(&format!("fearlezz = \"latest\"\n{CHECK}"), &[]);
    let out = repo.fz(&["--plan"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stderr.contains("fearlezz = \"latest\" is no version")
            && out.stderr.contains(&format!("fearlezz = \"{OURS}\"")),
        "{}",
        out.stderr
    );
}

#[test]
fn init_pins_the_running_version() {
    let repo = Repo::with_skill("", &[("README.md", "# app\n")]);
    repo.remove("fearlezz.toml");
    repo.commit("without fearlezz");
    let out = repo.fz(&["setup", "init"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    assert!(
        config.contains(&format!("fearlezz = \"{OURS}\"")),
        "{config}"
    );
}

#[test]
fn lowering_the_pin_weakens_nothing() {
    // The pin says which binary can read the config; it is no behaviour the checks prove.
    let repo = Repo::new(&format!("fearlezz = \"{OURS}\"\n{CHECK}"), &[]);
    repo.branch("feat/x");
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    repo.write(
        "fearlezz.toml",
        &config.replace(&format!("fearlezz = \"{OURS}\""), "fearlezz = \"0.1.0\""),
    );
    repo.commit("lower the pin");
    let out = repo.fz(&["run", "--ci"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    assert!(!out.stdout.contains("weakens"), "{}", out.stdout);
}

/// The action's install step, run with an installer that only says which release it was asked
/// for, in a repository with `config`.
fn action_installs(config: &str, version: &str, action_ref: &str) -> String {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let action = std::fs::read_to_string(root.join("action.yml")).unwrap();
    let (_, step) = action.split_once("- name: Install Fearlezz").unwrap();
    let (_, run) = step.split_once("run: |\n").unwrap();
    let script = run
        .lines()
        .take_while(|l| l.starts_with("        ") || l.is_empty())
        .fold(String::new(), |script, l| {
            script + l.trim_start_matches("        ") + "\n"
        });
    let dir = tempfile::tempdir().unwrap();
    let at = dir.path();
    std::fs::write(at.join("fearlezz.toml"), config).unwrap();
    std::fs::write(
        at.join("install.sh"),
        "printf '%s' \"${FEARLEZZ_VERSION:-newest}\" > asked\n",
    )
    .unwrap();
    let out = std::process::Command::new("bash")
        .args(["-euo", "pipefail", "-c", &script])
        .current_dir(at)
        .env("VERSION", version)
        .env("ACTION_REF", action_ref)
        .env("FEARLEZZ_INSTALL_DIR", at.join("bin"))
        .env("GITHUB_ACTION_PATH", at)
        .env("GITHUB_PATH", at.join("path"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::read_to_string(at.join("asked")).unwrap()
}

#[test]
fn the_action_installs_at_least_the_pinned_version() {
    let pinned = "# the pin\nfearlezz = \"999.1\"\n\n[check.tests]\nfearlezz = \"1000.0.0\"\n";
    // Taken at an older tag, or asked for an older release, it installs the pin.
    assert_eq!(action_installs(pinned, "latest", "v0.4.0"), "v999.1.0");
    assert_eq!(action_installs(pinned, "v0.5.0", "main"), "v999.1.0");
    // The newest release meets any pin; a newer one asked for stays.
    assert_eq!(action_installs(pinned, "latest", "v0"), "newest");
    assert_eq!(action_installs(pinned, "v1000.0.0", "v0"), "v1000.0.0");
    // Without a pin, the release of the action's tag, as before.
    assert_eq!(
        action_installs("[check.tests]\n", "latest", "v0.4.0"),
        "v0.4.0"
    );
    assert_eq!(
        action_installs("fearlezz = \"0.1.0\"\n", "latest", "v0.4.0"),
        "v0.4.0"
    );
}
