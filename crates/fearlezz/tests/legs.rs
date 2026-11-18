//! Proving a commit in several CI jobs: checks bound to platforms, a plan of the legs a
//! matrix runs, each leg running its own checks, and a gate that combines their proofs into
//! one, the required check.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::{Repo, runs};
use serde_json::Value;

/// An OS that is never this machine's, so a platform on it is never local.
fn elsewhere() -> &'static str {
    if std::env::consts::OS == "linux" {
        "windows"
    } else {
        "linux"
    }
}

/// Two platforms; `fmt` runs on the default one, `tests` on both, `ui` in a job of its own on
/// macos. Each check appends a line to `.runs/<name>`; `tests` fails while `src/broken` exists.
const CONFIG: &str = r#"
no_proof = ["docs/**"]

[platform.linux]
runs-on = "ubuntu-latest"
default = true

[platform.mac]
runs-on = "macos-latest"
matrix = { xcode = "16" }

[check.fmt]
run = "echo run >> .runs/fmt"
stage = "working"
inputs = ["src/**"]
timeout = "5m"

[check.tests]
run = "echo run >> .runs/tests; test ! -e src/broken"
stage = "working"
on = ["linux", "mac"]
inputs = ["src/**"]
timeout = "20m"

[check.ui]
run = "echo run >> .runs/ui"
stage = "merge"
on = ["mac"]
job = "ui"
inputs = ["ui/**"]
"#;

fn repo_with(config: &str) -> Repo {
    let repo = Repo::new(
        config,
        &[
            (".gitignore", ".runs/\nlegs/\nout/\nbase/\n"),
            ("src/lib.rs", "fn a() {}\n"),
            ("ui/app.rs", "fn ui() {}\n"),
            ("docs/readme.md", "docs\n"),
        ],
    );
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    repo
}

/// A branch off `main` with a change the checks read.
fn branch() -> Repo {
    let repo = repo_with(CONFIG);
    repo.branch("feat/x");
    repo.write("src/lib.rs", "fn b() {}\n");
    repo.commit("change");
    repo
}

fn plan(repo: &Repo, extra: &[&str], env: &[(&str, &str)]) -> Vec<Value> {
    let mut args = vec!["run", "--ci", "--plan"];
    args.extend_from_slice(extra);
    let out = repo.fz_env(&args, env);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let legs: Value =
        serde_json::from_str(out.stdout.trim()).unwrap_or_else(|e| panic!("{e}: {}", out.stdout));
    legs.as_array().unwrap().clone()
}

fn leg<'a>(legs: &'a [Value], id: &str) -> &'a Value {
    legs.iter()
        .find(|l| l["leg"] == id)
        .unwrap_or_else(|| panic!("no leg {id} in {legs:?}"))
}

/// Runs leg `id` with its bundle in `legs/<id>`.
fn run_leg(repo: &Repo, id: &str, checks: &[&str], env: &[(&str, &str)]) -> common::Output {
    let dir = repo.path(&format!("legs/{}", id.replace('/', "-")));
    let mut args = vec!["run", "--ci", "--leg", id, "--proof", dir.to_str().unwrap()];
    args.extend_from_slice(checks);
    repo.fz_env(&args, env)
}

