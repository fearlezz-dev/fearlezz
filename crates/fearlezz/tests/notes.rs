//! The feedback loop from the agent's side: `fearlezz setup login` signs the CLI in to the
//! proof host through GitHub's device flow and keeps its token in the user's config folder;
//! `fearlezz` then shows what waits on the host for the branch, `fearlezz --notes` saves each
//! open note's pictures with its region drawn, and the hooks hold the agent from stopping
//! while notes are open. Without a sign in nothing is asked of the host.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use common::host::{FakeHost, HOST_TOKEN};
use common::{Output, Repo};
use serde_json::{Value, json};

/// A branch `feat/x` of a GitHub repository whose config names `url` as its proof host.
fn repo(url: &str) -> Repo {
    let repo = Repo::new(
        &format!("[check.tests]\nrun = \"true\"\nstage = \"working\"\n\n[host]\nurl = \"{url}\"\n"),
        &[("src/a.txt", "a\n")],
    );
    repo.git(&[
        "remote",
        "add",
        "origin",
        "https://github.com/owner/repo.git",
    ]);
    repo.branch("feat/x");
    repo
}

/// `fearlezz` with the user's config folder at `config` and `env` besides.
fn fz(repo: &Repo, config: &Path, args: &[&str]) -> Output {
    let config = config.to_str().unwrap();
    repo.fz_env(args, &[("XDG_CONFIG_HOME", config)])
}

/// The user's config folder holding a sign in to `url`.
fn signed_in(url: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("fearlezz");
    std::fs::create_dir_all(&folder).unwrap();
    let logins = json!({ url: {"login": "writer", "token": HOST_TOKEN} });
    std::fs::write(folder.join("logins.json"), logins.to_string()).unwrap();
    dir
}

fn note(id: &str, picture: &str, side: &str, region: [f64; 4], time: Option<f64>) -> Value {
    let [x, y, w, h] = region;
    let mut note = json!({
        "id": id, "check": "screens", "picture": picture, "run": "r1", "head": "abc1234def",
        "side": side, "region": {"x": x, "y": y, "w": w, "h": h},
        "words": format!("Fix {picture}"), "by": "writer", "at": 10.0, "state": "open",
    });
    if let Some(t) = time {
        note["time"] = json!(t);
    }
    note
}

/// What the fake host says waits for `feat/x`: a still with one note, which needs change though
/// it was said to look right before, a film with one note at a moment, a still nobody looked at
/// yet and one that looks right.
fn feedback(host: &FakeHost) -> Value {
    let still = note("n1", "card.png", "after", [0.25, 0.25, 0.5, 0.5], None);
    let film = note("n2", "flow.gif", "both", [0.8, 0.1, 0.15, 0.2], Some(1.5));
    json!({
        "work": {"repository": "owner/repo", "branch": "feat/x", "pull": {"number": 24, "title": "Card"}},
        "url": format!("{}/owner/repo/pull/24", host.url),
        "review": {
            "head": "abc1234def", "state": "needs_change", "weakened": [],
            "pictures": [
                {"check": "screens", "name": "card.png", "key": "k1", "standing": "needs_change",
                 "verdict": {"check": "screens", "picture": "card.png", "key": "k1",
                    "by": "writer", "at": 11.0, "run": "r1", "head": "abc1234def"}},
                {"check": "screens", "name": "flow.gif", "key": "k2", "standing": "needs_change"},
                {"check": "screens", "name": "menu.png", "key": "k3", "standing": "waits"},
                {"check": "screens", "name": "about.png", "key": "k4", "standing": "looks_right",
                 "verdict": {"check": "screens", "picture": "about.png", "key": "k4",
                    "by": "writer", "at": 12.0, "run": "r1", "head": "abc1234def"}},
            ],
            "notes": [still, film],
        },
        "notes": [
            {"note": still, "kind": "still",
             "before": "looks/screens/before/card.png", "after": "looks/screens/after/card.png"},
            {"note": film, "kind": "film",
             "before": "looks/screens/before/flow.gif", "after": "looks/screens/after/flow.gif"},
        ],
    })
}

