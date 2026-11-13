//! The proof host, driven over HTTP as CI and people use it: the upload protocol with its OIDC
//! check, the pages with their files and ranges, and who may see them. One worker under
//! `wrangler dev --local` and one fake GitHub serve every test; each test uses its own pull
//! request numbers and file bytes, so they run side by side.

// Beside this file, not in tests/, where cargo would take each for a test of its own.
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
use worker::{AUDIENCE, INSTALL_URL, ISSUER, Worker};

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
        World {
            worker,
            github,
            oidc,
            stranger,
        }
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
            "a_proof_shows_nothing_before_done",
            a_proof_shows_nothing_before_done,
        ),
        (
            "pull_leads_to_the_latest_whole_proof",
            pull_leads_to_the_latest_whole_proof,
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
    ];
    let trials = tests
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

    let answer = get(&folder, &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.body, page);
    assert!(
        answer
            .header("content-type")
            .unwrap()
            .starts_with("text/html")
    );

    let answer = get(&format!("{folder}media/after.png"), &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.body, picture);
    assert_eq!(answer.header("content-type").unwrap(), "image/png");

    assert_eq!(get(&format!("{folder}media/none.png"), &[]).status, 404);

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
    let path = format!("/owner/repo/proofs/{id}/media/run.mp4");

    let answer = get(&path, &[("Range", "bytes=2-5")]);
    assert_eq!(answer.status, 206, "{}", answer.text());
    assert_eq!(answer.body, b"2345");
    assert_eq!(
        answer.header("content-range").unwrap(),
        format!("bytes 2-5/{}", film.len())
    );
    assert_eq!(answer.header("content-type").unwrap(), "video/mp4");

    let answer = get(&path, &[("Range", "bytes=-5")]);
    assert_eq!(answer.status, 206, "{}", answer.text());
    assert_eq!(answer.body, &film[film.len() - 5..]);

    let answer = get(&path, &[("Range", "bytes=40-")]);
    assert_eq!(answer.status, 206, "{}", answer.text());
    assert_eq!(answer.body, &film[40..]);

    let answer = get(&path, &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.header("accept-ranges").unwrap(), "bytes");
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
    let session = sign_in("reader-code", "/");
    let answer = get("/logout", &[("Cookie", &session)]);
    assert_eq!(answer.status, 302, "{}", answer.text());
    let cleared = cookie_set(&answer, "session").unwrap();
    assert!(cleared.contains("Max-Age=0"), "{cleared}");
}

// Helpers.

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
    let header = json!({"alg": "RS256", "typ": "JWT", "kid": github::KID});
    let signed = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    let signature = SigningKey::<Sha256>::new(key.clone()).sign(signed.as_bytes());
    format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature.to_bytes()))
}

/// A `proof.json` as CI writes it, cut to what the host reads.
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
    let token = ci(repository, pull);
    let body = upload_body(&proof(repository, pull), files);
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
    let answer = api("POST", &format!("/api/v1/proofs/{id}/done"), &token, None);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.json()["url"], created["url"]);
    id
}

/// Signs in with a GitHub OAuth code, starting from `next`; the session cookie.
fn sign_in(code: &str, next: &str) -> String {
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
    cookie(&answer, "session").unwrap()
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
    let url = format!("{}{path}", world().worker.url);
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

fn sha256(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