/// The gate over `legs/`, its job summary and its combined proof.
fn gate(repo: &Repo, extra: &[&str], env: &[(&str, &str)]) -> (common::Output, String, Value) {
    let summary = repo.path(".runs/summary.md");
    if summary.exists() {
        std::fs::remove_file(&summary).unwrap();
    }
    let legs = repo.path("legs");
    std::fs::create_dir_all(&legs).unwrap();
    let out_dir = repo.path("out");
    let mut all = vec![("GITHUB_STEP_SUMMARY", summary.to_str().unwrap())];
    all.extend_from_slice(env);
    let mut args = vec![
        "run",
        "--ci",
        "--gate",
        legs.to_str().unwrap(),
        "--proof",
        out_dir.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    let out = repo.fz_env(&args, &all);
    let text = std::fs::read_to_string(&summary)
        .unwrap_or_else(|_| panic!("{}\n{}", out.stdout, out.stderr));
    let proof = std::fs::read(out_dir.join("proof.json"))
        .map(|b| serde_json::from_slice(&b).unwrap())
        .unwrap_or(Value::Null);
    (out, text, proof)
}

fn assert_ok(out: &common::Output) {
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
}

// Platforms in fearlezz.toml

#[test]
fn a_check_on_an_unknown_platform_is_an_error() {
    let repo = repo_with(&CONFIG.replace(r#"on = ["mac"]"#, r#"on = ["windows"]"#));
    let out = repo.fz(&["run", "--ci", "--plan"]);
    assert!(!out.ok);
    assert!(
        out.stderr
            .contains("check ui: no platform \"windows\"; platforms are linux, mac"),
        "{}",
        out.stderr
    );
}

#[test]
fn several_platforms_need_a_default() {
    let repo = repo_with(&CONFIG.replace("default = true\n", ""));
    let out = repo.fz(&["run", "--ci", "--plan"]);
    assert!(!out.ok);
    assert!(
        out.stderr
            .contains("mark the one a check without `on` runs on with default = true"),
        "{}",
        out.stderr
    );
}

// The plan

#[test]
fn the_plan_has_one_leg_per_platform_and_job() {
    let repo = branch();
    let legs = plan(&repo, &[], &[]);
    assert_eq!(legs.len(), 3, "{legs:?}");
    let linux = leg(&legs, "linux");
    assert_eq!(linux["platform"], "linux");
    assert_eq!(linux["runs-on"], "ubuntu-latest");
    assert_eq!(linux["checks"], "fmt tests");
    // The budgets of its checks, and time to set the job up.
    assert_eq!(linux["timeout-minutes"], 25 + 15);
    let mac = leg(&legs, "mac");
    assert_eq!(mac["checks"], "tests");
    assert_eq!(mac["runs-on"], "macos-latest");
    assert_eq!(mac["xcode"], "16");
    let ui = leg(&legs, "mac/ui");
    assert_eq!(ui["checks"], "ui");
    // A check without a budget leaves the job GitHub's own limit.
    assert_eq!(ui["timeout-minutes"], 360);
}

#[test]
fn a_project_without_platforms_plans_one_leg() {
    let repo = repo_with("[check.a]\nrun = \"true\"\nstage = \"working\"\n");
    let legs = plan(&repo, &[], &[]);
    assert_eq!(legs.len(), 1, "{legs:?}");
    assert_eq!(legs[0]["leg"], "proof");
    assert_eq!(legs[0]["runs-on"], "ubuntu-latest");
    assert_eq!(legs[0]["checks"], "a");
}

// A leg

#[test]
fn a_leg_runs_only_its_checks_and_names_its_platform() {
    let repo = branch();
    let out = run_leg(&repo, "mac", &[], &[]);
    assert_ok(&out);
    assert_eq!(runs(&repo.path(".runs/tests")), 1);
    assert_eq!(runs(&repo.path(".runs/fmt")), 0);
    assert_eq!(runs(&repo.path(".runs/ui")), 0);
    let proof: Value =
        serde_json::from_slice(&std::fs::read(repo.path("legs/mac/proof.json")).unwrap()).unwrap();
    assert_eq!(proof["leg"], "mac");
    assert_eq!(proof["checks"][0]["id"], "tests");
    assert_eq!(proof["checks"][0]["platform"], "mac");
}

#[test]
fn a_leg_runs_the_checks_the_plan_named() {
    let repo = branch();
    let out = run_leg(&repo, "linux", &["tests"], &[]);
    assert_ok(&out);
    assert_eq!(runs(&repo.path(".runs/tests")), 1);
    assert_eq!(runs(&repo.path(".runs/fmt")), 0);
}

#[test]
fn a_leg_refuses_a_check_it_does_not_hold() {
    let repo = branch();
    let out = run_leg(&repo, "linux", &["ui"], &[]);
    assert!(!out.ok);
    assert!(
        out.stderr
            .contains("check ui is not in leg linux; its legs are mac/ui"),
        "{}",
        out.stderr
    );
    let out = run_leg(&repo, "windows", &[], &[]);
    assert!(!out.ok);
    assert!(
        out.stderr
            .contains("no leg \"windows\"; legs are linux, mac, mac/ui"),
        "{}",
        out.stderr
    );
}

#[test]
fn a_leg_leaves_what_the_change_weakens_to_the_gate() {
    let repo = branch();
    repo.write(
        "fearlezz.toml",
        &std::fs::read_to_string(repo.path("fearlezz.toml"))
            .unwrap()
            .replace(r#"on = ["linux", "mac"]"#, r#"on = ["linux"]"#),
    );
    repo.commit("drop mac");
    let out = run_leg(&repo, "linux", &[], &[]);
    assert_ok(&out);
    assert!(
        out.stdout
            .contains("the gate judges what the change weakens"),
        "{}",
        out.stdout
    );
}

// The gate

#[test]
fn the_gate_combines_the_legs_into_one_proof() {
    let repo = branch();
    for id in ["linux", "mac", "mac/ui"] {
        assert_ok(&run_leg(&repo, id, &[], &[]));
    }
    let (out, text, proof) = gate(&repo, &[], &[]);
    assert_ok(&out);
    assert!(text.contains("**Proven**"), "{text}");
    assert!(text.contains("| tests on linux | passed |"), "{text}");
    assert!(text.contains("| tests on mac | passed |"), "{text}");
    assert!(text.contains("| ui on mac | passed |"), "{text}");
    assert_eq!(proof["checks"].as_array().unwrap().len(), 4, "{proof}");
    assert!(proof.get("leg").is_none(), "{proof}");
    assert_eq!(
        out.last_line(),
        "next: this proved the commit, not the work; fearlezz shows where the work stands"
    );
}

#[test]
fn the_gate_fails_when_a_leg_did_not_finish() {
    let repo = branch();
    assert_ok(&run_leg(&repo, "linux", &[], &[]));
    assert_ok(&run_leg(&repo, "mac/ui", &[], &[]));
    let (out, text, _) = gate(&repo, &[], &[]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(text.contains("**Not proven**"), "{text}");
    assert!(text.contains("check `tests` on mac did not run"), "{text}");
    assert!(text.contains("its leg mac did not finish"), "{text}");
}

#[test]
fn the_gate_fails_when_a_leg_failed() {
    let repo = branch();
    repo.write("src/broken", "x\n");
    repo.commit("break");
    for id in ["linux", "mac", "mac/ui"] {
        run_leg(&repo, id, &[], &[]);
    }
    let (out, text, _) = gate(&repo, &[], &[]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(text.contains("| tests on mac | failed |"), "{text}");
}

#[test]
fn dropping_a_platform_from_a_check_weakens_it_at_the_gate() {
    let repo = branch();
    repo.write(
        "fearlezz.toml",
        &std::fs::read_to_string(repo.path("fearlezz.toml"))
            .unwrap()
            .replace(r#"on = ["linux", "mac"]"#, r#"on = ["linux"]"#),
    );
    repo.commit("drop mac");
    for id in ["linux", "mac/ui"] {
        assert_ok(&run_leg(&repo, id, &[], &[]));
    }
    let (out, text, _) = gate(&repo, &[], &[]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(text.contains("it no longer runs on mac"), "{text}");
}

#[test]
fn moving_a_platform_to_another_machine_weakens_it_at_the_gate() {
    let repo = branch();
    repo.write(
        "fearlezz.toml",
        &std::fs::read_to_string(repo.path("fearlezz.toml"))
            .unwrap()
            .replace(
                r#"matrix = { xcode = "16" }"#,
                r#"matrix = { xcode = "15" }"#,
            ),
    );
    repo.commit("older xcode");
    for id in ["linux", "mac", "mac/ui"] {
        assert_ok(&run_leg(&repo, id, &[], &[]));
    }
    let (out, text, _) = gate(&repo, &[], &[]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(text.contains("it runs on another machine"), "{text}");
}

// Time budgets

#[test]
fn a_check_over_its_budget_is_stopped_and_fails() {
    let repo =
        repo_with("[check.slow]\nrun = \"sleep 30\"\nstage = \"working\"\ntimeout = \"1s\"\n");
    let clock = std::time::Instant::now();
    let out = repo.fz(&["run", "--ci"]);
    assert!(!out.ok);
    assert!(clock.elapsed().as_secs() < 20, "it waited for the command");
    assert!(
        out.stdout.contains("exceeded its 1s budget"),
        "{}",
        out.stdout
    );
}

// Locally

#[test]
fn locally_a_check_no_platform_fits_is_left_to_ci() {
    let config = format!(
        "[platform.here]\nruns-on = \"ubuntu-latest\"\ndefault = true\n\n\
         [platform.there]\nruns-on = \"windows-latest\"\nos = \"{}\"\n\n\
         [check.a]\nrun = \"echo run >> .runs/a\"\nstage = \"working\"\n\n\
         [check.b]\nrun = \"echo run >> .runs/b\"\nstage = \"working\"\non = [\"there\"]\n",
        elsewhere()
    );
    let repo = repo_with(&config);
    repo.branch("feat/x");
    repo.write("src/lib.rs", "fn b() {}\n");
    repo.commit("change");
    let out = repo.fz(&["run", "--stage", "working"]);
    assert_ok(&out);
    assert_eq!(runs(&repo.path(".runs/a")), 1, "{}", out.stdout);
    assert_eq!(runs(&repo.path(".runs/b")), 0);
    assert!(
        out.stdout.contains("left to CI b: runs on there"),
        "{}",
        out.stdout
    );
    let out = repo.fz(&["run", "--stage", "merge"]);
    assert_ok(&out);
    assert_eq!(
        out.last_line(),
        "next: all proof is in: ready to merge",
        "{}",
        out.stdout
    );
}

// Reusing a pass the base commit's CI saw

/// The event file of a pull request, so the run reads as one.
fn pull_request(repo: &Repo) -> std::path::PathBuf {
    let path = repo.path(".runs/event.json");
    std::fs::write(
        &path,
        r#"{"pull_request":{"number":7,"title":"feat: x","head":{"sha":"abc","ref":"feat/x"}}}"#,
    )
    .unwrap();
    path
}

/// The base commit's CI: every leg on `main`, combined by the gate into `base/`.
fn base_proof(repo: &Repo) {
    repo.git(&["checkout", "-q", "main"]);
    for id in ["linux", "mac", "mac/ui"] {
        assert_ok(&run_leg(repo, id, &[], &[]));
    }
    let (out, _, _) = gate(repo, &[], &[]);
    assert_ok(&out);
    std::fs::rename(repo.path("out"), repo.path("base")).unwrap();
    std::fs::remove_dir_all(repo.path("legs")).unwrap();
    repo.git(&["checkout", "-q", "feat/x"]);
}

#[test]
fn a_pull_request_reuses_a_pass_its_base_proof_shows_for_the_same_inputs() {
    let repo = branch();
    base_proof(&repo);
    let event = pull_request(&repo);
    let base = repo.path("base");
    let env = [("GITHUB_EVENT_PATH", event.to_str().unwrap())];
    let reuse = ["--reuse", base.to_str().unwrap()];
    // The change touches src/, which ui does not read, so its pass on main still holds.
    let legs = plan(&repo, &reuse, &env);
    assert!(legs.iter().all(|l| l["leg"] != "mac/ui"), "{legs:?}");
    for l in &legs {
        let id = l["leg"].as_str().unwrap();
        let checks: Vec<&str> = l["checks"].as_str().unwrap().split(' ').collect();
        assert_ok(&run_leg(&repo, id, &checks, &env));
    }
    let (out, text, _) = gate(&repo, &reuse, &env);
    assert_ok(&out);
    assert!(text.contains("| ui on mac | reused |"), "{text}");
}

#[test]
fn a_push_reuses_nothing() {
    let repo = branch();
    base_proof(&repo);
    let base = repo.path("base");
    let legs = plan(&repo, &["--reuse", base.to_str().unwrap()], &[]);
    assert!(legs.iter().any(|l| l["leg"] == "mac/ui"), "{legs:?}");
}

#[test]
fn a_plan_without_a_base_proof_prints_only_its_json() {
    let repo = branch();
    let event = pull_request(&repo);
    let missing = repo.path("base");
    // `plan` parses the whole of stdout as JSON.
    let legs = plan(
        &repo,
        &["--reuse", missing.to_str().unwrap()],
        &[("GITHUB_EVENT_PATH", event.to_str().unwrap())],
    );
    assert_eq!(legs.len(), 3, "{legs:?}");
}

#[test]
fn ci_without_a_leg_does_not_prove_a_check_this_machine_cannot_run() {
    let config = format!(
        "[platform.here]\nruns-on = \"ubuntu-latest\"\ndefault = true\n\n\
         [platform.there]\nruns-on = \"windows-latest\"\nos = \"{}\"\n\n\
         [check.b]\nrun = \"true\"\nstage = \"working\"\non = [\"there\"]\n",
        elsewhere()
    );
    let repo = repo_with(&config);
    let out = repo.fz(&["run", "--ci"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("check `b` on there did not run"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_leg_refuses_a_machine_without_its_platforms_os() {
    let config = format!(
        "[platform.there]\nruns-on = \"windows-latest\"\nos = \"{}\"\n\n\
         [check.b]\nrun = \"true\"\nstage = \"working\"\n",
        elsewhere()
    );
    let repo = repo_with(&config);
    let out = run_leg(&repo, "there", &[], &[]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stderr
            .contains(&format!("leg there needs {}", elsewhere())),
        "{}",
        out.stderr
    );
}

#[test]
fn a_job_name_has_no_slash() {
    let repo = repo_with(&CONFIG.replace(r#"job = "ui""#, r#"job = "u/i""#));
    let out = repo.fz(&["run", "--ci", "--plan"]);
    assert!(!out.ok);
    assert!(
        out.stderr
            .contains("check ui: a job's name has no / or space"),
        "{}",
        out.stderr
    );
}
