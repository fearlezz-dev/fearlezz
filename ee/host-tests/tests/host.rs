//! The proof host, driven as CI and people use it: protocol version 2 with its OIDC check over
//! HTTP, the work pages and who may see them, the files origin, and the live page in headless
//! Chrome. One worker under `wrangler dev --local` and one fake GitHub serve every test; each
//! test uses its own branches, pull request numbers and file bytes, so they run side by side.
//! The worker answers on two origins, the app origin and the files origin, under two host names
//! for the same local port.

// Beside this file, not in tests/, where cargo would take each for a test of its own.
#[path = "host/browser.rs"]
mod browser;
#[path = "host/github.rs"]
mod github;
#[path = "host/worker.rs"]
mod worker;

use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use libtest_mimic::{Arguments, Failed, Trial};
use rsa::pkcs1::EncodeRsaPrivateKey as _;
use rsa::pkcs1v15::SigningKey;
use rsa::signature::{SignatureEncoding as _, Signer as _};
use rsa::{RsaPrivateKey, RsaPublicKey};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use github::FakeGitHub;
use worker::{AUDIENCE, INSTALL_URL, ISSUER, SIGNING_KEY, Worker};

struct World {
    worker: Worker,
    github: FakeGitHub,
    /// Signs the OIDC tokens Actions would give CI.
    oidc: RsaPrivateKey,
    /// A key GitHub never published, for forged tokens.
    stranger: RsaPrivateKey,
}

static WORLD: OnceLock<World> = OnceLock::new();

fn world() -> &'static World {
    WORLD.get_or_init(|| {
        let mut rng = rand::thread_rng();
        let oidc = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let app = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let stranger = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let github = FakeGitHub::start(RsaPublicKey::from(&oidc), RsaPublicKey::from(&app));
        // GitHub hands out app keys as PKCS#1 PEM; the host takes them as they come.
        let pem = app.to_pkcs1_pem(rsa::pkcs8::LineEnding::LF).unwrap();
        let worker = Worker::start(&github.url, &pem);
        let world = World {
            worker,
            github,
            oidc,
            stranger,
        };
        // Fills the host's JWKS cache before any test runs, so the only fetches of GitHub's keys
        // after this are the refetches a test asks for.
        let auth = format!("Bearer {}", sign(&world.oidc, &claims("owner/repo", 1)));
        let headers = [("Authorization", auth.as_str())];
        let answer = request_to(
            &world.worker.url,
            "GET",
            "/api/v2/installed",
            &headers,
            None,
        );
        if answer.status != 200 {
            // The world never gets stored, so main cannot stop the worker: stop it here.
            world.worker.stop();
            panic!(
                "warming the JWKS cache answered {}: {}\nwrangler dev said:\n{}",
                answer.status,
                answer.text(),
                world.worker.log()
            );
        }
        world
    })
}

fn main() {
    let args = Arguments::from_args();
    let tests: &[(&str, fn())] = &[
        (
            "installed_says_yes_for_a_repository_with_the_app",
            installed_says_yes_for_a_repository_with_the_app,
        ),
        (
            "installed_says_no_with_the_install_link",
            installed_says_no_with_the_install_link,
        ),
        (
            "a_call_without_a_token_is_refused",
            a_call_without_a_token_is_refused,
        ),
        (
            "a_token_signed_by_another_key_is_refused",
            a_token_signed_by_another_key_is_refused,
        ),
        ("an_expired_token_is_refused", an_expired_token_is_refused),
        (
            "a_token_for_another_audience_is_refused",
            a_token_for_another_audience_is_refused,
        ),
        (
            "a_token_from_another_issuer_is_refused",
            a_token_from_another_issuer_is_refused,
        ),
        (
            "a_run_is_refused_unless_the_app_is_installed",
            a_run_is_refused_unless_the_app_is_installed,
        ),
        (
            "a_run_for_another_repository_is_refused",
            a_run_for_another_repository_is_refused,
        ),
        (
            "a_run_for_another_pull_request_is_refused",
            a_run_for_another_pull_request_is_refused,
        ),
        (
            "a_run_for_another_branch_is_refused",
            a_run_for_another_branch_is_refused,
        ),
        (
            "a_run_opens_and_answers_the_page_of_its_work",
            a_run_opens_and_answers_the_page_of_its_work,
        ),
        ("checks_stream_into_the_run", checks_stream_into_the_run),
        (
            "a_finished_check_asks_only_for_the_files_the_repository_lacks",
            a_finished_check_asks_only_for_the_files_the_repository_lacks,
        ),
        (
            "a_blob_whose_bytes_do_not_hash_to_its_name_is_refused",
            a_blob_whose_bytes_do_not_hash_to_its_name_is_refused,
        ),
        (
            "a_blob_no_run_names_is_refused",
            a_blob_no_run_names_is_refused,
        ),
        (
            "done_is_refused_while_a_file_is_missing",
            done_is_refused_while_a_file_is_missing,
        ),
        (
            "a_run_takes_calls_only_from_its_own_repository_and_ref",
            a_run_takes_calls_only_from_its_own_repository_and_ref,
        ),
        (
            "a_closed_run_takes_nothing_more",
            a_closed_run_takes_nothing_more,
        ),
        (
            "done_refuses_a_proof_of_an_unknown_schema",
            done_refuses_a_proof_of_an_unknown_schema,
        ),
        (
            "the_work_page_is_served_for_a_navigation",
            the_work_page_is_served_for_a_navigation,
        ),
        ("an_unknown_work_is_not_found", an_unknown_work_is_not_found),
        (
            "the_app_origin_serves_no_files_and_the_files_origin_no_pages",
            the_app_origin_serves_no_files_and_the_files_origin_no_pages,
        ),
        (
            "a_private_work_sends_a_stranger_to_sign_in",
            a_private_work_sends_a_stranger_to_sign_in,
        ),
        (
            "a_reader_signs_in_and_sees_a_private_work",
            a_reader_signs_in_and_sees_a_private_work,
        ),
        (
            "a_signed_in_person_without_access_gets_not_found",
            a_signed_in_person_without_access_gets_not_found,
        ),
        (
            "an_unknown_repository_looks_like_a_private_one",
            an_unknown_repository_looks_like_a_private_one,
        ),
        (
            "a_forged_session_counts_as_none",
            a_forged_session_counts_as_none,
        ),
        (
            "sign_in_refuses_a_state_it_did_not_give",
            sign_in_refuses_a_state_it_did_not_give,
        ),
        ("logout_ends_the_session", logout_ends_the_session),
        (
            "the_session_cookie_holds_only_a_random_id",
            the_session_cookie_holds_only_a_random_id,
        ),
        (
            "a_second_view_does_not_ask_github_who_may_read",
            a_second_view_does_not_ask_github_who_may_read,
        ),
        (
            "a_second_view_does_not_ask_github_whether_public",
            a_second_view_does_not_ask_github_whether_public,
        ),
        (
            "an_unknown_key_id_refetches_the_jwks_once",
            an_unknown_key_id_refetches_the_jwks_once,
        ),
        (
            "a_file_is_served_in_ranges_under_a_grant",
            a_file_is_served_in_ranges_under_a_grant,
        ),
        (
            "the_files_origin_refuses_a_missing_tampered_or_expired_grant",
            the_files_origin_refuses_a_missing_tampered_or_expired_grant,
        ),
        (
            "the_files_origin_refuses_a_grant_for_another_run",
            the_files_origin_refuses_a_grant_for_another_run,
        ),
        (
            "the_files_origin_ignores_cookies_and_sets_none",
            the_files_origin_ignores_cookies_and_sets_none,
        ),
        (
            "a_page_shows_a_check_move_from_running_to_passed_live",
            a_page_shows_a_check_move_from_running_to_passed_live,
        ),
        (
            "a_page_opened_mid_run_shows_the_current_state",
            a_page_opened_mid_run_shows_the_current_state,
        ),
        ("two_pages_both_update", two_pages_both_update),
    ];
    let trials: Vec<Trial> = tests
        .iter()
        .map(|&(name, test)| Trial::test(name, move || run(test)))
        .collect();
    let conclusion = libtest_mimic::run(&args, trials);
    if let Some(world) = WORLD.get() {
        if conclusion.has_failed() {
            eprintln!("wrangler dev said:\n{}", world.worker.log());
        }
        world.worker.stop();
    }
    conclusion.exit();
}

