//! Who may do what on the host, one viewer kind at a time: someone anonymous, someone holding a
//! share link, a person signed in by session, and a person's CLI token. Each reads a private
//! repository's pages, a run and a file, and says a verdict, accepts and writes notes, and gets
//! the status and words the host has always answered. `owner/policy` is private and no other
//! test uses it; each `lapsed-*` login signs in and then loses their GitHub token, one per test
//! since GitHub hands a login the same token at each sign in.

use super::*;

const REPOSITORY: &str = "owner/policy";

/// A finished CI run of pull request `number` of `owner/policy`, with one changed still and no
/// weakening; the run's id.
fn policy_run(number: u64, branch: &str) -> String {
    let job = Job::pull(REPOSITORY, number, branch).at(&commit(&format!("p{number}")), number);
    let (look, files) = one_still(
        format!("policy {number} before").as_bytes(),
        format!("policy {number} after").as_bytes(),
    );
    proven(&job, &look, &files, &json!({}))
}

/// The error a JSON refusal says.
fn error(answer: &Answer) -> String {
    answer.json()["error"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// Asserts `answer` is the refusal `status` saying `words`, as JSON.
fn refused(answer: &Answer, status: u16, words: &str, what: &str) {
    assert_eq!(answer.status, status, "{what}: {}", answer.text());
    assert_eq!(error(answer), words, "{what}");
}

/// Asserts a navigation was sent to sign in and back to `path`.
fn to_sign_in(answer: &Answer, path: &str, what: &str) {
    assert_eq!(answer.status, 302, "{what}: {}", answer.text());
    let next: String = path.replace('/', "%2F");
    assert_eq!(
        answer.header("location"),
        Some(format!("/login?next={next}").as_str()),
        "{what}"
    );
}

/// A session of `lapsed-<test>` whose GitHub token GitHub no longer takes.
fn lapsed_session(test: &str) -> String {
    let session = sign_in(&format!("lapsed-{test}-code"), "/");
    world().github.forget(&format!("person-lapsed-{test}"));
    session
}

fn grant(run: &str, headers: &[(&str, &str)]) -> Answer {
    get(&format!("/api/v2/runs/{run}/grant"), headers)
}

/// A link to the work of `branch` of `owner/policy`, made by a writer: its token.
fn share_token(branch: &str) -> String {
    let writer = sign_in("writer-code", "/");
    let body = json!({"repository": REPOSITORY, "branch": branch});
    let answer = say_to("/api/v2/shares", Some(&writer), &[], &body);
    assert_eq!(answer.status, 201, "{}", answer.text());
    let url = answer.json()["url"].as_str().unwrap_or_default().to_owned();
    let token = url.rsplit('/').next().unwrap_or_default().to_owned();
    assert!(!token.is_empty(), "a share link holds its token: {url}");
    token
}

/// What the page posts to `path`: JSON from the app origin with the session `cookie`, and
/// `headers` besides.
fn say_to(path: &str, cookie: Option<&str>, headers: &[(&str, &str)], body: &Value) -> Answer {
    let origin = world().worker.url.clone();
    let mut all: Vec<(&str, &str)> =
        vec![("Origin", &origin), ("Content-Type", "application/json")];
    all.extend_from_slice(headers);
    if let Some(cookie) = cookie {
        all.push(("Cookie", cookie));
    }
    request("POST", path, &all, Some(body.to_string().into_bytes()))
}

pub(crate) fn the_policy_decides_who_reads_a_private_repository() {
    let run = policy_run(1, "policy/read");
    let other = policy_run(2, "policy/other");
    let page = "/owner/policy/pull/1";
    let sign_in_to_see = "sign in to see this work";

    // Anonymous: sent to sign in, and every API says so.
    to_sign_in(&navigate(page, &[]), page, "anonymous page");
    to_sign_in(
        &navigate("/owner/policy", &[]),
        "/owner/policy",
        "anonymous repository page",
    );
    refused(&live(page, &[]), 401, sign_in_to_see, "anonymous live");
    refused(
        &get("/live/owner/policy", &[]),
        401,
        "sign in to see this repository",
        "anonymous repository live",
    );
    refused(&grant(&run, &[]), 401, sign_in_to_see, "anonymous grant");
    refused(
        &get("/api/v2/viewer/owner/policy", &[]),
        401,
        sign_in_to_see,
        "anonymous viewer",
    );

    // A CLI token opens no page: it is no session.
    let cli = format!("Bearer {}", super::notes::cli_token("reader"));
    let bearer = [("Authorization", cli.as_str())];
    to_sign_in(&navigate(page, &bearer), page, "CLI page");
    refused(&live(page, &bearer), 401, sign_in_to_see, "CLI live");
    refused(&grant(&run, &bearer), 401, sign_in_to_see, "CLI grant");

    // A share link opens its own work's runs and live page, and nothing else.
    let token = share_token("policy/read");
    let shared = get(&format!("/api/v2/runs/{run}/grant?share={token}"), &[]);
    assert_eq!(shared.status, 200, "share grant: {}", shared.text());
    refused(
        &get(&format!("/api/v2/runs/{other}/grant?share={token}"), &[]),
        404,
        "not found",
        "share grant of another work",
    );
    refused(
        &get(&format!("/api/v2/runs/{run}/grant?share=nothing"), &[]),
        404,
        "not found",
        "unknown share grant",
    );
    let shared = get(&format!("/live/-/share/{token}"), &[]);
    assert_eq!(shared.status, 200, "share live: {}", shared.text());
    assert_eq!(
        navigate(&format!("/-/share/{token}"), &[]).status,
        200,
        "share page"
    );
    to_sign_in(&navigate(page, &[]), page, "a share link opens no page");

    // A person GitHub shows nothing of the repository: it is not there.
    let stranger = sign_in("stranger-code", "/");
    let cookie = [("Cookie", stranger.as_str())];
    let answer = navigate(page, &cookie);
    assert_eq!(answer.status, 404, "stranger page: {}", answer.text());
    assert_eq!(
        answer.text(),
        "not found",
        "stranger page says nothing more"
    );
    refused(&live(page, &cookie), 404, "not found", "stranger live");
    refused(&grant(&run, &cookie), 404, "not found", "stranger grant");
    refused(
        &get("/api/v2/viewer/owner/policy", &cookie),
        404,
        "not found",
        "stranger viewer",
    );
    assert_eq!(
        navigate("/owner/policy", &cookie).status,
        404,
        "stranger repository page"
    );

    // A reader sees it all.
    let reader = sign_in("reader-code", "/");
    let cookie = [("Cookie", reader.as_str())];
    assert_eq!(navigate(page, &cookie).status, 200, "reader page");
    assert_eq!(
        navigate("/owner/policy", &cookie).status,
        200,
        "reader repository page"
    );
    assert_eq!(live(page, &cookie).status, 200, "reader live");
    assert_eq!(
        get("/live/owner/policy", &cookie).status,
        200,
        "reader repository live"
    );
    assert_eq!(grant(&run, &cookie).status, 200, "reader grant");
    let viewer = get("/api/v2/viewer/owner/policy", &cookie);
    assert_eq!(
        viewer.json(),
        json!({"login": "reader", "role": "read"}),
        "reader viewer"
    );
    let writer = sign_in("writer-code", "/");
    let viewer = get("/api/v2/viewer/owner/policy", &[("Cookie", &writer)]);
    assert_eq!(
        viewer.json(),
        json!({"login": "writer", "role": "write"}),
        "writer viewer"
    );

    // A session whose GitHub token lapsed is someone anonymous.
    let lapsed = lapsed_session("read");
    let cookie = [("Cookie", lapsed.as_str())];
    to_sign_in(&navigate(page, &cookie), page, "lapsed page");
    refused(&live(page, &cookie), 401, sign_in_to_see, "lapsed live");
    refused(&grant(&run, &cookie), 401, sign_in_to_see, "lapsed grant");
}

pub(crate) fn the_policy_decides_who_reads_a_file_with_the_cli() {
    let run = policy_run(3, "policy/file");
    let file = format!("/api/v2/runs/{run}/files/looks/screens/after/card/light.png");
    let hint = "sign in to the proof host: fearlezz setup login";

    refused(&get(&file, &[]), 401, hint, "no token");
    let reader = sign_in("reader-code", "/");
    refused(
        &get(&file, &[("Cookie", &reader)]),
        401,
        hint,
        "a session is no CLI token",
    );
    refused(
        &api("GET", &file, "fzh_nothing", None),
        401,
        hint,
        "an unknown token",
    );
    let stranger = super::notes::cli_token("stranger");
    refused(
        &api("GET", &file, &stranger, None),
        404,
        "not found",
        "stranger",
    );
    let reader = super::notes::cli_token("reader");
    let answer = api("GET", &file, &reader, None);
    assert_eq!(answer.status, 200, "reader: {}", answer.text());
    assert_eq!(
        answer.header("content-type"),
        Some("image/png"),
        "reader file type"
    );
    assert_eq!(answer.body, b"policy 3 after", "reader file bytes");
    assert_eq!(
        answer.header("content-disposition"),
        Some("attachment"),
        "reader file is a download"
    );
    refused(
        &api("GET", "/api/v2/runs/nothing/files/a.png", &reader, None),
        404,
        "not found",
        "an unknown run",
    );
    refused(
        &api(
            "GET",
            &format!("/api/v2/runs/{run}/files/looks/screens/after/card/light.html"),
            &reader,
            None,
        ),
        404,
        "not found",
        "a type never served",
    );

    // What waits on a work reads the same way.
    let answer = super::notes::feedback(&reader, REPOSITORY, "policy/file");
    assert_eq!(answer.status, 200, "{}", answer.text());
    let answer = super::notes::feedback(&stranger, REPOSITORY, "policy/file");
    refused(&answer, 404, "not found", "stranger feedback");
    let answer = get("/api/v2/work/owner/policy/policy/file", &[]);
    refused(&answer, 401, hint, "feedback without a token");

    // Sending a local run needs write access.
    let installed = "/api/v2/installed?repository=owner/policy";
    refused(
        &api("GET", installed, &reader, None),
        403,
        "@reader cannot send local runs on owner/policy: that needs write access on GitHub",
        "a reader sends no local run",
    );
    let writer = super::notes::cli_token("writer");
    let answer = api("GET", installed, &writer, None);
    assert_eq!(
        answer.json(),
        json!({"installed": true}),
        "writer sends local runs"
    );
}

pub(crate) fn the_policy_decides_who_says_a_verdict_or_accepts() {
    let run = policy_run(4, "policy/say");
    let verdict = looks_right("card/light.png");
    let cli = format!("Bearer {}", super::notes::cli_token("writer"));
    let stranger = sign_in("stranger-code", "/");
    let reader = sign_in("reader-code", "/");
    let writer = sign_in("writer-code", "/");
    let admin = sign_in("admin-code", "/");
    let lapsed = lapsed_session("say");

    refused(
        &say(&run, "verdicts", None, &[], &verdict),
        401,
        "sign in to give a verdict",
        "anonymous verdict",
    );
    refused(
        &say(&run, "verdicts", None, &[("Authorization", &cli)], &verdict),
        403,
        "refused: only a person on the work's page can give a verdict, never a token",
        "CLI verdict",
    );
    refused(
        &say(&run, "verdicts", Some(&stranger), &[], &verdict),
        404,
        "not found",
        "stranger verdict",
    );
    refused(
        &say(&run, "verdicts", Some(&lapsed), &[], &verdict),
        404,
        "not found",
        "lapsed verdict",
    );
    refused(
        &say(&run, "verdicts", Some(&reader), &[], &verdict),
        403,
        "@reader cannot give verdicts on owner/policy: that needs write access on GitHub",
        "reader verdict",
    );
    let answer = say(&run, "verdicts", Some(&writer), &[], &verdict);
    assert_eq!(answer.status, 201, "writer verdict: {}", answer.text());

    refused(
        &say(&run, "accept", None, &[], &json!({})),
        401,
        "sign in to accept a weakened proof",
        "anonymous accept",
    );
    refused(
        &say(&run, "accept", None, &[("Authorization", &cli)], &json!({})),
        403,
        "refused: only a person on the work's page can accept a weakened proof, never a token",
        "CLI accept",
    );
    refused(
        &say(&run, "accept", Some(&stranger), &[], &json!({})),
        404,
        "not found",
        "stranger accept",
    );
    refused(
        &say(&run, "accept", Some(&writer), &[], &json!({})),
        403,
        "@writer cannot accept a weakened proof on owner/policy: that needs admin rights on GitHub",
        "writer accept",
    );
    refused(
        &say(&run, "accept", Some(&admin), &[], &json!({})),
        409,
        "this run weakens nothing to accept",
        "admin accept of a run that weakens nothing",
    );
    refused(
        &say("nothing", "accept", Some(&admin), &[], &json!({})),
        404,
        "not found",
        "accept of an unknown run",
    );
}

pub(crate) fn the_policy_decides_who_writes_notes() {
    let run = policy_run(5, "policy/notes");
    let note = super::notes::ask("card/light.png", "after", [0.1, 0.1, 0.2, 0.2], "Too dark.");
    let cli = format!("Bearer {}", super::notes::cli_token("writer"));
    let stranger = sign_in("stranger-code", "/");
    let reader = sign_in("reader-code", "/");
    let writer = sign_in("writer-code", "/");
    let lapsed = lapsed_session("notes");

    refused(
        &say(&run, "notes", None, &[], &note),
        401,
        "sign in to make a note",
        "anonymous note",
    );
    refused(
        &say(&run, "notes", None, &[("Authorization", &cli)], &note),
        403,
        "refused: only a person on the work's page can make a note, never a token",
        "CLI note",
    );
    refused(
        &say(&run, "notes", Some(&stranger), &[], &note),
        404,
        "not found",
        "stranger note",
    );
    refused(
        &say(&run, "notes", Some(&lapsed), &[], &note),
        404,
        "not found",
        "lapsed note",
    );
    refused(
        &say(&run, "notes", Some(&reader), &[], &note),
        403,
        "@reader cannot make notes on owner/policy: that needs write access on GitHub",
        "reader note",
    );
    let answer = say(&run, "notes", Some(&writer), &[], &note);
    assert_eq!(answer.status, 201, "writer note: {}", answer.text());
    let id = answer.json()["note"]["id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!id.is_empty(), "a note has its id");

    let resolve = format!("/api/v2/notes/{id}/resolve");
    refused(
        &say_to(&resolve, None, &[], &json!({})),
        401,
        "sign in to resolve a note",
        "anonymous resolve",
    );
    refused(
        &say_to(&resolve, None, &[("Authorization", &cli)], &json!({})),
        403,
        "refused: only a person on the work's page can resolve a note, never a token",
        "CLI resolve",
    );
    refused(
        &say_to(&resolve, Some(&stranger), &[], &json!({})),
        404,
        "not found",
        "stranger resolve",
    );
    refused(
        &say_to(&resolve, Some(&reader), &[], &json!({})),
        403,
        "@reader cannot resolve notes on owner/policy: that needs write access on GitHub",
        "reader resolve",
    );
    let answer = say_to(&resolve, Some(&writer), &[], &json!({}));
    assert_eq!(answer.status, 200, "writer resolve: {}", answer.text());
    refused(
        &say_to(
            "/api/v2/notes/nothing/resolve",
            Some(&writer),
            &[],
            &json!({}),
        ),
        404,
        "not found",
        "an unknown note",
    );

    // Share links follow the same rule: a writer, never a reader or a stranger.
    let shares = "/api/v2/shares?repository=owner/policy&branch=policy/notes";
    refused(
        &get(shares, &[]),
        401,
        "sign in to see the share links",
        "anonymous list",
    );
    refused(
        &get(shares, &[("Cookie", &stranger)]),
        404,
        "not found",
        "stranger list",
    );
    refused(
        &get(shares, &[("Cookie", &reader)]),
        403,
        "@reader cannot share a work on owner/policy: that needs write access on GitHub",
        "reader list",
    );
    assert_eq!(
        get(shares, &[("Cookie", &writer)]).status,
        200,
        "writer list"
    );
}
