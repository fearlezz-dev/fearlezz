//! The machine's heavy queue: with `FEARLEZZ_QUEUE=<slots>` on a developer's machine, heavy
//! work of every worktree and repository waits for one of that many slots instead of all
//! running at once, says what it waits behind, and reuses a result another worktree produced
//! for the same inputs meanwhile. Off by default and on CI. Agents queue a tool they run by
//! hand with `fearlezz run -- <command>`.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use common::{Output, Repo, Worktree};

const QUEUE: (&str, &str) = ("FEARLEZZ_QUEUE", "1");

/// `gate` runs until the test opens it; `other` and the light `quick` run at once. Each run
/// appends its name to `runs` in the test's own folder. `shots` takes pictures, so it runs on
/// the base commit too, and runs until the test opens it for the content of `src/a.txt`. `svc`
/// needs the worktree's services. The light `snap` takes the still the test rendered for the
/// content of `src/a.txt`.
fn config(dir: &Path) -> String {
    let dir = dir.display();
    format!(
        r#"
[env]
up = "true"
down = "true"

[check.gate]
run = "echo gate >> {dir}/runs; touch {dir}/started; while [ ! -e {dir}/open ]; do sleep 0.05; done"
stage = "working"
inputs = ["src/**"]

[check.other]
run = "echo other >> {dir}/runs"
stage = "working"
inputs = ["src/**"]

[check.shots]
run = "touch {dir}/started-$(cat src/a.txt); while [ ! -e {dir}/open-$(cat src/a.txt) ]; do sleep 0.05; done"
stage = "working"
inputs = ["src/**"]
stills = "*.png"

[check.svc]
run = "echo svc >> {dir}/runs"
stage = "working"
inputs = ["src/**"]
needs = ["services"]

[check.quick]
run = "echo quick >> {dir}/runs"
stage = "working"
inputs = ["src/**"]
light = true

[check.snap]
run = "cp {dir}/snap-$(cat src/a.txt).png \"$FEARLEZZ_OUT/snap.png\""
stage = "working"
inputs = ["src/**"]
stills = "*.png"
light = true
"#
    )
}

/// A repository on `feat/a` and a second worktree of it on `feat/b`, sharing the machine's
/// queue and result store, and the folder the checks mark.
struct Machine {
    dir: tempfile::TempDir,
    repo: Repo,
    b: Worktree,
}