fn run(test: fn()) -> Result<(), Failed> {
    std::panic::catch_unwind(test).map_err(|panic| {
        let message = panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_owned()))
            .unwrap_or_default();
        Failed::from(message)
    })
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .try_into()
        .unwrap()
}

// The protocol.

fn installed_says_yes_for_a_repository_with_the_app() {
    let answer = api("GET", "/api/v2/installed", &ci("owner/repo", 1), None);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.json(), json!({"installed": true}));
}

fn installed_says_no_with_the_install_link() {
    let answer = api("GET", "/api/v2/installed", &ci("owner/bare", 1), None);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(
        answer.json(),
        json!({"installed": false, "install": INSTALL_URL})
    );
}

fn a_call_without_a_token_is_refused() {
    let answer = request("GET", "/api/v2/installed", &[], None);
    assert_eq!(answer.status, 401, "{}", answer.text());
    let answer = request("POST", "/api/v2/runs", &[], Some(b"{}".to_vec()));
    assert_eq!(answer.status, 401, "{}", answer.text());
}

fn a_token_signed_by_another_key_is_refused() {
    let token = sign(&world().stranger, &claims("owner/repo", 1));
    let answer = api("GET", "/api/v2/installed", &token, None);
    assert_eq!(answer.status, 401, "{}", answer.text());
}

fn an_expired_token_is_refused() {
    let mut claims = claims("owner/repo", 1);
    claims["iat"] = json!(now() - 900);
    claims["nbf"] = json!(now() - 900);
    claims["exp"] = json!(now() - 300);
    let answer = api("GET", "/api/v2/installed", &oidc(&claims), None);
    assert_eq!(answer.status, 401, "{}", answer.text());
}

fn a_token_for_another_audience_is_refused() {
    let mut claims = claims("owner/repo", 1);
    claims["aud"] = json!("https://elsewhere.test");
    let answer = api("GET", "/api/v2/installed", &oidc(&claims), None);
    assert_eq!(answer.status, 401, "{}", answer.text());
}

fn a_token_from_another_issuer_is_refused() {
    let mut claims = claims("owner/repo", 1);
    claims["iss"] = json!("https://issuer.elsewhere.test");
    let answer = api("GET", "/api/v2/installed", &oidc(&claims), None);
    assert_eq!(answer.status, 401, "{}", answer.text());
}

fn a_run_is_refused_unless_the_app_is_installed() {
    let job = Job::push("owner/bare", "bare");
    let answer = job.open_answer(&job.opening(&["tests"]));
    assert_eq!(answer.status, 403, "{}", answer.text());
    assert!(answer.text().contains(INSTALL_URL), "{}", answer.text());
}

fn a_run_for_another_repository_is_refused() {
    let job = Job::push("owner/repo", "theirs");
    let mut body = job.opening(&["tests"]);
    body["repository"] = json!("other/repo");
    let answer = job.open_answer(&body);
    assert_eq!(answer.status, 403, "{}", answer.text());
}

fn a_run_for_another_pull_request_is_refused() {
    // A pull request's token names its merge ref; it opens no run of another pull request.
    let job = Job::pull("owner/repo", 4, "four");
    let mut body = job.opening(&["tests"]);
    body["pull"]["number"] = json!(5);
    let answer = job.open_answer(&body);
    assert_eq!(answer.status, 403, "{}", answer.text());
    // Nor of a push.
    let mut body = job.opening(&["tests"]);
    body.as_object_mut().unwrap().remove("pull");
    let answer = job.open_answer(&body);
    assert_eq!(answer.status, 403, "{}", answer.text());
}

fn a_run_for_another_branch_is_refused() {
    // A push of one branch opens no run of another.
    let job = Job::push("owner/repo", "mine");
    let mut body = job.opening(&["tests"]);
    body["branch"] = json!("main");
    let answer = job.open_answer(&body);
    assert_eq!(answer.status, 403, "{}", answer.text());
    // A pull request's run is of its own head branch.
    let job = Job::pull("owner/repo", 6, "six");
    let mut body = job.opening(&["tests"]);
    body["branch"] = json!("main");
    let answer = job.open_answer(&body);
    assert_eq!(answer.status, 403, "{}", answer.text());
}

