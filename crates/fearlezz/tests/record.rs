//! Recording an app's window: a `record` table on a check, and a still on request. Recording
//! itself needs macOS with Screen Recording; these tests turn the recorder off
//! (`FEARLEZZ_RECORDER=off`) so they say the same on every machine.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;

const CONFIG: &str = r#"
[check.app-look]
run = "echo ran > .runs/app-look"
stage = "look"
inputs = ["app/**"]
record = { film = "home", seconds = 2 }

[check.unit]
run = "true"
stage = "working"
inputs = ["app/**"]
"#;

const OFF: (&str, &str) = ("FEARLEZZ_RECORDER", "off");

fn repo() -> Repo {
    let repo = Repo::new(
        CONFIG,
        &[(".gitignore", ".runs/\n"), ("app/main.txt", "v1")],
    );
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    repo.branch("feat/change");
    repo.write("app/main.txt", "v2");
    repo
}

#[test]
fn ci_does_not_run_a_check_that_cannot_record_there_and_says_so() {
    let repo = repo();
    repo.commit("change");
    let out = repo.fz_env(&["run", "--ci"], &[OFF]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("not run app-look: the recorder is off"),
        "{}",
        out.stdout
    );
    assert!(
        !repo.path(".runs/app-look").exists(),
        "its command never ran"
    );
    assert!(out.stdout.contains("1 not run"), "{}", out.stdout);
}

