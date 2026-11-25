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
use serde_json::{Value, json};

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
    all.push(("GITHUB_RUN_ID", "4242"));
    all.push(("GITHUB_RUN_ATTEMPT", "2"));
    all.push(("RUNNER_NAME", "GitHub Actions 7"));
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
    assert_eq!(open["checks"], json!(["lint", "tests"]), "{open:#}");
    assert_eq!(open["source"]["kind"], "ci", "{open:#}");
    assert_eq!(open["source"]["job"], "proof", "{open:#}");
    // Every leg of a matrix has the same job id; the run, its attempt and the runner that
    // took the leg tell them apart.
    assert_eq!(open["source"]["run_id"], 4242, "{open:#}");
    assert_eq!(open["source"]["run_attempt"], 2, "{open:#}");
    assert_eq!(
        open["source"]["runner_name"], "GitHub Actions 7",
        "{open:#}"
    );
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
        .map(|r| serde_json::from_slice::<Value>(&r.body).unwrap_or_default())
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
    // Off GitHub Actions nothing could be sent, so nothing is said.
    assert!(!out.stdout.contains("id-token"), "{}", out.stdout);
}

#[test]
fn on_github_actions_without_the_id_token_permission_it_says_why_there_is_no_page() {
    let host = FakeHost::start();
    let repo = repo(&host);
    let (out, text) = ci(&repo, &[("GITHUB_ACTIONS".into(), "true".into())]);
    assert!(out.ok, "the proof holds: {}", out.stdout);
    let why = "proof page not sent: this job has no id-token: write, so it cannot sign in to the \
               proof host; add it to the workflow's permissions to get the page";
    assert!(out.stdout.contains(why), "{}", out.stdout);
    assert!(!out.stdout.contains("::warning"), "{}", out.stdout);
    assert!(text.contains("this job has no `id-token: write`"), "{text}");
    assert!(text.contains("**Proven**"), "{text}");
    assert!(host.api().is_empty(), "nothing is asked");
}

#[test]
fn a_repository_with_the_host_off_sends_nothing_and_says_nothing() {
    let host = FakeHost::start();
    let repo = repo(&host);
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    let url = format!("url = \"{}\"\n", host.url);
    repo.write(
        "fearlezz.toml",
        &config.replace(&url, &format!("{url}off = true\n")),
    );
    repo.commit("no host");
    let (out, text) = ci(&repo, &[("GITHUB_ACTIONS".into(), "true".into())]);
    assert!(out.ok, "{}", out.stdout);
    assert!(!out.stdout.contains("proof page"), "{}", out.stdout);
    assert!(!text.contains("id-token"), "{text}");
    // Even a job that may sign in sends nothing.
    let (out, _) = ci(&repo, &host.token_env());
    assert!(out.ok, "{}", out.stdout);
    assert!(host.api().is_empty(), "nothing is asked: {:?}", host.api());
}

