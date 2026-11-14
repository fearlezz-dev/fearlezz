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
            "sign_in_refuses_a_state_it_did_not_give",
            sign_in_refuses_a_state_it_did_not_give,
        ),
        (
            "the_session_cookie_holds_only_a_random_id",
            the_session_cookie_holds_only_a_random_id,
        ),
        (
            "an_unknown_key_id_refetches_the_jwks_once",
            an_unknown_key_id_refetches_the_jwks_once,
        ),
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

// Signing in.

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