/// A white still of 160 by 120, and a film of the same size whose blue box shows from 1 s on.
fn pictures(host: &FakeHost, dir: &Path) {
    let still = dir.join("card.png");
    common::render(&still, 1.0, "");
    let film = dir.join("flow.gif");
    common::render(&film, 2.0, &common::box_at(20, 1.0));
    let mut state = host.state.lock().unwrap();
    for side in ["before", "after"] {
        for (name, path) in [("card.png", &still), ("flow.gif", &film)] {
            state.pictures.insert(
                format!("/api/v2/runs/r1/files/looks/screens/{side}/{name}"),
                std::fs::read(path).unwrap(),
            );
        }
    }
}

/// The RGB of pixel `(x, y)` of a PNG.
fn pixel(png: &Path, x: usize, y: usize) -> [u8; 3] {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(png)
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let at = (y * 160 + x) * 3;
    [out.stdout[at], out.stdout[at + 1], out.stdout[at + 2]]
}

fn red(rgb: [u8; 3]) -> bool {
    rgb[0] > 180 && rgb[1] < 90 && rgb[2] < 90
}

fn white(rgb: [u8; 3]) -> bool {
    rgb.iter().all(|c| *c > 230)
}

fn blue(rgb: [u8; 3]) -> bool {
    rgb[2] > 180 && rgb[0] < 90
}

/// The paths `--notes` printed, by what they end in.
fn saved(out: &Output, ending: &str) -> PathBuf {
    let line = out
        .stdout
        .lines()
        .find(|l| {
            l.split_whitespace()
                .next()
                .is_some_and(|p| p.ends_with(ending))
        })
        .unwrap_or_else(|| panic!("no {ending} in\n{}", out.stdout));
    PathBuf::from(line.split_whitespace().next().unwrap())
}