#[test]
fn a_pull_request_from_a_fork_is_not_told_to_add_the_permission() {
    let host = FakeHost::start();
    let repo = repo(&host);
    let event = repo.path(".runs/event.json");
    std::fs::write(
        &event,
        r#"{"pull_request":{"number":7,"title":"fix: x","head":{"sha":"abc","ref":"x","repo":{"fork":true}}}}"#,
    )
    .unwrap();
    let (out, text) = ci(
        &repo,
        &[
            ("GITHUB_ACTIONS".into(), "true".into()),
            ("GITHUB_EVENT_NAME".into(), "pull_request".into()),
            ("GITHUB_EVENT_PATH".into(), event.to_str().unwrap().into()),
        ],
    );
    assert!(!out.stdout.contains("id-token"), "{}", out.stdout);
    assert!(!text.contains("id-token"), "{text}");
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

/// A branch of `repo` whose config makes `lint` run another command than on `main`: it
/// weakens the proof the base branch requires.
fn weakening(host: &FakeHost) -> Repo {
    let repo = repo(host);
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    repo.write(
        "fearlezz.toml",
        &config.replace("echo all clean", "echo all clean anyway"),
    );
    repo.commit("weaken lint");
    repo
}

/// An admin's accept of `weakened` at `head`, as the host answers it.
fn accept(head: &str, weakened: &Value) -> Value {
    json!({"head": head, "weakened": weakened, "by": "admin", "at": 9.0})
}

/// What `weakening` weakens, with the check as it was and is now.
fn lint_weakened(host: &FakeHost) -> Value {
    let check = |run: &str| {
        format!(
            "[check.lint]\ninputs = [\"src/**\"]\nrun = \"curl -s {}/mark/lint > /dev/null && \
             {run}\"\nstage = \"working\"\n",
            host.url
        )
    };
    json!([{
        "check": "lint",
        "hows": ["it runs another command"],
        "was": check("echo all clean"),
        "now": check("echo all clean anyway"),
    }])
}

#[test]
fn a_weakening_fails_the_proof_while_no_admin_accepted_it() {
    // The host took the run, but the job is the gate: the weakening fails it as without a host,
    // and the run asked the host first, with what it weakens.
    let host = FakeHost::start();
    let repo = weakening(&host);
    let (out, text) = ci(&repo, &host.token_env());
    assert!(!out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("weakens lint: it runs another command"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout
            .contains("an admin accepts it on the proof page, which runs this job again"),
        "{}",
        out.stdout
    );
    let asked = host.sent("POST", "/api/v2/runs/r1/weakenings");
    assert_eq!(asked["weakened"], lint_weakened(&host), "{asked:#}");
    let calls = host.calls();
    let ask = calls
        .iter()
        .position(|c| c.ends_with("/weakenings"))
        .unwrap();
    let done = calls.iter().position(|c| c.ends_with("/done")).unwrap();
    assert!(ask < done, "it asks before it judges: {calls:#?}");
    let done = host.sent("POST", "/api/v2/runs/r1/done");
    assert!(done["proof"].get("accepted").is_none(), "{done:#}");
    assert_eq!(done["proof"]["weakened"][0]["check"], "lint", "{done:#}");
    assert!(text.contains("**Not proven**"), "{text}");
    assert!(text.contains("accept what it weakens on"), "{text}");
}

#[test]
fn an_accepted_weakening_of_this_head_passes() {
    let host = FakeHost::start();
    let repo = weakening(&host);
    let head = repo.git(&["rev-parse", "HEAD"]).trim().to_owned();
    host.state.lock().unwrap().accepted = Some(accept(&head, &lint_weakened(&host)));
    let (out, text) = ci(&repo, &host.token_env());
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("weakens lint: it runs another command"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("accepted by admin on the proof page"),
        "{}",
        out.stdout
    );
    let done = host.sent("POST", "/api/v2/runs/r1/done");
    assert_eq!(done["proof"]["accepted"]["by"], "admin", "{done:#}");
    assert!(text.contains("**Proven**"), "{text}");
    assert!(text.contains("accepted by admin"), "{text}");
}

#[test]
fn an_accept_of_other_weakenings_or_another_head_fails() {
    for (what, accepted) in [
        (
            "other weakenings",
            json!([{"check": "tests", "hows": ["it is gone"]}]),
        ),
        ("fewer weakenings", json!([])),
    ] {
        let host = FakeHost::start();
        let repo = weakening(&host);
        let head = repo.git(&["rev-parse", "HEAD"]).trim().to_owned();
        host.state.lock().unwrap().accepted = Some(accept(&head, &accepted));
        let (out, text) = ci(&repo, &host.token_env());
        assert!(!out.ok, "{what}: {}\n{}", out.stdout, out.stderr);
        assert!(text.contains("**Not proven**"), "{what}: {text}");
    }
    let host = FakeHost::start();
    let repo = weakening(&host);
    host.state.lock().unwrap().accepted = Some(accept(&"0".repeat(40), &lint_weakened(&host)));
    let (out, _) = ci(&repo, &host.token_env());
    assert!(!out.ok, "another head: {}\n{}", out.stdout, out.stderr);
}

#[test]
fn a_weakening_the_host_could_not_be_asked_about_fails_as_without_a_host() {
    // The ask fails, so the host never gets done and no accept can reach this run: the run
    // says so, and gives the fix it gives without a host.
    let host = FakeHost::start();
    let repo = weakening(&host);
    host.state
        .lock()
        .unwrap()
        .broken
        .insert("/api/v2/runs/r1/weakenings".into());
    let (out, text) = ci(&repo, &host.token_env());
    assert!(!out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        !out.stdout.contains("accepts it on the proof page"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains(
            "the proof host could not be asked, so it fails as without a host: keep the proof \
             the base branch requires; when it must change, a person with admin rights"
        ),
        "{}",
        out.stdout
    );
    assert!(!text.contains("accept what it weakens on"), "{text}");
    assert!(text.contains("The proof host could not be asked"), "{text}");
}

#[test]
fn a_weakening_fails_the_proof_when_the_host_cannot_say_it_was_accepted() {
    // Fail safe: a host that cannot be reached or refuses means not accepted.
    for broken in [
        "/api/v2/runs",
        "/api/v2/runs/r1/weakenings",
        "/api/v2/runs/r1/done",
        "not installed",
    ] {
        let host = FakeHost::start();
        let repo = weakening(&host);
        let head = repo.git(&["rev-parse", "HEAD"]).trim().to_owned();
        let mut state = host.state.lock().unwrap();
        state.accepted = Some(accept(&head, &lint_weakened(&host)));
        if broken == "not installed" {
            state.installed = false;
        } else {
            state.broken.insert(broken.into());
        }
        drop(state);
        let (out, text) = ci(&repo, &host.token_env());
        if broken == "/api/v2/runs/r1/done" {
            // The accept was read before done, so the proof holds though done failed.
            assert!(out.ok, "{broken}: {}\n{}", out.stdout, out.stderr);
            continue;
        }
        assert!(!out.ok, "{broken}: {}\n{}", out.stdout, out.stderr);
        assert!(text.contains("**Not proven**"), "{broken}: {text}");
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

/// A check that takes two stills, the first in the colour `src/color.txt` names.
const STILLS: &str = r#"set -e
still() {
  ffmpeg -v error -nostdin -f lavfi -i "color=c=$2:s=64x48" -frames:v 1 "$FEARLEZZ_OUT/$1.png"
  echo "{\"kind\":\"still\",\"file\":\"$1.png\"}" >> "$FEARLEZZ_OUT/proof.jsonl"
}
still home "$(cat src/color.txt)"
still about white
"#;

#[test]
fn a_finished_check_sends_its_look_with_the_pictures() {
    // A look arrives with its check, so a person can start looking before the run ends.
    let host = FakeHost::start();
    let config = format!(
        "{}\n[check.screens]\nrun = \"sh record.sh\"\nstage = \"working\"\ninputs = [\"src/**\", \"record.sh\"]\n\n[host]\nurl = \"{}\"\n",
        checks(&host),
        host.url
    );
    let repo = Repo::new(
        &config,
        &[
            (".gitignore", ".runs/\n"),
            ("record.sh", STILLS),
            ("src/a.txt", "a\n"),
            ("src/color.txt", "red"),
        ],
    );
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    repo.branch("feat/x");
    repo.write("src/color.txt", "blue");
    repo.commit("blue");
    let (out, _) = ci(&repo, &host.token_env());
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);

    let finished = host
        .api()
        .into_iter()
        .map(|r| serde_json::from_slice::<Value>(&r.body).unwrap_or_default())
        .find(|b| b["finished"]["check"]["id"] == "screens")
        .expect("a finished screens check");
    let look = &finished["finished"]["look"];
    assert_eq!(look["check"], "screens", "{finished:#}");
    let home = look["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == "home.png")
        .unwrap_or_else(|| panic!("{finished:#}"));
    assert_eq!(home["change"], "changed", "{finished:#}");
    assert!(
        look.get("dir").is_none(),
        "no folder of this machine: {finished:#}"
    );
    // Its pictures are named as in the bundle, and go with the check.
    let held = host.state.lock().unwrap().blobs.clone();
    for side in ["before", "after"] {
        let path = home[side].as_str().unwrap();
        assert!(path.starts_with("looks/"), "{path}");
        let file = &finished["finished"]["files"][path];
        let sha = file["sha256"]
            .as_str()
            .unwrap_or_else(|| panic!("{finished:#}"));
        assert!(held.contains_key(sha), "{side} was sent");
    }
    // And done names the same pictures at the same paths.
    let done = host.sent("POST", "/api/v2/runs/r1/done");
    let path = home["after"].as_str().unwrap();
    assert!(done["files"].get(path).is_some(), "{done:#}");
    assert_eq!(
        done["proof"]["looks"][0]["items"], look["items"],
        "{done:#}"
    );
}

#[test]
fn a_finished_check_says_whether_each_picture_looks_like_the_approved_ones() {
    // Before the check says it finished, CI fetches each picture a person said something
    // about and compares its own with it, so a looks right follows a picture that looks the
    // same though its bytes differ.
    let host = FakeHost::start();
    let dir = tempfile::tempdir().unwrap();
    let still = |color: &str, level: &str| {
        let path = dir.path().join(format!("{color}{level}.png"));
        let made = std::process::Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-f", "lavfi", "-i"])
            .arg(format!("color=c={color}:s=64x48"))
            .args(["-frames:v", "1", "-compression_level", level])
            .arg(&path)
            .status()
            .unwrap();
        assert!(made.success());
        std::fs::read(path).unwrap()
    };
    // The same blue, written another way, and the red of before.
    let blue = still("blue", "0");
    let red = still("red", "0");
    {
        let mut s = host.state.lock().unwrap();
        s.approved = Some(
            json!({"screens": {"home.png": {sha256(&blue): null, sha256(&red): null, "gone:aa": null}}}),
        );
        s.approved_files.insert(sha256(&blue), blue.clone());
        s.approved_files.insert(sha256(&red), red.clone());
    }
    let config = format!(
        "{}\n[check.screens]\nrun = \"sh record.sh\"\nstage = \"working\"\ninputs = [\"src/**\", \"record.sh\"]\n\n[host]\nurl = \"{}\"\n",
        checks(&host),
        host.url
    );
    let repo = Repo::new(
        &config,
        &[
            (".gitignore", ".runs/\n"),
            ("record.sh", STILLS),
            ("src/a.txt", "a\n"),
            ("src/color.txt", "red"),
        ],
    );
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    repo.branch("feat/x");
    repo.write("src/color.txt", "blue");
    repo.commit("blue");
    let (out, _) = ci(&repo, &host.token_env());
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);

    let finished = host
        .api()
        .into_iter()
        .map(|r| serde_json::from_slice::<Value>(&r.body).unwrap_or_default())
        .find(|b| b["finished"]["check"]["id"] == "screens")
        .expect("a finished screens check");
    let after = finished["finished"]["look"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == "home.png")
        .and_then(|i| i["after"].as_str())
        .unwrap_or_else(|| panic!("{finished:#}"));
    assert_ne!(
        finished["finished"]["files"][after]["sha256"],
        sha256(&blue),
        "the bytes differ"
    );
    // A gone picture has no file to compare with.
    assert_eq!(
        finished["finished"]["since"],
        json!({"home.png": {sha256(&blue): true, sha256(&red): false}}),
        "{finished:#}"
    );
}

#[test]
fn an_approved_film_that_loaded_faster_is_alike_timed_from_its_marks() {
    // The film a person approved loaded for a second; this run's loads for two. Each film
    // marks where its load ended, so they are compared from there and look the same.
    let host = FakeHost::start();
    let dir = tempfile::tempdir().unwrap();
    let approved = dir.path().join("approved.webm");
    let made = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-f", "lavfi", "-i"])
        .arg("color=c=black:s=96x64:r=10:d=1[a];testsrc=s=96x64:r=10:d=1,drawbox=w=20:h=20:color=blue:t=fill[b];[a][b]concat=n=2")
        .args(["-c:v", "libvpx", "-b:v", "200k"])
        .arg(&approved)
        .status()
        .unwrap();
    assert!(made.success());
    let approved = std::fs::read(approved).unwrap();
    let key = sha256(&approved);
    {
        let mut s = host.state.lock().unwrap();
        s.approved = Some(
            json!({"screens": {"home.webm": {&key: {"marks": [{"name": "loaded", "at": 1.0}]}}}}),
        );
        s.approved_files.insert(key.clone(), approved);
    }
    let repo = filmed(&host, 2, 1, Some("feat/x"));
    repo.write(
        "record.sh",
        &format!("{FILM}echo \"{{\\\"kind\\\":\\\"mark\\\",\\\"film\\\":\\\"home.webm\\\",\\\"name\\\":\\\"loaded\\\",\\\"at\\\":$lead}}\" >> \"$FEARLEZZ_OUT/proof.jsonl\"\n"),
    );
    repo.commit("mark where the load ends");
    let (out, _) = ci(&repo, &host.token_env());
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let finished = host
        .api()
        .into_iter()
        .map(|r| serde_json::from_slice::<Value>(&r.body).unwrap_or_default())
        .find(|b| b["finished"]["check"]["id"] == "screens")
        .expect("a finished screens check");
    assert_eq!(
        finished["finished"]["since"],
        json!({"home.webm": {&key: true}}),
        "{finished:#}"
    );
}

/// `fearlezz run --ci` with `args` on a push of `feat/x`, as `ci` runs it.
fn ci_with(repo: &Repo, args: &[&str], env: &[(String, String)]) -> common::Output {
    let mut all: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    all.push(("GITHUB_REPOSITORY", "owner/repo"));
    all.push(("GITHUB_REF", "refs/heads/feat/x"));
    all.push(("GITHUB_JOB", "proof"));
    all.push(("GITHUB_RUN_ID", "4242"));
    all.push(("GITHUB_RUN_ATTEMPT", "1"));
    all.push(("RUNNER_NAME", "GitHub Actions 7"));
    let mut full = vec!["run", "--ci"];
    full.extend_from_slice(args);
    repo.fz_env(&full, &all)
}

#[test]
fn a_leg_names_itself_and_its_plan_and_leaves_what_it_weakens_to_the_gate() {
    let host = FakeHost::start();
    let repo = weakening(&host);
    let dir = repo.path(".runs/legs/proof");
    let entry = r#"{"leg":"proof","checks":"lint tests","planned":"proof"}"#;
    let out = ci_with(
        &repo,
        &["--leg", entry, "--proof", dir.to_str().unwrap()],
        &host.token_env(),
    );
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("the gate judges what the change weakens"),
        "{}",
        out.stdout
    );
    let open = host.sent("POST", "/api/v2/runs");
    assert_eq!(open["source"]["leg"], "proof", "{open:#}");
    assert_eq!(open["source"]["planned"], json!(["proof"]), "{open:#}");
    // The host shows what it weakens.
    let done = host.sent("POST", "/api/v2/runs/r1/done");
    assert_eq!(done["proof"]["weakened"][0]["check"], "lint", "{done:#}");
    assert_eq!(done["proof"]["leg"], "proof", "{done:#}");
    assert_eq!(std::fs::read_to_string(dir.join("host-run")).unwrap(), "r1");

    // The gate asks the host through the leg's run: not accepted fails, accepted passes.
    let legs = repo.path(".runs/legs");
    let gate = ["--gate", legs.to_str().unwrap()];
    let out = ci_with(&repo, &gate, &host.token_env());
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("an admin accepts it on the proof page, which runs this job again"),
        "{}",
        out.stdout
    );
    let head = repo.git(&["rev-parse", "HEAD"]).trim().to_owned();
    host.state.lock().unwrap().accepted = Some(accept(&head, &lint_weakened(&host)));
    let out = ci_with(&repo, &gate, &host.token_env());
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("accepted by admin on the proof page"),
        "{}",
        out.stdout
    );
}

/// A check that takes one film, `home.webm`, 96 by 64 at 10 frames a second: the test pattern
/// for as long as `src/seconds.txt` says, with a box in the colour `src/color.txt` names, after
/// as many seconds of black as `src/lead.txt` says.
const FILM: &str = r#"set -e
pattern="testsrc=s=96x64:r=10:d=$(cat src/seconds.txt),drawbox=w=20:h=20:color=$(cat src/color.txt):t=fill"
lead=$(cat src/lead.txt)
if [ "$lead" = 0 ]; then graph="$pattern"; else
  graph="color=c=black:s=96x64:r=10:d=$lead[a];$pattern[b];[a][b]concat=n=2"
fi
ffmpeg -v error -nostdin -f lavfi -i "$graph" -c:v libvpx -b:v 200k "$FEARLEZZ_OUT/home.webm"
echo '{"kind":"film","file":"home.webm"}' >> "$FEARLEZZ_OUT/proof.jsonl"
"#;

/// A repository whose check `screens` films `seconds` of content after `lead` seconds of
/// black, with a commit that changes its box: on `branch` when one is given, as a pull request
/// has it, otherwise on `main` itself, as a push to the default branch lands it.
fn filmed(host: &FakeHost, lead: u32, seconds: u32, branch: Option<&str>) -> Repo {
    let config = format!(
        "[check.screens]\nrun = \"sh record.sh\"\nstage = \"working\"\ninputs = [\"src/**\", \"record.sh\"]\n\n[host]\nurl = \"{}\"\n",
        host.url
    );
    let repo = Repo::new(
        &config,
        &[
            (".gitignore", ".runs/\n"),
            ("record.sh", FILM),
            ("src/seconds.txt", &seconds.to_string()),
            ("src/lead.txt", &lead.to_string()),
            ("src/color.txt", "red"),
        ],
    );
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    if let Some(branch) = branch {
        repo.branch(branch);
    }
    repo.write("src/color.txt", "blue");
    repo.commit("a blue box");
    repo
}

/// `fearlezz run --ci` on `repo` as GitHub starts it for `event` on `git_ref`, with an event
/// file naming `main` as the default branch (and, for a pull request, the pull request).
fn ci_on(repo: &Repo, host: &FakeHost, event: &str, git_ref: &str) -> common::Output {
    let file = repo.path(".runs/event.json");
    let mut body = json!({"repository": {"default_branch": "main"}});
    if event == "pull_request" {
        body["pull_request"] = json!({
            "number": 7,
            "title": "feat: a blue box",
            "head": {"ref": "feat/film", "sha": repo.git(&["rev-parse", "HEAD"]).trim()},
        });
    }
    std::fs::write(&file, body.to_string()).unwrap();
    let mut env = host.token_env();
    for (k, v) in [
        ("GITHUB_EVENT_NAME", event),
        ("GITHUB_EVENT_PATH", file.to_str().unwrap()),
        ("GITHUB_REF", git_ref),
    ] {
        env.push((k.into(), v.into()));
    }
    let mut all: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    all.push(("GITHUB_REPOSITORY", "owner/repo"));
    all.push(("GITHUB_JOB", "proof"));
    all.push(("GITHUB_RUN_ID", "4242"));
    all.push(("GITHUB_RUN_ATTEMPT", "1"));
    all.push(("RUNNER_NAME", "GitHub Actions 7"));
    repo.fz_env(&["run", "--ci"], &all)
}

/// The film `home.webm` of the `screens` look as the host got it when the check ended, with
/// the files that went with it.
fn film_sent(host: &FakeHost) -> (Value, Value) {
    let finished = host
        .api()
        .into_iter()
        .map(|r| serde_json::from_slice::<Value>(&r.body).unwrap_or_default())
        .find(|b| b["finished"]["check"]["id"] == "screens")
        .expect("a finished screens check");
    let film = finished["finished"]["look"]["items"]
        .as_array()
        .and_then(|items| items.iter().find(|i| i["name"] == "home.webm"))
        .unwrap_or_else(|| panic!("no film in {finished:#}"))
        .clone();
    (film, finished["finished"]["files"].clone())
}

/// The bytes of the animated WebP copy the host got for `film`, sent with `files`.
fn copy_of(host: &FakeHost, film: &Value, files: &Value) -> (String, Vec<u8>) {
    let copy = film["webp"]
        .as_str()
        .unwrap_or_else(|| panic!("no WebP copy: {film:#}"))
        .to_string();
    let sha = files[&copy]["sha256"]
        .as_str()
        .unwrap_or_else(|| panic!("{files:#}"));
    let held = host.state.lock().unwrap().blobs.clone();
    let bytes = held.get(sha).expect("the copy was sent").clone();
    (copy, bytes)
}

/// How long an animated WebP plays once, in milliseconds: the sum of its frames' durations,
/// each a 24 bit number 12 bytes into its `ANMF` chunk.
fn plays_ms(webp: &[u8]) -> u32 {
    let mut at = 12;
    let mut total = 0;
    while let Some(head) = webp.get(at..at + 8) {
        let size = u32::from_le_bytes(head[4..8].try_into().unwrap()) as usize;
        if &head[..4] == b"ANMF" {
            let [low, mid, high] = webp[at + 8 + 12..at + 8 + 15] else {
                panic!("a short ANMF chunk");
            };
            total += u32::from(low) | (u32::from(mid) << 8) | (u32::from(high) << 16);
        }
        at += 8 + size + size % 2;
    }
    total
}

#[test]
fn a_push_to_the_default_branch_sends_an_animated_webp_copy_of_each_film() {
    // GitHub plays no film in a README, only an animated image, so the run whose pictures the
    // host serves at a stable address sends its films as an animated WebP too.
    let host = FakeHost::start();
    let repo = filmed(&host, 0, 2, None);
    let out = ci_on(&repo, &host, "push", "refs/heads/main");
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);

    let (film, files) = film_sent(&host);
    let (copy, bytes) = copy_of(&host, &film, &files);
    assert!(
        copy.starts_with("looks/") && copy.to_ascii_lowercase().ends_with(".webp"),
        "{copy}"
    );
    assert_ne!(
        Some(copy.as_str()),
        film["after"].as_str(),
        "beside the film"
    );
    // An animated WebP: a RIFF WEBP file with an animation chunk.
    assert_eq!(bytes.get(..4), Some(&b"RIFF"[..]));
    assert_eq!(bytes.get(8..12), Some(&b"WEBP"[..]));
    assert!(bytes.windows(4).any(|w| w == b"ANIM"), "it moves");
    // Two seconds of film, then its last frame held 1.5 s.
    let ms = plays_ms(&bytes);
    assert!((3300..=3700).contains(&ms), "plays {ms} ms");
    // And done names it too, in the proof and among the files.
    let done = host.sent("POST", "/api/v2/runs/r1/done");
    assert!(done["files"].get(&copy).is_some(), "{done:#}");
    let items = done["proof"]["looks"][0]["items"].to_string();
    assert!(items.contains(&copy), "{done:#}");
}