fn a_run_opens_and_answers_the_page_of_its_work() {
    let app = &world().worker.url;
    let push = Job::push("owner/repo", "feat/opens");
    let opened = push.open(&["tests"]);
    assert_eq!(opened.url, format!("{app}/owner/repo/work/feat/opens"));
    assert_eq!(opened.id.len(), 32, "{}", opened.id);
    // A pull request's work shows at the pull request.
    let pull = Job::pull("owner/repo", 7, "feat/seven");
    let opened = pull.open(&["tests"]);
    assert_eq!(opened.url, format!("{app}/owner/repo/pull/7"));
    // Once the branch has a pull request, its branch page is the same page.
    let snapshot = live("/owner/repo/work/feat/seven", &[]);
    assert_eq!(snapshot.status, 200, "{}", snapshot.text());
    assert_eq!(snapshot.json()["work"]["pull"]["number"], 7);
    // A branch with characters a path gives a meaning to is escaped.
    let odd = Job::push("owner/repo", "odd#branch?");
    let opened = odd.open(&["tests"]);
    assert_eq!(opened.url, format!("{app}/owner/repo/work/odd%23branch%3F"));
    let snapshot = live("/owner/repo/work/odd%23branch%3F", &[]);
    assert_eq!(snapshot.status, 200, "{}", snapshot.text());
    assert_eq!(snapshot.json()["work"]["branch"], "odd#branch?");
}

fn checks_stream_into_the_run() {
    let job = Job::push("owner/repo", "feat/stream");
    let run = job.open(&["fmt", "tests", "lint"]);
    let path = "/owner/repo/work/feat/stream";
    let first = live(path, &[]).json();
    assert_eq!(first["kind"], "snapshot");
    assert_eq!(first["work"]["repository"], "owner/repo");
    assert_eq!(first["work"]["branch"], "feat/stream");
    let ids: Vec<&str> = first["runs"][0]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        ["fmt", "tests", "lint"],
        "the planned checks, in order"
    );
    assert_eq!(first["runs"][0]["state"], "running");
    assert_eq!(first["runs"][0]["source"]["job"], "proof");

    assert_eq!(job.started(&run.id, "fmt").status, 200);
    let answer = job.finished(&run.id, &entry("fmt", "passed"), &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.json(), json!({"missing": []}));
    assert_eq!(job.started(&run.id, "tests").status, 200);
    // A check the run did not plan joins at the end.
    assert_eq!(job.started(&run.id, "extra").status, 200);

    let now = live(path, &[]).json();
    let checks = &now["runs"][0]["checks"];
    assert_eq!(checks[0]["entry"]["outcome"], "passed", "{now:#}");
    assert_eq!(checks[0]["entry"]["seconds"], 1.5, "{now:#}");
    assert!(checks[1]["started"].is_number(), "{now:#}");
    assert!(checks[1].get("entry").is_none(), "{now:#}");
    assert!(
        checks[2].get("started").is_none(),
        "lint has not started: {now:#}"
    );
    assert_eq!(checks[3]["id"], "extra", "{now:#}");
    // Each event counts: the snapshot says which one it holds.
    assert_eq!(
        now["seq"].as_u64().unwrap(),
        first["seq"].as_u64().unwrap() + 4,
        "{now:#}"
    );
}

fn a_finished_check_asks_only_for_the_files_the_repository_lacks() {
    let log: &[u8] = b"the output of tests, which does not change";
    let job = Job::push("owner/repo", "feat/lacks");
    let run = job.open(&["tests"]);
    let answer = job.finished(
        &run.id,
        &entry("tests", "passed"),
        &[("outputs/tests.log", log)],
    );
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.json(), json!({"missing": [sha256(log)]}));
    assert_eq!(job.send(log).status, 204);

    // The next run of the repository holds it already.
    let run = job.open(&["tests"]);
    let answer = job.finished(
        &run.id,
        &entry("tests", "passed"),
        &[("outputs/tests.log", log)],
    );
    assert_eq!(answer.json(), json!({"missing": []}));

    // Blobs are kept per repository: another repository sends its own.
    let other = Job::push("other/repo", "feat/lacks");
    let run = other.open(&["tests"]);
    let answer = other.finished(
        &run.id,
        &entry("tests", "passed"),
        &[("outputs/tests.log", log)],
    );
    assert_eq!(answer.json(), json!({"missing": [sha256(log)]}));
}

fn a_blob_whose_bytes_do_not_hash_to_its_name_is_refused() {
    let honest: &[u8] = b"<p>honest bytes</p>";
    let job = Job::push("owner/repo", "feat/honest");
    let run = job.open(&["tests"]);
    job.finished(
        &run.id,
        &entry("tests", "passed"),
        &[("outputs/tests.log", honest)],
    );
    // As long as the honest bytes, so only their hash tells them apart.
    let forged: &[u8] = b"<p>forged bytes</p>";
    assert_eq!(forged.len(), honest.len());
    let path = format!("/api/v2/blobs/{}", sha256(honest));
    let answer = api("PUT", &path, &job.token(), Some(forged.to_vec()));
    assert_eq!(answer.status, 400, "{}", answer.text());
    assert!(answer.text().contains("hash"), "{}", answer.text());
    // Nothing was kept: the honest bytes are still asked for, and still taken.
    let answer = job.done(&run.id, &[entry("tests", "passed")], &[]);
    assert_eq!(answer.status, 409, "{}", answer.text());
    assert_eq!(job.send(honest).status, 204);
}

fn a_blob_no_run_names_is_refused() {
    let job = Job::push("owner/repo", "feat/nobody");
    let answer = job.send(b"bytes nobody asked for");
    assert_eq!(answer.status, 409, "{}", answer.text());
}

fn done_is_refused_while_a_file_is_missing() {
    let log: &[u8] = b"the output of a check whose bytes never came";
    let picture: &[u8] = b"\x89PNG a picture only done names";
    let job = Job::push("owner/repo", "feat/missing");
    let run = job.open(&["tests"]);
    job.finished(
        &run.id,
        &entry("tests", "passed"),
        &[("outputs/tests.log", log)],
    );
    // Done names the files the bundle holds besides; it lacks both, and says which.
    let files: &[(&str, &[u8])] = &[
        ("outputs/tests.log", log),
        ("looks/screens/after.png", picture),
    ];
    let answer = job.done(&run.id, &[entry("tests", "passed")], files);
    assert_eq!(answer.status, 409, "{}", answer.text());
    let mut wanted = vec![sha256(log), sha256(picture)];
    wanted.sort();
    assert_eq!(answer.json()["missing"], json!(wanted), "{}", answer.text());
    let path = "/owner/repo/work/feat/missing";
    assert_eq!(live(path, &[]).json()["runs"][0]["state"], "running");

    assert_eq!(job.send(log).status, 204);
    assert_eq!(job.send(picture).status, 204);
    // The proof's entries are the final word on each check.
    let mut last = entry("tests", "passed");
    last["unstable"] = json!(true);
    let answer = job.done(&run.id, &[last], files);
    assert_eq!(answer.status, 200, "{}", answer.text());
    let url = format!("{}{path}", world().worker.url);
    assert_eq!(answer.json(), json!({"url": url}));
    let now = live(path, &[]).json();
    assert_eq!(now["runs"][0]["state"], "done", "{now:#}");
    assert_eq!(now["runs"][0]["checks"][0]["entry"]["unstable"], true);
}

