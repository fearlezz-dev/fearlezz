//! The proof host: on GitHub Actions, a job that may request an OIDC token streams its run to
//! the host (protocol version 2 of `ee/README.md`) when the host says the repository installed
//! the app: it opens the run before any check runs, says when each check starts and ends with
//! the files it made, and closes the run with `proof.json`. The log and the job summary link
//! the host's page. Nothing leaves CI otherwise, and a host that fails never fails the proof.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;
use common::host::{FakeHost, OIDC};

/// Two checks, each calling the fake host when its command runs, so the calls show when.
fn checks(host: &FakeHost) -> String {
    format!(
        r#"
[check.tests]
run = "curl -s {url}/mark/tests > /dev/null && echo all tests passed"
stage = "working"
inputs = ["src/**"]

[check.lint]
run = "curl -s {url}/mark/lint > /dev/null && echo all clean"
stage = "working"
inputs = ["src/**"]
"#,
        url = host.url
    )
}

/// A branch with a change, whose config names `host` as its proof host.
fn repo(host: &FakeHost) -> Repo {
    let repo = Repo::new(
        &format!("{}\n[host]\nurl = \"{}\"\n", checks(host), host.url),
        &[(".gitignore", ".runs/\n"), ("src/a.txt", "a\n")],
    );
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    repo.branch("feat/x");
    repo.write("src/a.txt", "b\n");
    repo.commit("change");
    repo
}

/// `fearlezz run --ci` on a push of `feat/x` with the job summary in a file, and `env`
/// besides.
fn ci(repo: &Repo, env: &[(String, String)]) -> (common::Output, String) {
    let summary = repo.path(".runs/summary.md");
    // The summary is appended to, as on GitHub; each run here reads its own.
    if summary.exists() {
        std::fs::remove_file(&summary).unwrap();
    }
    let mut all: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let path = summary.to_str().unwrap().to_string();
    all.push(("GITHUB_REPOSITORY", "owner/repo"));
    all.push(("GITHUB_REF", "refs/heads/feat/x"));
    all.push(("GITHUB_JOB", "proof"));
    all.push(("GITHUB_STEP_SUMMARY", &path));
    let out = repo.fz_env(&["run", "--ci"], &all);
    let text = std::fs::read_to_string(&summary)
        .unwrap_or_else(|_| panic!("{}\n{}", out.stdout, out.stderr));
    (out, text)
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};
    hex::encode(Sha256::digest(bytes))
}

#[test]
fn the_run_streams_to_the_host_while_its_checks_run() {
    let host = FakeHost::start();
    let repo = repo(&host);
    let (out, _) = ci(&repo, &host.token_env());
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let calls: Vec<String> = host
        .calls()
        .into_iter()
        // The blobs a check sends come between its end and the next start; another test
        // reads them.
        .filter(|c| !c.starts_with("PUT "))
        .collect();
    assert_eq!(
        calls,
        [
            "GET /api/v2/installed",
            "POST /api/v2/runs",
            "POST /api/v2/runs/r1/checks started lint",
            "GET /mark/lint",
            "POST /api/v2/runs/r1/checks finished lint",
            "POST /api/v2/runs/r1/checks started tests",
            "GET /mark/tests",
            "POST /api/v2/runs/r1/checks finished tests",
            "POST /api/v2/runs/r1/done",
        ],
        "it opens the run before any check runs, each check starts and ends around its \
         command, and done comes last"
    );
    assert!(
        host.api()
            .iter()
            .all(|r| r.auth.as_deref() == Some(&format!("Bearer {OIDC}"))),
        "every call carries the OIDC token"
    );

    let open = host.sent("POST", "/api/v2/runs");
    assert_eq!(open["repository"], "owner/repo", "{open:#}");
    assert_eq!(open["branch"], "feat/x", "{open:#}");
    assert_eq!(open["base"], "main", "{open:#}");
    assert_eq!(
        open["checks"],
        serde_json::json!(["lint", "tests"]),
        "{open:#}"
    );
    assert_eq!(open["source"]["kind"], "ci", "{open:#}");
    assert_eq!(open["source"]["job"], "proof", "{open:#}");
    assert_eq!(
        open["head"].as_str().unwrap().len(),
        40,
        "the full commit: {open:#}"
    );
    assert!(open.get("pull").is_none(), "a push names no pull request");

    let done = host.sent("POST", "/api/v2/runs/r1/done");
    assert_eq!(done["proof"]["schema"], 1, "{done:#}");
    let ids: Vec<&str> = done["proof"]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["lint", "tests"], "{done:#}");
    let files = done["files"].as_object().unwrap();
    assert!(files.contains_key("outputs/tests.log"), "{done:#}");
    assert!(!files.contains_key("proof.json"), "the proof goes as JSON");
    assert!(
        !files.contains_key("index.html"),
        "only the host has a page"
    );
}