#[test]
fn login_keeps_a_token_only_the_user_can_read_and_logout_removes_it() {
    let host = FakeHost::start();
    let repo = repo(&host.url);
    let config = tempfile::tempdir().unwrap();
    let out = fz(&repo, config.path(), &["setup", "login"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("FZ12-3456"), "{}", out.stdout);
    assert!(
        out.stdout.contains("https://github.com/login/device"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("signed in as writer"), "{}", out.stdout);
    assert!(
        out.last_line().starts_with("next: fearlezz"),
        "{}",
        out.stdout
    );
    let file = config.path().join("fearlezz/logins.json");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains(HOST_TOKEN), "{text}");
    assert!(text.contains(&host.url), "{text}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "{mode:o}");
    }
    assert!(!repo.git(&["status", "--porcelain"]).contains("logins"));

    let out = fz(&repo, config.path(), &["setup", "logout"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(out.last_line().starts_with("next:"), "{}", out.stdout);
    assert!(!std::fs::read_to_string(&file).unwrap().contains(HOST_TOKEN));
    assert!(host.calls().contains(&"DELETE /api/v2/login".to_string()));
}

#[test]
fn login_says_whether_the_app_is_installed_on_the_repository() {
    let host = FakeHost::start();
    let repo = repo(&host.url);
    let config = tempfile::tempdir().unwrap();
    let out = fz(&repo, config.path(), &["setup", "login"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("the Fearlezz GitHub App is installed on owner/repo"),
        "{}",
        out.stdout
    );
    assert!(
        host.calls()
            .contains(&"GET /api/v2/installed?repository=owner%2Frepo".to_string()),
        "{:?}",
        host.calls()
    );
    assert!(out.last_line().starts_with("next:"), "{}", out.stdout);

    host.state.lock().unwrap().installed = false;
    let out = fz(&repo, config.path(), &["setup", "login"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains(
            "the Fearlezz GitHub App is not installed on owner/repo, so its runs show no page; \
             install it: https://github.com/apps/fearlezz"
        ),
        "{}",
        out.stdout
    );

    // A host that cannot say is a line, never a failure.
    host.state
        .lock()
        .unwrap()
        .broken
        .insert("/api/v2/installed".into());
    let out = fz(&repo, config.path(), &["setup", "login"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("could not ask whether the Fearlezz GitHub App is installed on owner/repo"),
        "{}",
        out.stdout
    );
    assert!(out.last_line().starts_with("next:"), "{}", out.stdout);
}

#[test]
fn status_shows_what_waits_on_the_host_and_leads_to_the_notes() {
    let host = FakeHost::start();
    host.state.lock().unwrap().feedback = Some(feedback(&host));
    let repo = repo(&host.url);
    let config = signed_in(&host.url);
    let out = fz(&repo, config.path(), &[]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("proof host"), "{}", out.stdout);
    assert!(out.stdout.contains("pull request #24"), "{}", out.stdout);
    let card = out
        .lines()
        .into_iter()
        .find(|l| l.contains("card.png"))
        .unwrap_or_default();
    assert!(
        card.contains("needs change")
            && card.contains("1 open note")
            && !card.contains("looks right"),
        "an open note outweighs looks right: {}",
        out.stdout
    );
    let line = |name: &str| {
        out.lines()
            .into_iter()
            .find(|l| l.contains(name))
            .unwrap_or_default()
    };
    assert!(line("flow.gif").contains("needs change"), "{}", out.stdout);
    assert!(
        line("menu.png").contains("not looked at yet"),
        "{}",
        out.stdout
    );
    assert!(
        line("about.png").contains("looks right, said by writer"),
        "{}",
        out.stdout
    );
    assert!(
        out.last_line().starts_with("next: fearlezz --notes"),
        "{}",
        out.stdout
    );
    let asked = host
        .requests()
        .into_iter()
        .find(|r| r.path.starts_with("/api/v2/work/"))
        .unwrap();
    assert_eq!(asked.path, "/api/v2/work/owner/repo/feat/x");
}

#[test]
fn without_a_sign_in_status_asks_nothing_of_the_host() {
    let host = FakeHost::start();
    let repo = repo(&host.url);
    let config = tempfile::tempdir().unwrap();
    let out = fz(&repo, config.path(), &[]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(host.requests().is_empty(), "{:?}", host.calls());
    assert!(!out.stdout.contains("proof host"), "{}", out.stdout);
}

#[test]
fn a_host_that_cannot_be_reached_is_one_line_and_status_goes_on() {
    // Nothing listens on port 9 of this machine.
    let url = "http://127.0.0.1:9";
    let repo = repo(url);
    let config = signed_in(url);
    let started = Instant::now();
    let out = fz(&repo, config.path(), &[]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(started.elapsed().as_secs() < 8, "{:?}", started.elapsed());
    let said: Vec<&str> = out
        .lines()
        .into_iter()
        .filter(|l| l.contains("proof host"))
        .collect();
    assert_eq!(said.len(), 1, "{}", out.stdout);
    assert!(said[0].contains("could not be reached"), "{}", out.stdout);
    assert!(out.last_line().starts_with("next:"), "{}", out.stdout);
}

#[test]
fn notes_saves_each_notes_pictures_with_its_region_drawn() {
    let host = FakeHost::start();
    host.state.lock().unwrap().feedback = Some(feedback(&host));
    let made = tempfile::tempdir().unwrap();
    pictures(&host, made.path());
    let repo = repo(&host.url);
    let config = signed_in(&host.url);
    let out = fz(&repo, config.path(), &["--notes"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("\"Fix card.png\""), "{}", out.stdout);
    assert!(out.stdout.contains("at 1.50 s"), "{}", out.stdout);
    assert!(out.last_line().starts_with("next:"), "{}", out.stdout);

    // The still: the region on the after, its border red and its inside as it was; the before
    // beside it, untouched, to compare.
    let after = saved(&out, "1-after.png");
    let before = saved(&out, "1-before.png");
    assert!(
        after.starts_with(&repo.cache),
        "outside the repository: {}",
        after.display()
    );
    assert!(red(pixel(&after, 41, 60)), "{:?}", pixel(&after, 41, 60));
    assert!(red(pixel(&after, 80, 31)), "{:?}", pixel(&after, 80, 31));
    assert!(white(pixel(&after, 80, 60)), "{:?}", pixel(&after, 80, 60));
    assert!(white(pixel(&after, 10, 10)), "{:?}", pixel(&after, 10, 10));
    assert!(
        white(pixel(&before, 41, 60)),
        "{:?}",
        pixel(&before, 41, 60)
    );

    // The film: the frame at 1.5 s, where the blue box shows, drawn on both sides.
    for side in ["before", "after"] {
        let frame = saved(&out, &format!("2-{side}.png"));
        assert!(
            blue(pixel(&frame, 45, 50)),
            "{side}: {:?}",
            pixel(&frame, 45, 50)
        );
        assert!(
            red(pixel(&frame, 129, 30)),
            "{side}: {:?}",
            pixel(&frame, 129, 30)
        );
    }
}

#[test]
fn notes_saves_a_noted_text_as_it_is() {
    let host = FakeHost::start();
    let mut said = feedback(&host);
    let text = note("n3", "app", "after", [0.0, 0.0, 1.0, 1.0], None);
    said["notes"] = json!([{"note": text, "kind": "text",
        "before": "looks/render/media/before-app.yaml.txt",
        "after": "looks/render/media/after-app.yaml.txt"}]);
    {
        let mut state = host.state.lock().unwrap();
        state.feedback = Some(said);
        for side in ["before", "after"] {
            state.pictures.insert(
                format!("/api/v2/runs/r1/files/looks/render/media/{side}-app.yaml.txt"),
                format!("color: {side}\n").into_bytes(),
            );
        }
    }
    let repo = repo(&host.url);
    let config = signed_in(&host.url);
    let out = fz(&repo, config.path(), &["--notes"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("the after text"), "{}", out.stdout);
    let after = saved(&out, "1-after.txt");
    assert_eq!(std::fs::read_to_string(after).unwrap(), "color: after\n");
    let before = saved(&out, "1-before.txt");
    assert_eq!(std::fs::read_to_string(before).unwrap(), "color: before\n");
}

#[test]
fn notes_without_a_sign_in_leads_to_login() {
    let host = FakeHost::start();
    let repo = repo(&host.url);
    let config = tempfile::tempdir().unwrap();
    let out = fz(&repo, config.path(), &["--notes"]);
    assert!(!out.ok);
    assert_eq!(
        out.last_line(),
        "next: fearlezz setup login",
        "{}",
        out.stdout
    );
}

#[test]
fn open_notes_hold_the_agent_from_stopping_once() {
    let host = FakeHost::start();
    host.state.lock().unwrap().feedback = Some(feedback(&host));
    let repo = repo(&host.url);
    let config = signed_in(&host.url);
    let input = |again: bool| json!({"cwd": repo.root, "stop_hook_active": again}).to_string();
    let hook = |again: bool| {
        let out = assert_cmd::Command::cargo_bin("fearlezz")
            .unwrap()
            .args(["setup", "hook", "claude", "stop"])
            .current_dir(&repo.root)
            .env("XDG_CACHE_HOME", &repo.cache)
            .env("XDG_CONFIG_HOME", config.path())
            .env_remove("CI")
            .write_stdin(input(again))
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    };
    let first = hook(false);
    let answer: Value = serde_json::from_str(&first).unwrap_or_else(|_| panic!("{first}"));
    assert_eq!(answer["decision"], "block", "{answer}");
    let reason = answer["reason"].as_str().unwrap();
    assert!(reason.contains("fearlezz --notes"), "{reason}");
    assert!(reason.contains("2 open notes"), "{reason}");
    assert_eq!(hook(true).trim(), "", "only once");
}
