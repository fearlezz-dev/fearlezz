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
#[path = "host/local.rs"]
mod local;
#[path = "host/media.rs"]
mod media;
#[path = "host/notes.rs"]
mod notes;
#[path = "host/repository.rs"]
mod repository;
#[path = "host/share.rs"]
mod share;
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
use worker::{AUDIENCE, INSTALL_URL, ISSUER, SIGNING_KEY, WEBHOOK_SECRET, Worker};

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
            "the_files_origin_serves_only_pictures_and_films",
            the_files_origin_serves_only_pictures_and_films,
        ),
        (
            "the_page_gets_a_grant_for_the_files_of_a_run_it_shows",
            the_page_gets_a_grant_for_the_files_of_a_run_it_shows,
        ),
        (
            "a_finished_check_brings_its_look_to_the_page",
            a_finished_check_brings_its_look_to_the_page,
        ),
        (
            "a_look_of_another_check_is_refused",
            a_look_of_another_check_is_refused,
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
        (
            "the_page_shows_a_changed_picture_with_its_before_and_after",
            the_page_shows_a_changed_picture_with_its_before_and_after,
        ),
        (
            "the_compare_modes_switch_by_click_and_by_key",
            the_compare_modes_switch_by_click_and_by_key,
        ),
        (
            "a_film_shows_its_lanes_on_one_clock",
            a_film_shows_its_lanes_on_one_clock,
        ),
        (
            "a_flaky_picture_is_shown_and_marked",
            a_flaky_picture_is_shown_and_marked,
        ),
        (
            "a_look_that_arrives_shows_without_a_reload",
            a_look_that_arrives_shows_without_a_reload,
        ),
        (
            "a_look_shows_only_once_its_pictures_have_arrived",
            a_look_shows_only_once_its_pictures_have_arrived,
        ),
        (
            "pictures_group_by_the_parts_of_their_names",
            pictures_group_by_the_parts_of_their_names,
        ),
        (
            "a_verdict_needs_a_session_write_access_the_app_origin_and_json",
            a_verdict_needs_a_session_write_access_the_app_origin_and_json,
        ),
        (
            "a_bearer_token_never_gives_a_verdict_or_an_accept",
            a_bearer_token_never_gives_a_verdict_or_an_accept,
        ),
        (
            "a_verdict_on_a_local_run_is_refused",
            a_verdict_on_a_local_run_is_refused,
        ),
        (
            "a_verdict_carries_over_to_a_later_run_with_the_same_pictures",
            a_verdict_carries_over_to_a_later_run_with_the_same_pictures,
        ),
        (
            "a_film_verdict_follows_its_changed_states",
            a_film_verdict_follows_its_changed_states,
        ),
        ("accept_needs_an_admin", accept_needs_an_admin),
        (
            "the_check_run_follows_the_review_through_a_whole_flow",
            the_check_run_follows_the_review_through_a_whole_flow,
        ),
        (
            "one_comment_is_created_then_edited",
            one_comment_is_created_then_edited,
        ),
        (
            "a_webhook_with_a_wrong_signature_is_refused",
            a_webhook_with_a_wrong_signature_is_refused,
        ),
        (
            "a_job_that_ends_without_done_cancels_its_run",
            a_job_that_ends_without_done_cancels_its_run,
        ),
        (
            "a_queued_job_shows_as_waiting_for_a_runner",
            a_queued_job_shows_as_waiting_for_a_runner,
        ),
        (
            "a_verdict_given_in_one_tab_shows_in_another",
            a_verdict_given_in_one_tab_shows_in_another,
        ),
        (
            "the_page_says_what_each_person_can_do",
            the_page_says_what_each_person_can_do,
        ),
        (
            "a_note_needs_a_session_write_access_and_a_picture_of_the_run",
            notes::a_note_needs_a_session_write_access_and_a_picture_of_the_run,
        ),
        (
            "needs_change_needs_an_open_note_and_fails_the_check_run",
            notes::needs_change_needs_an_open_note_and_fails_the_check_run,
        ),
        (
            "an_open_note_follows_the_work_until_a_writer_resolves_it",
            notes::an_open_note_follows_the_work_until_a_writer_resolves_it,
        ),
        (
            "a_cli_token_reads_but_never_writes",
            notes::a_cli_token_reads_but_never_writes,
        ),
        (
            "the_cli_signs_in_through_the_host_with_githubs_device_flow",
            notes::the_cli_signs_in_through_the_host_with_githubs_device_flow,
        ),
        (
            "the_cli_reads_open_notes_with_their_pictures",
            notes::the_cli_reads_open_notes_with_their_pictures,
        ),
        (
            "a_note_drawn_on_the_page_is_saved_and_shows_in_another_tab",
            notes::a_note_drawn_on_the_page_is_saved_and_shows_in_another_tab,
        ),
        (
            "a_film_note_takes_the_moment_of_the_film_clock",
            notes::a_film_note_takes_the_moment_of_the_film_clock,
        ),
        (
            "an_old_note_shows_dashed_with_its_run_on_a_later_run",
            notes::an_old_note_shows_dashed_with_its_run_on_a_later_run,
        ),
        (
            "a_cli_token_opens_a_local_run_only_with_write_access",
            local::a_cli_token_opens_a_local_run_only_with_write_access,
        ),
        (
            "a_local_run_takes_notes_but_never_a_verdict_or_an_accept",
            local::a_local_run_takes_notes_but_never_a_verdict_or_an_accept,
        ),
        (
            "a_local_run_never_touches_the_check_run_or_the_comment",
            local::a_local_run_never_touches_the_check_run_or_the_comment,
        ),
        (
            "a_note_on_a_local_run_shows_on_a_later_ci_run_of_the_work",
            local::a_note_on_a_local_run_shows_on_a_later_ci_run_of_the_work,
        ),
        (
            "a_silent_local_run_is_cancelled_after_an_hour",
            local::a_silent_local_run_is_cancelled_after_an_hour,
        ),
        (
            "a_local_run_uploading_its_files_is_not_silent",
            local::a_local_run_uploading_its_files_is_not_silent,
        ),
        (
            "the_host_renews_an_expired_github_token",
            local::the_host_renews_an_expired_github_token,
        ),
        (
            "a_cli_token_unused_for_30_days_is_refused",
            local::a_cli_token_unused_for_30_days_is_refused,
        ),
        (
            "the_page_marks_a_local_run_unproven_and_offers_only_notes",
            local::the_page_marks_a_local_run_unproven_and_offers_only_notes,
        ),
        (
            "a_share_link_shows_the_live_work_and_follows_a_new_run",
            share::a_share_link_shows_the_live_work_and_follows_a_new_run,
        ),
        (
            "a_share_link_never_writes",
            share::a_share_link_never_writes,
        ),
        (
            "a_revoked_share_link_is_not_found_and_names_no_repository",
            share::a_revoked_share_link_is_not_found_and_names_no_repository,
        ),
        (
            "a_writer_makes_lists_and_revokes_share_links_and_a_reader_cannot",
            share::a_writer_makes_lists_and_revokes_share_links_and_a_reader_cannot,
        ),
        (
            "the_repository_page_lists_its_works_with_what_waits",
            repository::the_repository_page_lists_its_works_with_what_waits,
        ),
        (
            "a_private_repository_page_needs_a_reader",
            repository::a_private_repository_page_needs_a_reader,
        ),
        (
            "the_repository_page_updates_live",
            repository::the_repository_page_updates_live,
        ),
    ];
    let mut trials: Vec<Trial> = tests
        .iter()
        .map(|&(name, test)| Trial::test(name, move || run(test)))
        .collect();
    trials.push(Trial::test("pictures", || run(pictures)).with_ignored_flag(true));
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
    let mut body = job.closing(&[], &[], &[]);
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
        // Pictures and films load from the files origin only.
        let files = &world().worker.files;
        for directive in [format!("img-src {files}"), format!("media-src {files}")] {
            assert!(policy.contains(&directive), "{policy}");
        }
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
        &[("looks/tests/home.png", log)],
    );
    job.send(log);
    let page = "/owner/repo/work/feat/origins";
    let answer = navigate(page, &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert!(!answer.text().contains("steal"));
    let answer = get(&format!("{page}/looks/tests/home.png"), &[]);
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
    // Under a grant, the file is there, as the picture its name says and nothing else.
    let grant = files_base("owner/repo", &run.id, &[]);
    let answer = get_url(&format!("{grant}looks/tests/home.png"), &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.body, log);
    assert_eq!(answer.header("content-type").unwrap(), "image/png");
    assert_eq!(answer.header("x-content-type-options").unwrap(), "nosniff");
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
    let run = run_with_file("feat/grant", "looks/tests/home.png", b"granted bytes");
    let files = &world().worker.files;
    for path in [
        "/looks/tests/home.png".to_owned(),
        format!("/{run}/looks/tests/home.png"),
    ] {
        let answer = get_url(&format!("{files}{path}"), &[]);
        assert_eq!(answer.status, 404, "{path}: {}", answer.text());
    }
    let fresh = files_base("owner/repo", &run, &[]);
    assert_eq!(
        get_url(&format!("{fresh}looks/tests/home.png"), &[]).status,
        200
    );
    let expired = files_base("owner/repo", &run, &[("exp", json!(now() - 1))]);
    assert_eq!(
        get_url(&format!("{expired}looks/tests/home.png"), &[]).status,
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
    let answer = get_url(&format!("{files}/{longer}.{mac}/looks/tests/home.png"), &[]);
    assert_eq!(answer.status, 404, "{}", answer.text());
    let mut mac = mac.to_owned();
    let last = mac.pop().unwrap();
    mac.push(if last == 'A' { 'B' } else { 'A' });
    let answer = get_url(
        &format!("{files}/{payload}.{mac}/looks/tests/home.png"),
        &[],
    );
    assert_eq!(answer.status, 404, "{}", answer.text());
}

fn the_files_origin_refuses_a_grant_for_another_run() {
    let mine = run_with_file("feat/mine", "looks/tests/home.png", b"mine");
    let job = Job::push("owner/secret", "feat/theirs");
    let theirs = job.open(&["tests"]);
    job.finished(
        &theirs.id,
        &entry("tests", "passed"),
        &[("looks/tests/home.png", b"theirs")],
    );
    job.send(b"theirs");
    let base = files_base("owner/repo", &mine, &[]);
    assert_eq!(
        get_url(&format!("{base}looks/tests/home.png"), &[]).body,
        b"mine"
    );
    // A grant naming a run of another repository than its own opens nothing.
    let crossed = files_base("owner/repo", &theirs.id, &[]);
    let answer = get_url(&format!("{crossed}looks/tests/home.png"), &[]);
    assert_eq!(answer.status, 404, "{}", answer.text());
    let unknown = files_base("owner/repo", "0123456789abcdef0123456789abcdef", &[]);
    assert_eq!(
        get_url(&format!("{unknown}looks/tests/home.png"), &[]).status,
        404
    );
}

fn the_files_origin_ignores_cookies_and_sets_none() {
    let run = run_with_file("feat/cookies", "looks/tests/home.png", b"cookies");
    let session = sign_in("reader-code", "/");
    let base = files_base("owner/repo", &run, &[]);
    for headers in [vec![("Cookie", session.as_str())], vec![]] {
        let answer = get_url(&format!("{base}looks/tests/home.png"), &headers);
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

fn the_files_origin_serves_only_pictures_and_films() {
    let job = Job::push("owner/repo", "feat/types");
    let run = job.open(&["screens"]);
    let served: &[(&str, &[u8], &str)] = &[
        ("looks/s/a.png", b"types: a png", "image/png"),
        ("looks/s/b.JPG", b"types: a jpg", "image/jpeg"),
        ("looks/s/c.jpeg", b"types: a jpeg", "image/jpeg"),
        ("looks/s/d.webp", b"types: a webp", "image/webp"),
        ("looks/s/e.gif", b"types: a gif", "image/gif"),
        ("looks/s/f.avif", b"types: an avif", "image/avif"),
        ("looks/s/g.mp4", b"types: an mp4", "video/mp4"),
        ("looks/s/h.webm", b"types: a webm", "video/webm"),
    ];
    // What CI wrote is content of a pull request: anything a browser could run, or guess at,
    // is never served.
    let refused: &[(&str, &[u8])] = &[
        ("looks/s/evil.svg", b"<svg onload=alert(1)></svg>"),
        ("looks/s/evil.html", b"<script>alert(1)</script> html"),
        ("looks/s/evil.htm", b"<script>alert(1)</script> htm"),
        ("looks/s/evil.xml", b"<x/> xml"),
        ("outputs/screens.log", b"types: a log"),
        ("looks/s/clip.mov", b"types: a mov"),
        ("looks/s/blob.bin", b"types: a bin"),
        ("looks/s/noextension", b"types: none"),
    ];
    let mut files: Vec<(&str, &[u8])> = served.iter().map(|(p, b, _)| (*p, *b)).collect();
    files.extend_from_slice(refused);
    job.finished(&run.id, &entry("screens", "passed"), &files);
    for (_, bytes) in &files {
        assert_eq!(job.send(bytes).status, 204);
    }
    let base = files_base("owner/repo", &run.id, &[]);
    for (path, bytes, kind) in served {
        let answer = get_url(&format!("{base}{path}"), &[]);
        assert_eq!(answer.status, 200, "{path}: {}", answer.text());
        assert_eq!(answer.body, *bytes, "{path}");
        assert_eq!(answer.header("content-type").unwrap(), *kind, "{path}");
        assert_eq!(
            answer.header("x-content-type-options").unwrap(),
            "nosniff",
            "{path}"
        );
    }
    for (path, _) in refused {
        let answer = get_url(&format!("{base}{path}"), &[]);
        assert_eq!(answer.status, 404, "{path}: {}", answer.text());
        assert!(answer.body.len() < 40, "{path}: {}", answer.text());
    }
}

fn the_page_gets_a_grant_for_the_files_of_a_run_it_shows() {
    let still: &[u8] = b"a still of a public work";
    let mine = run_with_file("feat/granted", "looks/s/home.png", still);
    let answer = get(&format!("/api/v2/runs/{mine}/grant"), &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.header("cache-control").unwrap(), "no-store");
    let grant = answer.json();
    let files = grant["files"].as_str().unwrap();
    assert!(
        files.starts_with(&format!("{}/", world().worker.files)) && files.ends_with('/'),
        "{grant}"
    );
    let expires = grant["expires"].as_i64().unwrap();
    assert!(
        (now() + 3000..=now() + 3700).contains(&expires),
        "an hour: {grant}"
    );
    let answer = get_url(&format!("{files}looks/s/home.png"), &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.body, still);

    // The grant opens its own run only.
    let other: &[u8] = b"a still of another run";
    let theirs = run_with_file("feat/granted-other", "looks/s/home.png", other);
    let answer = get_url(&format!("{files}looks/s/home.png"), &[]);
    assert_eq!(answer.body, still);
    let answer = get(&format!("/api/v2/runs/{theirs}/grant"), &[]);
    let files = answer.json()["files"].as_str().unwrap().to_owned();
    assert_eq!(
        get_url(&format!("{files}looks/s/home.png"), &[]).body,
        other
    );

    // A private work's grant goes only to someone who may read it.
    let job = Job::push("owner/secret", "feat/granted");
    let secret = job.open(&["screens"]);
    let path = format!("/api/v2/runs/{}/grant", secret.id);
    assert_eq!(get(&path, &[]).status, 401);
    let stranger = sign_in("stranger-code", "/");
    assert_eq!(get(&path, &[("Cookie", &stranger)]).status, 404);
    let reader = sign_in("reader-code", "/");
    let answer = get(&path, &[("Cookie", &reader)]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(
        get("/api/v2/runs/0123456789abcdef0123456789abcdef/grant", &[]).status,
        404
    );
}

// Looks.

fn a_finished_check_brings_its_look_to_the_page() {
    let job = Job::push("owner/repo", "feat/look");
    let run = job.open(&["fmt", "screens"]);
    let before: &[u8] = b"look: the before";
    let after: &[u8] = b"look: the after";
    let look = look_of(
        "screens",
        &[still(
            "home.png",
            "changed",
            Some("looks/screens/before/home.png"),
            Some("looks/screens/after/home.png"),
        )],
    );
    let answer = job.finished_with_look(
        &run.id,
        &entry("screens", "passed"),
        &[
            ("looks/screens/before/home.png", before),
            ("looks/screens/after/home.png", after),
        ],
        &look,
    );
    assert_eq!(answer.status, 200, "{}", answer.text());
    job.send(before);
    job.send(after);
    let page = "/owner/repo/work/feat/look";
    let snapshot = live(page, &[]).json();
    let checks = &snapshot["runs"][0]["checks"];
    assert!(checks[0].get("look").is_none(), "{snapshot:#}");
    assert_eq!(
        checks[1]["look"]["items"][0]["name"], "home.png",
        "{snapshot:#}"
    );

    // Done's looks are the final word.
    let mut last = look.clone();
    last["items"][0]["flaky"] = json!(true);
    let answer = job.close(
        &run.id,
        &[entry("fmt", "passed"), entry("screens", "passed")],
        &[last],
        &[
            ("looks/screens/before/home.png", before),
            ("looks/screens/after/home.png", after),
        ],
    );
    assert_eq!(answer.status, 200, "{}", answer.text());
    let snapshot = live(page, &[]).json();
    let look = &snapshot["runs"][0]["checks"][1]["look"];
    assert_eq!(look["items"][0]["flaky"], true, "{snapshot:#}");
}

fn a_look_of_another_check_is_refused() {
    let job = Job::push("owner/repo", "feat/look-other");
    let run = job.open(&["screens"]);
    let answer = job.finished_with_look(
        &run.id,
        &entry("screens", "passed"),
        &[],
        &look_of("tests", &[]),
    );
    assert_eq!(answer.status, 400, "{}", answer.text());
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

// Pictures on the page, in a browser.

/// A look of check `screens`: two changed stills of a card, a new one, a flaky one, one that
/// stayed the same, and a changed film, with the files they name.
struct Shots {
    look: Value,
    files: Vec<(String, Vec<u8>)>,
}

static SHOTS: OnceLock<Shots> = OnceLock::new();

fn shots() -> &'static Shots {
    SHOTS.get_or_init(|| {
        let green = "0x2f9e6e";
        let blue = "0x3366d6";
        let mut files = Vec::new();
        let mut file = |path: &str, bytes: Vec<u8>| -> Value {
            files.push((format!("looks/screens/{path}"), bytes));
            json!(format!("looks/screens/{path}"))
        };
        let changed_still = |area: [u32; 4]| {
            json!({
                "verdict": "changed", "still": true,
                "size_before": [640, 400], "size_after": [640, 400],
                "total_before": 0.0, "total_after": 0.0, "max_shift": 0.0, "max_shift_at": null,
                "states_before": 1, "states_after": 1,
                "moments": [{"before": [0.0, 0.0], "after": [0.0, 0.0], "box": area, "px": 4000}],
                "pairs": [],
            })
        };
        let mut items = Vec::new();
        for theme in ["light", "dark"] {
            let name = format!("card/{theme}.png");
            items.push(json!({
                "name": name, "kind": "still", "change": "changed",
                "summary": "4000 pixels differ",
                "before": file(&format!("before/{name}"), media::still(&media::card(160, green))),
                "after": file(&format!("after/{name}"), media::still(&media::card(96, green))),
                "comparison": changed_still([72, 280, 232, 328]),
            }));
        }
        items.push(json!({
            "name": "settings/light.png", "kind": "still", "change": "new", "summary": "new",
            "before": null,
            "after": file("after/settings/light.png", media::still(&media::card(200, blue))),
            "comparison": null,
        }));
        items.push(json!({
            "name": "menu/light.png", "kind": "still", "change": "changed", "flaky": true,
            "summary": "a clock in the corner",
            "before": file("before/menu/light.png", media::still(&media::card(120, blue))),
            "after": file("after/menu/light.png", media::still(&media::card(124, blue))),
            "comparison": changed_still([72, 280, 196, 328]),
        }));
        let about = media::still(&media::card(140, green));
        items.push(json!({
            "name": "about/light.png", "kind": "still", "change": "same", "summary": "same",
            "before": file("before/about/light.png", about.clone()),
            "after": file("after/about/light.png", about),
            "comparison": null,
        }));
        let area = [72, 280, 292, 328];
        let pair = |state: usize, before_at: f64, after_at: f64, px: usize| {
            let boxes = if px > 0 { json!([area]) } else { json!([]) };
            json!({
                "before": state, "after": state, "before_frame": 0, "after_frame": 0,
                "before_at": before_at, "after_at": after_at, "px": px, "boxes": boxes,
            })
        };
        items.push(json!({
            "name": "checkout.webm", "kind": "film", "change": "changed",
            "summary": "changes at 1.00 s",
            "before": file("before/checkout.webm", media::film(&[
                (1.0, media::card(160, green)),
                (1.0, media::card(160, blue)),
                (1.0, media::card(160, green)),
            ])),
            "after": file("after/checkout.webm", media::film(&[
                (1.0, media::card(160, green)),
                (1.4, media::card(220, blue)),
                (1.0, media::card(220, green)),
            ])),
            "comparison": {
                "verdict": "changed", "still": false,
                "size_before": [640, 400], "size_after": [640, 400],
                "total_before": 3.0, "total_after": 3.4, "max_shift": 0.4, "max_shift_at": 2.0,
                "states_before": 3, "states_after": 3,
                "moments": [{"before": [1.0, 3.0], "after": [1.0, 3.4], "box": area, "px": 2880}],
                "pairs": [pair(0, 0.0, 0.0, 0), pair(1, 1.0, 1.0, 2880), pair(2, 2.0, 2.4, 2880)],
            },
        }));
        Shots {
            look: look_of("screens", &items),
            files,
        }
    })
}

/// A run of `branch` whose `screens` check ended with the look of `shots`, all its files sent.
fn run_with_pictures(job: &Job, run: &str) {
    let shots = shots();
    let files: Vec<(&str, &[u8])> = shots
        .files
        .iter()
        .map(|(p, b)| (p.as_str(), b.as_slice()))
        .collect();
    let answer = job.finished_with_look(run, &entry("screens", "passed"), &files, &shots.look);
    assert_eq!(answer.status, 200, "{}", answer.text());
    for (_, bytes) in &files {
        job.send(bytes);
    }
}

/// Opens the page of a work whose run shows the pictures of `shots`.
fn page_with_pictures(branch: &str) -> browser::Browser {
    let job = Job::push("owner/repo", branch);
    let run = job.open(&["screens"]);
    run_with_pictures(&job, &run.id);
    let mut browser = browser::Browser::start();
    browser.open(&format!("{}/owner/repo/work/{branch}", world().worker.url));
    browser.wait_for("!!document.querySelector('[data-stage]')");
    browser
}

/// Whether the stage shows `side` of the current picture, loaded from the files origin.
fn loaded(tag: &str, side: &str) -> String {
    let files = &world().worker.files;
    let ready = if tag == "img" {
        "m.complete && m.naturalWidth > 0"
    } else {
        "m.readyState >= 1"
    };
    format!(
        r#"[...document.querySelectorAll('[data-stage] {tag}[data-side="{side}"]')].some(m => m.src.startsWith("{files}/") && {ready})"#
    )
}

fn current() -> &'static str {
    "document.querySelector('[data-stage]').dataset.key"
}

fn the_page_shows_a_changed_picture_with_its_before_and_after() {
    let mut browser = page_with_pictures("feat/pictures");
    // The first changed picture is open, its two sides loaded from the files origin.
    assert_eq!(browser.eval(current()), json!("screens/card/light.png"));
    browser.wait_for(&loaded("img", "before"));
    browser.wait_for(&loaded("img", "after"));
    // Its outline of where it differs sits on the picture.
    assert_eq!(
        browser.eval("document.querySelectorAll('[data-stage] .mark').length > 0"),
        json!(true)
    );
    // A new picture has only its after.
    browser
        .eval(r#"document.querySelector('[data-picture="screens/settings/light.png"]').click()"#);
    browser.wait_for(&loaded("img", "after"));
    assert_eq!(
        browser.eval(r#"document.querySelectorAll('[data-stage] [data-side="before"]').length"#),
        json!(0)
    );
    // The counts say what there is, and one filters to its kind.
    assert_eq!(
        browser.eval(r#"document.querySelector('[data-filter="changed"] b').textContent"#),
        json!("4")
    );
    browser.eval(r#"document.querySelector('[data-filter="new"]').click()"#);
    browser.wait_for("document.querySelectorAll('[data-picture]').length === 1");
    // What stayed the same is there, folded.
    browser.eval(r#"document.querySelector('[data-filter="new"]').click()"#);
    browser.wait_for(r#"!!document.querySelector('[data-group-key="same"]')"#);
    assert_eq!(
        browser.eval(r#"document.querySelector('[data-group-key="same"]').open"#),
        json!(false)
    );
}

fn the_compare_modes_switch_by_click_and_by_key() {
    let mut browser = page_with_pictures("feat/modes");
    let mode = "document.querySelector('[data-stage]').dataset.mode";
    assert_eq!(browser.eval(mode), json!("side"));
    browser.eval(r#"document.querySelector('button[data-mode="slider"]').click()"#);
    browser.wait_for(&format!(r#"{mode} === "slider""#));
    assert_eq!(
        browser.eval(
            r#"document.querySelector('button[data-mode="slider"]').getAttribute('aria-pressed')"#
        ),
        json!("true")
    );
    for (key, want) in [("3", "onion"), ("4", "diff"), ("5", "blink"), ("1", "side")] {
        browser.press(key);
        browser.wait_for(&format!(r#"{mode} === "{want}""#));
    }
    // j and k step between pictures, in the order shown.
    browser.press("j");
    browser.wait_for(&format!(r#"{} === "screens/card/dark.png""#, current()));
    browser.press("k");
    browser.wait_for(&format!(r#"{} === "screens/card/light.png""#, current()));
    // The mode stays as the person moves on.
    browser.press("2");
    browser.press("j");
    browser.wait_for(&format!(r#"{} === "screens/card/dark.png""#, current()));
    assert_eq!(browser.eval(mode), json!("slider"));
}

fn a_film_shows_its_lanes_on_one_clock() {
    let mut browser = page_with_pictures("feat/film");
    browser.eval(r#"document.querySelector('[data-picture="screens/checkout.webm"]').click()"#);
    browser.wait_for(&loaded("video", "before"));
    browser.wait_for(&loaded("video", "after"));
    let lane = |side: &str| {
        format!(
            r#"[...document.querySelectorAll('[data-lane="{side}"] i')].map(i => i.dataset.differs)"#
        )
    };
    // Three states each: the first the same, the other two differing.
    assert_eq!(
        browser.eval(&lane("before")),
        json!(["false", "true", "true"])
    );
    assert_eq!(
        browser.eval(&lane("after")),
        json!(["false", "true", "true"])
    );
    // A jump to where they differ moves both films there.
    browser.eval(r#"document.querySelector('[data-seek]').click()"#);
    browser.wait_for(
        r#"[...document.querySelectorAll('[data-stage] video')].every(v => Math.abs(v.currentTime - 1) < 0.05)"#,
    );
    assert_eq!(
        browser.eval("document.querySelector('[data-clock]').textContent.trim()"),
        json!("1.00 s")
    );
}

fn a_flaky_picture_is_shown_and_marked() {
    let mut browser = page_with_pictures("feat/flaky");
    let flaky = r#"document.querySelector('[data-picture="screens/menu/light.png"]')"#;
    assert_eq!(
        browser.eval(&format!("{flaky}.dataset.flaky")),
        json!("true")
    );
    // After the other changed pictures.
    assert_eq!(
        browser.eval(
            r#"[...document.querySelectorAll('[data-group-key="changed"] [data-picture]')].map(b => b.dataset.picture)"#
        ),
        json!([
            "screens/card/light.png",
            "screens/card/dark.png",
            "screens/checkout.webm",
            "screens/menu/light.png"
        ])
    );
    browser.eval(&format!("{flaky}.click()"));
    browser.wait_for(r#"document.querySelector('[data-stage]').dataset.flaky === "true""#);
    assert_eq!(
        browser.eval("document.querySelector('[data-stage] [data-flaky-note]') !== null"),
        json!(true)
    );
}

fn a_look_that_arrives_shows_without_a_reload() {
    let job = Job::push("owner/repo", "feat/arrives");
    let run = job.open(&["fmt", "screens"]);
    let mut browser = browser::Browser::start();
    browser.open(&format!(
        "{}/owner/repo/work/feat/arrives",
        world().worker.url
    ));
    browser.wait_for(&shows("screens", "planned"));
    browser.eval("window.stayed = true");
    run_with_pictures(&job, &run.id);
    browser.wait_for(r#"!!document.querySelector('[data-picture="screens/card/light.png"]')"#);
    browser.wait_for(&loaded("img", "after"));
    assert_eq!(
        browser.eval("window.stayed === true"),
        json!(true),
        "no reload"
    );
}

fn a_look_shows_only_once_its_pictures_have_arrived() {
    // CI sends a check's end with its look, then the pictures the host lacks. Until the last
    // one is in, a page could only show them broken, so the look waits for them.
    let job = Job::push("owner/repo", "feat/waits");
    let run = job.open(&["screens"]);
    let page = "/owner/repo/work/feat/waits";
    let mut browser = browser::Browser::start();
    browser.open(&format!("{}{page}", world().worker.url));
    browser.wait_for(&shows("screens", "planned"));
    browser.eval("window.stayed = true");
    // Bytes no other test sends, since the host keeps files per repository.
    let before = media::still(&media::card(150, "0x8a4fbf"));
    let after = media::still(&media::card(190, "0x8a4fbf"));
    let files: Vec<(&str, &[u8])> = vec![
        ("looks/screens/before/card/light.png", &before),
        ("looks/screens/after/card/light.png", &after),
    ];
    let look = look_of(
        "screens",
        &[still(
            "card/light.png",
            "changed",
            Some(files[0].0),
            Some(files[1].0),
        )],
    );
    let answer = job.finished_with_look(&run.id, &entry("screens", "passed"), &files, &look);
    assert_eq!(answer.status, 200, "{}", answer.text());
    browser.wait_for(&shows("screens", "passed"));
    let pictures = "document.querySelectorAll('[data-picture]').length";
    assert_eq!(
        browser.eval(pictures),
        json!(0),
        "no picture before its files"
    );
    let snapshot = live(page, &[]).json();
    assert!(
        snapshot["runs"][0]["checks"][0].get("look").is_none(),
        "{snapshot:#}"
    );
    // Every file but the last: still nothing.
    let (last, rest) = files.split_last().unwrap();
    for (_, bytes) in rest {
        assert_eq!(job.send(bytes).status, 204);
    }
    assert!(
        live(page, &[]).json()["runs"][0]["checks"][0]
            .get("look")
            .is_none()
    );
    assert_eq!(browser.eval(pictures), json!(0));
    // The last one shows the look, live, every picture loaded.
    assert_eq!(job.send(last.1).status, 204);
    browser.wait_for(r#"!!document.querySelector('[data-picture="screens/card/light.png"]')"#);
    browser.wait_for(&loaded("img", "before"));
    browser.wait_for(&loaded("img", "after"));
    assert_eq!(
        browser.eval("window.stayed === true"),
        json!(true),
        "no reload"
    );
    let snapshot = live(page, &[]).json();
    assert_eq!(
        snapshot["runs"][0]["checks"][0]["look"]["check"], "screens",
        "{snapshot:#}"
    );
}

fn pictures_group_by_the_parts_of_their_names() {
    let mut browser = page_with_pictures("feat/groups");
    let groups =
        "[...document.querySelectorAll('[data-group-label]')].map(g => g.dataset.groupLabel)";
    assert_eq!(browser.eval(groups), json!(["Changed", "New", "Unchanged"]));
    browser.eval(r#"document.querySelector('button[data-group="theme"]').click()"#);
    browser.wait_for(&format!(r#"{groups}[0] === "light""#));
    assert_eq!(
        browser.eval(groups),
        json!(["light", "dark", "Other", "Unchanged"])
    );
    browser.eval(r#"document.querySelector('button[data-group="screen"]').click()"#);
    browser.wait_for(&format!(r#"{groups}[0] === "card""#));
    assert_eq!(
        browser.eval(groups),
        json!(["card", "checkout", "menu", "settings", "Unchanged"])
    );
}

// Review: verdicts and accepts on the page, the check run and comment the app writes, and
// GitHub's webhooks. Every review test works on `owner/review`, where `writer` has write
// access and `admin` admin rights, each on its own pull request.

/// A commit of forty hex digits, from a short name.
fn commit(name: &str) -> String {
    format!("{name:0<40}")
}

/// The look of check `screens` with one changed still, and its files, by their bytes.
fn one_still(before: &[u8], after: &[u8]) -> (Value, Vec<(&'static str, Vec<u8>)>) {
    let files = vec![
        ("looks/screens/before/card/light.png", before.to_vec()),
        ("looks/screens/after/card/light.png", after.to_vec()),
    ];
    let look = look_of(
        "screens",
        &[still(
            "card/light.png",
            "changed",
            Some(files[0].0),
            Some(files[1].0),
        )],
    );
    (look, files)
}

/// A run of `job` whose check `screens` ends with `look` and its files, every file sent, and
/// then done with `extra` over its proof.
fn proven(job: &Job, look: &Value, files: &[(&str, Vec<u8>)], extra: &Value) -> String {
    let run = job.open(&["screens"]);
    let files: Vec<(&str, &[u8])> = files.iter().map(|(p, b)| (*p, b.as_slice())).collect();
    let answer = job.finished_with_look(&run.id, &entry("screens", "passed"), &files, look);
    assert_eq!(answer.status, 200, "{}", answer.text());
    for (_, bytes) in &files {
        job.send(bytes);
    }
    let answer = job.close_with(
        &run.id,
        &[entry("screens", "passed")],
        std::slice::from_ref(look),
        &files,
        extra,
    );
    assert_eq!(answer.status, 200, "{}", answer.text());
    run.id
}

/// The work's review as its page gets it.
fn review_of(pull: u64) -> Value {
    live(&format!("/owner/review/pull/{pull}"), &[]).json()["review"].clone()
}

/// What the page sends to say something about run `run` (`verdicts` or `accept`): JSON from
/// the app origin with the session `cookie`, and `headers` besides or instead.
fn say(
    run: &str,
    what: &str,
    cookie: Option<&str>,
    headers: &[(&str, &str)],
    body: &Value,
) -> Answer {
    let origin = world().worker.url.clone();
    let mut all: Vec<(&str, &str)> =
        vec![("Origin", &origin), ("Content-Type", "application/json")];
    for (name, value) in headers {
        all.retain(|(n, _)| !n.eq_ignore_ascii_case(name));
        all.push((name, value));
    }
    if let Some(cookie) = cookie {
        all.push(("Cookie", cookie));
    }
    let path = format!("/api/v2/runs/{run}/{what}");
    request("POST", &path, &all, Some(body.to_string().into_bytes()))
}

fn looks_right(picture: &str) -> Value {
    json!({"check": "screens", "picture": picture, "kind": "looks_right"})
}

/// A weakening that waits for an admin's accept, as a hosted CI run closes with it.
fn weakened() -> Value {
    json!({
        "weakened": [{"check": "tests", "how": "it runs another command"}],
        "weakenings_wait": true,
    })
}

/// Sends a GitHub webhook of `event`, signed with the app's webhook secret.
fn webhook(event: &str, payload: &Value) -> Answer {
    let body = payload.to_string().into_bytes();
    let signature = github::signature(WEBHOOK_SECRET, &body);
    let headers = [
        ("X-GitHub-Event", event),
        ("X-Hub-Signature-256", signature.as_str()),
        ("Content-Type", "application/json"),
    ];
    request("POST", "/api/github/webhook", &headers, Some(body))
}

/// The `pull_request` webhook of `action` for pull request `number` of `owner/review` at `head`.
fn pull_request(action: &str, number: u64, branch: &str, head: &str) -> Value {
    json!({
        "action": action,
        "number": number,
        "pull_request": {
            "number": number,
            "title": format!("Pull request {number}"),
            "html_url": format!("https://github.com/owner/review/pull/{number}"),
            "head": {"sha": head, "ref": branch, "repo": {"full_name": "owner/review"}},
            "base": {"ref": "main", "repo": {"full_name": "owner/review"}},
        },
        "repository": {"full_name": "owner/review"},
        "installation": {"id": 6},
    })
}

/// The `workflow_job` webhook of `action` for job `name` of workflow run `run`.
fn workflow_job(action: &str, run: u64, name: &str, branch: &str, head: &str) -> Value {
    json!({
        "action": action,
        "workflow_job": {
            "id": run * 10,
            "run_id": run,
            "name": name,
            "head_sha": head,
            "head_branch": branch,
            "status": action,
            "conclusion": if action == "completed" { json!("cancelled") } else { Value::Null },
            "html_url": format!("https://github.com/owner/review/actions/runs/{run}/job/{}", run * 10),
        },
        "repository": {"full_name": "owner/review"},
        "installation": {"id": 6},
    })
}

/// The latest check run the app wrote for `head` of `owner/review`.
fn check_run(head: &str) -> github::CheckRun {
    let runs = world().github.check_runs("owner/review", head);
    runs.last()
        .cloned()
        .unwrap_or_else(|| panic!("no check run for {head}"))
}

fn a_verdict_needs_a_session_write_access_the_app_origin_and_json() {
    let job = Job::pull("owner/review", 101, "review/guarded").at(&commit("a101"), 1101);
    let (look, files) = one_still(b"guarded before", b"guarded after");
    let run = proven(&job, &look, &files, &json!({}));
    let body = looks_right("card/light.png");
    let writer = sign_in("writer-code", "/");
    let reader = sign_in("reader-code", "/");

    let answer = say(&run, "verdicts", None, &[], &body);
    assert_eq!(answer.status, 401, "without a session: {}", answer.text());
    let answer = say(
        &run,
        "verdicts",
        Some(&writer),
        &[("Origin", "https://elsewhere.test")],
        &body,
    );
    assert_eq!(answer.status, 403, "from another site: {}", answer.text());
    let answer = say(
        &run,
        "verdicts",
        Some(&writer),
        &[("Origin", "null")],
        &body,
    );
    assert_eq!(answer.status, 403, "from no site: {}", answer.text());
    let form = [("Content-Type", "application/x-www-form-urlencoded")];
    let answer = say(&run, "verdicts", Some(&writer), &form, &body);
    assert_eq!(answer.status, 415, "a form: {}", answer.text());
    let answer = say(&run, "verdicts", Some(&reader), &[], &body);
    assert_eq!(answer.status, 403, "a reader: {}", answer.text());
    assert!(answer.text().contains("write access"), "{}", answer.text());
    let answer = say(
        &run,
        "verdicts",
        Some(&writer),
        &[],
        &looks_right("nothing.png"),
    );
    assert_eq!(
        answer.status,
        409,
        "a picture the run lacks: {}",
        answer.text()
    );
    assert_eq!(review_of(101)["state"], "waits");

    let answer = say(&run, "verdicts", Some(&writer), &[], &body);
    assert_eq!(answer.status, 201, "{}", answer.text());
    let review = review_of(101);
    assert_eq!(review["state"], "approved", "{review:#}");
    let verdict = &review["pictures"][0]["verdict"];
    assert_eq!(verdict["by"], "writer", "{review:#}");
    assert_eq!(verdict["kind"], "looks_right", "{review:#}");
    assert_eq!(verdict["run"], run.as_str(), "{review:#}");
}

fn a_bearer_token_never_gives_a_verdict_or_an_accept() {
    // A token is how programs speak to the host (CI's OIDC token now, the CLI's own later);
    // only a person in a browser gives a verdict or an accept.
    let job = Job::pull("owner/review", 102, "review/bearer").at(&commit("a102"), 1102);
    let (look, files) = one_still(b"bearer before", b"bearer after");
    let run = proven(&job, &look, &files, &weakened());
    let auth = format!("Bearer {}", job.token());
    let bearer = [("Authorization", auth.as_str())];
    let writer = sign_in("writer-code", "/");
    let admin = sign_in("admin-code", "/");
    let body = looks_right("card/light.png");
    for cookie in [None, Some(writer.as_str())] {
        let answer = say(&run, "verdicts", cookie, &bearer, &body);
        assert_eq!(answer.status, 403, "{}", answer.text());
    }
    let answer = say(&run, "accept", Some(&admin), &bearer, &json!({}));
    assert_eq!(answer.status, 403, "{}", answer.text());
    let review = review_of(102);
    assert!(review["pictures"][0]["verdict"].is_null(), "{review:#}");
    assert!(review["accept"].is_null(), "{review:#}");
}

fn a_verdict_on_a_local_run_is_refused() {
    let job = Job::pull("owner/review", 103, "review/local").at(&commit("a103"), 1103);
    let (look, files) = one_still(b"local before", b"local after");
    let run = proven(&job, &look, &files, &json!({}));
    // No call opens a local run yet; its record is what one will look like.
    world().worker.sql(&format!(
        r#"UPDATE runs SET source = '{{"kind":"local"}}' WHERE id = '{run}'"#
    ));
    let writer = sign_in("writer-code", "/");
    let answer = say(
        &run,
        "verdicts",
        Some(&writer),
        &[],
        &looks_right("card/light.png"),
    );
    assert_eq!(answer.status, 409, "{}", answer.text());
    assert!(answer.text().contains("local run"), "{}", answer.text());
}

fn a_verdict_carries_over_to_a_later_run_with_the_same_pictures() {
    let first = Job::pull("owner/review", 104, "review/carries").at(&commit("a104"), 1104);
    let (look, files) = one_still(b"carries before", b"carries after");
    let run = proven(&first, &look, &files, &json!({}));
    let writer = sign_in("writer-code", "/");
    let answer = say(
        &run,
        "verdicts",
        Some(&writer),
        &[],
        &looks_right("card/light.png"),
    );
    assert_eq!(answer.status, 201, "{}", answer.text());

    // A later push whose picture is the same keeps the verdict, and says where it came from.
    let second = first.at(&commit("b104"), 2104);
    let later = proven(&second, &look, &files, &json!({}));
    let review = review_of(104);
    assert_eq!(review["head"], commit("b104"), "{review:#}");
    assert_eq!(review["state"], "approved", "{review:#}");
    let verdict = &review["pictures"][0]["verdict"];
    assert_eq!(verdict["head"], commit("a104"), "{review:#}");
    assert_eq!(verdict["run"], run.as_str(), "{review:#}");
    assert_ne!(verdict["run"], later.as_str());

    // Another after loses it.
    let third = first.at(&commit("c104"), 3104);
    let (changed, changed_files) = one_still(b"carries before", b"carries after, changed");
    proven(&third, &changed, &changed_files, &json!({}));
    let review = review_of(104);
    assert_eq!(review["state"], "waits", "{review:#}");
    assert!(review["pictures"][0]["verdict"].is_null(), "{review:#}");
}

/// A changed film whose states named in `changed` differ, recorded as `bytes`.
fn film_look(bytes: &[u8], changed: &[usize]) -> (Value, Vec<(&'static str, Vec<u8>)>) {
    let files = vec![
        (
            "looks/screens/before/checkout.webm",
            b"film before".to_vec(),
        ),
        ("looks/screens/after/checkout.webm", bytes.to_vec()),
    ];
    let pairs: Vec<Value> = (0..3)
        .map(|state| {
            let px = if changed.contains(&state) { 900 } else { 0 };
            json!({
                "before": state, "after": state, "before_frame": 0, "after_frame": 0,
                "before_at": state as f64, "after_at": state as f64, "px": px, "boxes": [],
            })
        })
        .collect();
    let item = json!({
        "name": "checkout.webm", "kind": "film", "change": "changed", "summary": "changes",
        "before": files[0].0, "after": files[1].0,
        "comparison": {
            "verdict": "changed", "still": false,
            "size_before": [640, 400], "size_after": [640, 400],
            "total_before": 3.0, "total_after": 3.0, "max_shift": 0.0, "max_shift_at": null,
            "states_before": 3, "states_after": 3, "moments": [], "pairs": pairs,
        },
    });
    (look_of("screens", &[item]), files)
}

fn a_film_verdict_follows_its_changed_states() {
    let first = Job::pull("owner/review", 105, "review/film").at(&commit("a105"), 1105);
    let (look, files) = film_look(b"film after, recording 1", &[1]);
    let run = proven(&first, &look, &files, &json!({}));
    let writer = sign_in("writer-code", "/");
    let answer = say(
        &run,
        "verdicts",
        Some(&writer),
        &[],
        &looks_right("checkout.webm"),
    );
    assert_eq!(answer.status, 201, "{}", answer.text());
    // Films differ in their bytes each time they are recorded; the same change keeps it.
    let (again, again_files) = film_look(b"film after, recording 2", &[1]);
    proven(
        &first.at(&commit("b105"), 2105),
        &again,
        &again_files,
        &json!({}),
    );
    assert_eq!(review_of(105)["state"], "approved");
    // Another state that changed is another change.
    let (other, other_files) = film_look(b"film after, recording 3", &[1, 2]);
    proven(
        &first.at(&commit("c105"), 3105),
        &other,
        &other_files,
        &json!({}),
    );
    assert_eq!(review_of(105)["state"], "waits");
}

fn accept_needs_an_admin() {
    let job = Job::pull("owner/review", 106, "review/accept").at(&commit("a106"), 1106);
    let (look, files) = one_still(b"accept before", b"accept before");
    let mut look = look;
    look["items"][0]["change"] = json!("same");
    let run = proven(&job, &look, &files, &weakened());
    let review = review_of(106);
    assert_eq!(review["state"], "waits", "{review:#}");
    assert_eq!(review["weakened"][0]["check"], "tests", "{review:#}");

    let writer = sign_in("writer-code", "/");
    let answer = say(&run, "accept", Some(&writer), &[], &json!({}));
    assert_eq!(answer.status, 403, "{}", answer.text());
    assert!(answer.text().contains("admin"), "{}", answer.text());
    let admin = sign_in("admin-code", "/");
    let answer = say(&run, "accept", Some(&admin), &[], &json!({}));
    assert_eq!(answer.status, 201, "{}", answer.text());
    let review = review_of(106);
    assert_eq!(review["state"], "approved", "{review:#}");
    assert_eq!(review["accept"]["by"], "admin", "{review:#}");

    // A run that weakens nothing has nothing to accept.
    let plain = job.at(&commit("b106"), 2106);
    let other = proven(&plain, &look, &files, &json!({}));
    let answer = say(&other, "accept", Some(&admin), &[], &json!({}));
    assert_eq!(answer.status, 409, "{}", answer.text());
}

fn the_check_run_follows_the_review_through_a_whole_flow() {
    let page = format!("{}/owner/review/pull/107", world().worker.url);
    let first = commit("a107");
    // A push to the pull request: its head waits for CI.
    let answer = webhook(
        "pull_request",
        &pull_request("synchronize", 107, "review/flow", &first),
    );
    assert_eq!(answer.status, 202, "{}", answer.text());
    let check = check_run(&first);
    assert_eq!(check.field("name"), "Fearlezz review");
    assert_eq!(check.state(), "queued");
    assert_eq!(check.field("details_url"), page.as_str());
    assert!(
        check.field("output")["title"]
            .as_str()
            .unwrap()
            .contains("Waiting for CI"),
        "{:?}",
        check.writes
    );

    // CI opens its run: in progress.
    let job = Job::pull("owner/review", 107, "review/flow").at(&first, 1107);
    let run = job.open(&["screens"]);
    assert_eq!(check_run(&first).state(), "in_progress");

    // Done, with a changed picture and a weakening: both wait for a person.
    let (look, files) = one_still(b"flow before", b"flow after");
    let listed: Vec<(&str, &[u8])> = files.iter().map(|(p, b)| (*p, b.as_slice())).collect();
    job.finished_with_look(&run.id, &entry("screens", "passed"), &listed, &look);
    for (_, bytes) in &listed {
        job.send(bytes);
    }
    let answer = job.close_with(
        &run.id,
        &[entry("screens", "passed")],
        &[look],
        &listed,
        &weakened(),
    );
    assert_eq!(answer.status, 200, "{}", answer.text());
    let check = check_run(&first);
    assert_eq!(check.state(), "action_required", "{:?}", check.writes);
    let summary = check.field("output")["summary"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(summary.contains("card/light.png"), "{summary}");
    assert!(summary.contains("check `tests` is weaker"), "{summary}");

    // A writer's verdict leaves the weakening waiting; an admin's accept settles the head.
    let writer = sign_in("writer-code", "/");
    let answer = say(
        &run.id,
        "verdicts",
        Some(&writer),
        &[],
        &looks_right("card/light.png"),
    );
    assert_eq!(answer.status, 201, "{}", answer.text());
    assert_eq!(check_run(&first).state(), "action_required");
    let admin = sign_in("admin-code", "/");
    let answer = say(&run.id, "accept", Some(&admin), &[], &json!({}));
    assert_eq!(answer.status, 201, "{}", answer.text());
    let check = check_run(&first);
    assert_eq!(check.state(), "success", "{:?}", check.writes);
    assert!(
        check.tokens.iter().all(|t| t == "installation-6"),
        "the app writes it: {:?}",
        check.tokens
    );

    // The next push gets a check run of its own, which fails with its proof.
    let second = commit("b107");
    webhook(
        "pull_request",
        &pull_request("synchronize", 107, "review/flow", &second),
    );
    assert_eq!(check_run(&second).state(), "queued");
    let next = job.at(&second, 2107);
    let run = next.open(&["tests"]);
    next.done(&run.id, &[entry("tests", "failed")], &[]);
    assert_eq!(check_run(&second).state(), "failure");
    assert_eq!(
        check_run(&first).state(),
        "success",
        "the first head keeps its own"
    );
}

fn one_comment_is_created_then_edited() {
    let first = commit("a108");
    webhook(
        "pull_request",
        &pull_request("opened", 108, "review/comment", &first),
    );
    assert!(
        world().github.comments("owner/review", 108).is_empty(),
        "no hosted run yet"
    );
    let job = Job::pull("owner/review", 108, "review/comment").at(&first, 1108);
    let run = job.open(&["tests"]);
    let comments = world().github.comments("owner/review", 108);
    assert_eq!(comments.len(), 1, "{comments:?}");
    let page = format!("{}/owner/review/pull/108", world().worker.url);
    assert!(comments[0].bodies[0].contains(&page), "{comments:?}");
    assert!(comments[0].bodies[0].contains("proving"), "{comments:?}");

    job.done(&run.id, &[entry("tests", "passed")], &[]);
    let next = job.at(&commit("b108"), 2108);
    let run = next.open(&["tests"]);
    next.done(&run.id, &[entry("tests", "failed")], &[]);
    let comments = world().github.comments("owner/review", 108);
    assert_eq!(comments.len(), 1, "one comment, edited: {comments:?}");
    let bodies = &comments[0].bodies;
    assert!(bodies.len() >= 3, "{bodies:?}");
    assert!(bodies.last().unwrap().contains("failed"), "{bodies:?}");
    assert!(
        bodies.iter().all(|b| b.lines().count() == 1),
        "one line: {bodies:?}"
    );
    assert!(
        comments[0].tokens.iter().all(|t| t == "installation-6"),
        "as the app: {comments:?}"
    );
}

fn a_webhook_with_a_wrong_signature_is_refused() {
    let body = json!({"zen": "Keep it logically awesome.", "hook_id": 1}).to_string();
    let wrong = github::signature("another secret", body.as_bytes());
    for headers in [
        vec![("X-GitHub-Event", "ping")],
        vec![
            ("X-GitHub-Event", "ping"),
            ("X-Hub-Signature-256", wrong.as_str()),
        ],
        vec![
            ("X-GitHub-Event", "ping"),
            ("X-Hub-Signature-256", "sha256=00"),
        ],
    ] {
        let answer = request(
            "POST",
            "/api/github/webhook",
            &headers,
            Some(body.clone().into_bytes()),
        );
        assert_eq!(answer.status, 401, "{headers:?}: {}", answer.text());
    }
    // A forged push to a pull request writes nothing to GitHub.
    let head = commit("a109");
    let payload = pull_request("synchronize", 109, "review/forged", &head).to_string();
    let wrong = github::signature("another secret", payload.as_bytes());
    let headers = [
        ("X-GitHub-Event", "pull_request"),
        ("X-Hub-Signature-256", wrong.as_str()),
    ];
    let answer = request(
        "POST",
        "/api/github/webhook",
        &headers,
        Some(payload.into_bytes()),
    );
    assert_eq!(answer.status, 401, "{}", answer.text());
    assert!(world().github.check_runs("owner/review", &head).is_empty());
    let answer = webhook(
        "ping",
        &json!({"zen": "Keep it logically awesome.", "hook_id": 1}),
    );
    assert_eq!(answer.status, 202, "{}", answer.text());
}

fn a_job_that_ends_without_done_cancels_its_run() {
    let head = commit("a110");
    let job = Job::pull("owner/review", 110, "review/cancel").at(&head, 1110);
    let run = job.open(&["tests"]);
    job.started(&run.id, "tests");
    // Another job of the same workflow run ending leaves it running.
    let answer = webhook(
        "workflow_job",
        &workflow_job("completed", 1110, "build", "review/cancel", &head),
    );
    assert_eq!(answer.status, 202, "{}", answer.text());
    let snapshot = live("/owner/review/pull/110", &[]).json();
    assert_eq!(snapshot["runs"][0]["state"], "running", "{snapshot:#}");
    // Its own job ending without done closes it as cancelled.
    webhook(
        "workflow_job",
        &workflow_job("completed", 1110, "proof", "review/cancel", &head),
    );
    let snapshot = live("/owner/review/pull/110", &[]).json();
    assert_eq!(snapshot["runs"][0]["state"], "cancelled", "{snapshot:#}");
    assert_eq!(snapshot["review"]["state"], "cancelled", "{snapshot:#}");
    assert_eq!(check_run(&head).state(), "cancelled");
    let answer = job.finished(&run.id, &entry("tests", "passed"), &[]);
    assert_eq!(
        answer.status,
        409,
        "a closed run takes nothing more: {}",
        answer.text()
    );
}

fn a_queued_job_shows_as_waiting_for_a_runner() {
    // The work and its job are known from an earlier run.
    let job = Job::pull("owner/review", 111, "review/queued").at(&commit("a111"), 1111);
    let run = job.open(&["tests"]);
    job.done(&run.id, &[entry("tests", "passed")], &[]);
    let head = commit("b111");
    let page = "/owner/review/pull/111";
    webhook(
        "workflow_job",
        &workflow_job("queued", 2111, "proof", "review/queued", &head),
    );
    // A job that never ran Fearlezz is not shown.
    webhook(
        "workflow_job",
        &workflow_job("queued", 2111, "lint", "review/queued", &head),
    );
    let snapshot = live(page, &[]).json();
    let waiting = snapshot["waiting"].as_array().cloned().unwrap_or_default();
    assert_eq!(waiting.len(), 1, "{snapshot:#}");
    assert_eq!(waiting[0]["name"], "proof", "{snapshot:#}");
    assert_eq!(waiting[0]["head"], head.as_str(), "{snapshot:#}");
    let mut browser = browser::Browser::start();
    browser.open(&format!("{}{page}", world().worker.url));
    browser.wait_for(r#"!!document.querySelector('[data-waiting="proof"]')"#);
    // Once a runner takes it, it no longer waits.
    webhook(
        "workflow_job",
        &workflow_job("in_progress", 2111, "proof", "review/queued", &head),
    );
    browser.wait_for(r#"!document.querySelector('[data-waiting="proof"]')"#);
    let snapshot = live(page, &[]).json();
    assert!(
        snapshot
            .get("waiting")
            .is_none_or(|w| w.as_array().unwrap().is_empty()),
        "{snapshot:#}"
    );
}

/// A pull request of `owner/review` whose run shows the pictures of `shots`, done with `extra`
/// over its proof.
fn review_with_pictures(number: u64, branch: &str, extra: &Value) -> (Job, String) {
    let job = Job::pull("owner/review", number, branch).at(&commit(&format!("a{number}")), number);
    let run = job.open(&["screens"]);
    run_with_pictures(&job, &run.id);
    let shots = shots();
    let files: Vec<(&str, &[u8])> = shots
        .files
        .iter()
        .map(|(p, b)| (p.as_str(), b.as_slice()))
        .collect();
    let answer = job.close_with(
        &run.id,
        &[entry("screens", "passed")],
        std::slice::from_ref(&shots.look),
        &files,
        extra,
    );
    assert_eq!(answer.status, 200, "{}", answer.text());
    (job, run.id)
}

/// Whether the picture on the stage shows the verdict `state`.
fn verdict_shows(state: &str) -> String {
    format!(r#"document.querySelector('[data-said]')?.dataset.said === "{state}""#)
}

fn a_verdict_given_in_one_tab_shows_in_another() {
    review_with_pictures(112, "review/tabs", &json!({}));
    let url = format!("{}/owner/review/pull/112", world().worker.url);
    let mut browser = browser::Browser::start();
    browser.sign_in(&world().worker.url, &sign_in("writer-code", "/"));
    let first = browser.open(&url);
    browser.wait_for(&verdict_shows("none"));
    let second = browser.open(&url);
    browser.wait_for(&verdict_shows("none"));
    browser.eval("window.stayed = true");
    // The headline says what waits for this person, once the host says who they are.
    browser.wait_for(
        "document.querySelector('[data-standing]').textContent.includes('wait for your look')",
    );

    browser.on(&first);
    browser.eval(r#"document.querySelector('button[data-say="looks_right"]').click()"#);
    browser.wait_for(&verdict_shows("looks_right"));
    browser.on(&second);
    browser.wait_for(&verdict_shows("looks_right"));
    assert_eq!(
        browser.eval("document.querySelector('[data-said]').textContent.includes('writer')"),
        json!(true)
    );
    browser.wait_for(r#"document.querySelector('[data-picture="screens/card/light.png"]').dataset.verdict === "looks_right""#);
    assert_eq!(
        browser.eval("window.stayed === true"),
        json!(true),
        "no reload"
    );
}

fn the_page_says_what_each_person_can_do() {
    review_with_pictures(113, "review/roles", &weakened());
    let url = format!("{}/owner/review/pull/113", world().worker.url);
    let app = world().worker.url.clone();
    // Signed out: a way to sign in.
    let mut browser = browser::Browser::start();
    browser.open(&url);
    browser.wait_for(&verdict_shows("none"));
    browser.wait_for(r#"!!document.querySelector('[data-said] a[data-sign-in]')"#);
    assert_eq!(
        browser.eval("!!document.querySelector('button[data-say]')"),
        json!(false)
    );
    // A reader: why there is nothing to press.
    let mut reader = browser::Browser::start();
    reader.sign_in(&app, &sign_in("reader-code", "/"));
    reader.open(&url);
    reader.wait_for(r#"!!document.querySelector('[data-said] [data-cannot]')"#);
    assert_eq!(
        reader.eval("document.querySelector('[data-said] [data-cannot]').textContent.includes('write access')"),
        json!(true)
    );
    reader.wait_for(r#"!!document.querySelector('[data-weakenings] [data-cannot]')"#);
    // A writer gives verdicts but cannot accept.
    let mut writer = browser::Browser::start();
    writer.sign_in(&app, &sign_in("writer-code", "/"));
    writer.open(&url);
    writer.wait_for(r#"!!document.querySelector('button[data-say="looks_right"]')"#);
    writer.wait_for(r#"!!document.querySelector('[data-weakenings] [data-cannot]')"#);
    assert_eq!(
        writer.eval("!!document.querySelector('button[data-accept]')"),
        json!(false)
    );
    // An admin accepts beside what it weakens.
    let mut admin = browser::Browser::start();
    admin.sign_in(&app, &sign_in("admin-code", "/"));
    admin.open(&url);
    admin.wait_for(r#"!!document.querySelector('[data-weakenings] button[data-accept]')"#);
    assert_eq!(
        admin.eval("document.querySelector('[data-weakenings]').textContent.includes('it runs another command')"),
        json!(true)
    );
    admin.eval("document.querySelector('button[data-accept]').click()");
    admin.wait_for(r#"document.querySelector('[data-weakenings]').dataset.accepted === "true""#);
    reader.wait_for(r#"document.querySelector('[data-weakenings]').dataset.accepted === "true""#);
}

/// Not a test of behaviour: pictures of the page with a changed still, a new still and a
/// changed film, light and dark and at phone width, written to `ee/target/tmp` for a person
/// to look at. `cargo test --manifest-path ee/Cargo.toml --test host -- --ignored pictures`.
fn pictures() {
    let mut browser = page_with_pictures("feat/shots");
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let shoot = |browser: &mut browser::Browser, name: &str| {
        // Motion settles before the picture.
        std::thread::sleep(Duration::from_millis(900));
        let path = dir.join(format!("page-{name}.png"));
        std::fs::write(&path, browser.screenshot()).unwrap();
        eprintln!("{}", path.display());
    };
    for (view, (width, height, mobile)) in
        [("wide", (1280, 900, false)), ("phone", (390, 844, true))]
    {
        for dark in [false, true] {
            browser.view(width, height, mobile, dark);
            let scheme = if dark { "dark" } else { "light" };
            browser.eval(
                r#"document.querySelector('[data-picture="screens/card/light.png"]').click()"#,
            );
            browser.wait_for(&loaded("img", "after"));
            shoot(&mut browser, &format!("{view}-{scheme}-still"));
            browser.eval(
                r#"document.querySelector('[data-picture="screens/settings/light.png"]').click()"#,
            );
            browser.wait_for(&loaded("img", "after"));
            shoot(&mut browser, &format!("{view}-{scheme}-new"));
            browser.eval(
                r#"document.querySelector('[data-picture="screens/checkout.webm"]').click()"#,
            );
            browser.wait_for(&loaded("video", "after"));
            browser.eval(r#"document.querySelector('[data-seek]').click()"#);
            shoot(&mut browser, &format!("{view}-{scheme}-film"));
        }
    }
    browser.view(1280, 900, false, false);
    for mode in ["slider", "onion", "diff"] {
        browser
            .eval(r#"document.querySelector('[data-picture="screens/card/light.png"]').click()"#);
        browser.eval(&format!(
            r#"document.querySelector('button[data-mode="{mode}"]').click()"#
        ));
        browser.wait_for(&loaded("img", "after"));
        shoot(&mut browser, &format!("wide-light-{mode}"));
    }

    // Review: what waits for an admin, before and after their verdict, then a reader's view.
    review_with_pictures(190, "review/shots", &weakened());
    let url = format!("{}/owner/review/pull/190", world().worker.url);
    let mut admin = browser::Browser::start();
    admin.sign_in(&world().worker.url, &sign_in("admin-code", "/"));
    admin.open(&url);
    for dark in [false, true] {
        admin.view(1280, 900, false, dark);
        admin.wait_for(&verdict_shows("none"));
        admin.wait_for(&loaded("img", "after"));
        let scheme = if dark { "dark" } else { "light" };
        shoot(&mut admin, &format!("review-{scheme}-waits"));
    }
    admin.eval(r#"document.querySelector('button[data-say="looks_right"]').click()"#);
    admin.wait_for(&verdict_shows("looks_right"));
    for dark in [false, true] {
        admin.view(1280, 900, false, dark);
        let scheme = if dark { "dark" } else { "light" };
        shoot(&mut admin, &format!("review-{scheme}-looks-right"));
    }
    admin.view(390, 844, true, false);
    shoot(&mut admin, "review-phone-light");
    let mut reader = browser::Browser::start();
    reader.sign_in(&world().worker.url, &sign_in("reader-code", "/"));
    reader.open(&url);
    reader.view(1280, 900, false, false);
    reader.wait_for(r#"!!document.querySelector('[data-weakenings] [data-cannot]')"#);
    reader.wait_for(&loaded("img", "after"));
    shoot(&mut reader, "review-light-reader");

    notes::pictures(&shoot);
    local::pictures(&shoot);
    share::pictures(&shoot);
    repository::pictures(&shoot);
}

// Helpers.

/// A CI job of one repository: a pull request's run from its head branch, or a push of a
/// branch. It speaks protocol version 2 with the OIDC token Actions would give it.
#[derive(Clone)]
struct Job {
    repository: &'static str,
    branch: String,
    pull: Option<u64>,
    /// The commit it proves.
    head: String,
    /// The workflow run it is a job of, as its run page names it.
    workflow_run: u64,
    /// A developer's CLI token instead of CI's: the run is a local one.
    cli: Option<String>,
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
            head: HEAD.to_owned(),
            workflow_run: 1,
            cli: None,
        }
    }

    fn push(repository: &'static str, branch: &str) -> Job {
        Job {
            repository,
            branch: branch.to_owned(),
            pull: None,
            head: HEAD.to_owned(),
            workflow_run: 1,
            cli: None,
        }
    }

    /// A developer's local run of `branch`, sent with their CLI `token`: no pull request, and
    /// no job.
    fn local(repository: &'static str, branch: &str, token: &str) -> Job {
        Job {
            cli: Some(token.to_owned()),
            ..Job::push(repository, branch)
        }
    }

    /// The same job proving another commit, as a later push brings, in workflow run `run`.
    fn at(&self, head: &str, workflow_run: u64) -> Job {
        Job {
            head: head.to_owned(),
            workflow_run,
            ..self.clone()
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
        match &self.cli {
            Some(token) => token.clone(),
            None => oidc(&self.claims()),
        }
    }

    /// What the job sends to open a run of `checks`.
    fn opening(&self, checks: &[&str]) -> Value {
        let mut body = json!({
            "repository": self.repository,
            "branch": self.branch,
            "head": self.head,
            "base": "main",
            "source": {
                "kind": "ci",
                "job": "proof",
                "run_url": format!("https://github.com/{}/actions/runs/{}", self.repository, self.workflow_run),
            },
            "checks": checks,
        });
        if let Some(n) = self.pull {
            body["pull"] = json!({"number": n, "title": format!("Pull request {n}")});
        }
        if self.cli.is_some() {
            body["source"] = json!({"kind": "local"});
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

    /// A check that took pictures ends, with its look.
    fn finished_with_look(
        &self,
        run: &str,
        check: &Value,
        files: &[(&str, &[u8])],
        look: &Value,
    ) -> Answer {
        let finished = json!({"check": check, "files": manifest(files), "look": look});
        self.step(run, &json!({ "finished": finished }))
    }

    /// The bytes of one file, by content.
    fn send(&self, bytes: &[u8]) -> Answer {
        let path = format!("/api/v2/blobs/{}", sha256(bytes));
        api("PUT", &path, &self.token(), Some(bytes.to_vec()))
    }

    /// What the job sends to close a run whose checks came out as `checks`, with `looks`.
    fn closing(&self, checks: &[Value], looks: &[Value], files: &[(&str, &[u8])]) -> Value {
        let mut proof = json!({
            "schema": 1,
            "repository": self.repository,
            "commit": &self.head[..7],
            "head": self.head,
            "base": "main",
            "checks": checks,
            "weakened": [],
            "looks": looks,
        });
        if let Some(n) = self.pull {
            proof["pull"] = json!({"number": n, "title": format!("Pull request {n}")});
        }
        json!({"proof": proof, "files": manifest(files)})
    }

    /// Closes a run as `close` does, with `extra` laid over its proof.
    fn close_with(
        &self,
        run: &str,
        checks: &[Value],
        looks: &[Value],
        files: &[(&str, &[u8])],
        extra: &Value,
    ) -> Answer {
        let path = format!("/api/v2/runs/{run}/done");
        let mut body = self.closing(checks, looks, files);
        for (name, value) in extra.as_object().unwrap() {
            body["proof"][name] = value.clone();
        }
        api(
            "POST",
            &path,
            &self.token(),
            Some(body.to_string().into_bytes()),
        )
    }

    fn done(&self, run: &str, checks: &[Value], files: &[(&str, &[u8])]) -> Answer {
        self.close(run, checks, &[], files)
    }

    fn close(
        &self,
        run: &str,
        checks: &[Value],
        looks: &[Value],
        files: &[(&str, &[u8])],
    ) -> Answer {
        let path = format!("/api/v2/runs/{run}/done");
        let body = self.closing(checks, looks, files);
        api(
            "POST",
            &path,
            &self.token(),
            Some(body.to_string().into_bytes()),
        )
    }
}

/// The commit a job proves unless it says another.
const HEAD: &str = "abc1234abc1234abc1234abc1234abc1234abc1";

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

/// A check's look: its pictures compared with the base commit.
fn look_of(check: &str, items: &[Value]) -> Value {
    json!({
        "check": check,
        "base_commit": "0123456789abcdef0123456789abcdef01234567",
        "base_result": "passed",
        "items": items,
    })
}

/// One still of a look, compared as `change`.
fn still(name: &str, change: &str, before: Option<&str>, after: Option<&str>) -> Value {
    json!({
        "name": name,
        "kind": "still",
        "change": change,
        "summary": format!("{name} {change}"),
        "before": before,
        "after": after,
        "comparison": null,
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
