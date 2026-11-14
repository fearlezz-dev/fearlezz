//! The proof host: on GitHub Actions, a job that may request an OIDC token sends its proof
//! bundle to the host (`ee/README.md`), when the host says the repository installed the app,
//! and the status comment links the page. Nothing leaves CI otherwise, and a host that fails
//! never fails the proof.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;
use common::host::{FakeHost, OIDC};
use serde_json::Value;

const CHECKS: &str = r#"
[check.tests]
run = "echo all tests passed"
stage = "working"
inputs = ["src/**"]
"#;

/// A branch with a change, whose config names `host` as its proof host.
fn repo(host: &FakeHost) -> Repo {
    let repo = Repo::new(
        &format!("{CHECKS}\n[host]\nurl = \"{}\"\n", host.url),
        &[(".gitignore", ".runs/\n"), ("src/a.txt", "a\n")],
    );
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    repo.branch("feat/x");
    repo.write("src/a.txt", "b\n");
    repo.commit("change");
    repo
}

/// `fearlezz run --ci` with the status comment in a file, and `env` besides.
fn ci(repo: &Repo, env: &[(String, String)]) -> (common::Output, String) {
    let comment = repo.path(".runs/comment.md");
    let mut all: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let path = comment.to_str().unwrap().to_string();
    all.push(("GITHUB_REPOSITORY", "owner/repo"));
    let out = repo.fz_env(&["run", "--ci", "--comment", &path], &all);
    let text = std::fs::read_to_string(&comment)
        .unwrap_or_else(|_| panic!("{}\n{}", out.stdout, out.stderr));
    (out, text)
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};
    hex::encode(Sha256::digest(bytes))
}

#[test]
fn the_bundle_goes_to_the_host_and_the_comment_links_its_page() {
    let host = FakeHost::start();
    let repo = repo(&host);
    let (out, text) = ci(&repo, &host.token_env());
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let api = host.api();
    let calls: Vec<String> = api
        .iter()
        .map(|r| format!("{} {}", r.method, r.path))
        .collect();
    assert_eq!(
        calls.first().map(String::as_str),
        Some("GET /api/v1/installed"),
        "{calls:?}"
    );
    assert!(
        calls.contains(&"POST /api/v1/proofs".to_string()),
        "{calls:?}"
    );
    assert_eq!(
        calls.last().map(String::as_str),
        Some("POST /api/v1/proofs/p1/done"),
        "{calls:?}"
    );
    assert!(
        api.iter()
            .all(|r| r.auth.as_deref() == Some(&format!("Bearer {OIDC}"))),
        "every call carries the OIDC token"
    );
    let sent: Value = serde_json::from_slice(
        &api.iter()
            .find(|r| r.path == "/api/v1/proofs")
            .unwrap()
            .body,
    )
    .unwrap();
    assert_eq!(sent["proof"]["schema"], 1, "{sent:#}");
    assert_eq!(sent["proof"]["checks"][0]["id"], "tests", "{sent:#}");
    let files = sent["files"].as_object().unwrap();
    assert!(files.contains_key("index.html"), "{sent:#}");
    assert!(files.contains_key("outputs/tests.log"), "{sent:#}");
    assert!(
        !files.contains_key("proof.json"),
        "the proof goes as JSON: {sent:#}"
    );
    // Every file it named was sent, under its hash, with its bytes.
    let held = host.state.lock().unwrap().blobs.clone();
    for (path, file) in files {
        let sha = file["sha256"].as_str().unwrap();
        let bytes = held.get(sha).unwrap_or_else(|| panic!("{path} not sent"));
        assert_eq!(sha256(bytes), sha, "{path}");
        assert_eq!(file["size"], bytes.len(), "{path}");
    }
    assert!(
        text.contains(&format!(
            "[The proof page]({}/owner/repo/proofs/p1/)",
            host.url
        )),
        "{text}"
    );
}

#[test]
fn the_token_is_asked_for_the_hosts_origin() {
    let host = FakeHost::start();
    let repo = repo(&host);
    ci(&repo, &host.token_env());
    let ask = host
        .requests()
        .into_iter()
        .find(|r| r.path.starts_with("/token"))
        .expect("an OIDC token request");
    let audience = format!(
        "audience={}",
        host.url.replace(':', "%3A").replace('/', "%2F")
    );
    assert!(ask.path.contains(&audience), "{}", ask.path);
}

#[test]
fn files_the_host_holds_are_not_sent_again() {
    let host = FakeHost::start();
    let repo = repo(&host);
    ci(&repo, &host.token_env());
    let first = host.api().iter().filter(|r| r.method == "PUT").count();
    assert!(first > 0);
    // The same proof again: the host holds every file now.
    ci(&repo, &host.token_env());
    let all = host.api().iter().filter(|r| r.method == "PUT").count();
    assert!(
        all < first * 2,
        "only what changed is sent again ({first} then {all})"
    );
}

#[test]
fn nothing_leaves_ci_when_the_app_is_not_installed() {
    let host = FakeHost::start();
    host.state.lock().unwrap().installed = false;
    let repo = repo(&host);
    let (out, text) = ci(&repo, &host.token_env());
    assert!(out.ok, "{}", out.stdout);
    let calls: Vec<String> = host.api().iter().map(|r| r.path.clone()).collect();
    assert_eq!(calls, ["/api/v1/installed"], "only the question");
    assert!(
        out.stdout.contains("https://github.com/apps/fearlezz"),
        "it says how to turn it on: {}",
        out.stdout
    );
    assert!(!text.contains("proof page"), "{text}");
}

#[test]
fn without_the_id_token_permission_nothing_is_asked() {
    let host = FakeHost::start();
    let repo = repo(&host);
    let (out, _) = ci(&repo, &[]);
    assert!(out.ok, "{}", out.stdout);
    assert!(host.requests().is_empty(), "{:?}", host.requests());
}

#[test]
fn a_host_that_fails_never_fails_the_proof() {
    let host = FakeHost::start();
    host.state
        .lock()
        .unwrap()
        .broken
        .insert("/api/v1/proofs".into());
    let repo = repo(&host);
    let (out, text) = ci(&repo, &host.token_env());
    assert!(out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("proof page not sent"),
        "it says so: {}",
        out.stdout
    );
    assert!(!text.contains("The proof page"), "{text}");
}

#[test]
fn every_call_to_the_host_asks_for_a_fresh_token() {
    // An OIDC token lives minutes; sending large films can take longer, so no call reuses one.
    let host = FakeHost::start();
    let repo = repo(&host);
    let (out, _) = ci(&repo, &host.token_env());
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let tokens = host
        .requests()
        .iter()
        .filter(|r| r.path.starts_with("/token"))
        .count();
    assert_eq!(tokens, host.api().len(), "one token for each call");
}