impl Machine {
    /// With `same_inputs`, the second worktree holds the same files as the first.
    fn new(same_inputs: bool) -> Machine {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repo::new(&config(dir.path()), &[("src/a.txt", "base\n")]);
        repo.branch("feat/a");
        repo.write("src/a.txt", "change\n");
        repo.commit("change a");
        let b = repo.worktree("feat/b");
        if !same_inputs {
            b.write("src/a.txt", "other\n");
        }
        Machine { dir, repo, b }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// The names of the checks that ran, in order.
    fn runs(&self) -> Vec<String> {
        std::fs::read_to_string(self.path("runs"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    /// Starts `gate` in the first worktree with the queue on and returns once it runs.
    fn hold(&self) -> JoinHandle<Output> {
        let root = self.repo.root.clone();
        let cache = self.repo.cache.clone();
        let held = thread::spawn(move || fz(&root, &cache, &["run", "gate"], &[QUEUE]));
        let deadline = Instant::now() + Duration::from_secs(60);
        while !self.path("started").exists() {
            if held.is_finished() {
                let out = held.join().unwrap();
                panic!(
                    "gate ended before it started:\n{}\n{}",
                    out.stdout, out.stderr
                );
            }
            assert!(Instant::now() < deadline, "gate never started");
            thread::sleep(Duration::from_millis(20));
        }
        held
    }

    fn open(&self) {
        std::fs::write(self.path("open"), "").unwrap();
    }

    /// Runs `args` in the second worktree with the queue on, and returns once it says it waits.
    fn queued_in_b(&self, args: &[&str]) -> JoinHandle<Output> {
        self.queued(&self.b.root, args, "b")
    }

    /// Runs `args` in `dir` with the queue on, and returns once it says it waits. Its stderr
    /// goes to `<name>.stderr` in the test's folder.
    fn queued(&self, dir: &Path, args: &[&str], name: &str) -> JoinHandle<Output> {
        let (mut child, stderr) = self.start(dir, args, name);
        let deadline = Instant::now() + Duration::from_secs(60);
        while !stderr().contains("wait ") {
            if let Some(status) = child.try_wait().unwrap() {
                panic!("{args:?} ended ({status}) without waiting:\n{}", stderr());
            }
            assert!(Instant::now() < deadline, "{args:?} never waited");
            thread::sleep(Duration::from_millis(20));
        }
        finish(child, stderr)
    }

    /// Starts `args` in `dir` with the queue on, its stderr in `<name>.stderr`, and how to
    /// read that so far.
    fn start(
        &self,
        dir: &Path,
        args: &[&str],
        name: &str,
    ) -> (std::process::Child, impl Fn() -> String + Send + 'static) {
        let log = self.path(&format!("{name}.stderr"));
        let mut cmd = std::process::Command::new(assert_cmd::cargo::cargo_bin!("fearlezz"));
        cmd.args(args)
            .current_dir(dir)
            .env("XDG_CACHE_HOME", &self.repo.cache)
            .env("XDG_CONFIG_HOME", common::no_config())
            .env(QUEUE.0, QUEUE.1)
            .env_remove("CI")
            .env_remove("FEARLEZZ_JOURNAL")
            .stdout(Stdio::piped())
            .stderr(File::create(&log).unwrap());
        for name in common::ACTIONS_ENV {
            cmd.env_remove(name);
        }
        let child = cmd.spawn().unwrap();
        (child, move || {
            std::fs::read_to_string(&log).unwrap_or_default()
        })
    }

    /// Runs `args` in the second worktree on a thread, with `env`.
    fn in_b(&self, args: &[&str], env: &[(&str, &str)]) -> JoinHandle<Output> {
        let root = self.b.root.clone();
        let cache = self.repo.cache.clone();
        let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();
        let env: Vec<(String, String)> = env
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        thread::spawn(move || {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let env: Vec<(&str, &str)> =
                env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            fz(&root, &cache, &args, &env)
        })
    }
}

/// Waits for `child` on a thread, its stderr read from where `stderr` reads it.
fn finish(
    child: std::process::Child,
    stderr: impl Fn() -> String + Send + 'static,
) -> JoinHandle<Output> {
    thread::spawn(move || {
        let out = child.wait_with_output().unwrap();
        Output {
            ok: out.status.success(),
            stdout: String::from_utf8(out.stdout).unwrap(),
            stderr: stderr(),
        }
    })
}

/// Runs `fearlezz` in `dir` as the common helpers do, so a thread can own what it needs.
fn fz(dir: &Path, cache: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut env: Vec<(&str, &str)> = env.to_vec();
    let cache = cache.to_str().unwrap();
    env.push(("XDG_CACHE_HOME", cache));
    common::fz_at_env(dir, args, &env)
}

/// Whether `handle` finishes within `wait`.
fn finishes(handle: &JoinHandle<Output>, wait: Duration) -> bool {
    let deadline = Instant::now() + wait;
    while Instant::now() < deadline {
        if handle.is_finished() {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    handle.is_finished()
}

#[test]
fn a_heavy_check_waits_for_the_one_another_worktree_runs_and_says_behind_what() {
    let m = Machine::new(false);
    let held = m.hold();

    let waiting = m.queued_in_b(&["run", "gate"]);
    thread::sleep(Duration::from_millis(500));
    assert_eq!(m.runs(), ["gate"], "the second gate waits before it runs");

    m.open();
    let first = held.join().unwrap();
    let second = waiting.join().unwrap();
    assert!(first.ok, "{}\n{}", first.stdout, first.stderr);
    assert!(second.ok, "{}\n{}", second.stdout, second.stderr);
    assert_eq!(m.runs(), ["gate", "gate"]);
    let folder = m.repo.root.file_name().unwrap().to_string_lossy();
    assert!(
        second
            .stderr
            .contains("wait gate: the heavy queue is full (1 slot, FEARLEZZ_QUEUE=1)")
            && second
                .stderr
                .contains(&format!("behind gate in {folder} (pid ")),
        "{}",
        second.stderr
    );
}

#[test]
fn heavy_work_runs_at_once_without_the_queue_on_ci_and_for_a_light_check() {
    let m = Machine::new(false);
    let held = m.hold();

    for (args, env) in [
        (&["run", "other"][..], &[][..]),
        (
            &["run", "other", "--fresh"][..],
            &[QUEUE, ("CI", "true")][..],
        ),
        (&["run", "quick"][..], &[QUEUE][..]),
    ] {
        let run = m.in_b(args, env);
        assert!(
            finishes(&run, Duration::from_secs(30)),
            "{args:?} with {env:?} waited for the slot"
        );
        let out = run.join().unwrap();
        assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
        assert!(!out.stderr.contains("wait "), "{}", out.stderr);
    }

    m.open();
    assert!(held.join().unwrap().ok);
    assert_eq!(m.runs(), ["gate", "other", "other", "quick"]);
}

#[test]
fn a_waiting_check_reuses_what_another_worktree_proved_for_the_same_inputs() {
    let m = Machine::new(true);
    let held = m.hold();

    let waiting = m.queued_in_b(&["run", "gate"]);
    m.open();
    assert!(held.join().unwrap().ok);
    let second = waiting.join().unwrap();

    assert!(second.ok, "{}\n{}", second.stdout, second.stderr);
    assert_eq!(m.runs(), ["gate"], "the waiting gate never ran itself");
    assert!(
        second
            .stderr
            .contains("wait gate: the same inputs ran meanwhile; reused"),
        "{}",
        second.stderr
    );
    assert!(second.stdout.contains("1 reused"), "{}", second.stdout);
}

#[test]
fn a_tool_run_by_hand_queues_behind_heavy_work_and_exits_with_its_code() {
    let m = Machine::new(false);
    let held = m.hold();
    let runs = m.path("runs");

    let by_hand = m.queued_in_b(&[
        "run",
        "--",
        "sh",
        "-c",
        &format!("echo by-hand >> {}; exit 3", runs.display()),
    ]);
    thread::sleep(Duration::from_millis(500));
    assert_eq!(m.runs(), ["gate"]);

    m.open();
    assert!(held.join().unwrap().ok);
    let out = by_hand.join().unwrap();
    assert_eq!(m.runs(), ["gate", "by-hand"]);
    assert!(
        out.stderr.contains("wait sh -c echo by-hand"),
        "{}",
        out.stderr
    );
    let code = std::process::Command::new(assert_cmd::cargo::cargo_bin!("fearlezz"))
        .args(["run", "--", "sh", "-c", "exit 3"])
        .current_dir(m.dir.path())
        .env_remove("FEARLEZZ_QUEUE")
        .status()
        .unwrap()
        .code();
    assert_eq!(code, Some(3), "outside a repository too");
    assert!(!out.ok);
}

#[test]
fn fearlezz_inside_a_queued_command_runs_at_once_instead_of_waiting_on_itself() {
    let m = Machine::new(false);
    let fearlezz = assert_cmd::cargo::cargo_bin!("fearlezz");
    let (sent, done) = mpsc::channel();
    let b = m.b.root.clone();
    let cache = m.repo.cache.clone();
    thread::spawn(move || {
        let nested = format!("'{}' run -- true", fearlezz.display());
        let out = fz(&b, &cache, &["run", "--", "sh", "-c", &nested], &[QUEUE]);
        sent.send(out).unwrap();
    });

    let out = done
        .recv_timeout(Duration::from_secs(30))
        .expect("the nested fearlezz waited on the slot its parent holds");
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
}

#[test]
fn a_run_on_the_base_commit_holds_a_slot_too() {
    let m = Machine::new(false);
    let root = m.repo.root.clone();
    let cache = m.repo.cache.clone();
    let looking = thread::spawn(move || fz(&root, &cache, &["run", "shots"], &[QUEUE]));
    let wait_for = |name: &str| {
        let deadline = Instant::now() + Duration::from_secs(60);
        while !m.path(name).exists() {
            assert!(Instant::now() < deadline, "{name} never appeared");
            thread::sleep(Duration::from_millis(20));
        }
    };
    wait_for("started-change");
    std::fs::write(m.path("open-change"), "").unwrap();
    // The change's own run is done; the same check on the base commit holds the slot now.
    wait_for("started-base");

    let waiting = m.queued_in_b(&["run", "other"]);
    std::fs::write(m.path("open-base"), "").unwrap();
    let looked = looking.join().unwrap();
    let out = waiting.join().unwrap();

    assert!(looked.ok, "{}\n{}", looked.stdout, looked.stderr);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let folder = m.repo.root.file_name().unwrap().to_string_lossy();
    assert!(
        out.stderr.contains(&format!(
            "behind shots on the base commit in {folder} (pid "
        )),
        "{}",
        out.stderr
    );
}

/// Two runs in one worktree share its services, held by one run at a time. The hold always
/// comes before the slot: a run that waits for the services holds no slot, so a run that holds
/// the services and waits for a slot can always get one.
#[test]
fn a_run_waiting_for_its_worktrees_services_holds_no_slot() {
    let m = Machine::new(false);
    let held = m.hold();
    let first = m.queued(&m.repo.root, &["run", "svc"], "first");

    let (child, stderr) = m.start(&m.repo.root, &["run", "svc", "--fresh"], "second");
    let deadline = Instant::now() + Duration::from_secs(60);
    while !stderr().contains("wait svc: another run in this worktree holds its services") {
        assert!(
            Instant::now() < deadline,
            "the second run never said what it waits for:\n{}",
            stderr()
        );
        thread::sleep(Duration::from_millis(20));
    }
    thread::sleep(Duration::from_millis(500));
    assert!(
        !stderr().contains("heavy queue"),
        "the second run waits for the services, not for a slot:\n{}",
        stderr()
    );

    m.open();
    let second = finish(child, stderr);
    for out in [
        held.join().unwrap(),
        first.join().unwrap(),
        second.join().unwrap(),
    ] {
        assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    }
    assert_eq!(m.runs(), ["gate", "svc", "svc"]);
}

/// A light check never waits, not for its pictures' comparison either.
#[test]
fn a_light_checks_pictures_are_compared_without_waiting() {
    let m = Machine::new(false);
    common::render(&m.path("snap-base.png"), 1.0, "");
    common::render(&m.path("snap-change.png"), 1.0, &common::box_at(20, 0.0));
    let held = m.hold();

    let root = m.repo.root.clone();
    let cache = m.repo.cache.clone();
    let snap = thread::spawn(move || fz(&root, &cache, &["run", "snap"], &[QUEUE]));
    let done = finishes(&snap, Duration::from_secs(60));
    m.open();
    let out = snap.join().unwrap();
    assert!(held.join().unwrap().ok);
    assert!(
        done,
        "the light check waited:\n{}\n{}",
        out.stdout, out.stderr
    );
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("snap.png"), "{}", out.stdout);
    assert!(!out.stderr.contains("wait "), "{}", out.stderr);
}

/// A value that is no number of slots says so instead of quietly leaving the queue off.
#[test]
fn a_queue_value_that_is_no_number_of_slots_says_the_queue_is_off() {
    let m = Machine::new(false);
    for value in ["two", "0"] {
        let out = fz(
            &m.repo.root,
            &m.repo.cache,
            &["run", "other", "--fresh"],
            &[(QUEUE.0, value)],
        );
        assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
        assert!(
            out.stderr.contains(&format!(
                "queue: FEARLEZZ_QUEUE={value} is not a number of slots above zero, so the heavy queue is off"
            )),
            "{}",
            out.stderr
        );
    }
}
