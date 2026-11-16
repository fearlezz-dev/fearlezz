//! Local runs on the proof host: once the developer signed in (`fearlezz setup login`), every
//! `fearlezz run` streams to the host as CI's runs do, with the CLI's token and a local source,
//! so a person can leave notes on its pictures before a pull request exists. Without a sign in
//! nothing leaves the machine, and a host that fails never fails the run and costs it at most
//! one short wait.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use std::path::Path;

use common::host::{FakeHost, HOST_TOKEN, OIDC};
use common::{Output, Repo};
use serde_json::json;

/// Two checks, each calling the fake host when its command runs, so the calls show when.
fn checks(url: &str) -> String {
    format!(
        r#"
[check.tests]
run = "curl -s {url}/mark/tests > /dev/null; echo all tests passed"
stage = "working"
inputs = ["src/**"]

[check.lint]
run = "curl -s {url}/mark/lint > /dev/null; echo all clean"
stage = "working"
inputs = ["src/**"]
"#
    )
}

/// A branch `feat/x` of a GitHub repository with a change, whose config names `url` as its
/// proof host.
fn repo(url: &str) -> Repo {
    let repo = Repo::new(
        &format!("{}\n[host]\nurl = \"{url}\"\n", checks(url)),
        &[("src/a.txt", "a\n")],
    );
    repo.git(&[
        "remote",
        "add",
        "origin",
        "https://github.com/owner/repo.git",
    ]);
    repo.branch("feat/x");
    repo.write("src/a.txt", "b\n");
    repo.commit("change");
    repo
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

/// `fearlezz run` at the stage of both checks, with the user's config folder at `config`.
fn run(repo: &Repo, config: &Path) -> Output {
    repo.fz_env(
        &["run", "--stage", "working"],
        &[("XDG_CONFIG_HOME", config.to_str().unwrap())],
    )
}

#[test]
fn a_signed_in_run_opens_a_local_run_before_its_checks_and_streams_them() {
    let host = FakeHost::start();
    let repo = repo(&host.url);
    let config = signed_in(&host.url);
    let out = run(&repo, config.path());
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let calls: Vec<String> = host
        .calls()
        .into_iter()
        .filter(|c| !c.starts_with("PUT "))
        .collect();
    assert_eq!(
        calls,
        [
            "GET /api/v2/installed?repository=owner%2Frepo",
            "POST /api/v2/runs",
            "POST /api/v2/runs/r1/checks started lint",
            "GET /mark/lint",
            "POST /api/v2/runs/r1/checks finished lint",
            "POST /api/v2/runs/r1/checks started tests",
            "GET /mark/tests",
            "POST /api/v2/runs/r1/checks finished tests",
            "POST /api/v2/runs/r1/done",
        ],
        "it opens the run before any check runs and closes it last, as CI does"
    );
    assert!(
        host.api()
            .iter()
            .all(|r| r.auth.as_deref() == Some(&format!("Bearer {HOST_TOKEN}"))),
        "every call carries the CLI's token, never an OIDC token ({OIDC})"
    );
    assert!(
        host.requests()
            .iter()
            .all(|r| !r.path.starts_with("/token")),
        "nothing asks GitHub Actions for a token"
    );

    let open = host.sent("POST", "/api/v2/runs");
    assert_eq!(open["source"]["kind"], "local", "{open:#}");
    assert_eq!(open["repository"], "owner/repo", "{open:#}");
    assert_eq!(open["branch"], "feat/x", "{open:#}");
    assert_eq!(open["checks"], json!(["lint", "tests"]), "{open:#}");
    assert!(open.get("pull").is_none(), "{open:#}");
    assert_eq!(open["head"].as_str().unwrap().len(), 40, "{open:#}");

    let done = host.sent("POST", "/api/v2/runs/r1/done");
    let ids: Vec<&str> = done["proof"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["lint", "tests"], "{done:#}");
    assert_eq!(done["proof"]["repository"], "owner/repo", "{done:#}");
    assert!(
        done["proof"].get("accepted").is_none()
            && !host.calls().iter().any(|c| c.ends_with("/weakenings")),
        "a local run never asks for an accept: {done:#}"
    );

    // The page once, with the one line way to stop, and next: last.
    let page = format!("{}/owner/repo/work/feat/x", host.url);
    let named: Vec<&str> = out
        .lines()
        .into_iter()
        .filter(|l| l.contains(&page))
        .collect();
    assert_eq!(named.len(), 1, "{}", out.stdout);
    assert!(named[0].contains("unproven"), "{}", out.stdout);
    assert!(named[0].contains("fearlezz setup logout"), "{}", out.stdout);
    assert!(out.last_line().starts_with("next:"), "{}", out.stdout);
}

#[test]
fn without_a_sign_in_a_run_sends_nothing() {
    let host = FakeHost::start();
    let repo = repo(&host.url);
    let config = tempfile::tempdir().unwrap();
    let out = run(&repo, config.path());
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let calls: Vec<String> = host
        .calls()
        .into_iter()
        .filter(|c| !c.contains("/mark/"))
        .collect();
    assert!(calls.is_empty(), "{calls:#?}");
    assert!(!out.stdout.contains("proof page"), "{}", out.stdout);
}

#[test]
fn a_repository_without_the_app_sends_no_run() {
    let host = FakeHost::start();
    host.state.lock().unwrap().installed = false;
    let repo = repo(&host.url);
    let config = signed_in(&host.url);
    let out = run(&repo, config.path());
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let api: Vec<String> = host
        .calls()
        .into_iter()
        .filter(|c| c.contains("/api/"))
        .collect();
    assert_eq!(
        api,
        ["GET /api/v2/installed?repository=owner%2Frepo"],
        "{api:#?}"
    );
    let said: Vec<&str> = out
        .lines()
        .into_iter()
        .filter(|l| l.contains("proof page"))
        .collect();
    assert_eq!(said.len(), 1, "{}", out.stdout);
    assert!(said[0].contains("not installed"), "{}", out.stdout);
}

#[test]
fn a_host_that_fails_never_fails_the_run_and_is_given_up_at_once() {
    for broken in [
        "/api/v2/runs",
        "/api/v2/runs/r1/checks",
        "/api/v2/runs/r1/done",
    ] {
        let host = FakeHost::start();
        host.state.lock().unwrap().broken.insert(broken.into());
        let repo = repo(&host.url);
        let config = signed_in(&host.url);
        let out = run(&repo, config.path());
        assert!(out.ok, "{broken}: {}\n{}", out.stdout, out.stderr);
        assert!(
            out.stdout.contains("2 checks: 2 passed"),
            "{broken}: every check still runs: {}",
            out.stdout
        );
        let said: Vec<&str> = out
            .lines()
            .into_iter()
            .filter(|l| l.contains("proof page not sent"))
            .collect();
        assert_eq!(said.len(), 1, "{broken}: said once: {}", out.stdout);
        let calls = host.calls();
        let at = calls
            .iter()
            .position(|c| c.split(' ').nth(1) == Some(broken))
            .unwrap();
        assert!(
            calls[at + 1..].iter().all(|c| c.contains("/mark/")),
            "{broken}: after a failure nothing more is sent: {calls:#?}"
        );
        assert!(out.last_line().starts_with("next:"), "{}", out.stdout);
    }
}

#[test]
fn a_host_that_hangs_costs_the_run_one_short_wait() {
    let host = FakeHost::start();
    host.state.lock().unwrap().hangs = true;
    let repo = repo(&host.url);
    let config = signed_in(&host.url);
    let out = run(&repo, config.path());
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("2 checks: 2 passed"), "{}", out.stdout);
    let api = host.api();
    assert_eq!(api.len(), 1, "given up after the first call: {api:#?}");
    // The fake never answers, so only the run's own limit (5s without a byte) ends the wait.
    let waited = host.state.lock().unwrap().hung_up.clone();
    assert!(
        waited.len() == 1 && waited[0].as_secs() < 8,
        "one call, hung up within the run's limit: {waited:?}"
    );
    assert!(out.stdout.contains("proof page not sent"), "{}", out.stdout);
}