#[test]
fn locally_a_check_that_cannot_record_fails_with_the_one_time_fix() {
    let repo = repo();
    repo.fz(&["run", "--stage", "look", "--no-run"]);
    let out = repo.fz_env(&["run"], &[OFF]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("fail app-look: the recorder is off"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("  fix: "), "{}", out.stdout);
    assert!(
        !repo.path(".runs/app-look").exists(),
        "its command never ran"
    );
    // Nothing is stored, so once recording works the check is not called unstable.
    let again = repo.fz_env(&["run"], &[OFF]);
    assert!(!again.stdout.contains("unstable"), "{}", again.stdout);
    assert!(
        again.last_line().contains("permission") && again.last_line().contains("fearlezz run"),
        "{}",
        again.stdout
    );
    let status = repo.fz(&[]);
    assert!(
        status.last_line().contains("app-look"),
        "it is still needed: {}",
        status.stdout
    );
}

#[test]
fn a_still_on_request_says_what_to_fix_when_nothing_can_record() {
    let repo = repo();
    let out = repo.fz_env(&["--still", "Demo"], &[OFF]);
    assert!(!out.ok);
    assert!(out.stderr.contains("the recorder is off"), "{}", out.stderr);
}

#[test]
fn a_record_names_a_still_or_a_film() {
    let repo = Repo::new(
        "[check.look]\nrun = \"true\"\nstage = \"look\"\nrecord = { window = \"Demo\" }\n",
        &[],
    );
    let out = repo.fz(&[]);
    assert!(!out.ok);
    assert!(
        out.stderr.contains("record: name a still, a film or both"),
        "{}",
        out.stderr
    );
}

#[test]
fn recording_less_weakens_the_proof() {
    let repo = repo();
    repo.write(
        "fearlezz.toml",
        &format!(
            "{}\n[settings.skill]\nagents = []\n\n[settings.hooks]\nagents = []\n",
            CONFIG.replace(
                "record = { film = \"home\", seconds = 2 }",
                "record = { still = \"home\", seconds = 1, wait = 600 }"
            )
        ),
    );
    repo.commit("change");
    let out = repo.fz_env(&["run", "--ci"], &[OFF]);
    assert!(!out.ok, "{}", out.stdout);
    for how in [
        "it no longer records the film home",
        "it records for less time (2 s, now 1 s)",
        "it waits longer for its window (300 s, now 600 s)",
    ] {
        assert!(out.stdout.contains(how), "{how}: {}", out.stdout);
    }
}

#[test]
fn an_empty_window_name_is_refused_since_it_would_match_any_window() {
    let repo = Repo::new(
        "[check.look]\nrun = \"true\"\nstage = \"look\"\nrecord = { film = \"app\", window = \" \" }\n",
        &[],
    );
    let out = repo.fz(&[]);
    assert!(!out.ok);
    assert!(
        out.stderr.contains("record: window names no window"),
        "{}",
        out.stderr
    );
}

#[test]
fn dropping_the_record_of_a_check_weakens_the_proof() {
    let repo = repo();
    repo.write(
        "fearlezz.toml",
        &format!(
            "{}\n[settings.skill]\nagents = []\n\n[settings.hooks]\nagents = []\n",
            CONFIG.replace("record = { film = \"home\", seconds = 2 }\n", "")
        ),
    );
    repo.commit("change");
    let out = repo.fz_env(&["run", "--ci"], &[OFF]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("it no longer records its app's window"),
        "{}",
        out.stdout
    );
}

/// The screen this machine can never have: a Linux display on a Mac, the iOS simulator
/// elsewhere; with what it needs, and the OS as a platform's `os` and as people write it.
const ELSEWHERE: (&str, &str, &str, &str) = if cfg!(target_os = "macos") {
    ("linux", "needs Linux", "linux", "Linux")
} else {
    ("ios", "needs macOS", "macos", "macOS")
};

#[test]
fn locally_a_screen_this_machine_never_has_is_left_to_ci() {
    let (screen, needs, platform_os, os) = ELSEWHERE;
    // No record: needing the screen is enough.
    let repo = Repo::new(
        &format!(
            "[check.app-look]\nrun = \"echo ran > .runs/app-look\"\nstage = \"look\"\n\
             needs = [\"{screen}\"]\n"
        ),
        &[(".gitignore", ".runs/\n")],
    );
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    repo.branch("feat/change");
    repo.write("app.txt", "v2");
    repo.fz(&["run", "--stage", "look", "--no-run"]);
    let out = repo.fz(&["run"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains(&format!("left to CI app-look: {needs}")),
        "{}",
        out.stdout
    );
    assert!(
        !repo.path(".runs/app-look").exists(),
        "its command never ran"
    );
    // CI that runs it on a machine without the screen has not proven it.
    let ci = repo.fz(&["run", "--ci"]);
    assert!(!ci.ok, "{}", ci.stdout);
    assert!(
        ci.stdout
            .contains(&format!("check `app-look` on {os} did not run")),
        "{}",
        ci.stdout
    );
    assert!(
        ci.stdout.contains(&format!(
            "fix: add a [platform.<name>] with os = \"{platform_os}\""
        )),
        "{}",
        ci.stdout
    );
}

#[test]
fn a_check_that_needs_a_screen_runs_on_the_platforms_of_its_os() {
    let (screen, _, os, _) = ELSEWHERE;
    let repo = Repo::new(
        &format!(
            "[platform.anywhere]\nruns-on = \"ubuntu-latest\"\ndefault = true\n\n\
             [platform.screen]\nruns-on = \"some-runner\"\nos = \"{os}\"\n\n\
             [check.unit]\nrun = \"true\"\nstage = \"working\"\n\n\
             [check.app-look]\nrun = \"true\"\nstage = \"look\"\nneeds = [\"{screen}\"]\n"
        ),
        &[],
    );
    let out = repo.fz(&["run", "--ci", "--plan"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let legs: serde_json::Value = serde_json::from_str(out.stdout.trim()).unwrap();
    let checks = |id: &str| {
        legs.as_array()
            .unwrap()
            .iter()
            .find(|l| l["leg"] == id)
            .map(|l| l["checks"].to_string())
            .unwrap_or_default()
    };
    assert!(checks("screen").contains("app-look"), "{legs}");
    assert!(!checks("anywhere").contains("app-look"), "{legs}");
}

#[test]
fn in_a_leg_a_screen_this_machine_never_has_fails() {
    let (screen, needs, _, os) = ELSEWHERE;
    let repo = Repo::new(
        &format!(
            "[platform.anywhere]\nruns-on = \"ubuntu-latest\"\ndefault = true\n\n\
             [check.app-look]\nrun = \"echo ran > .runs/app-look\"\nstage = \"look\"\n\
             needs = [\"{screen}\"]\n"
        ),
        &[(".gitignore", ".runs/\n")],
    );
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    let out = repo.fz(&["run", "--ci", "--leg", "anywhere"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains(&format!("fail app-look: {needs}")),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout
            .contains(&format!("fix: put it on a platform whose os is {os}")),
        "{}",
        out.stdout
    );
    assert!(
        !repo.path(".runs/app-look").exists(),
        "its command never ran"
    );
}

#[test]
fn a_check_needs_a_screen_fearlezz_knows() {
    let repo = Repo::new(
        "[check.look]\nrun = \"true\"\nstage = \"look\"\nneeds = [\"android\"]\n",
        &[],
    );
    let out = repo.fz(&[]);
    assert!(!out.ok);
    assert!(
        out.stderr.contains("android") && out.stderr.contains("linux"),
        "{}",
        out.stderr
    );
}

#[test]
fn a_check_refuses_what_its_screen_does_not_have() {
    for (keys, says) in [
        (
            "needs = [\"ios\"]\nrecord = { film = \"app\", crop_top = 28 }",
            "crop_top is only for a window",
        ),
        (
            "device = \"iPhone 17\"",
            "device is only for a check that needs = [\"ios\"]",
        ),
        (
            "display = \"1280x720\"",
            "display is only for a check that needs = [\"linux\"]",
        ),
        (
            "needs = [\"linux\"]\ndisplay = \"wide\"",
            "display = \"wide\" is not a width and height such as \"1280x720\"",
        ),
        ("needs = [\"ios\", \"linux\"]", "needs both ios and linux"),
    ] {
        let repo = Repo::new(
            &format!("[check.look]\nrun = \"true\"\nstage = \"look\"\n{keys}\n"),
            &[],
        );
        let out = repo.fz(&[]);
        assert!(!out.ok, "{keys}");
        assert!(out.stderr.contains(says), "{keys}: {}", out.stderr);
    }
}

#[test]
fn leaving_a_screen_weakens_the_proof() {
    let base = "[check.app-look]\nrun = \"true\"\nstage = \"look\"\nneeds = [\"linux\"]\n";
    let repo = Repo::new(base, &[]);
    repo.branch("feat/change");
    repo.write(
        "fearlezz.toml",
        "[check.app-look]\nrun = \"true\"\nstage = \"look\"\n\n\
         [settings.skill]\nagents = []\n\n[settings.hooks]\nagents = []\n",
    );
    repo.commit("change");
    let out = repo.fz_env(&["run", "--ci"], &[OFF]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("it runs on another screen (a Linux display, now this machine's screen)"),
        "{}",
        out.stdout
    );
}

/// The check's output in the proof bundle a run wrote with `--proof proof`, where the
/// recorder says what it wrote (`record: still home.png 320x200 in 0.12 s`).
fn recorder_log(repo: &Repo) -> String {
    std::fs::read_to_string(repo.path("proof/outputs/app-look.log")).unwrap()
}

/// A machine without the screen's tool skips a recording test; CI's runners have it, so
/// there a missing tool fails the test rather than letting it pass without proving anything.
fn skip_off_ci(why: &str) {
    assert!(
        std::env::var_os("CI").is_none(),
        "CI must prove this: {why}"
    );
    eprintln!("skipped: {why}");
}

/// The size of the still `home.png` the recorder says it wrote.
fn still_size(log: &str) -> (u32, u32) {
    log.lines()
        .find_map(|l| l.trim().strip_prefix("record: still home.png "))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|wh| wh.split_once('x'))
        .map_or_else(
            || panic!("a still: {log}"),
            |(w, h)| (w.parse().unwrap(), h.parse().unwrap()),
        )
}

/// Under a Linux display Fearlezz starts itself (Xvfb), the first window that appears is
/// recorded, still and film, without any screen of the developer's. The window comes from
/// this test binary run again as a tiny X client ([`x_client`]).
#[cfg(target_os = "linux")]
#[test]
fn a_linux_display_records_the_window_that_appears() {
    if std::process::Command::new("Xvfb")
        .arg("-help")
        .output()
        .is_err()
    {
        skip_off_ci("no Xvfb on this machine");
        return;
    }
    let me = std::env::current_exe().unwrap();
    let repo = Repo::new(
        &format!(
            "[check.app-look]\nrun = \"FZ_X_CLIENT=6 '{}' --ignored --exact x_client\"\n\
             stage = \"look\"\nneeds = [\"linux\"]\ndisplay = \"640x480\"\nrecord = {{ still = \"home\", film = \"home\" }}\n",
            me.display()
        ),
        &[],
    );
    let out = repo.fz(&["run", "--ci", "--proof", "proof"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let log = recorder_log(&repo);
    assert_eq!(still_size(&log), (320, 200), "the window, not the display");
    assert!(log.contains("record: film home.mp4 of "), "{log}");
}

/// A check that needs a Linux display gets one in `DISPLAY` without recording it: the X
/// client connects there or fails.
#[cfg(target_os = "linux")]
#[test]
fn a_check_that_needs_a_linux_display_gets_one_without_recording() {
    if std::process::Command::new("Xvfb")
        .arg("-help")
        .output()
        .is_err()
    {
        skip_off_ci("no Xvfb on this machine");
        return;
    }
    let me = std::env::current_exe().unwrap();
    let repo = Repo::new(
        &format!(
            "[check.app]\nrun = \"FZ_X_CLIENT=1 '{}' --ignored --exact x_client\"\n\
             stage = \"working\"\nneeds = [\"linux\"]\n",
            me.display()
        ),
        &[],
    );
    let out = repo.fz(&["run", "--ci"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
}

/// A window larger than the display and placed across its edge, odd sized: X reads only
/// what is on the display, so the rest is black, and the still keeps the window's size.
#[cfg(target_os = "linux")]
#[test]
fn a_linux_display_records_a_window_larger_than_it_and_off_its_edge() {
    if std::process::Command::new("Xvfb")
        .arg("-help")
        .output()
        .is_err()
    {
        skip_off_ci("no Xvfb on this machine");
        return;
    }
    let me = std::env::current_exe().unwrap();
    let repo = Repo::new(
        &format!(
            "[check.app-look]\nrun = \"FZ_X_CLIENT=6,701,501,-5,-7 '{}' --ignored --exact x_client\"\n\
             stage = \"look\"\nneeds = [\"linux\"]\ndisplay = \"640x480\"\nrecord = {{ still = \"home\", film = \"home\" }}\n",
            me.display()
        ),
        &[],
    );
    let out = repo.fz(&["run", "--ci", "--proof", "proof"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let log = recorder_log(&repo);
    assert_eq!(still_size(&log), (701, 501), "{log}");
    assert!(log.contains("record: film home.mp4 of "), "{log}");
}

/// Not a test: run by the Linux display tests with `FZ_X_CLIENT=<seconds>` (a 320x200
/// window at the corner) or `<seconds>,<width>,<height>,<x>,<y>`, it shows a window on
/// `DISPLAY` for that long.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "an X client the Linux display test starts"]
fn x_client() {
    use x11rb::connection::Connection as _;
    use x11rb::protocol::xproto::{ConnectionExt as _, CreateWindowAux, WindowClass};
    // "<seconds>" or "<seconds>,<width>,<height>,<x>,<y>".
    let Some(spec) = std::env::var("FZ_X_CLIENT").ok() else {
        return;
    };
    let n: Vec<i32> = spec.split(',').map(|p| p.parse().unwrap()).collect();
    let (seconds, width, height, x, y) = match n[..] {
        [s] => (s, 320, 200, 0, 0),
        [s, w, h, x, y] => (s, w, h, x, y),
        _ => panic!("FZ_X_CLIENT={spec}"),
    };
    let (conn, screen) = x11rb::connect(None).unwrap();
    let root = conn.setup().roots[screen].clone();
    let id = conn.generate_id().unwrap();
    conn.create_window(
        root.root_depth,
        id,
        root.root,
        i16::try_from(x).unwrap(),
        i16::try_from(y).unwrap(),
        u16::try_from(width).unwrap(),
        u16::try_from(height).unwrap(),
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new().background_pixel(0x00_33_99_ff),
    )
    .unwrap();
    conn.map_window(id).unwrap();
    conn.flush().unwrap();
    std::thread::sleep(std::time::Duration::from_secs(
        u64::try_from(seconds).unwrap(),
    ));
}

/// On the iOS simulator Fearlezz leases, the app the command launches is recorded.
#[cfg(target_os = "macos")]
#[test]
fn an_ios_simulator_records_the_app_the_command_launches() {
    // An iOS runtime, not only simctl: Xcode can be there without one.
    if std::process::Command::new("/usr/bin/xcrun")
        .args(["simctl", "list", "runtimes"])
        .output()
        .map_or(true, |o| {
            !o.status.success() || !String::from_utf8_lossy(&o.stdout).contains("iOS")
        })
    {
        skip_off_ci("no iOS simulator on this machine");
        return;
    }
    let repo = Repo::new(
        "[check.app-look]\n\
         run = \"/usr/bin/xcrun simctl launch \\\"$FEARLEZZ_DEVICE\\\" com.apple.Preferences && sleep 3\"\n\
         stage = \"look\"\nneeds = [\"ios\"]\nrecord = { still = \"home\", film = \"home\" }\n",
        &[],
    );
    let out = repo.fz(&["run", "--ci", "--proof", "proof"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let log = recorder_log(&repo);
    let (w, h) = still_size(&log);
    assert!(h > w && w > 300, "a phone's screen: {w}x{h}");
    assert!(log.contains("record: film home.mp4 of "), "{log}");
}