fn a_run_takes_calls_only_from_its_own_repository_and_ref() {
    let job = Job::pull("owner/repo", 8, "feat/eight");
    let run = job.open(&["tests"]);
    // Another repository's token does not find it.
    let stranger = Job::pull("other/repo", 8, "feat/eight");
    assert_eq!(stranger.started(&run.id, "tests").status, 404);
    assert_eq!(stranger.done(&run.id, &[], &[]).status, 404);
    // A token of the same repository for another pull request may not write it.
    let other = Job::pull("owner/repo", 9, "feat/eight");
    let answer = other.started(&run.id, "tests");
    assert_eq!(answer.status, 403, "{}", answer.text());
    assert_eq!(other.done(&run.id, &[], &[]).status, 403);
    // Nor may a push of the same branch.
    let push = Job::push("owner/repo", "feat/eight");
    assert_eq!(push.started(&run.id, "tests").status, 403);
    // Its own job may.
    assert_eq!(job.started(&run.id, "tests").status, 200);
}

fn a_closed_run_takes_nothing_more() {
    let job = Job::push("owner/repo", "feat/closed");
    let run = job.open(&["tests"]);
    assert_eq!(
        job.done(&run.id, &[entry("tests", "passed")], &[]).status,
        200
    );
    let answer = job.started(&run.id, "tests");
    assert_eq!(answer.status, 409, "{}", answer.text());
    assert_eq!(
        job.done(&run.id, &[entry("tests", "passed")], &[]).status,
        409
    );
}

fn done_refuses_a_proof_of_an_unknown_schema() {
    let job = Job::push("owner/repo", "feat/schema");
    let run = job.open(&["tests"]);
    let mut body = job.closing(&[], &[]);
    body["proof"]["schema"] = json!(99);
    let path = format!("/api/v2/runs/{}/done", run.id);
    let answer = api("POST", &path, &job.token(), Some(body.to_string().into()));
    assert_eq!(answer.status, 400, "{}", answer.text());
    assert!(answer.text().contains("99"), "{}", answer.text());
}

// The pages.

