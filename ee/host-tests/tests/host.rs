//! The proof host, driven over HTTP as CI and people use it: the upload protocol with its OIDC
//! check, the pages with their files and ranges, and who may see them. One worker under
//! `wrangler dev --local` and one fake GitHub serve every test; each test uses its own pull
//! request numbers and file bytes, so they run side by side. The worker answers on two origins,
//! the app origin and the files origin, under two host names for the same local port.

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
            "/api/v1/installed",
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
            "a_proof_is_refused_unless_the_app_is_installed",
            a_proof_is_refused_unless_the_app_is_installed,
        ),
        (
            "a_proof_for_another_repository_is_refused",
            a_proof_for_another_repository_is_refused,
        ),
        (
            "a_proof_for_another_pull_request_is_refused",
            a_proof_for_another_pull_request_is_refused,
        ),
        (
            "a_proof_of_an_unknown_schema_is_refused",
            a_proof_of_an_unknown_schema_is_refused,
        ),
        (
            "a_proof_asks_only_for_the_files_the_repository_lacks",
            a_proof_asks_only_for_the_files_the_repository_lacks,
        ),
        (
            "a_blob_whose_bytes_do_not_hash_to_its_name_is_refused",
            a_blob_whose_bytes_do_not_hash_to_its_name_is_refused,
        ),
        (
            "a_blob_no_proof_names_is_refused",
            a_blob_no_proof_names_is_refused,
        ),
        (
            "done_is_refused_while_a_file_is_missing",
            done_is_refused_while_a_file_is_missing,
        ),
        (
            "done_is_refused_for_another_repositorys_token",
            done_is_refused_for_another_repositorys_token,
        ),
        (
            "a_public_proof_shows_to_anyone",
            a_public_proof_shows_to_anyone,
        ),
        ("a_film_is_served_in_ranges", a_film_is_served_in_ranges),
        (
            "the_app_origin_never_serves_bundle_files",
            the_app_origin_never_serves_bundle_files,
        ),
        (
            "the_shell_frames_the_files_origin_in_a_sandbox",
            the_shell_frames_the_files_origin_in_a_sandbox,
        ),
        (
            "the_files_origin_refuses_a_missing_grant",
            the_files_origin_refuses_a_missing_grant,
        ),
        (
            "the_files_origin_refuses_a_tampered_grant",
            the_files_origin_refuses_a_tampered_grant,
        ),
        (
            "the_files_origin_refuses_an_expired_grant",
            the_files_origin_refuses_an_expired_grant,
        ),
        (
            "the_files_origin_refuses_a_grant_for_another_proof",
            the_files_origin_refuses_a_grant_for_another_proof,
        ),
        (
            "the_files_origin_ignores_cookies_and_sets_none",
            the_files_origin_ignores_cookies_and_sets_none,
        ),
        (
            "a_proof_shows_nothing_before_done",
            a_proof_shows_nothing_before_done,
        ),
        (
            "pull_leads_to_the_latest_whole_proof",
            pull_leads_to_the_latest_whole_proof,
        ),
        (
            "an_older_head_finishing_last_keeps_the_newer_proof_latest",
            an_older_head_finishing_last_keeps_the_newer_proof_latest,
        ),
        (
            "a_private_proof_sends_a_stranger_to_sign_in",
            a_private_proof_sends_a_stranger_to_sign_in,
        ),
        (
            "a_reader_signs_in_and_sees_a_private_proof",
            a_reader_signs_in_and_sees_a_private_proof,
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
            "an_approval_on_the_page_posts_the_comment_as_the_person",
            an_approval_on_the_page_posts_the_comment_as_the_person,
        ),
        (
            "a_forks_approval_names_its_head",
            a_forks_approval_names_its_head,
        ),
        (
            "an_admin_accepts_a_weakening_on_the_page",
            an_admin_accepts_a_weakening_on_the_page,
        ),
        (
            "accepting_on_the_page_needs_admin_rights",
            accepting_on_the_page_needs_admin_rights,
        ),
        (
            "approving_on_the_page_needs_write_access",
            approving_on_the_page_needs_write_access,
        ),
        (
            "approving_on_the_page_needs_a_session",
            approving_on_the_page_needs_a_session,
        ),
        (
            "approving_from_another_site_is_refused",
            approving_from_another_site_is_refused,
        ),
        (
            "approving_needs_words_on_one_line",
            approving_needs_words_on_one_line,
        ),
        (
            "approving_an_older_proof_is_refused",
            approving_an_older_proof_is_refused,
        ),
        (
            "approving_after_a_newer_push_is_refused",
            approving_after_a_newer_push_is_refused,
        ),
        (
            "approving_needs_something_that_waits",
            approving_needs_something_that_waits,
        ),
        (
            "approving_a_private_proof_without_access_gets_not_found",
            approving_a_private_proof_without_access_gets_not_found,
        ),
        (
            "approving_needs_the_shells_view_token",
            approving_needs_the_shells_view_token,
        ),
        (
            "the_shell_offers_approve_and_accept_by_role",
            the_shell_offers_approve_and_accept_by_role,
        ),
        (
            "the_shell_offers_nothing_to_settle_when_nothing_waits",
            the_shell_offers_nothing_to_settle_when_nothing_waits,
        ),
        (
            "the_shell_takes_only_words_from_its_frame",
            the_shell_takes_only_words_from_its_frame,
        ),
        (
            "a_page_that_asks_on_load_gets_nothing_posted",
            a_page_that_asks_on_load_gets_nothing_posted,
        ),
        (
            "only_the_shells_own_button_posts",
            only_the_shells_own_button_posts,
        ),
        (
            "a_writer_makes_a_share_link_anyone_can_open",
            a_writer_makes_a_share_link_anyone_can_open,
        ),
        (
            "a_share_link_opens_the_latest_proof_of_its_pull_request",
            a_share_link_opens_the_latest_proof_of_its_pull_request,
        ),
        (
            "a_share_link_opens_nothing_else",
            a_share_link_opens_nothing_else,
        ),
        (
            "a_revoked_share_link_finds_nothing",
            a_revoked_share_link_finds_nothing,
        ),
        (
            "an_unknown_share_link_finds_nothing",
            an_unknown_share_link_finds_nothing,
        ),
        ("sharing_needs_write_access", sharing_needs_write_access),
        (
            "sharing_from_another_site_is_refused",
            sharing_from_another_site_is_refused,
        ),
        (
            "share_links_are_listed_to_writers",
            share_links_are_listed_to_writers,
        ),
        (
            "revoking_needs_write_access_and_a_link_of_that_pull_request",
            revoking_needs_write_access_and_a_link_of_that_pull_request,
        ),
        (
            "the_shell_offers_share_to_writers_alone",
            the_shell_offers_share_to_writers_alone,
        ),
        (
            "a_share_link_view_offers_no_posting",
            a_share_link_view_offers_no_posting,
        ),
    ];
    let mut trials: Vec<Trial> = tests
        .iter()
        .map(|&(name, test)| Trial::test(name, move || run(test)))
        .collect();
    // Pictures of the shell's bar for a person to look at, not a test: run with --ignored.
    trials.push(
        Trial::test("pictures_of_the_shells_bar", || {
            run(pictures_of_the_shells_bar)
        })
        .with_ignored_flag(true),
    );
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
    let answer = api("GET", "/api/v1/installed", &ci("owner/repo", 1), None);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.json(), json!({"installed": true}));
}

fn installed_says_no_with_the_install_link() {
    let answer = api("GET", "/api/v1/installed", &ci("owner/bare", 1), None);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(
        answer.json(),
        json!({"installed": false, "install": INSTALL_URL})
    );
}

fn a_call_without_a_token_is_refused() {
    let answer = request("GET", "/api/v1/installed", &[], None);
    assert_eq!(answer.status, 401, "{}", answer.text());
}

fn a_token_signed_by_another_key_is_refused() {
    let token = sign(&world().stranger, &claims("owner/repo", 1));
    let answer = api("GET", "/api/v1/installed", &token, None);
    assert_eq!(answer.status, 401, "{}", answer.text());
}

fn an_expired_token_is_refused() {
    let mut claims = claims("owner/repo", 1);
    claims["iat"] = json!(now() - 900);
    claims["nbf"] = json!(now() - 900);
    claims["exp"] = json!(now() - 300);
    let answer = api("GET", "/api/v1/installed", &oidc(&claims), None);
    assert_eq!(answer.status, 401, "{}", answer.text());
}

fn a_token_for_another_audience_is_refused() {
    let mut claims = claims("owner/repo", 1);
    claims["aud"] = json!("https://elsewhere.test");
    let answer = api("GET", "/api/v1/installed", &oidc(&claims), None);
    assert_eq!(answer.status, 401, "{}", answer.text());
}