#[test]
fn a_finished_check_sends_its_entry_and_output_and_only_missing_blobs() {
    let host = FakeHost::start();
    let repo = repo(&host);
    let (out, _) = ci(&repo, &host.token_env());
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let finished = host
        .api()
        .into_iter()
        .map(|r| serde_json::from_slice::<serde_json::Value>(&r.body).unwrap_or_default())
        .find(|b| b["finished"]["check"]["id"] == "tests")
        .expect("a finished tests check");
    let check = &finished["finished"]["check"];
    assert_eq!(check["outcome"], "passed", "{finished:#}");
    assert_eq!(check["output"], "outputs/tests.log", "{finished:#}");
    let log = &finished["finished"]["files"]["outputs/tests.log"];
    let sha = log["sha256"].as_str().unwrap();
    let held = host.state.lock().unwrap().blobs.clone();
    let bytes = held.get(sha).expect("the output was sent");
    assert_eq!(sha256(bytes), sha);
    assert_eq!(log["size"], bytes.len());
    assert!(String::from_utf8_lossy(bytes).contains("all tests passed"));
    // The blob goes right after its check ends, before anything else is sent.
    let calls = host.calls();
    let end = calls
        .iter()
        .position(|c| c.ends_with("finished tests"))
        .unwrap();
    assert_eq!(
        calls[end + 1],
        format!("PUT /api/v2/blobs/{sha}"),
        "{calls:#?}"
    );

    // The same run again: the host holds every output now, so nothing is sent twice.
    let before = host.api().iter().filter(|r| r.method == "PUT").count();
    ci(&repo, &host.token_env());
    let after = host.api().iter().filter(|r| r.method == "PUT").count();
    assert_eq!(before, after, "no blob the host holds is sent again");
}

#[test]
fn done_sends_the_files_the_host_still_lacks_and_closes_again() {
    // Done names every file of the bundle; one the host lacks then (a picture's copy, say, or
    // an output it lost) is refused with its name, sent, and done is asked again.
    let host = FakeHost::start();
    host.state.lock().unwrap().forget_before_done = true;
    let repo = repo(&host);
    let (out, text) = ci(&repo, &host.token_env());
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let calls = host.calls();
    let dones: Vec<usize> = calls
        .iter()
        .enumerate()
        .filter(|(_, c)| c.ends_with("/done"))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(dones.len(), 2, "{calls:#?}");
    let between = &calls[dones[0] + 1..dones[1]];
    assert!(
        !between.is_empty() && between.iter().all(|c| c.starts_with("PUT /api/v2/blobs/")),
        "what done lacked goes between the two: {calls:#?}"
    );
    assert_eq!(dones[1], calls.len() - 1, "done comes last");
    assert!(text.contains("[The proof page]"), "{text}");
    assert!(!out.stdout.contains("not sent"), "{}", out.stdout);
}

#[test]
fn the_log_and_the_summary_link_the_hosts_page() {
    let host = FakeHost::start();
    let repo = repo(&host);
    let (out, text) = ci(&repo, &host.token_env());
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let page = format!("{}/owner/repo/work/feat/x", host.url);
    assert!(
        text.contains(&format!("[The proof page]({page})")),
        "{text}"
    );
    let line = format!("proof page: {page}");
    assert!(out.stdout.contains(&line), "{}", out.stdout);
    // The link shows before the checks run, so a reader can watch them.
    let link = out.stdout.find(&line).unwrap();
    let first = out.stdout.find("passed lint").unwrap();
    assert!(link < first, "{}", out.stdout);
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
fn nothing_leaves_ci_when_the_app_is_not_installed() {
    let host = FakeHost::start();
    host.state.lock().unwrap().installed = false;
    let repo = repo(&host);
    let (out, text) = ci(&repo, &host.token_env());
    assert!(out.ok, "{}", out.stdout);
    let calls: Vec<String> = host.api().iter().map(|r| r.path.clone()).collect();
    assert_eq!(calls, ["/api/v2/installed"], "only the question");
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
    let asked: Vec<String> = host
        .calls()
        .into_iter()
        .filter(|c| !c.contains("/mark/"))
        .collect();
    assert!(asked.is_empty(), "{asked:?}");
}

#[test]
fn a_host_that_fails_never_fails_the_proof() {
    for broken in [
        "/api/v2/runs",
        "/api/v2/runs/r1/checks",
        "/api/v2/runs/r1/done",
    ] {
        let host = FakeHost::start();
        host.state.lock().unwrap().broken.insert(broken.into());
        let repo = repo(&host);
        let (out, text) = ci(&repo, &host.token_env());
        assert!(out.ok, "{broken}: {}\n{}", out.stdout, out.stderr);
        assert!(
            out.stdout.contains("passed tests") && out.stdout.contains("passed lint"),
            "{broken}: every check still runs: {}",
            out.stdout
        );
        assert!(
            out.stdout.contains("proof page not sent"),
            "{broken}: it says so: {}",
            out.stdout
        );
        let calls = host.calls();
        let at = calls
            .iter()
            .position(|c| c.split(' ').nth(1) == Some(broken))
            .unwrap();
        assert!(
            calls[at + 1..].iter().all(|c| c.contains("/mark/")),
            "{broken}: after a failure nothing more is sent: {calls:#?}"
        );
        if broken == "/api/v2/runs" {
            assert!(!text.contains("The proof page"), "{text}");
        }
    }
}

#[test]
fn every_call_to_the_host_asks_for_a_fresh_token() {
    // An OIDC token lives minutes; a run with large films can take longer, so no call reuses one.
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