fn the_work_page_is_served_for_a_navigation() {
    let job = Job::pull("owner/repo", 20, "feat/page");
    job.open(&["tests"]);
    for path in ["/owner/repo/work/feat/page", "/owner/repo/pull/20"] {
        let answer = navigate(path, &[]);
        assert_eq!(answer.status, 200, "{path}: {}", answer.text());
        assert!(
            answer
                .header("content-type")
                .unwrap()
                .starts_with("text/html"),
            "{path}"
        );
        let html = answer.text();
        assert!(html.contains(r#"<div id="app">"#), "{path}: {html}");
        // The app's script is served beside it, from the same origin.
        let start = html.find("/assets/").unwrap();
        let end = start + html[start..].find('"').unwrap();
        let script = get(&html[start..end], &[]);
        assert_eq!(script.status, 200, "{}", &html[start..end]);
        let policy = answer.header("content-security-policy").unwrap();
        assert!(policy.contains("frame-ancestors 'none'"), "{policy}");
    }
}

fn an_unknown_work_is_not_found() {
    for path in [
        "/owner/repo/work/no-such-branch",
        "/owner/repo/pull/999",
        "/owner/repo/elsewhere/x",
    ] {
        assert_eq!(navigate(path, &[]).status, 404, "{path}");
    }
    assert_eq!(live("/owner/repo/work/no-such-branch", &[]).status, 404);
}

fn the_app_origin_serves_no_files_and_the_files_origin_no_pages() {
    let log: &[u8] = b"<script>steal(document.cookie)</script>";
    let job = Job::push("owner/repo", "feat/origins");
    let run = job.open(&["tests"]);
    job.finished(
        &run.id,
        &entry("tests", "passed"),
        &[("outputs/tests.log", log)],
    );
    job.send(log);
    let page = "/owner/repo/work/feat/origins";
    let answer = navigate(page, &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert!(!answer.text().contains("steal"));
    let answer = get(&format!("{page}/outputs/tests.log"), &[]);
    assert_eq!(answer.status, 404, "{}", answer.text());
    // The files origin answers none of the app's paths.
    for path in [
        page,
        "/login",
        "/api/v2/installed",
        "/live/owner/repo/work/feat/origins",
    ] {
        let answer = get_url(&format!("{}{path}", world().worker.files), &[]);
        assert_eq!(answer.status, 404, "{path}: {}", answer.text());
    }
    // Under a grant, the file is there, as a file.
    let grant = files_base("owner/repo", &run.id, &[]);
    let answer = get_url(&format!("{grant}outputs/tests.log"), &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.body, log);
    let policy = answer.header("content-security-policy").unwrap();
    assert!(
        policy.contains(&format!("frame-ancestors {}", world().worker.url)),
        "{policy}"
    );
}

// Who sees a page.

fn a_private_work_sends_a_stranger_to_sign_in() {
    Job::push("owner/secret", "feat/stranger").open(&["tests"]);
    let path = "/owner/secret/work/feat/stranger";
    let answer = navigate(path, &[]);
    assert_eq!(answer.status, 302, "{}", answer.text());
    assert_eq!(
        answer.header("location").unwrap(),
        format!("/login?next={}", path.replace('/', "%2F"))
    );
    // Its live connection answers without a sign in, and says nothing of the work.
    let answer = live(path, &[]);
    assert_eq!(answer.status, 401, "{}", answer.text());
    assert!(
        !answer.text().contains("feat/stranger"),
        "{}",
        answer.text()
    );
}

fn a_reader_signs_in_and_sees_a_private_work() {
    Job::pull("owner/secret", 31, "feat/reader").open(&["tests"]);
    let path = "/owner/secret/pull/31";
    let session = sign_in("reader-code", path);
    let answer = navigate(path, &[("Cookie", &session)]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    let answer = live(path, &[("Cookie", &session)]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.json()["work"]["branch"], "feat/reader");
}

fn a_signed_in_person_without_access_gets_not_found() {
    Job::push("owner/secret", "feat/no-access").open(&["tests"]);
    let path = "/owner/secret/work/feat/no-access";
    let session = sign_in("stranger-code", path);
    assert_eq!(navigate(path, &[("Cookie", &session)]).status, 404);
    assert_eq!(live(path, &[("Cookie", &session)]).status, 404);
}

fn an_unknown_repository_looks_like_a_private_one() {
    let path = "/nobody/nothing/work/main";
    let answer = navigate(path, &[]);
    assert_eq!(answer.status, 302, "{}", answer.text());
    let session = sign_in("stranger-code", path);
    assert_eq!(navigate(path, &[("Cookie", &session)]).status, 404);
}

fn a_forged_session_counts_as_none() {
    Job::push("owner/secret", "feat/forged").open(&["tests"]);
    let path = "/owner/secret/work/feat/forged";
    let session = sign_in("reader-code", path);
    let mut forged = session.clone();
    let last = forged.pop().unwrap();
    forged.push(if last == 'A' { 'B' } else { 'A' });
    let answer = navigate(path, &[("Cookie", &forged)]);
    assert_eq!(answer.status, 302, "{}", answer.text());
}

fn sign_in_refuses_a_state_it_did_not_give() {
    let answer = get("/login?next=%2F", &[]);
    let cookie = cookie(&answer, "signin").unwrap();
    let answer = get(
        "/callback?code=reader-code&state=not-the-state",
        &[("Cookie", &cookie)],
    );
    assert_eq!(answer.status, 400, "{}", answer.text());
    assert!(cookie_set(&answer, "session").is_none());
}

fn logout_ends_the_session() {
    Job::push("owner/secret", "feat/logout").open(&["tests"]);
    let path = "/owner/secret/work/feat/logout";
    let session = sign_in("reader-code", path);
    assert_eq!(navigate(path, &[("Cookie", &session)]).status, 200);
    let answer = get("/logout", &[("Cookie", &session)]);
    assert_eq!(answer.status, 302, "{}", answer.text());
    let cleared = cookie_set(&answer, "session").unwrap();
    assert!(cleared.contains("Max-Age=0"), "{cleared}");
    // The host forgot the session: the old cookie, sent again, is no sign in.
    let answer = navigate(path, &[("Cookie", &session)]);
    assert_eq!(answer.status, 302, "{}", answer.text());
    let location = answer.header("location").unwrap();
    assert!(location.starts_with("/login?"), "{location}");
}

fn the_session_cookie_holds_only_a_random_id() {
    let answer = sign_in_answer("reader-code", "/");
    let set = cookie_set(&answer, "session").unwrap();
    for attribute in ["HttpOnly", "Secure", "SameSite=Lax"] {
        assert!(set.contains(attribute), "{set}");
    }
    let session = cookie(&answer, "session").unwrap();
    let value = session.strip_prefix("session=").unwrap();
    // 128 random bits or more, and nothing else: no login, no GitHub token.
    assert!(value.len() >= 32, "{value}");
    assert!(value.bytes().all(|b| b.is_ascii_hexdigit()), "{value}");
    // Each sign in gets its own.
    let again = sign_in("reader-code", "/");
    assert_ne!(again, session);
}

// Caching GitHub's answers.

fn a_second_view_does_not_ask_github_who_may_read() {
    Job::push("owner/cached", "feat/cached").open(&["tests"]);
    let path = "/owner/cached/work/feat/cached";
    let session = sign_in("reader-code", path);
    let github = &world().github;
    let before = github.asked("GET", "/repos/owner/cached");
    assert_eq!(navigate(path, &[("Cookie", &session)]).status, 200);
    assert_eq!(navigate(path, &[("Cookie", &session)]).status, 200);
    assert_eq!(live(path, &[("Cookie", &session)]).status, 200);
    assert_eq!(github.asked("GET", "/repos/owner/cached"), before + 1);
}

fn a_second_view_does_not_ask_github_whether_public() {
    Job::push("owner/open", "feat/open").open(&["tests"]);
    let path = "/owner/open/work/feat/open";
    let github = &world().github;
    let installation = "/repos/owner/open/installation";
    let (repo, install) = (
        github.asked("GET", "/repos/owner/open"),
        github.asked("GET", installation),
    );
    assert_eq!(navigate(path, &[]).status, 200);
    assert_eq!(navigate(path, &[]).status, 200);
    assert_eq!(live(path, &[]).status, 200);
    assert_eq!(github.asked("GET", "/repos/owner/open"), repo + 1);
    assert_eq!(github.asked("GET", installation), install + 1);
}

fn an_unknown_key_id_refetches_the_jwks_once() {
    let github = &world().github;
    let jwks = "/.well-known/jwks";
    let before = github.asked("GET", jwks);
    // Known keys come from the cache.
    for _ in 0..2 {
        let answer = api("GET", "/api/v2/installed", &ci("owner/repo", 1), None);
        assert_eq!(answer.status, 200, "{}", answer.text());
    }
    assert_eq!(github.asked("GET", jwks), before);

    // A key id GitHub never published: one refetch, then refused.
    let mut rng = rand::thread_rng();
    let rotated = RsaPrivateKey::new(&mut rng, 2048).unwrap();
    let token = sign_with_kid(&rotated, "rotated-key", &claims("owner/repo", 1));
    let answer = api("GET", "/api/v2/installed", &token, None);
    assert_eq!(answer.status, 401, "{}", answer.text());
    assert_eq!(github.asked("GET", jwks), before + 1);

    // Once GitHub publishes it, the next token naming it refetches and passes, and later ones
    // find it in the cache.
    github.publish("rotated-key", RsaPublicKey::from(&rotated));
    let answer = api("GET", "/api/v2/installed", &token, None);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(github.asked("GET", jwks), before + 2);
    let answer = api("GET", "/api/v2/installed", &token, None);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(github.asked("GET", jwks), before + 2);
}

// The files origin.

/// A run of `owner/repo` holding `bytes` at `path`, sent; its id.
fn run_with_file(branch: &str, path: &str, bytes: &[u8]) -> String {
    let job = Job::push("owner/repo", branch);
    let run = job.open(&["tests"]);
    job.finished(&run.id, &entry("tests", "passed"), &[(path, bytes)]);
    assert_eq!(job.send(bytes).status, 204);
    run.id
}

fn a_file_is_served_in_ranges_under_a_grant() {
    let film: &[u8] = b"0123456789 a film of eleven and more bytes";
    let run = run_with_file("feat/film", "looks/screens/run.mp4", film);
    let path = format!(
        "{}looks/screens/run.mp4",
        files_base("owner/repo", &run, &[])
    );

    let answer = get_url(&path, &[("Range", "bytes=2-5")]);
    assert_eq!(answer.status, 206, "{}", answer.text());
    assert_eq!(answer.body, b"2345");
    assert_eq!(
        answer.header("content-range").unwrap(),
        format!("bytes 2-5/{}", film.len())
    );
    assert_eq!(answer.header("content-type").unwrap(), "video/mp4");

    let answer = get_url(&path, &[("Range", "bytes=-5")]);
    assert_eq!(answer.status, 206, "{}", answer.text());
    assert_eq!(answer.body, &film[film.len() - 5..]);

    let answer = get_url(&path, &[("Range", "bytes=40-")]);
    assert_eq!(answer.status, 206, "{}", answer.text());
    assert_eq!(answer.body, &film[40..]);

    let answer = get_url(&path, &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.header("accept-ranges").unwrap(), "bytes");
    let base = files_base("owner/repo", &run, &[]);
    assert_eq!(get_url(&format!("{base}looks/none.png"), &[]).status, 404);
}

fn the_files_origin_refuses_a_missing_tampered_or_expired_grant() {
    let run = run_with_file("feat/grant", "outputs/tests.log", b"granted bytes");
    let files = &world().worker.files;
    for path in [
        "/outputs/tests.log".to_owned(),
        format!("/{run}/outputs/tests.log"),
    ] {
        let answer = get_url(&format!("{files}{path}"), &[]);
        assert_eq!(answer.status, 404, "{path}: {}", answer.text());
    }
    let fresh = files_base("owner/repo", &run, &[]);
    assert_eq!(
        get_url(&format!("{fresh}outputs/tests.log"), &[]).status,
        200
    );
    let expired = files_base("owner/repo", &run, &[("exp", json!(now() - 1))]);
    assert_eq!(
        get_url(&format!("{expired}outputs/tests.log"), &[]).status,
        404
    );
    // A later expiry under the old seal, and a seal changed by one character.
    let grant = fresh
        .strip_prefix(&format!("{files}/"))
        .unwrap()
        .strip_suffix('/')
        .unwrap();
    let (payload, mac) = grant.split_once('.').unwrap();
    let decoded = URL_SAFE_NO_PAD.decode(payload).unwrap();
    let mut claims: Value = serde_json::from_slice(&decoded).unwrap();
    claims["exp"] = json!(claims["exp"].as_i64().unwrap() + 86400);
    let longer = URL_SAFE_NO_PAD.encode(claims.to_string());
    let answer = get_url(&format!("{files}/{longer}.{mac}/outputs/tests.log"), &[]);
    assert_eq!(answer.status, 404, "{}", answer.text());
    let mut mac = mac.to_owned();
    let last = mac.pop().unwrap();
    mac.push(if last == 'A' { 'B' } else { 'A' });
    let answer = get_url(&format!("{files}/{payload}.{mac}/outputs/tests.log"), &[]);
    assert_eq!(answer.status, 404, "{}", answer.text());
}

fn the_files_origin_refuses_a_grant_for_another_run() {
    let mine = run_with_file("feat/mine", "outputs/tests.log", b"mine");
    let job = Job::push("owner/secret", "feat/theirs");
    let theirs = job.open(&["tests"]);
    job.finished(
        &theirs.id,
        &entry("tests", "passed"),
        &[("outputs/tests.log", b"theirs")],
    );
    job.send(b"theirs");
    let base = files_base("owner/repo", &mine, &[]);
    assert_eq!(
        get_url(&format!("{base}outputs/tests.log"), &[]).body,
        b"mine"
    );
    // A grant naming a run of another repository than its own opens nothing.
    let crossed = files_base("owner/repo", &theirs.id, &[]);
    let answer = get_url(&format!("{crossed}outputs/tests.log"), &[]);
    assert_eq!(answer.status, 404, "{}", answer.text());
    let unknown = files_base("owner/repo", "0123456789abcdef0123456789abcdef", &[]);
    assert_eq!(
        get_url(&format!("{unknown}outputs/tests.log"), &[]).status,
        404
    );
}

fn the_files_origin_ignores_cookies_and_sets_none() {
    let run = run_with_file("feat/cookies", "outputs/tests.log", b"cookies");
    let session = sign_in("reader-code", "/");
    let base = files_base("owner/repo", &run, &[]);
    for headers in [vec![("Cookie", session.as_str())], vec![]] {
        let answer = get_url(&format!("{base}outputs/tests.log"), &headers);
        assert_eq!(answer.status, 200, "{}", answer.text());
        assert!(answer.header("set-cookie").is_none());
    }
    // A session is no grant.
    let files = &world().worker.files;
    let answer = get_url(
        &format!("{files}/owner/repo/work/feat/cookies"),
        &[("Cookie", &session)],
    );
    assert_eq!(answer.status, 404, "{}", answer.text());
}

// The live page, in a browser.

/// Whether check `id` of the newest run shows in state `state` on the page.
fn shows(id: &str, state: &str) -> String {
    format!(
        r#"document.querySelector('section.run [data-check="{id}"]')?.dataset.state === "{state}""#
    )
}

fn a_page_shows_a_check_move_from_running_to_passed_live() {
    let job = Job::push("owner/repo", "feat/live");
    let run = job.open(&["fmt", "tests"]);
    let mut browser = browser::Browser::start();
    browser.open(&format!("{}/owner/repo/work/feat/live", world().worker.url));
    browser.wait_for(&shows("fmt", "planned"));
    assert_eq!(
        browser.eval("document.querySelector('[data-title]').textContent"),
        json!("feat/live")
    );
    // A mark on the page itself, which a reload would lose.
    browser.eval("window.stayed = true");

    job.started(&run.id, "fmt");
    browser.wait_for(&shows("fmt", "running"));
    job.finished(&run.id, &entry("fmt", "passed"), &[]);
    browser.wait_for(&shows("fmt", "passed"));
    browser.wait_for(
        r#"document.querySelector('[data-check="fmt"] .seconds').textContent === "1.5 s""#,
    );
    job.done(
        &run.id,
        &[entry("fmt", "passed"), entry("tests", "failed")],
        &[],
    );
    browser.wait_for(&shows("tests", "failed"));
    browser.wait_for(r#"document.querySelector('section.run').dataset.state === "done""#);
    assert_eq!(
        browser.eval("window.stayed === true"),
        json!(true),
        "no reload"
    );
}

fn a_page_opened_mid_run_shows_the_current_state() {
    let job = Job::pull("owner/repo", 40, "feat/midway");
    let run = job.open(&["fmt", "tests", "lint"]);
    job.started(&run.id, "fmt");
    job.finished(&run.id, &entry("fmt", "passed"), &[]);
    job.started(&run.id, "tests");
    let mut browser = browser::Browser::start();
    browser.open(&format!("{}/owner/repo/pull/40", world().worker.url));
    browser.wait_for(&shows("fmt", "passed"));
    assert_eq!(browser.eval(&shows("tests", "running")), json!(true));
    assert_eq!(browser.eval(&shows("lint", "planned")), json!(true));
    assert_eq!(
        browser.eval("document.querySelector('[data-title]').textContent"),
        json!("#40 Pull request 40")
    );
    // And it goes on live from there.
    job.finished(&run.id, &entry("tests", "passed"), &[]);
    browser.wait_for(&shows("tests", "passed"));
}

fn two_pages_both_update() {
    let job = Job::push("owner/repo", "feat/two");
    let run = job.open(&["tests"]);
    let url = format!("{}/owner/repo/work/feat/two", world().worker.url);
    let mut browser = browser::Browser::start();
    let first = browser.open(&url);
    browser.wait_for(&shows("tests", "planned"));
    let second = browser.open(&url);
    browser.wait_for(&shows("tests", "planned"));

    job.started(&run.id, "tests");
    for page in [&first, &second] {
        browser.on(page);
        browser.wait_for(&shows("tests", "running"));
    }
    // A second run shows on both, newest first.
    let next = job.open(&["tests"]);
    job.started(&next.id, "tests");
    job.finished(&next.id, &entry("tests", "passed"), &[]);
    for page in [&first, &second] {
        browser.on(page);
        browser.wait_for(&format!(
            r#"document.querySelector('section.run').dataset.run === "{}""#,
            next.id
        ));
        browser.wait_for(&shows("tests", "passed"));
        assert_eq!(
            browser.eval("document.querySelectorAll('section.run').length"),
            json!(2)
        );
    }
}

// Helpers.

/// A CI job of one repository: a pull request's run from its head branch, or a push of a
/// branch. It speaks protocol version 2 with the OIDC token Actions would give it.
struct Job {
    repository: &'static str,
    branch: String,
    pull: Option<u64>,
}

/// A run the host opened.
struct Opened {
    id: String,
    url: String,
}

impl Job {
    fn pull(repository: &'static str, number: u64, branch: &str) -> Job {
        Job {
            repository,
            branch: branch.to_owned(),
            pull: Some(number),
        }
    }

    fn push(repository: &'static str, branch: &str) -> Job {
        Job {
            repository,
            branch: branch.to_owned(),
            pull: None,
        }
    }

    /// The claims Actions puts in the job's token.
    fn claims(&self) -> Value {
        let now = now();
        let (git_ref, head_ref, event) = match self.pull {
            Some(n) => (
                format!("refs/pull/{n}/merge"),
                self.branch.clone(),
                "pull_request",
            ),
            None => (format!("refs/heads/{}", self.branch), String::new(), "push"),
        };
        json!({
            "iss": ISSUER,
            "aud": AUDIENCE,
            "sub": format!("repo:{}:{event}", self.repository),
            "repository": self.repository,
            "repository_owner": self.repository.split('/').next().unwrap(),
            "ref": git_ref,
            "head_ref": head_ref,
            "event_name": event,
            "iat": now,
            "nbf": now,
            "exp": now + 300,
        })
    }

    fn token(&self) -> String {
        oidc(&self.claims())
    }

    /// What the job sends to open a run of `checks`.
    fn opening(&self, checks: &[&str]) -> Value {
        let mut body = json!({
            "repository": self.repository,
            "branch": self.branch,
            "head": "abc1234abc1234abc1234abc1234abc1234abc1",
            "base": "main",
            "source": {"kind": "ci", "job": "proof", "run_url": "https://github.com/run/1"},
            "checks": checks,
        });
        if let Some(n) = self.pull {
            body["pull"] = json!({"number": n, "title": format!("Pull request {n}")});
        }
        body
    }

    fn open_answer(&self, body: &Value) -> Answer {
        api(
            "POST",
            "/api/v2/runs",
            &self.token(),
            Some(body.to_string().into_bytes()),
        )
    }

    /// Opens a run of `checks`.
    fn open(&self, checks: &[&str]) -> Opened {
        let answer = self.open_answer(&self.opening(checks));
        assert_eq!(answer.status, 201, "{}", answer.text());
        let opened = answer.json();
        Opened {
            id: opened["run"].as_str().unwrap().to_owned(),
            url: opened["url"].as_str().unwrap().to_owned(),
        }
    }

    fn step(&self, run: &str, step: &Value) -> Answer {
        let path = format!("/api/v2/runs/{run}/checks");
        api(
            "POST",
            &path,
            &self.token(),
            Some(step.to_string().into_bytes()),
        )
    }

    fn started(&self, run: &str, check: &str) -> Answer {
        self.step(run, &json!({"started": {"check": check}}))
    }

    fn finished(&self, run: &str, check: &Value, files: &[(&str, &[u8])]) -> Answer {
        self.step(
            run,
            &json!({"finished": {"check": check, "files": manifest(files)}}),
        )
    }

    /// The bytes of one file, by content.
    fn send(&self, bytes: &[u8]) -> Answer {
        let path = format!("/api/v2/blobs/{}", sha256(bytes));
        api("PUT", &path, &self.token(), Some(bytes.to_vec()))
    }

    /// What the job sends to close a run whose checks came out as `checks`.
    fn closing(&self, checks: &[Value], files: &[(&str, &[u8])]) -> Value {
        let mut proof = json!({
            "schema": 1,
            "repository": self.repository,
            "commit": "abc1234",
            "head": "abc1234abc1234abc1234abc1234abc1234abc1",
            "base": "main",
            "checks": checks,
            "weakened": [],
            "looks": [],
        });
        if let Some(n) = self.pull {
            proof["pull"] = json!({"number": n, "title": format!("Pull request {n}")});
        }
        json!({"proof": proof, "files": manifest(files)})
    }

    fn done(&self, run: &str, checks: &[Value], files: &[(&str, &[u8])]) -> Answer {
        let path = format!("/api/v2/runs/{run}/done");
        let body = self.closing(checks, files);
        api(
            "POST",
            &path,
            &self.token(),
            Some(body.to_string().into_bytes()),
        )
    }
}

/// A check's entry in the proof, as it ended.
fn entry(id: &str, outcome: &str) -> Value {
    json!({
        "id": id,
        "run": format!("run {id}"),
        "stage": "working",
        "outcome": outcome,
        "seconds": 1.5,
    })
}

/// Files as a call names them: `{path: {sha256, size}}`.
fn manifest(files: &[(&str, &[u8])]) -> Value {
    files
        .iter()
        .map(|(path, bytes)| {
            let file = json!({"sha256": sha256(bytes), "size": bytes.len()});
            ((*path).to_owned(), file)
        })
        .collect::<serde_json::Map<String, Value>>()
        .into()
}

/// The claims of a pull request run's token.
fn claims(repository: &'static str, pull: u64) -> Value {
    Job::pull(repository, pull, &format!("pull-{pull}")).claims()
}

fn ci(repository: &'static str, pull: u64) -> String {
    oidc(&claims(repository, pull))
}

fn oidc(claims: &Value) -> String {
    sign(&world().oidc, claims)
}

fn sign(key: &RsaPrivateKey, claims: &Value) -> String {
    sign_with_kid(key, github::KID, claims)
}

fn sign_with_kid(key: &RsaPrivateKey, kid: &str, claims: &Value) -> String {
    let header = json!({"alg": "RS256", "typ": "JWT", "kid": kid});
    let signed = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    let signature = SigningKey::<Sha256>::new(key.clone()).sign(signed.as_bytes());
    format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature.to_bytes()))
}

/// A page as a browser navigates to it.
fn navigate(path: &str, headers: &[(&str, &str)]) -> Answer {
    let mut all = vec![("Sec-Fetch-Mode", "navigate"), ("Accept", "text/html")];
    all.extend_from_slice(headers);
    get(path, &all)
}

/// The live snapshot of the work at a page's path, asked without upgrading.
fn live(path: &str, headers: &[(&str, &str)]) -> Answer {
    get(&format!("/live{path}"), headers)
}

/// The folder of a run on the files origin under a grant sealed here with the host's key, as
/// the host seals them: `base64url(json).base64url(HMAC-SHA256("grant." + payload))`. `changes`
/// overrides its claims.
fn files_base(repo: &str, run: &str, changes: &[(&str, Value)]) -> String {
    use hmac::Mac as _;
    let mut claims = json!({"repo": repo, "run": run, "exp": now() + 3600});
    for (name, value) in changes {
        claims[*name] = value.clone();
    }
    let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
    let mut mac = hmac::Hmac::<Sha256>::new_from_slice(SIGNING_KEY.as_bytes()).unwrap();
    mac.update(format!("grant.{payload}").as_bytes());
    let mac = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    format!("{}/{payload}.{mac}/", world().worker.files)
}

/// Signs in with a GitHub OAuth code, starting from `next`; the session cookie.
fn sign_in(code: &str, next: &str) -> String {
    cookie(&sign_in_answer(code, next), "session").unwrap()
}

/// Signs in with a GitHub OAuth code, starting from `next`; the callback's answer.
fn sign_in_answer(code: &str, next: &str) -> Answer {
    let answer = get(&format!("/login?next={}", next.replace('/', "%2F")), &[]);
    assert_eq!(answer.status, 302, "{}", answer.text());
    let authorize = answer.header("location").unwrap().to_owned();
    let github = &world().github.url;
    assert!(
        authorize.starts_with(&format!("{github}/login/oauth/authorize?")),
        "{authorize}"
    );
    assert!(
        authorize.contains(&format!("client_id={}", github::CLIENT_ID)),
        "{authorize}"
    );
    let state = authorize
        .split(['?', '&'])
        .find_map(|pair| pair.strip_prefix("state="))
        .unwrap()
        .to_owned();
    let signin = cookie(&answer, "signin").unwrap();
    let answer = get(
        &format!("/callback?code={code}&state={state}"),
        &[("Cookie", &signin)],
    );
    assert_eq!(answer.status, 302, "{}", answer.text());
    assert_eq!(answer.header("location").unwrap(), next);
    answer
}

struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Answer {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|_| panic!("not JSON: {}", self.text()))
    }
}

/// The `Set-Cookie` header that sets a cookie, as it came.
fn cookie_set(answer: &Answer, name: &str) -> Option<String> {
    answer
        .headers
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case("set-cookie"))
        .map(|(_, v)| v.clone())
        .find(|v| v.starts_with(&format!("{name}=")))
}