fn a_token_from_another_issuer_is_refused() {
    let mut claims = claims("owner/repo", 1);
    claims["iss"] = json!("https://issuer.elsewhere.test");
    let answer = api("GET", "/api/v1/installed", &oidc(&claims), None);
    assert_eq!(answer.status, 401, "{}", answer.text());
}

fn a_proof_is_refused_unless_the_app_is_installed() {
    let body = upload_body(&proof("owner/bare", 2), &[("index.html", b"<p>bare</p>")]);
    let answer = api("POST", "/api/v1/proofs", &ci("owner/bare", 2), Some(body));
    assert_eq!(answer.status, 403, "{}", answer.text());
}

fn a_proof_for_another_repository_is_refused() {
    let body = upload_body(&proof("other/repo", 3), &[("index.html", b"<p>other</p>")]);
    let answer = api("POST", "/api/v1/proofs", &ci("owner/repo", 3), Some(body));
    assert_eq!(answer.status, 403, "{}", answer.text());
}

fn a_proof_for_another_pull_request_is_refused() {
    let body = upload_body(&proof("owner/repo", 4), &[("index.html", b"<p>four</p>")]);
    let answer = api("POST", "/api/v1/proofs", &ci("owner/repo", 5), Some(body));
    assert_eq!(answer.status, 403, "{}", answer.text());
}

fn a_proof_of_an_unknown_schema_is_refused() {
    let mut proof = proof("owner/repo", 6);
    proof["schema"] = json!(99);
    let body = upload_body(&proof, &[("index.html", b"<p>six</p>")]);
    let answer = api("POST", "/api/v1/proofs", &ci("owner/repo", 6), Some(body));
    assert_eq!(answer.status, 400, "{}", answer.text());
}

fn a_proof_asks_only_for_the_files_the_repository_lacks() {
    let page: &[u8] = b"<p>lacks</p>";
    let picture: &[u8] = b"a picture that does not change";
    let files = [("index.html", page), ("media/after.png", picture)];
    let id = upload("owner/repo", 10, &files);
    assert!(!id.is_empty());

    let changed: &[u8] = b"<p>lacks, again</p>";
    let again = [("index.html", changed), ("media/after.png", picture)];
    let body = upload_body(&proof("owner/repo", 10), &again);
    let answer = api("POST", "/api/v1/proofs", &ci("owner/repo", 10), Some(body));
    assert_eq!(answer.status, 201, "{}", answer.text());
    assert_eq!(answer.json()["missing"], json!([sha256(changed)]));

    // Blobs are kept per repository: another repository sends its own.
    let body = upload_body(&proof("other/repo", 10), &files);
    let answer = api("POST", "/api/v1/proofs", &ci("other/repo", 10), Some(body));
    assert_eq!(answer.status, 201, "{}", answer.text());
    let mut missing: Vec<Value> = answer.json()["missing"].as_array().unwrap().clone();
    missing.sort_by_key(|m| m.as_str().unwrap().to_owned());
    let mut expected = vec![json!(sha256(page)), json!(sha256(picture))];
    expected.sort_by_key(|m| m.as_str().unwrap().to_owned());
    assert_eq!(missing, expected);
}

fn a_blob_whose_bytes_do_not_hash_to_its_name_is_refused() {
    let page: &[u8] = b"<p>honest bytes</p>";
    let body = upload_body(&proof("owner/repo", 11), &[("index.html", page)]);
    let answer = api("POST", "/api/v1/proofs", &ci("owner/repo", 11), Some(body));
    assert_eq!(answer.status, 201, "{}", answer.text());
    let path = format!("/api/v1/blobs/{}", sha256(page));
    // As long as the honest bytes, so only their hash tells them apart.
    let forged: &[u8] = b"<p>forged bytes</p>";
    assert_eq!(forged.len(), page.len());
    let answer = api("PUT", &path, &ci("owner/repo", 11), Some(forged.to_vec()));
    assert_eq!(answer.status, 400, "{}", answer.text());
    assert!(answer.text().contains("hash"), "{}", answer.text());

    // Nothing was kept: the honest bytes are still asked for, and still accepted.
    let body = upload_body(&proof("owner/repo", 11), &[("index.html", page)]);
    let answer = api("POST", "/api/v1/proofs", &ci("owner/repo", 11), Some(body));
    assert_eq!(answer.json()["missing"], json!([sha256(page)]));
    let answer = api("PUT", &path, &ci("owner/repo", 11), Some(page.to_vec()));
    assert_eq!(answer.status, 204, "{}", answer.text());
}

fn a_blob_no_proof_names_is_refused() {
    let bytes: &[u8] = b"bytes nobody asked for";
    let path = format!("/api/v1/blobs/{}", sha256(bytes));
    let answer = api("PUT", &path, &ci("owner/repo", 12), Some(bytes.to_vec()));
    assert_eq!(answer.status, 409, "{}", answer.text());
}

fn done_is_refused_while_a_file_is_missing() {
    let body = upload_body(
        &proof("owner/repo", 13),
        &[("index.html", b"<p>thirteen</p>")],
    );
    let answer = api("POST", "/api/v1/proofs", &ci("owner/repo", 13), Some(body));
    let id = answer.json()["id"].as_str().unwrap().to_owned();
    let answer = api(
        "POST",
        &format!("/api/v1/proofs/{id}/done"),
        &ci("owner/repo", 13),
        None,
    );
    assert_eq!(answer.status, 409, "{}", answer.text());
}

fn done_is_refused_for_another_repositorys_token() {
    let page: &[u8] = b"<p>fourteen</p>";
    let body = upload_body(&proof("owner/repo", 14), &[("index.html", page)]);
    let answer = api("POST", "/api/v1/proofs", &ci("owner/repo", 14), Some(body));
    let id = answer.json()["id"].as_str().unwrap().to_owned();
    let path = format!("/api/v1/blobs/{}", sha256(page));
    api("PUT", &path, &ci("owner/repo", 14), Some(page.to_vec()));
    let answer = api(
        "POST",
        &format!("/api/v1/proofs/{id}/done"),
        &ci("other/repo", 14),
        None,
    );
    assert_eq!(answer.status, 404, "{}", answer.text());
}

// The pages.