#[test]
fn a_webp_copy_starts_at_the_first_frame_with_content() {
    // A recorder that starts as the page opens leaves blank frames first, which would flash
    // on every loop of the copy; the film itself keeps them.
    let host = FakeHost::start();
    let repo = filmed(&host, 1, 2, None);
    let out = ci_on(&repo, &host, "push", "refs/heads/main");
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let (film, files) = film_sent(&host);
    let (_, bytes) = copy_of(&host, &film, &files);
    let ms = plays_ms(&bytes);
    assert!(
        (3300..=3700).contains(&ms),
        "plays {ms} ms, not the black second"
    );
}

#[test]
fn a_pull_request_sends_no_webp_copy() {
    // The host serves only pushes to the default branch at a stable address, so nothing else
    // pays for a copy.
    let host = FakeHost::start();
    let repo = filmed(&host, 0, 2, Some("feat/film"));
    let out = ci_on(&repo, &host, "pull_request", "refs/pull/7/merge");
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let (film, files) = film_sent(&host);
    assert!(film.get("webp").is_none(), "{film:#}");
    assert!(
        !files
            .as_object()
            .unwrap()
            .keys()
            .any(|k| k.to_ascii_lowercase().ends_with(".webp")),
        "{files:#}"
    );
}

#[test]
fn a_film_longer_than_thirty_seconds_gets_a_webp_copy_too() {
    // A film's length says little about what its copy weighs, so no length goes without one.
    let host = FakeHost::start();
    let repo = filmed(&host, 0, 31, None);
    let out = ci_on(&repo, &host, "push", "refs/heads/main");
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let (film, files) = film_sent(&host);
    let (_, bytes) = copy_of(&host, &film, &files);
    // Thirty one seconds of film, then its last frame held 1.5 s.
    let ms = plays_ms(&bytes);
    assert!((32300..=32700).contains(&ms), "plays {ms} ms");
    assert!(!out.stdout.contains("gets no WebP copy"), "{}", out.stdout);
}

