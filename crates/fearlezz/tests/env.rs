//! The environment: preparing fresh checkouts and running the services checks need.
//! Services here are shell commands that log what happens, so the order can be checked.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;

const CONFIG: &str = r#"
[env]
up = 'mkdir -p .svc && n=$(( $(cat .svc/n 2>/dev/null || echo 0) + 1 )) && echo $n > .svc/n && echo up >> "$LOG"'
ports = 'test -f .svc/n && echo "SVC_PORT=$(( 9000 + $(cat .svc/n) ))"'
down = 'rm -f .svc/n; echo down >> "$LOG"'
reset = 'echo reset >> "$LOG"'
env = ["LOG"]

[check.e2e]
run = 'echo "port=$SVC_PORT" > "$FEARLEZZ_OUT/port.txt"; echo e2e >> "$LOG"'
stage = "working"
inputs = ["src/**"]
env = ["LOG"]
needs = ["services"]

[check.unit]
run = 'echo unit >> "$LOG"'
stage = "working"
inputs = ["src/**"]
env = ["LOG"]
"#;

struct Setup {
    repo: Repo,
    log: String,
}

impl Setup {
    fn new(config: &str, files: &[(&str, &str)]) -> Setup {
        let mut all = vec![
            (".gitignore", ".svc/\n.runs/\ndeps/\n"),
            ("src/lib.txt", "a"),
        ];
        all.extend_from_slice(files);
        let repo = Repo::new(config, &all);
        std::fs::create_dir_all(repo.path(".runs")).unwrap();
        let log = repo.path(".runs/log").to_string_lossy().to_string();
        Setup { repo, log }
    }

    fn fz(&self, args: &[&str]) -> common::Output {
        self.repo.fz_env(args, &[("LOG", &self.log)])
    }

    fn fz_at(&self, args: &[&str], now: &str) -> common::Output {
        self.repo
            .fz_env(args, &[("LOG", &self.log), ("FEARLEZZ_NOW", now)])
    }

    fn events(&self) -> Vec<String> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    fn change(&self) {
        let n = std::fs::read_to_string(self.repo.path("src/lib.txt")).unwrap();
        self.repo.write("src/lib.txt", &format!("{n}a"));
    }
}

fn working() -> Setup {
    let s = Setup::new(CONFIG, &[]);
    s.repo.branch("feat/x");
    s.change();
    s.fz(&["run", "--stage", "working", "--no-run"]);
    s
}