fn a_public_proof_shows_to_anyone() {
    let page: &[u8] = b"<!doctype html><p>public</p>";
    let picture: &[u8] = b"\x89PNG a public picture";
    let id = upload(
        "owner/repo",
        20,
        &[("index.html", page), ("media/after.png", picture)],
    );
    let folder = format!("/owner/repo/proofs/{id}/");

    // The app origin answers with its own shell; the bundle's page comes from the files origin.
    let answer = get(&folder, &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert!(
        answer
            .header("content-type")
            .unwrap()
            .starts_with("text/html")
    );
    let frame = frame_url(&answer);
    let answer = get_url(&frame, &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.body, page);
    assert!(
        answer
            .header("content-type")
            .unwrap()
            .starts_with("text/html")
    );

    // The page's relative links resolve beside it, on the files origin.
    let base = frame.strip_suffix("index.html").unwrap();
    let answer = get_url(&format!("{base}media/after.png"), &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.body, picture);
    assert_eq!(answer.header("content-type").unwrap(), "image/png");

    assert_eq!(get_url(&format!("{base}media/none.png"), &[]).status, 404);

    // Relative links need the folder's slash.
    let answer = get(&format!("/owner/repo/proofs/{id}"), &[]);
    assert_eq!(answer.status, 301, "{}", answer.text());
    assert_eq!(answer.header("location").unwrap(), folder);
}

fn a_film_is_served_in_ranges() {
    let film: &[u8] = b"0123456789 a film of eleven and more bytes";
    let id = upload(
        "owner/repo",
        21,
        &[("index.html", b"<p>film</p>"), ("media/run.mp4", film)],
    );
    let path = format!("{}media/run.mp4", files_base("owner/repo", &id, &[]));

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
}

// The files origin.

fn the_app_origin_never_serves_bundle_files() {
    let page: &[u8] = b"<script>steal(document.cookie)</script>";
    let id = upload(
        "owner/repo",
        25,
        &[("index.html", page), ("media/after.png", b"a picture")],
    );
    let folder = format!("/owner/repo/proofs/{id}/");
    let shell = get(&folder, &[]);
    assert_eq!(shell.status, 200, "{}", shell.text());
    assert!(!shell.text().contains("steal"), "{}", shell.text());
    for path in ["index.html", "media/after.png"] {
        let answer = get(&format!("{folder}{path}"), &[]);
        assert_eq!(answer.status, 404, "{path}: {}", answer.text());
    }
    // Nor does the files origin answer the app's paths.
    for path in [folder.as_str(), "/login", "/owner/repo/pull/25"] {
        let answer = get_url(&format!("{}{path}", world().worker.files), &[]);
        assert_eq!(answer.status, 404, "{path}: {}", answer.text());
    }
}

fn the_shell_frames_the_files_origin_in_a_sandbox() {
    let id = upload("owner/repo", 26, &[("index.html", b"<p>framed</p>")]);
    let shell = get(&format!("/owner/repo/proofs/{id}/"), &[]);
    assert_eq!(shell.status, 200, "{}", shell.text());
    let html = shell.text();
    let frame = frame_url(&shell);
    assert!(
        frame.starts_with(&format!("{}/", world().worker.files)),
        "{frame}"
    );
    assert!(frame.ends_with("/index.html"), "{frame}");
    let sandbox = attribute(&html, "sandbox").unwrap_or_else(|| panic!("no sandbox: {html}"));
    let tokens: Vec<&str> = sandbox.split_whitespace().collect();
    assert!(tokens.contains(&"allow-scripts"), "{sandbox}");
    assert!(tokens.contains(&"allow-same-origin"), "{sandbox}");
    assert!(
        !tokens.iter().any(|t| t.starts_with("allow-top-navigation")),
        "{sandbox}"
    );
    // The shell holds a grant, so it is never cached, and nobody frames the app origin.
    assert_eq!(shell.header("cache-control").unwrap(), "no-store");
    let policy = shell.header("content-security-policy").unwrap();
    assert!(policy.contains("frame-ancestors 'none'"), "{policy}");
    assert!(
        policy.contains(&format!("frame-src {}", world().worker.files)),
        "{policy}"
    );

    // Only the app origin may frame the files.
    let answer = get_url(&frame, &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    let policy = answer.header("content-security-policy").unwrap();
    assert!(
        policy.contains(&format!("frame-ancestors {}", world().worker.url)),
        "{policy}"
    );
    assert!(answer.header("x-frame-options").is_some());
}

fn the_files_origin_refuses_a_missing_grant() {
    let id = upload("owner/repo", 27, &[("index.html", b"<p>granted</p>")]);
    let files = &world().worker.files;
    for path in [
        "/index.html".to_owned(),
        format!("/{id}/index.html"),
        format!("/owner/repo/proofs/{id}/index.html"),
    ] {
        let answer = get_url(&format!("{files}{path}"), &[]);
        assert_eq!(answer.status, 404, "{path}: {}", answer.text());
    }
}

fn the_files_origin_refuses_a_tampered_grant() {
    let id = upload("owner/repo", 28, &[("index.html", b"<p>tamper</p>")]);
    let frame = frame_url(&get(&format!("/owner/repo/proofs/{id}/"), &[]));
    assert_eq!(get_url(&frame, &[]).status, 200);
    let files = &world().worker.files;
    let grant = frame
        .strip_prefix(&format!("{files}/"))
        .unwrap()
        .strip_suffix("/index.html")
        .unwrap();
    let (payload, mac) = grant.split_once('.').unwrap();

    // A later expiry under the old seal.
    let decoded = URL_SAFE_NO_PAD.decode(payload).unwrap();
    let mut claims: Value = serde_json::from_slice(&decoded).unwrap();
    claims["exp"] = json!(claims["exp"].as_i64().unwrap() + 86400);
    let longer = URL_SAFE_NO_PAD.encode(claims.to_string());
    let answer = get_url(&format!("{files}/{longer}.{mac}/index.html"), &[]);
    assert_eq!(answer.status, 404, "{}", answer.text());

    // A seal changed by one character.
    let mut mac = mac.to_owned();
    let last = mac.pop().unwrap();
    mac.push(if last == 'A' { 'B' } else { 'A' });
    let answer = get_url(&format!("{files}/{payload}.{mac}/index.html"), &[]);
    assert_eq!(answer.status, 404, "{}", answer.text());
}

fn the_files_origin_refuses_an_expired_grant() {
    let id = upload("owner/repo", 29, &[("index.html", b"<p>expiry</p>")]);
    // A grant sealed with the host's key is good until its expiry, and not after.
    let fresh = files_base("owner/repo", &id, &[]);
    assert_eq!(get_url(&format!("{fresh}index.html"), &[]).status, 200);
    let expired = files_base("owner/repo", &id, &[("exp", json!(now() - 1))]);
    let answer = get_url(&format!("{expired}index.html"), &[]);
    assert_eq!(answer.status, 404, "{}", answer.text());
}

fn the_files_origin_refuses_a_grant_for_another_proof() {
    let mine = upload("owner/repo", 34, &[("index.html", b"<p>mine</p>")]);
    let theirs = upload("owner/secret", 34, &[("index.html", b"<p>theirs</p>")]);
    // The grant for one proof shows that proof, not another.
    let frame = frame_url(&get(&format!("/owner/repo/proofs/{mine}/"), &[]));
    assert_eq!(get_url(&frame, &[]).body, b"<p>mine</p>");
    // A grant naming a proof of another repository than its own opens nothing.
    let crossed = files_base("owner/repo", &theirs, &[]);
    let answer = get_url(&format!("{crossed}index.html"), &[]);
    assert_eq!(answer.status, 404, "{}", answer.text());
    // Nor does a grant for a proof that does not exist.
    let unknown = files_base("owner/repo", "0123456789abcdef0123456789abcdef", &[]);
    assert_eq!(get_url(&format!("{unknown}index.html"), &[]).status, 404);
}

fn the_files_origin_ignores_cookies_and_sets_none() {
    let id = upload("owner/secret", 35, &[("index.html", b"<p>cookies</p>")]);
    let path = format!("/owner/secret/proofs/{id}/");
    let session = sign_in("reader-code", &path);
    // A session is no grant on the files origin.
    let files = &world().worker.files;
    let answer = get_url(&format!("{files}{path}index.html"), &[("Cookie", &session)]);
    assert_eq!(answer.status, 404, "{}", answer.text());
    let frame = frame_url(&get(&path, &[("Cookie", &session)]));
    let answer = get_url(&frame, &[("Cookie", &session)]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert!(answer.header("set-cookie").is_none());
    let answer = get_url(&frame, &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert!(answer.header("set-cookie").is_none());
}

fn a_proof_shows_nothing_before_done() {
    let page: &[u8] = b"<p>not whole yet</p>";
    let body = upload_body(&proof("owner/repo", 22), &[("index.html", page)]);
    let answer = api("POST", "/api/v1/proofs", &ci("owner/repo", 22), Some(body));
    let created = answer.json();
    let path = format!("/api/v1/blobs/{}", sha256(page));
    api("PUT", &path, &ci("owner/repo", 22), Some(page.to_vec()));
    let id = created["id"].as_str().unwrap();
    assert_eq!(get(&format!("/owner/repo/proofs/{id}/"), &[]).status, 404);
}

fn pull_leads_to_the_latest_whole_proof() {
    upload("owner/repo", 23, &[("index.html", b"<p>first</p>")]);
    let second = upload("owner/repo", 23, &[("index.html", b"<p>second</p>")]);
    // A third that never finishes leaves the second as the latest.
    let body = upload_body(&proof("owner/repo", 23), &[("index.html", b"<p>third</p>")]);
    api("POST", "/api/v1/proofs", &ci("owner/repo", 23), Some(body));

    let answer = get("/owner/repo/pull/23", &[]);
    assert_eq!(answer.status, 302, "{}", answer.text());
    assert_eq!(
        answer.header("location").unwrap(),
        format!("/owner/repo/proofs/{second}/")
    );
    assert_eq!(get("/owner/repo/pull/24", &[]).status, 404);
}

fn an_older_head_finishing_last_keeps_the_newer_proof_latest() {
    // GitHub says the pull request is at HEAD; a slow run of an earlier push finishes last.
    let mut older = proof("owner/repo", 83);
    older["head"] = json!("0ld0000aaaa0000aaaa0000aaaa0000aaaa0000a");
    let newer = start_upload("owner/repo", 83, &proof("owner/repo", 83), b"<p>newer</p>");
    let late = start_upload("owner/repo", 83, &older, b"<p>older</p>");
    finish_upload("owner/repo", 83, &newer);
    finish_upload("owner/repo", 83, &late);
    let answer = get("/owner/repo/pull/83", &[]);
    assert_eq!(answer.status, 302, "{}", answer.text());
    assert_eq!(
        answer.header("location").unwrap(),
        format!("/owner/repo/proofs/{newer}/"),
        "the proof of the current head stays the latest"
    );
    // The older proof is still whole at its own address.
    assert_eq!(get(&format!("/owner/repo/proofs/{late}/"), &[]).status, 200);

    // An older head that finishes while nothing newer is whole is the latest there is.
    let mut older = proof("owner/repo", 84);
    older["head"] = json!("0ld0000aaaa0000aaaa0000aaaa0000aaaa0000a");
    let late = start_upload("owner/repo", 84, &older, b"<p>alone</p>");
    finish_upload("owner/repo", 84, &late);
    let answer = get("/owner/repo/pull/84", &[]);
    assert_eq!(
        answer.header("location").unwrap(),
        format!("/owner/repo/proofs/{late}/")
    );
}

// Who sees a page.

fn a_private_proof_sends_a_stranger_to_sign_in() {
    let id = upload("owner/secret", 30, &[("index.html", b"<p>secret</p>")]);
    let path = format!("/owner/secret/proofs/{id}/");
    let answer = get(&path, &[]);
    assert_eq!(answer.status, 302, "{}", answer.text());
    assert_eq!(
        answer.header("location").unwrap(),
        format!("/login?next={}", path.replace('/', "%2F"))
    );
}

fn a_reader_signs_in_and_sees_a_private_proof() {
    let page: &[u8] = b"<p>secret, for readers</p>";
    let id = upload("owner/secret", 31, &[("index.html", page)]);
    let path = format!("/owner/secret/proofs/{id}/");
    let session = sign_in("reader-code", &path);
    let answer = get(&path, &[("Cookie", &session)]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    let answer = get_url(&frame_url(&answer), &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.body, page);

    let answer = get("/owner/secret/pull/31", &[("Cookie", &session)]);
    assert_eq!(answer.status, 302, "{}", answer.text());
    assert_eq!(answer.header("location").unwrap(), path);
}

fn a_signed_in_person_without_access_gets_not_found() {
    let id = upload(
        "owner/secret",
        32,
        &[("index.html", b"<p>not for strangers</p>")],
    );
    let path = format!("/owner/secret/proofs/{id}/");
    let session = sign_in("stranger-code", &path);
    assert_eq!(get(&path, &[("Cookie", &session)]).status, 404);
    assert_eq!(
        get("/owner/secret/pull/32", &[("Cookie", &session)]).status,
        404
    );
}

fn an_unknown_repository_looks_like_a_private_one() {
    let path = "/nobody/nothing/proofs/0123456789abcdef/";
    let answer = get(path, &[]);
    assert_eq!(answer.status, 302, "{}", answer.text());
    let session = sign_in("stranger-code", path);
    assert_eq!(get(path, &[("Cookie", &session)]).status, 404);
}

fn a_forged_session_counts_as_none() {
    let id = upload("owner/secret", 33, &[("index.html", b"<p>forged</p>")]);
    let path = format!("/owner/secret/proofs/{id}/");
    let session = sign_in("reader-code", &path);
    let mut forged = session.clone();
    let last = forged.pop().unwrap();
    forged.push(if last == 'A' { 'B' } else { 'A' });
    let answer = get(&path, &[("Cookie", &forged)]);
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
    let id = upload("owner/secret", 36, &[("index.html", b"<p>logout</p>")]);
    let path = format!("/owner/secret/proofs/{id}/");
    let session = sign_in("reader-code", &path);
    assert_eq!(get(&path, &[("Cookie", &session)]).status, 200);
    let answer = get("/logout", &[("Cookie", &session)]);
    assert_eq!(answer.status, 302, "{}", answer.text());
    let cleared = cookie_set(&answer, "session").unwrap();
    assert!(cleared.contains("Max-Age=0"), "{cleared}");
    // The host forgot the session: the old cookie, sent again, is no sign in.
    let answer = get(&path, &[("Cookie", &session)]);
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
    let id = upload("owner/cached", 37, &[("index.html", b"<p>cached</p>")]);
    let path = format!("/owner/cached/proofs/{id}/");
    let session = sign_in("reader-code", &path);
    let github = &world().github;
    let before = github.asked("GET", "/repos/owner/cached");
    assert_eq!(get(&path, &[("Cookie", &session)]).status, 200);
    assert_eq!(get(&path, &[("Cookie", &session)]).status, 200);
    let answer = get("/owner/cached/pull/37", &[("Cookie", &session)]);
    assert_eq!(answer.status, 302, "{}", answer.text());
    assert_eq!(github.asked("GET", "/repos/owner/cached"), before + 1);
}

fn a_second_view_does_not_ask_github_whether_public() {
    let id = upload("owner/open", 38, &[("index.html", b"<p>open</p>")]);
    let path = format!("/owner/open/proofs/{id}/");
    let github = &world().github;
    let installation = "/repos/owner/open/installation";
    let (repo, install) = (
        github.asked("GET", "/repos/owner/open"),
        github.asked("GET", installation),
    );
    assert_eq!(get(&path, &[]).status, 200);
    assert_eq!(get(&path, &[]).status, 200);
    assert_eq!(github.asked("GET", "/repos/owner/open"), repo + 1);
    assert_eq!(github.asked("GET", installation), install + 1);
}

fn an_unknown_key_id_refetches_the_jwks_once() {
    let github = &world().github;
    let jwks = "/.well-known/jwks";
    let before = github.asked("GET", jwks);
    // Known keys come from the cache.
    for _ in 0..2 {
        let answer = api("GET", "/api/v1/installed", &ci("owner/repo", 1), None);
        assert_eq!(answer.status, 200, "{}", answer.text());
    }
    assert_eq!(github.asked("GET", jwks), before);

    // A key id GitHub never published: one refetch, then refused.
    let mut rng = rand::thread_rng();
    let rotated = RsaPrivateKey::new(&mut rng, 2048).unwrap();
    let token = sign_with_kid(&rotated, "rotated-key", &claims("owner/repo", 1));
    let answer = api("GET", "/api/v1/installed", &token, None);
    assert_eq!(answer.status, 401, "{}", answer.text());
    assert_eq!(github.asked("GET", jwks), before + 1);

    // Once GitHub publishes it, the next token naming it refetches and passes, and later ones
    // find it in the cache.
    github.publish("rotated-key", RsaPublicKey::from(&rotated));
    let answer = api("GET", "/api/v1/installed", &token, None);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(github.asked("GET", jwks), before + 2);
    let answer = api("GET", "/api/v1/installed", &token, None);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(github.asked("GET", jwks), before + 2);
}

// Approving and accepting on the page.

fn an_approval_on_the_page_posts_the_comment_as_the_person() {
    let id = upload_proof("owner/team", 50, &waiting("owner/team", 50));
    let session = sign_in("writer-code", "/");
    let words = json!({"check": "screens", "words": "  the header reads well "});
    let answer = act("owner/team", &id, "approve", Some(&session), words);
    assert_eq!(answer.status, 201, "{}", answer.text());
    let posted = world().github.posted("owner/team", 50);
    assert_eq!(posted.len(), 1, "{posted:?}");
    assert_eq!(
        posted[0].login, "writer",
        "posted with the person's own token"
    );
    assert_eq!(posted[0].body, "/fearlezz approve the header reads well");
    let url = answer.json()["url"].as_str().unwrap().to_owned();
    assert!(
        url.starts_with("https://github.com/owner/team/pull/50#issuecomment-"),
        "{url}"
    );
    assert_eq!(answer.json()["pull"], 50);
}

fn a_forks_approval_names_its_head() {
    let id = upload_proof("owner/team", 51, &waiting("owner/team", 51));
    world()
        .github
        .set_head("owner/team", 51, github::HEAD, "someone/team");
    let session = sign_in("writer-code", "/");
    let words = json!({"check": "screens", "words": "looks right"});
    let answer = act("owner/team", &id, "approve", Some(&session), words);
    assert_eq!(answer.status, 201, "{}", answer.text());
    let posted = world().github.posted("owner/team", 51);
    assert_eq!(posted[0].body, "/fearlezz approve abc1234 looks right");
}

fn an_admin_accepts_a_weakening_on_the_page() {
    let id = upload_proof("owner/team", 52, &waiting("owner/team", 52));
    let session = sign_in("admin-code", "/");
    let words = json!({"words": "the old command hid nothing"});
    let answer = act("owner/team", &id, "accept", Some(&session), words);
    assert_eq!(answer.status, 201, "{}", answer.text());
    let posted = world().github.posted("owner/team", 52);
    assert_eq!(posted.len(), 1, "{posted:?}");
    assert_eq!(posted[0].login, "admin");
    assert_eq!(
        posted[0].body,
        "/fearlezz accept the old command hid nothing"
    );
}

fn accepting_on_the_page_needs_admin_rights() {
    let id = upload_proof("owner/team", 53, &waiting("owner/team", 53));
    let session = sign_in("writer-code", "/");
    let answer = act(
        "owner/team",
        &id,
        "accept",
        Some(&session),
        json!({"words": "fine"}),
    );
    assert_eq!(answer.status, 403, "{}", answer.text());
    let error = answer.json()["error"].as_str().unwrap().to_owned();
    assert!(error.contains("admin rights"), "{error}");
    assert!(world().github.posted("owner/team", 53).is_empty());
}

fn approving_on_the_page_needs_write_access() {
    let id = upload_proof("owner/team", 54, &waiting("owner/team", 54));
    let session = sign_in("reader-code", "/");
    let words = json!({"check": "screens", "words": "fine"});
    let answer = act("owner/team", &id, "approve", Some(&session), words);
    assert_eq!(answer.status, 403, "{}", answer.text());
    let error = answer.json()["error"].as_str().unwrap().to_owned();
    assert!(error.contains("write access"), "{error}");
    assert!(world().github.posted("owner/team", 54).is_empty());
}

fn approving_on_the_page_needs_a_session() {
    let id = upload_proof("owner/team", 55, &waiting("owner/team", 55));
    let words = json!({"check": "screens", "words": "fine"});
    let answer = act("owner/team", &id, "approve", None, words);
    assert_eq!(answer.status, 401, "{}", answer.text());
    assert!(answer.json()["error"].as_str().unwrap().contains("sign in"));
    assert!(world().github.posted("owner/team", 55).is_empty());
}

fn approving_from_another_site_is_refused() {
    let id = upload_proof("owner/team", 56, &waiting("owner/team", 56));
    let session = sign_in("writer-code", "/");
    let path = format!("/owner/team/proofs/{id}/approve");
    let body = json!({"check": "screens", "words": "fine"})
        .to_string()
        .into_bytes();
    let files = world().worker.files.clone();
    let cookie = session.as_str();
    for origin in [Some("https://evil.test"), Some(files.as_str()), None] {
        let mut headers = vec![("Cookie", cookie), ("Content-Type", "application/json")];
        if let Some(origin) = origin {
            headers.push(("Origin", origin));
        }
        let answer = request("POST", &path, &headers, Some(body.clone()));
        assert_eq!(answer.status, 403, "{origin:?}: {}", answer.text());
    }
    // A form post from the app's own origin is no JSON, and gets nothing either.
    let origin = world().worker.url.clone();
    let headers = [
        ("Cookie", cookie),
        ("Origin", origin.as_str()),
        ("Content-Type", "application/x-www-form-urlencoded"),
    ];
    let answer = request("POST", &path, &headers, Some(b"words=fine".to_vec()));
    assert_eq!(answer.status, 415, "{}", answer.text());
    assert!(world().github.posted("owner/team", 56).is_empty());
}

fn approving_needs_words_on_one_line() {
    let id = upload_proof("owner/team", 57, &waiting("owner/team", 57));
    let session = sign_in("writer-code", "/");
    for words in ["", "   ", "fine\nreally", "fine\rreally"] {
        let body = json!({"check": "screens", "words": words});
        let answer = act("owner/team", &id, "approve", Some(&session), body);
        assert_eq!(answer.status, 400, "{words:?}: {}", answer.text());
    }
    let long = "a".repeat(501);
    let body = json!({"check": "screens", "words": long});
    let answer = act("owner/team", &id, "approve", Some(&session), body);
    assert_eq!(answer.status, 400, "{}", answer.text());
    assert!(world().github.posted("owner/team", 57).is_empty());
}

fn approving_an_older_proof_is_refused() {
    let older = upload_proof("owner/team", 58, &waiting("owner/team", 58));
    let newer = upload_proof("owner/team", 58, &waiting("owner/team", 58));
    let session = sign_in("writer-code", "/");
    let words = json!({"check": "screens", "words": "fine"});
    let answer = act(
        "owner/team",
        &older,
        "approve",
        Some(&session),
        words.clone(),
    );
    assert_eq!(answer.status, 409, "{}", answer.text());
    let error = answer.json()["error"].as_str().unwrap().to_owned();
    assert!(error.contains("newer proof"), "{error}");
    assert!(error.contains(&newer), "it names the newer one: {error}");
    assert!(world().github.posted("owner/team", 58).is_empty());
    let answer = act("owner/team", &newer, "approve", Some(&session), words);
    assert_eq!(answer.status, 201, "{}", answer.text());
}

fn approving_after_a_newer_push_is_refused() {
    let id = upload_proof("owner/team", 59, &waiting("owner/team", 59));
    world().github.set_head(
        "owner/team",
        59,
        "fff0000fff0000fff0000fff0000fff0000fff00",
        "owner/team",
    );
    let session = sign_in("writer-code", "/");
    let words = json!({"check": "screens", "words": "fine"});
    let answer = act("owner/team", &id, "approve", Some(&session), words);
    assert_eq!(answer.status, 409, "{}", answer.text());
    let error = answer.json()["error"].as_str().unwrap().to_owned();
    assert!(error.contains("fff0000"), "{error}");
    assert!(world().github.posted("owner/team", 59).is_empty());
}

fn approving_needs_something_that_waits() {
    // Nothing waits in a proof without pictures or weakenings.
    let id = upload_proof("owner/team", 60, &proof("owner/team", 60));
    let session = sign_in("admin-code", "/");
    let approve = json!({"check": "screens", "words": "fine"});
    let answer = act("owner/team", &id, "approve", Some(&session), approve);
    assert_eq!(answer.status, 409, "{}", answer.text());
    let answer = act(
        "owner/team",
        &id,
        "accept",
        Some(&session),
        json!({"words": "fine"}),
    );
    assert_eq!(answer.status, 409, "{}", answer.text());
    // Pictures of another check do not wait.
    let id = upload_proof("owner/team", 61, &waiting("owner/team", 61));
    let other = json!({"check": "others", "words": "fine"});
    let answer = act("owner/team", &id, "approve", Some(&session), other);
    assert_eq!(answer.status, 409, "{}", answer.text());
    assert!(world().github.posted("owner/team", 60).is_empty());
    assert!(world().github.posted("owner/team", 61).is_empty());
}

fn approving_a_private_proof_without_access_gets_not_found() {
    let id = upload_proof("owner/vault", 62, &waiting("owner/vault", 62));
    let session = sign_in("stranger-code", "/");
    let words = json!({"check": "screens", "words": "fine"});
    let answer = act("owner/vault", &id, "approve", Some(&session), words.clone());
    assert_eq!(answer.status, 404, "{}", answer.text());
    // A writer of the private repository approves it.
    let session = sign_in("writer-code", "/");
    let answer = act("owner/vault", &id, "approve", Some(&session), words);
    assert_eq!(answer.status, 201, "{}", answer.text());
}

fn approving_needs_the_shells_view_token() {
    let id = upload_proof("owner/team", 85, &waiting("owner/team", 85));
    let other = upload_proof("owner/team", 86, &waiting("owner/team", 86));
    let writer = sign_in("writer-code", "/");
    let admin = sign_in("admin-code", "/");
    let words = json!({"check": "screens", "words": "fine"});
    let path = format!("/owner/team/proofs/{id}/approve");
    let origin = world().worker.url.clone();
    let send = |session: &str, view: Option<&str>| {
        let mut headers = vec![
            ("Origin", origin.as_str()),
            ("Content-Type", "application/json"),
            ("Cookie", session),
        ];
        if let Some(view) = view {
            headers.push(("Fearlezz-View", view));
        }
        request(
            "POST",
            &path,
            &headers,
            Some(words.to_string().into_bytes()),
        )
    };
    // Without the token only the shell's page holds, nothing is posted, whoever asks.
    let answer = send(&writer, None);
    assert_eq!(answer.status, 403, "{}", answer.text());
    assert!(answer.json()["error"].as_str().unwrap().contains("reload"));
    // Nor with a token of another proof's shell, another person's, or a made up one.
    let theirs = view_token("owner/team", &id, &admin).unwrap();
    let elsewhere = view_token("owner/team", &other, &writer).unwrap();
    for view in [theirs.as_str(), elsewhere.as_str(), "made.up"] {
        let answer = send(&writer, Some(view));
        assert_eq!(answer.status, 403, "{view}: {}", answer.text());
    }
    assert!(world().github.posted("owner/team", 85).is_empty());
    // The token of this person's own view of this proof posts.
    let mine = view_token("owner/team", &id, &writer).unwrap();
    let answer = send(&writer, Some(&mine));
    assert_eq!(answer.status, 201, "{}", answer.text());
}

fn the_shell_offers_approve_and_accept_by_role() {
    let id = upload_proof("owner/team", 87, &waiting("owner/team", 87));
    let path = format!("/owner/team/proofs/{id}/");
    let approve = "data-kind=\"approve\" data-check=\"screens\"";
    let accept = "data-kind=\"accept\"";
    // An admin gets both, each with a field for their own words, from the host's own proof.
    let admin = sign_in("admin-code", "/");
    let html = get(&path, &[("Cookie", &admin)]).text();
    let bar = &html[html
        .find("id=\"waits\"")
        .unwrap_or_else(|| panic!("{html}"))..];
    let bar = &bar[..bar.find("</section>").unwrap()];
    assert_eq!(bar.matches(approve).count(), 1, "{bar}");
    assert_eq!(bar.matches(accept).count(), 1, "{bar}");
    assert_eq!(bar.matches("name=\"words\"").count(), 2, "{bar}");
    assert!(bar.contains("Check <code>tests</code> is weaker"), "{bar}");
    assert!(bar.contains("it runs another command"), "{bar}");
    assert!(
        !bar.contains(" value="),
        "nothing is filled in for them: {bar}"
    );
    // A writer approves, but only an admin accepts.
    let writer = sign_in("writer-code", "/");
    let html = get(&path, &[("Cookie", &writer)]).text();
    assert!(html.contains(approve), "{html}");
    assert!(!html.contains(accept), "{html}");
    // A reader gets nothing to press, and someone without a session a way to sign in.
    let reader = sign_in("reader-code", "/");
    let html = get(&path, &[("Cookie", &reader)]).text();
    assert!(!html.contains("data-kind="), "{html}");
    let html = get(&path, &[]).text();
    assert!(!html.contains("data-kind="), "{html}");
    let next = path.replace('/', "%2F");
    assert!(
        html.contains(&format!("href=\"/login?next={next}\"")),
        "{html}"
    );
    // A fork's head waits as a whole: one Approve that names no check.
    let fork = upload_proof("owner/team", 88, &waiting("owner/team", 88));
    world()
        .github
        .set_head("owner/team", 88, github::HEAD, "someone/team");
    let html = get(
        &format!("/owner/team/proofs/{fork}/"),
        &[("Cookie", &writer)],
    )
    .text();
    assert!(html.contains(approve), "{html}");
}

fn the_shell_offers_nothing_to_settle_when_nothing_waits() {
    let id = upload_proof("owner/team", 89, &proof("owner/team", 89));
    let admin = sign_in("admin-code", "/");
    let html = get(&format!("/owner/team/proofs/{id}/"), &[("Cookie", &admin)]).text();
    assert!(!html.contains("id=\"waits\""), "{html}");
    assert!(!html.contains("data-kind="), "{html}");
}

fn the_shell_takes_only_words_from_its_frame() {
    let id = upload_proof("owner/team", 63, &waiting("owner/team", 63));
    let writer = sign_in("writer-code", "/");
    let answer = get(&format!("/owner/team/proofs/{id}/"), &[("Cookie", &writer)]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    let csp = answer.header("content-security-policy").unwrap().to_owned();
    let html = answer.text();
    // Its one script runs by a nonce that changes per view, and may only call its own origin.
    let nonce = attribute(&html[html.find("<script").unwrap()..], "nonce").unwrap();
    assert!(nonce.len() >= 32, "{nonce}");
    assert!(
        csp.contains(&format!("script-src 'nonce-{nonce}'")),
        "{csp}"
    );
    assert!(csp.contains("connect-src 'self'"), "{csp}");
    let again = get(&format!("/owner/team/proofs/{id}/"), &[("Cookie", &writer)]).text();
    assert!(!again.contains(&nonce), "a fresh nonce per view");
    // It listens to its frame alone, from the files origin, for words to fill in.
    let files = &world().worker.files;
    assert!(html.contains("e.source !== frame.contentWindow"), "{html}");
    assert!(html.contains(&format!("\"{files}\"")), "{html}");
    assert!(html.contains("'fearlezz:fill'"), "{html}");
    assert!(!html.contains("'fearlezz:approve'"), "{html}");
    assert!(html.contains("credentials: 'same-origin'"), "{html}");
    assert!(
        html.contains("id=\"said\""),
        "the bar for what happened: {html}"
    );
}

/// A page that asks on load, as a pull request's CI could write it: with the old messages
/// that once had the shell post, fill messages, and a post of its own to the app origin, for
/// the proof its grant names.
fn asking_page(app: &str) -> String {
    format!(
        r#"<!doctype html><title>asks</title><p>a page that asks</p><script>
var p = window.parent;
p.postMessage({{ kind: 'fearlezz:approve', check: 'screens', words: 'approved by the page' }}, '*');
p.postMessage({{ kind: 'fearlezz:accept', check: null, words: 'accepted by the page' }}, '*');
p.postMessage({{ kind: 'fearlezz:fill', of: 'approve', check: 'nothing', words: 'not a check' }}, '*');
p.postMessage({{ kind: 'fearlezz:fill', of: 'approve', check: 'screens', words: 'words from the page' }}, '*');
p.postMessage({{ kind: 'fearlezz:fill', of: 'accept', check: null, words: 'why from the page' }}, '*');
var grant = JSON.parse(atob(location.pathname.split('/')[1].split('.')[0].replace(/-/g, '+').replace(/_/g, '/')));
fetch('{app}/' + grant.repo + '/proofs/' + grant.proof + '/accept', {{ method: 'POST', credentials: 'include',
  headers: {{ 'Content-Type': 'application/json' }}, body: JSON.stringify({{ words: 'fetched' }}) }}).catch(function () {{}});
</script>"#
    )
}

/// Opens the shell of a waiting proof whose page asks on load, signed in as an admin, in a
/// real browser.
fn open_asking_shell(pull: u64) -> browser::Browser {
    let app = world().worker.url.clone();
    let page = asking_page(&app);
    let id = upload_with(
        "owner/team",
        pull,
        &waiting("owner/team", pull),
        &[("index.html", page.as_bytes())],
    );
    let admin = sign_in("admin-code", "/");
    let (name, value) = admin.split_once('=').unwrap();
    let mut browser = browser::Browser::start();
    browser.cookie(&app, name, value);
    browser.open(&format!("{app}/owner/team/proofs/{id}/"));
    browser
}

fn a_page_that_asks_on_load_gets_nothing_posted() {
    let mut browser = open_asking_shell(90);
    // The page's words reach the shell's own fields, which shows its messages arrived...
    browser.wait_for(
        "(document.querySelector('[data-kind=accept] [name=words]') || {}).value === 'why from the page'",
    );
    // ...yet nothing is posted: not on load, not by the old messages, not by its own fetch.
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        world().github.posted("owner/team", 90).is_empty(),
        "{:?}",
        world().github.posted("owner/team", 90)
    );
    let approve = browser.eval("document.querySelector('[data-kind=approve] [name=words]').value");
    assert_eq!(approve, json!("words from the page"));
    assert_eq!(
        browser.eval("document.activeElement.name"),
        json!("words"),
        "the field it filled has focus"
    );
    assert_eq!(
        browser.eval("document.querySelectorAll('[data-kind]').length"),
        json!(2),
        "a message never adds a control"
    );
}

fn only_the_shells_own_button_posts() {
    let mut browser = open_asking_shell(91);
    browser.wait_for(
        "(document.querySelector('[data-kind=approve] [name=words]') || {}).value === 'words from the page'",
    );
    browser.eval("document.querySelector('[data-kind=approve] [name=words]').value = 'the header reads well'");
    browser.eval("document.querySelector('[data-kind=approve] button').click()");
    browser.wait_for("!document.getElementById('said').hidden");
    let posted = world().github.posted("owner/team", 91);
    assert_eq!(posted.len(), 1, "{posted:?}");
    assert_eq!(posted[0].login, "admin");
    assert_eq!(posted[0].body, "/fearlezz approve the header reads well");
    let said = browser.eval("document.getElementById('said').textContent");
    assert!(said.as_str().unwrap().contains("Posted"), "{said}");
}

/// Pictures of the shell's bar with Approve and Accept, light and dark, for a person to look at.
fn pictures_of_the_shells_bar() {
    let mut browser = open_asking_shell(92);
    browser.wait_for(
        "(document.querySelector('[data-kind=accept] [name=words]') || {}).value === 'why from the page'",
    );
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    for scheme in ["light", "dark"] {
        browser.scheme(scheme);
        std::thread::sleep(Duration::from_millis(500));
        let file = dir.join(format!("shell-bar-{scheme}.png"));
        std::fs::write(&file, browser.picture()).unwrap();
        println!("{}", file.display());
    }
}

// Share links.

fn a_writer_makes_a_share_link_anyone_can_open() {
    let page = format!("<p>shared vault {}</p>", now());
    let id = upload_with(
        "owner/vault",
        70,
        &proof("owner/vault", 70),
        &[("index.html", page.as_bytes())],
    );
    let session = sign_in("writer-code", "/");
    let answer = share("owner/vault", 70, Some(&session));
    assert_eq!(answer.status, 201, "{}", answer.text());
    let made = answer.json();
    let token = made["token"].as_str().unwrap().to_owned();
    assert!(token.len() >= 32, "unguessable: {token}");
    assert_eq!(
        made["url"].as_str().unwrap(),
        format!("{}/s/{token}", world().worker.url)
    );
    assert_eq!(made["made_by"], "writer");

    // Without any session, the private pull request's proof opens through the link.
    let answer = get(&format!("/s/{token}"), &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert!(cookie_set(&answer, "session").is_none());
    assert_eq!(answer.header("cache-control"), Some("no-store"));
    assert_eq!(answer.header("referrer-policy"), Some("no-referrer"));
    assert!(
        answer.text().contains("Pull request 70"),
        "{}",
        answer.text()
    );
    let frame = frame_url(&answer);
    assert!(frame.starts_with(&world().worker.files), "{frame}");
    let answer = get_url(&frame, &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.body, page.as_bytes());
    // The proof's own address still needs a session.
    let answer = get(&format!("/owner/vault/proofs/{id}/"), &[]);
    assert_eq!(answer.status, 302, "{}", answer.text());
}

fn a_share_link_opens_the_latest_proof_of_its_pull_request() {
    let first = format!("<p>first of 71 {}</p>", now());
    upload_with(
        "owner/vault",
        71,
        &proof("owner/vault", 71),
        &[("index.html", first.as_bytes())],
    );
    let session = sign_in("writer-code", "/");
    let token = share("owner/vault", 71, Some(&session)).json()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let second = format!("<p>second of 71 {}</p>", now());
    upload_with(
        "owner/vault",
        71,
        &proof("owner/vault", 71),
        &[("index.html", second.as_bytes())],
    );
    let answer = get(&format!("/s/{token}"), &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    let answer = get_url(&frame_url(&answer), &[]);
    assert_eq!(answer.body, second.as_bytes());
}

fn a_share_link_opens_nothing_else() {
    let other = upload_proof("owner/vault", 73, &proof("owner/vault", 73));
    upload_proof("owner/vault", 72, &proof("owner/vault", 72));
    let session = sign_in("writer-code", "/");
    let token = share("owner/vault", 72, Some(&session)).json()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(get(&format!("/s/{token}"), &[]).status, 200);
    // Another pull request of the same repository still asks for a sign in.
    assert_eq!(get("/owner/vault/pull/73", &[]).status, 302);
    let answer = get(&format!("/owner/vault/proofs/{other}/"), &[]);
    assert_eq!(answer.status, 302, "{}", answer.text());
    // The link's paths below it are nothing.
    assert_eq!(get(&format!("/s/{token}/index.html"), &[]).status, 404);
}

fn a_revoked_share_link_finds_nothing() {
    upload_proof("owner/vault", 74, &proof("owner/vault", 74));
    let session = sign_in("writer-code", "/");
    let token = share("owner/vault", 74, Some(&session)).json()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(get(&format!("/s/{token}"), &[]).status, 200);
    let answer = revoke("owner/vault", 74, &token, Some(&session));
    assert_eq!(answer.status, 200, "{}", answer.text());
    let answer = get(&format!("/s/{token}"), &[]);
    assert_eq!(answer.status, 404, "{}", answer.text());
    assert!(!answer.text().contains("vault"), "{}", answer.text());
    // Not even a writer's session brings it back.
    let answer = get(&format!("/s/{token}"), &[("Cookie", &session)]);
    assert_eq!(answer.status, 404, "{}", answer.text());
}

fn an_unknown_share_link_finds_nothing() {
    for token in ["0123456789abcdef0123456789abcdef", "nothing", ""] {
        let answer = get(&format!("/s/{token}"), &[]);
        assert_eq!(answer.status, 404, "{token}: {}", answer.text());
        assert!(!answer.text().contains("owner"), "{}", answer.text());
    }
}

fn sharing_needs_write_access() {
    upload_proof("owner/vault", 75, &proof("owner/vault", 75));
    let answer = share("owner/vault", 75, None);
    assert_eq!(answer.status, 401, "{}", answer.text());
    let reader = sign_in("reader-code", "/");
    let answer = share("owner/vault", 75, Some(&reader));
    assert_eq!(answer.status, 403, "{}", answer.text());
    let error = answer.json()["error"].as_str().unwrap().to_owned();
    assert!(error.contains("write access"), "{error}");
    // Someone GitHub shows nothing of the private repository learns nothing of it.
    let stranger = sign_in("stranger-code", "/");
    let answer = share("owner/vault", 75, Some(&stranger));
    assert_eq!(answer.status, 404, "{}", answer.text());
    // A pull request without a whole proof has nothing to share.
    let writer = sign_in("writer-code", "/");
    let answer = share("owner/vault", 76, Some(&writer));
    assert_eq!(answer.status, 404, "{}", answer.text());
    let answer = links("owner/vault", 75, Some(&writer));
    assert_eq!(answer.json()["links"], json!([]), "nothing was made");
}

fn sharing_from_another_site_is_refused() {
    upload_proof("owner/vault", 77, &proof("owner/vault", 77));
    let session = sign_in("writer-code", "/");
    let files = world().worker.files.clone();
    for origin in [Some("https://evil.test"), Some(files.as_str()), None] {
        let mut headers = vec![
            ("Cookie", session.as_str()),
            ("Content-Type", "application/json"),
        ];
        if let Some(origin) = origin {
            headers.push(("Origin", origin));
        }
        let path = "/owner/vault/pull/77/share";
        let answer = request("POST", path, &headers, Some(b"{}".to_vec()));
        assert_eq!(answer.status, 403, "{origin:?}: {}", answer.text());
    }
    let answer = links("owner/vault", 77, Some(&session));
    assert_eq!(answer.json()["links"], json!([]), "nothing was made");
}

fn share_links_are_listed_to_writers() {
    upload_proof("owner/vault", 78, &proof("owner/vault", 78));
    let session = sign_in("writer-code", "/");
    let made = || {
        share("owner/vault", 78, Some(&session)).json()["token"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let (kept, gone) = (made(), made());
    revoke("owner/vault", 78, &gone, Some(&session));
    let answer = links("owner/vault", 78, Some(&session));
    assert_eq!(answer.status, 200, "{}", answer.text());
    let listed = answer.json()["links"].as_array().unwrap().clone();
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert_eq!(listed[0]["token"].as_str().unwrap(), kept);
    assert_eq!(
        listed[0]["url"].as_str().unwrap(),
        format!("{}/s/{kept}", world().worker.url)
    );
    assert_eq!(listed[0]["made_by"], "writer");
    assert!(listed[0]["made_at"].as_i64().unwrap() > 0);
    let reader = sign_in("reader-code", "/");
    assert_eq!(links("owner/vault", 78, Some(&reader)).status, 403);
    assert_eq!(links("owner/vault", 78, None).status, 401);
}

fn revoking_needs_write_access_and_a_link_of_that_pull_request() {
    upload_proof("owner/vault", 79, &proof("owner/vault", 79));
    let writer = sign_in("writer-code", "/");
    let token = share("owner/vault", 79, Some(&writer)).json()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let reader = sign_in("reader-code", "/");
    assert_eq!(revoke("owner/vault", 79, &token, Some(&reader)).status, 403);
    assert_eq!(revoke("owner/vault", 79, &token, None).status, 401);
    // The token names another pull request, or another repository.
    assert_eq!(revoke("owner/vault", 80, &token, Some(&writer)).status, 404);
    assert_eq!(revoke("owner/team", 79, &token, Some(&writer)).status, 404);
    assert_eq!(get(&format!("/s/{token}"), &[]).status, 200, "still live");
}

fn the_shell_offers_share_to_writers_alone() {
    let id = upload_proof("owner/team", 81, &proof("owner/team", 81));
    let path = format!("/owner/team/proofs/{id}/");
    let writer = sign_in("writer-code", "/");
    let html = get(&path, &[("Cookie", &writer)]).text();
    assert!(html.contains("id=\"share\""), "{html}");
    assert!(html.contains("/owner/team/pull/81/share"), "{html}");
    let reader = sign_in("reader-code", "/");
    let html = get(&path, &[("Cookie", &reader)]).text();
    assert!(!html.contains("id=\"share\""), "{html}");
    let html = get(&path, &[]).text();
    assert!(!html.contains("id=\"share\""), "{html}");
}

fn a_share_link_view_offers_no_posting() {
    upload_proof("owner/team", 82, &waiting("owner/team", 82));
    let writer = sign_in("writer-code", "/");
    let token = share("owner/team", 82, Some(&writer)).json()["token"]
        .as_str()
        .unwrap()
        .to_owned();
    for session in [None, Some(writer.as_str())] {
        let headers: Vec<(&str, &str)> = session.iter().map(|s| ("Cookie", *s)).collect();
        let answer = get(&format!("/s/{token}"), &headers);
        assert_eq!(answer.status, 200, "{}", answer.text());
        let html = answer.text();
        // It answers its frame, but never posts and never offers to share.
        assert!(html.contains("e.source !== frame.contentWindow"), "{html}");
        assert!(!html.contains("fetch("), "{html}");
        assert!(!html.contains("id=\"share\""), "{html}");
        assert!(!html.contains("data-kind="), "no Approve or Accept: {html}");
        assert!(html.contains("share link"), "{html}");
        let csp = answer.header("content-security-policy").unwrap();
        assert!(csp.contains("connect-src 'none'"), "{csp}");
    }
}

// Helpers.

/// A proof whose pictures of `screens` wait for approval and whose weakening waits for an
/// admin's accept.
fn waiting(repository: &str, pull: u64) -> Value {
    let mut proof = proof(repository, pull);
    proof["looks"] = json!([{
        "check": "screens",
        "base_commit": "def5678",
        "base_result": "passed",
        "items": [
            {"name": "home.png", "kind": "still", "change": "changed", "summary": "changed", "flaky": false},
            {"name": "about.png", "kind": "still", "change": "same", "summary": "same", "flaky": false},
        ],
    }]);
    proof["weakened"] = json!([{"check": "tests", "how": "it runs another command"}]);
    proof["weakenings_wait"] = json!(true);
    proof
}

/// Approves or accepts a proof from its shell, as the shell's script asks: same origin, JSON,
/// with the token of the person's view of the shell when they may see it.
fn act(repository: &str, id: &str, kind: &str, session: Option<&str>, body: Value) -> Answer {
    let path = format!("/{repository}/proofs/{id}/{kind}");
    let origin = world().worker.url.clone();
    let mut headers = vec![
        ("Origin", origin.as_str()),
        ("Content-Type", "application/json"),
    ];
    let view = session.and_then(|s| view_token(repository, id, s));
    if let Some(session) = session {
        headers.push(("Cookie", session));
    }
    if let Some(view) = &view {
        headers.push(("Fearlezz-View", view));
    }
    request("POST", &path, &headers, Some(body.to_string().into_bytes()))
}

/// The token of a person's view of a proof's shell, which its script sends with a post; None
/// when the shell does not open for them.
fn view_token(repository: &str, id: &str, session: &str) -> Option<String> {
    let answer = get(
        &format!("/{repository}/proofs/{id}/"),
        &[("Cookie", session)],
    );
    if answer.status != 200 {
        return None;
    }
    let html = answer.text();
    let start = html.find("var VIEW = \"")? + "var VIEW = \"".len();
    let end = html[start..].find('"')?;
    Some(html[start..start + end].to_owned())
}

/// Makes a share link of a pull request, as the shell's script asks: same origin, JSON.
fn share(repository: &str, pull: u64, session: Option<&str>) -> Answer {
    post_json(&format!("/{repository}/pull/{pull}/share"), session)
}

/// Revokes a share link, as the shell's script asks.
fn revoke(repository: &str, pull: u64, token: &str, session: Option<&str>) -> Answer {
    post_json(
        &format!("/{repository}/pull/{pull}/share/{token}/revoke"),
        session,
    )
}

/// The live share links of a pull request.
fn links(repository: &str, pull: u64, session: Option<&str>) -> Answer {
    let headers: Vec<(&str, &str)> = session.iter().map(|s| ("Cookie", *s)).collect();
    get(&format!("/{repository}/pull/{pull}/share"), &headers)
}

fn post_json(path: &str, session: Option<&str>) -> Answer {
    let origin = world().worker.url.clone();
    let mut headers = vec![
        ("Origin", origin.as_str()),
        ("Content-Type", "application/json"),
    ];
    if let Some(session) = session {
        headers.push(("Cookie", session));
    }
    request("POST", path, &headers, Some(b"{}".to_vec()))
}

/// The claims Actions puts in a pull request run's token.
fn claims(repository: &str, pull: u64) -> Value {
    let now = now();
    json!({
        "iss": ISSUER,
        "aud": AUDIENCE,
        "sub": format!("repo:{repository}:pull_request"),
        "repository": repository,
        "repository_owner": repository.split('/').next().unwrap(),
        "ref": format!("refs/pull/{pull}/merge"),
        "event_name": "pull_request",
        "iat": now,
        "nbf": now,
        "exp": now + 300,
    })
}

fn ci(repository: &str, pull: u64) -> String {
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

/// A `proof.json` as CI writes it, with no checks or pictures.
fn proof(repository: &str, pull: u64) -> Value {
    json!({
        "schema": 1,
        "repository": repository,
        "pull": {"number": pull, "title": format!("Pull request {pull}")},
        "commit": "abc1234 merged into def5678",
        "head": "abc1234abc1234abc1234abc1234abc1234abc1",
        "base": "main",
        "checks": [],
        "weakened": [],
        "looks": [],
    })
}

fn upload_body(proof: &Value, files: &[(&str, &[u8])]) -> Vec<u8> {
    let files: serde_json::Map<String, Value> = files
        .iter()
        .map(|(path, bytes)| {
            let file = json!({"sha256": sha256(bytes), "size": bytes.len()});
            ((*path).to_owned(), file)
        })
        .collect();
    json!({"proof": proof, "files": files})
        .to_string()
        .into_bytes()
}

/// The whole upload as CI does it; the proof's id.
fn upload(repository: &str, pull: u64, files: &[(&str, &[u8])]) -> String {
    upload_with(repository, pull, &proof(repository, pull), files)
}

/// The upload of `proof` with a page of its own.
fn upload_proof(repository: &str, pull: u64, proof: &Value) -> String {
    let page = format!("<p>{repository} {pull} {}</p>", now());
    upload_with(repository, pull, proof, &[("index.html", page.as_bytes())])
}

fn upload_with(repository: &str, pull: u64, proof: &Value, files: &[(&str, &[u8])]) -> String {
    let id = start_upload_with(repository, pull, proof, files);
    finish_upload(repository, pull, &id);
    id
}

/// An upload with every file sent but not yet done, with `page` as its page; the proof's id.
fn start_upload(repository: &str, pull: u64, proof: &Value, page: &[u8]) -> String {
    start_upload_with(repository, pull, proof, &[("index.html", page)])
}

fn start_upload_with(
    repository: &str,
    pull: u64,
    proof: &Value,
    files: &[(&str, &[u8])],
) -> String {
    let token = ci(repository, pull);
    let body = upload_body(proof, files);
    let answer = api("POST", "/api/v1/proofs", &token, Some(body));
    assert_eq!(answer.status, 201, "{}", answer.text());
    let created = answer.json();
    let id = created["id"].as_str().unwrap().to_owned();
    assert_eq!(
        created["url"].as_str().unwrap(),
        format!("{}/{repository}/proofs/{id}/", world().worker.url)
    );
    for missing in created["missing"].as_array().unwrap() {
        let missing = missing.as_str().unwrap();
        let (_, bytes) = files.iter().find(|(_, b)| sha256(b) == missing).unwrap();
        let answer = api(
            "PUT",
            &format!("/api/v1/blobs/{missing}"),
            &token,
            Some(bytes.to_vec()),
        );
        assert_eq!(answer.status, 204, "{}", answer.text());
    }
    id
}

/// Marks an upload done, as CI does once every file is sent.
fn finish_upload(repository: &str, pull: u64, id: &str) {
    let token = ci(repository, pull);
    let answer = api("POST", &format!("/api/v1/proofs/{id}/done"), &token, None);
    assert_eq!(answer.status, 200, "{}", answer.text());
    let url = format!("{}/{repository}/proofs/{id}/", world().worker.url);
    assert_eq!(answer.json()["url"], json!(url));
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

/// The `src` of the frame in a proof's shell page.
fn frame_url(shell: &Answer) -> String {
    let html = shell.text();
    let frame = &html[html
        .find("<iframe")
        .unwrap_or_else(|| panic!("no frame: {html}"))..];
    attribute(frame, "src")
        .unwrap_or_else(|| panic!("no src: {html}"))
        .replace("&amp;", "&")
}

/// The value of the first `name="..."` attribute in some HTML.
fn attribute(html: &str, name: &str) -> Option<String> {
    let start = html.find(&format!(" {name}=\""))? + name.len() + 3;
    let end = html[start..].find('"')?;
    Some(html[start..start + end].to_owned())
}

/// The folder of a proof on the files origin under a grant sealed here with the host's key, as
/// the host seals them: `base64url(json).base64url(HMAC-SHA256("grant." + payload))`. `changes`
/// overrides its claims.
fn files_base(repo: &str, proof: &str, changes: &[(&str, Value)]) -> String {
    use hmac::Mac as _;
    let mut claims = json!({"repo": repo, "proof": proof, "exp": now() + 3600});
    for (name, value) in changes {
        claims[*name] = value.clone();
    }
    let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
    let mut mac = hmac::Hmac::<Sha256>::new_from_slice(SIGNING_KEY.as_bytes()).unwrap();
    mac.update(format!("grant.{payload}").as_bytes());
    let mac = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    format!("{}/{payload}.{mac}/", world().worker.files)
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