/// A cookie set by an answer, as a `Cookie` header sends it back.
fn cookie(answer: &Answer, name: &str) -> Option<String> {
    cookie_set(answer, name).map(|v| v.split(';').next().unwrap().to_owned())
}

pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(60)))
        .build()
        .new_agent()
}

fn request(method: &str, path: &str, headers: &[(&str, &str)], body: Option<Vec<u8>>) -> Answer {
    request_to(&world().worker.url, method, path, headers, body)
}

fn request_to(
    origin: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<Vec<u8>>,
) -> Answer {
    let url = format!("{origin}{path}");
    let mut builder = ureq::http::Request::builder().method(method).uri(url);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let agent = agent();
    let response = match body {
        Some(body) => agent.run(builder.body(body).unwrap()),
        None if method == "GET" => agent.run(builder.body(()).unwrap()),
        None => agent.run(builder.body(Vec::new()).unwrap()),
    }
    .unwrap();
    let status = response.status().as_u16();
    let headers = response
        .headers()
        .iter()
        .map(|(n, v)| {
            (
                n.as_str().to_owned(),
                v.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    let body = response.into_body().read_to_vec().unwrap();
    Answer {
        status,
        headers,
        body,
    }
}

fn api(method: &str, path: &str, token: &str, body: Option<Vec<u8>>) -> Answer {
    let auth = format!("Bearer {token}");
    request(method, path, &[("Authorization", &auth)], body)
}

fn get(path: &str, headers: &[(&str, &str)]) -> Answer {
    request("GET", path, headers, None)
}

/// A GET of a whole URL, such as one on the files origin.
fn get_url(url: &str, headers: &[(&str, &str)]) -> Answer {
    request_to(url, "GET", "", headers, None)
}

fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