#[test]
fn services_start_once_and_their_ports_reach_the_check() {
    let s = working();
    let out = s.fz(&["run", "e2e"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("services up"), "{}", out.stdout);
    assert!(out.stdout.contains("SVC_PORT=9001"), "{}", out.stdout);
    let result: serde_json::Value =
        serde_json::from_str(&s.fz(&["--check", "e2e"]).stdout).unwrap();
    let dir = std::path::Path::new(result["dir"].as_str().unwrap());
    assert_eq!(
        std::fs::read_to_string(dir.join("port.txt"))
            .unwrap()
            .trim(),
        "port=9001"
    );

    s.change();
    let again = s.fz(&["run", "e2e"]);
    assert!(!again.stdout.contains("services up"), "{}", again.stdout);
    assert_eq!(s.events(), ["up", "e2e", "e2e"]);
    let status = s.fz(&[]);
    assert!(
        status
            .stdout
            .contains("services: started just now  SVC_PORT=9001"),
        "{}",
        status.stdout
    );
}

#[test]
fn a_check_without_services_never_starts_them() {
    let s = working();
    s.fz(&["run", "unit"]);
    assert_eq!(s.events(), ["unit"]);
}

#[test]
fn the_final_stage_starts_from_reset_data_once_and_stops_when_all_proof_is_in() {
    let s = working();
    s.fz(&["run"]);
    s.fz(&["run", "--stage", "merge", "--no-run"]);
    let out = s.fz(&["run"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("services fresh"), "{}", out.stdout);
    assert!(
        out.stdout.contains("services stopped: all proof is in"),
        "{}",
        out.stdout
    );
    assert_eq!(out.last_line(), "next: all proof is in: ready to merge");
    let events = s.events();
    let after_merge: Vec<&str> = events[events.iter().position(|e| e == "down").unwrap()..]
        .iter()
        .map(String::as_str)
        .collect();
    assert!(
        after_merge.starts_with(&["down", "reset", "up"]),
        "{events:?}"
    );
    assert_eq!(after_merge.last(), Some(&"down"), "{events:?}");
}

#[test]
fn idle_services_of_another_journal_stop_on_the_next_command() {
    let s = Setup::new(CONFIG, &[]);
    let wt = s.repo.worktree("feat/idle");
    wt.write("src/lib.txt", "b");
    let env = [
        ("LOG", s.log.as_str()),
        ("FEARLEZZ_NOW", "2026-11-10T10:00:00Z"),
    ];
    wt.fz_env(&["run", "--stage", "working", "--no-run"], &env);
    assert!(wt.fz_env(&["run", "e2e"], &env).ok);
    // Twenty minutes later nothing stops; forty minutes later the services are idle.
    s.fz_at(&[], "2026-11-10T10:20:00Z");
    assert_eq!(s.events(), ["up", "e2e"]);
    let later = s.fz_at(&[], "2026-11-10T10:40:00Z");
    assert!(
        later
            .stderr
            .contains("stopped the idle services of feat/idle"),
        "{}",
        later.stderr
    );
    assert_eq!(s.events(), ["up", "e2e", "down"]);
}

#[test]
fn down_stops_the_services_of_this_checkout() {
    let s = working();
    s.fz(&["run", "e2e"]);
    let out = s.fz(&["setup", "down"]);
    assert!(out.stdout.contains("stopped services"), "{}", out.stdout);
    assert_eq!(s.events(), ["up", "e2e", "down"]);
    assert!(
        s.fz(&["setup", "down"])
            .stdout
            .contains("no services running")
    );
}

#[test]
fn a_failed_down_keeps_the_services_known_so_down_can_try_again() {
    let config = CONFIG.replace(
        "down = 'rm -f .svc/n;",
        "down = 'test ! -f .svc/stuck || exit 3; rm -f .svc/n;",
    );
    let s = Setup::new(&config, &[]);
    s.repo.branch("feat/x");
    s.change();
    s.fz(&["run", "--stage", "working", "--no-run"]);
    assert!(s.fz(&["run", "e2e"]).ok);
    s.repo.write(".svc/stuck", "");
    let out = s.fz(&["setup", "down"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stderr.contains("[env] down failed"), "{}", out.stderr);
    assert!(s.fz(&[]).stdout.contains("services: started"));

    s.repo.remove(".svc/stuck");
    assert!(s.fz(&["setup", "down"]).stdout.contains("stopped services"));
    assert!(!s.fz(&[]).stdout.contains("services:"));
}

#[test]
fn a_start_that_never_finished_is_stopped_before_the_next_start() {
    // The first up kills Fearlezz itself, like a closed terminal would.
    let config = CONFIG.replace(
        "up = 'mkdir -p .svc &&",
        "up = 'mkdir -p .svc && if [ ! -f .svc/once ]; then touch .svc/once; echo killed >> \"$LOG\"; kill -9 $PPID; exit 1; fi &&",
    );
    let s = Setup::new(&config, &[]);
    s.repo.branch("feat/x");
    s.change();
    s.fz(&["run", "--stage", "working", "--no-run"]);
    assert!(!s.fz(&["run", "e2e"]).ok);
    assert!(
        s.fz(&[]).stdout.contains("start did not finish"),
        "{:?}",
        s.events()
    );
    let out = s.fz(&["run", "e2e"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert_eq!(s.events(), ["killed", "down", "up", "e2e"]);
}

#[test]
fn a_service_left_running_in_the_background_does_not_hold_up_the_start() {
    // The background process keeps the output of up open until it exits.
    let config = CONFIG.replace(
        "up = 'mkdir -p .svc &&",
        "up = 'sleep 30 & mkdir -p .svc &&",
    );
    let s = Setup::new(&config, &[]);
    s.repo.branch("feat/x");
    s.change();
    s.fz(&["run", "--stage", "working", "--no-run"]);
    let clock = std::time::Instant::now();
    assert!(s.fz(&["run", "e2e"]).ok);
    assert!(clock.elapsed().as_secs() < 20, "{:?}", clock.elapsed());
}

#[test]
fn services_a_running_check_uses_are_left_alone() {
    let config = CONFIG.replace(
        "run = 'echo \"port=$SVC_PORT\" > \"$FEARLEZZ_OUT/port.txt\"; echo e2e >> \"$LOG\"'",
        "run = 'touch \"$LOG.busy\"; while [ -f \"$LOG.busy\" ]; do sleep 0.1; done; echo e2e >> \"$LOG\"'",
    );
    let s = Setup::new(&config, &[]);
    let wt = s.repo.worktree("feat/busy");
    wt.write("src/lib.txt", "b");
    let env = [
        ("LOG", s.log.as_str()),
        ("FEARLEZZ_NOW", "2026-11-10T10:00:00Z"),
    ];
    wt.fz_env(&["run", "--stage", "working", "--no-run"], &env);
    let mut check = std::process::Command::new(env!("CARGO_BIN_EXE_fearlezz"))
        .args(["run", "e2e"])
        .current_dir(&wt.root)
        .envs(env)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let busy = std::path::PathBuf::from(format!("{}.busy", s.log));
    while !busy.exists() {
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    // An hour later by the clock, the services are in use, so idle cleanup skips them and
    // down refuses.
    let later = s.fz_at(&[], "2026-11-10T11:00:00Z");
    assert!(!later.stderr.contains("stopped"), "{}", later.stderr);
    let down = wt.fz_env(&["setup", "down"], &env);
    assert!(!down.ok);
    assert!(
        down.stderr.contains("in use by a running check"),
        "{}",
        down.stderr
    );

    std::fs::remove_file(&busy).unwrap();
    assert!(check.wait().unwrap().success());
    assert_eq!(s.events(), ["up", "e2e"]);
}

#[test]
fn idle_services_of_a_removed_checkout_are_forgotten_with_a_warning() {
    let s = Setup::new(CONFIG, &[]);
    let wt = s.repo.worktree("feat/gone");
    wt.write("src/lib.txt", "b");
    let env = [
        ("LOG", s.log.as_str()),
        ("FEARLEZZ_NOW", "2026-11-10T10:00:00Z"),
    ];
    wt.fz_env(&["run", "--stage", "working", "--no-run"], &env);
    assert!(wt.fz_env(&["run", "e2e"], &env).ok);
    std::fs::remove_dir_all(&wt.root).unwrap();
    let later = s.fz_at(&[], "2026-11-10T10:40:00Z");
    assert!(
        later.stderr.contains("services is gone"),
        "{}",
        later.stderr
    );
    assert!(
        later.stderr.contains("stop them by hand"),
        "{}",
        later.stderr
    );
    assert!(
        !later.stderr.contains("stopped the idle"),
        "{}",
        later.stderr
    );
}

#[test]
fn a_check_needing_services_without_an_up_command_is_a_config_error() {
    let config = "[check.e2e]\nrun = \"true\"\nstage = \"working\"\nneeds = [\"services\"]\n";
    let s = Setup::new(config, &[]);
    let out = s.fz(&["--plan"]);
    assert!(!out.ok);
    assert!(out.stderr.contains("no up command"), "{}", out.stderr);
}

const PREPARE: &str = r#"
[env]
prepare = 'echo prepare >> "$LOG"'
inputs = ["shell.nix"]
clone = ["deps"]
clone_when = ["lock.txt"]
env = ["LOG"]

[check.build]
run = 'test -f deps/lib.txt'
stage = "working"
inputs = ["src/**"]
"#;

fn with_deps() -> Setup {
    let s = Setup::new(PREPARE, &[("lock.txt", "v1")]);
    s.repo.write("deps/lib.txt", "installed");
    s
}

#[test]
fn a_fresh_checkout_gets_ignored_folders_cloned_and_is_prepared_once() {
    let s = with_deps();
    let wt = s.repo.worktree("feat/fresh");
    wt.write("src/lib.txt", "b");
    let env = [("LOG", s.log.as_str())];
    wt.fz_env(&["run", "--stage", "working", "--no-run"], &env);
    let out = wt.fz_env(&["run"], &env);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("prepared (cloned deps)"),
        "{}",
        out.stdout
    );
    wt.write("src/lib.txt", "c");
    let again = wt.fz_env(&["run"], &env);
    assert!(!again.stdout.contains("prepared"), "{}", again.stdout);
    assert_eq!(s.events(), ["prepare"]);
}

#[test]
fn a_changed_prepare_input_prepares_again() {
    let s = with_deps();
    s.repo.branch("feat/shell");
    s.change();
    s.fz(&["run", "--stage", "working", "--no-run"]);
    s.fz(&["run"]);
    s.repo.write("shell.nix", "with more tools");
    s.change();
    let out = s.fz(&["run"]);
    assert!(out.stdout.contains("prepared"), "{}", out.stdout);
    assert_eq!(s.events(), ["prepare", "prepare"]);
}

#[test]
fn folders_are_not_cloned_when_the_lockfile_differs() {
    let s = with_deps();
    let wt = s.repo.worktree("feat/newer");
    wt.write("lock.txt", "v2");
    wt.write("src/lib.txt", "b");
    let env = [("LOG", s.log.as_str())];
    wt.fz_env(&["run", "--stage", "working", "--no-run"], &env);
    let out = wt.fz_env(&["run"], &env);
    assert!(out.stdout.contains("prepared,"), "{}", out.stdout);
    assert!(!wt.root.join("deps").exists());
}

#[test]
fn the_base_checkout_is_prepared_before_the_look() {
    let record = r#"set -e
test -f deps/lib.txt
ffmpeg -v error -nostdin -f lavfi -i "color=c=$(cat src/color.txt):s=32x24" -frames:v 1 "$FEARLEZZ_OUT/home.png"
echo '{"kind":"still","file":"home.png"}' > "$FEARLEZZ_OUT/proof.jsonl"
"#;
    let config = r#"
[env]
prepare = 'echo prepare >> "$LOG"'
clone = ["deps"]
clone_when = ["lock.txt"]
env = ["LOG"]

[check.screens]
run = "sh record.sh"
stage = "look"
inputs = ["src/**", "record.sh"]
"#;
    let s = Setup::new(
        config,
        &[
            ("lock.txt", "v1"),
            ("record.sh", record),
            ("src/color.txt", "red"),
        ],
    );
    s.repo.write("deps/lib.txt", "installed");
    s.repo.branch("feat/blue");
    s.repo.write("src/color.txt", "blue");
    s.fz(&["run", "--stage", "look", "--no-run"]);
    let out = s.fz(&["run"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("base prepared (cloned deps)"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("look screens: 1 changed"),
        "{}",
        out.stdout
    );
}

#[test]
fn checks_run_inside_the_shell_and_its_files_are_inputs() {
    let config = r#"
[env]
shell = "env INSIDE=yes"
inputs = ["shell.nix"]

[check.where]
run = 'echo "inside=$INSIDE out=${FEARLEZZ_OUT:+set}" > "$FEARLEZZ_OUT/where.txt"'
stage = "working"
inputs = ["src/**"]
"#;
    let s = Setup::new(config, &[("shell.nix", "v1")]);
    s.repo.branch("feat/shell");
    s.change();
    s.fz(&["run", "--stage", "working", "--no-run"]);
    let out = s.fz(&["run", "where"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let result: serde_json::Value =
        serde_json::from_str(&s.fz(&["--check", "where"]).stdout).unwrap();
    let dir = std::path::Path::new(result["dir"].as_str().unwrap());
    assert_eq!(
        std::fs::read_to_string(dir.join("where.txt")).unwrap(),
        "inside=yes out=set\n"
    );
    // A new toolchain makes every result inside the shell stale.
    s.repo.write("shell.nix", "v2");
    let status = s.fz(&[]);
    assert!(
        status.stdout.contains("shell.nix changed"),
        "{}",
        status.stdout
    );
}