#[test]
fn a_webp_copy_is_made_with_the_ffmpeg_of_the_project_shell() {
    // A devenv project keeps ffmpeg in its shell, not on the PATH Fearlezz runs with, so the
    // copy is made with the ffmpeg Fearlezz finds there, as every picture it compares is.
    let tools = tempfile::tempdir().unwrap();
    let calls = tools.path().join("calls");
    let path = std::env::var_os("PATH").unwrap();
    for name in ["ffmpeg", "ffprobe"] {
        use std::os::unix::fs::PermissionsExt as _;
        let real = std::env::split_paths(&path)
            .map(|dir| dir.join(name))
            .find(|p| p.is_file())
            .unwrap_or_else(|| panic!("{name} is needed for this test"));
        let wrapper = tools.path().join(name);
        std::fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\necho \"{name} $*\" >> '{}'\nexec '{}' \"$@\"\n",
                calls.display(),
                real.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let host = FakeHost::start();
    let repo = filmed(&host, 0, 2, None);
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    repo.write(
        "fearlezz.toml",
        &format!(
            "{config}\n[env]\nshell = 'env \"PATH={}:$PATH\"'\n",
            tools.path().display()
        ),
    );
    repo.commit("ffmpeg in the shell");
    let out = ci_on(&repo, &host, "push", "refs/heads/main");
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let (film, files) = film_sent(&host);
    copy_of(&host, &film, &files);
    let calls = std::fs::read_to_string(&calls).unwrap_or_default();
    assert!(
        calls
            .lines()
            .any(|call| call.starts_with("ffmpeg ") && call.contains("libwebp_anim")),
        "{calls}"
    );
}
