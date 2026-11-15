//! Share links: a person with write access makes a link that shows anyone a work's live page,
//! view only, until someone with write access revokes it. A link names the work, so it follows
//! new runs; it only views, gets the files of that work's runs alone, sends no referrer and is
//! not indexed. The host keeps only the token's HMAC, so the list shows who made each link and
//! when, never the link again.

use super::*;

/// What the page sends to the share API at `path`: JSON from the app origin with the session
/// `cookie`, and `headers` besides or instead.
fn post(path: &str, cookie: Option<&str>, headers: &[(&str, &str)], body: &Value) -> Answer {
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
    request("POST", path, &all, Some(body.to_string().into_bytes()))
}

/// Asks for a link to the work of `branch` of `owner/shared`.
fn make(cookie: Option<&str>, headers: &[(&str, &str)], branch: &str) -> Answer {
    let body = json!({"repository": "owner/shared", "branch": branch});
    post("/api/v2/shares", cookie, headers, &body)
}

/// A link to the work of `branch` made by `writer`: its path on the app origin and its id.
fn link(writer: &str, branch: &str) -> (String, String) {
    let answer = make(Some(writer), &[], branch);
    assert_eq!(answer.status, 201, "{}", answer.text());
    let made = answer.json();
    let url = made["url"].as_str().unwrap();
    let path = url
        .strip_prefix(&world().worker.url)
        .unwrap_or_else(|| panic!("{url} is not on the app origin"))
        .to_owned();
    assert!(path.starts_with("/-/share/"), "{path}");
    (path, made["share"]["id"].as_str().unwrap().to_owned())
}

fn token(path: &str) -> &str {
    path.rsplit('/').next().unwrap()
}

/// The live links of the work of `branch`, as `cookie` sees them.
fn list(cookie: Option<&str>, branch: &str) -> Answer {
    let mut headers = Vec::new();
    if let Some(cookie) = cookie {
        headers.push(("Cookie", cookie));
    }
    get(
        &format!("/api/v2/shares?repository=owner/shared&branch={branch}"),
        &headers,
    )
}

fn revoke(cookie: Option<&str>, id: &str) -> Answer {
    post(
        &format!("/api/v2/shares/{id}/revoke"),
        cookie,
        &[],
        &json!({}),
    )
}

fn grant(run: &str, share: Option<&str>) -> Answer {
    match share {
        Some(token) => get(&format!("/api/v2/runs/{run}/grant?share={token}"), &[]),
        None => get(&format!("/api/v2/runs/{run}/grant"), &[]),
    }
}

/// A finished CI run of pull request `number` of `owner/shared` with the pictures of `shots`.
fn shared_with_pictures(number: u64, branch: &str) -> (Job, String) {
    let job = Job::pull("owner/shared", number, branch).at(&commit(&format!("s{number}")), number);
    let run = job.open(&["screens"]);
    run_with_pictures(&job, &run.id);
    let shots = shots();
    let files: Vec<(&str, &[u8])> = shots
        .files
        .iter()
        .map(|(p, b)| (p.as_str(), b.as_slice()))
        .collect();
    let answer = job.close(
        &run.id,
        &[entry("screens", "passed")],
        std::slice::from_ref(&shots.look),
        &files,
    );
    assert_eq!(answer.status, 200, "{}", answer.text());
    (job, run.id)
}

/// Whether the page shows the picture on its stage and says it only views.
const VIEWS: &str =
    "!!document.querySelector('[data-shared]') && !!document.querySelector('[data-stage]')";

pub fn a_share_link_shows_the_live_work_and_follows_a_new_run() {
    let (job, first) = shared_with_pictures(301, "share/live");
    let writer = sign_in("writer-code", "/");
    let (path, _) = link(&writer, "share/live");

    // Anyone with the link, without a session, though the repository is private.
    let answer = navigate(&path, &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.header("referrer-policy"), Some("no-referrer"));
    assert!(
        answer
            .header("x-robots-tag")
            .is_some_and(|t| t.contains("noindex")),
        "{:?}",
        answer.headers
    );
    assert_eq!(answer.header("cache-control"), Some("no-store"));
    let snapshot = live(&path, &[]);
    assert_eq!(snapshot.status, 200, "{}", snapshot.text());
    let snapshot = snapshot.json();
    assert_eq!(snapshot["work"]["pull"]["number"], 301);
    assert_eq!(snapshot["runs"][0]["id"], first.as_str());

    // The files of the work's runs, under a fresh grant; nothing of another work.
    let answer = grant(&first, Some(token(&path)));
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert!(
        answer.json()["files"]
            .as_str()
            .unwrap()
            .starts_with(&world().worker.files)
    );
    assert_eq!(grant(&first, None).status, 401);
    let other = Job::push("owner/shared", "share/elsewhere").open(&["tests"]);
    let answer = grant(&other.id, Some(token(&path)));
    assert_eq!(answer.status, 404, "{}", answer.text());

    // The page, live: a new run of the work shows without a reload.
    let mut browser = browser::Browser::start();
    browser.open(&format!("{}{path}", world().worker.url));
    browser.wait_for(VIEWS);
    browser.wait_for(&loaded("img", "after"));
    assert_eq!(
        browser.eval("document.querySelector('[data-shared]').textContent.includes('only views')"),
        json!(true)
    );
    let next = job.at(&commit("s301b"), 302).open(&["screens"]);
    browser.wait_for(&format!(
        r#"!!document.querySelector('section.run[data-run="{}"]')"#,
        next.id
    ));
    assert_eq!(live(&path, &[]).json()["runs"][0]["id"], next.id.as_str());
}

pub fn a_share_link_never_writes() {
    let (_, run) = shared_with_pictures(303, "share/views");
    let writer = sign_in("writer-code", "/");
    let (path, _) = link(&writer, "share/views");
    let bearer = format!("Bearer {}", token(&path));
    // The link is no session and no token: every write still needs a person signed in.
    for (what, body) in [
        ("verdicts", looks_right("card/light.png")),
        (
            "notes",
            super::notes::ask("card/light.png", "after", [0.1, 0.1, 0.2, 0.2], "Too dark."),
        ),
        ("accept", json!({})),
    ] {
        let answer = say(&run, what, None, &[], &body);
        assert_eq!(answer.status, 401, "{what}: {}", answer.text());
        let answer = say(&run, what, None, &[("Authorization", &bearer)], &body);
        assert_eq!(answer.status, 403, "{what}: {}", answer.text());
    }
    assert_eq!(make(None, &[], "share/views").status, 401);

    // Through the link the page offers nothing to press, even to a signed in writer.
    let mut browser = browser::Browser::start();
    browser.sign_in(&world().worker.url, &writer);
    browser.open(&format!("{}{path}", world().worker.url));
    browser.wait_for(VIEWS);
    browser.wait_for(&loaded("img", "after"));
    assert_eq!(
        browser.eval(
            "!!document.querySelector('button[data-say], button[data-accept], [data-whole], [data-resolve], [data-share], a[data-sign-in]')"
        ),
        json!(false)
    );
}

pub fn a_revoked_share_link_is_not_found_and_names_no_repository() {
    let (_, run) = shared_with_pictures(304, "share/revoked");
    let writer = sign_in("writer-code", "/");
    let (path, id) = link(&writer, "share/revoked");
    let mut browser = browser::Browser::start();
    browser.open(&format!("{}{path}", world().worker.url));
    browser.wait_for(VIEWS);

    let answer = revoke(Some(&writer), &id);
    assert_eq!(answer.status, 200, "{}", answer.text());
    // A page open on the link stops showing the work.
    browser.wait_for(
        "!document.querySelector('[data-stage]') && document.body.textContent.includes('revoked')",
    );
    for answer in [
        navigate(&path, &[]),
        live(&path, &[]),
        grant(&run, Some(token(&path))),
    ] {
        assert_eq!(answer.status, 404, "{}", answer.text());
        assert!(!answer.text().contains("shared"), "{}", answer.text());
    }
    assert_eq!(revoke(Some(&writer), &id).status, 404);
}

pub fn a_writer_makes_lists_and_revokes_share_links_and_a_reader_cannot() {
    Job::push("owner/shared", "share/roles").open(&["tests"]);
    let writer = sign_in("writer-code", "/");
    let reader = sign_in("reader-code", "/");
    let stranger = sign_in("stranger-code", "/");

    // Making one needs a session with write access, from this site, as JSON, never a token.
    assert_eq!(make(None, &[], "share/roles").status, 401);
    assert_eq!(make(Some(&reader), &[], "share/roles").status, 403);
    assert_eq!(make(Some(&stranger), &[], "share/roles").status, 404);
    assert_eq!(
        make(
            Some(&writer),
            &[("Origin", "https://elsewhere.example")],
            "share/roles"
        )
        .status,
        403
    );
    assert_eq!(
        make(
            Some(&writer),
            &[("Content-Type", "text/plain")],
            "share/roles"
        )
        .status,
        415
    );
    assert_eq!(
        make(
            Some(&writer),
            &[("Authorization", "Bearer x")],
            "share/roles"
        )
        .status,
        403
    );
    assert_eq!(make(Some(&writer), &[], "share/nothing").status, 404);

    let (first, first_id) = link(&writer, "share/roles");
    let (second, second_id) = link(&writer, "share/roles");
    assert_eq!(list(None, "share/roles").status, 401);
    assert_eq!(list(Some(&reader), "share/roles").status, 403);
    assert_eq!(list(Some(&stranger), "share/roles").status, 404);
    let listed = list(Some(&writer), "share/roles");
    assert_eq!(listed.status, 200, "{}", listed.text());
    // Who made each and when, newest first; the link itself is shown only once.
    let shares = listed.json()["shares"].clone();
    let ids: Vec<&str> = shares
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [second_id.as_str(), first_id.as_str()]);
    assert_eq!(shares[0]["made_by"], "writer");
    assert!(shares[0]["made_at"].as_f64().is_some(), "{shares}");
    for path in [&first, &second] {
        assert!(!listed.text().contains(token(path)), "{}", listed.text());
    }

    assert_eq!(revoke(Some(&reader), &first_id).status, 403);
    assert_eq!(revoke(Some(&stranger), &first_id).status, 404);
    assert_eq!(revoke(None, &first_id).status, 401);
    let answer = revoke(Some(&writer), &first_id);
    assert_eq!(answer.status, 200, "{}", answer.text());
    let left = list(Some(&writer), "share/roles").json()["shares"].clone();
    assert_eq!(left.as_array().unwrap().len(), 1, "{left}");
    assert_eq!(navigate(&first, &[]).status, 404);
    assert_eq!(navigate(&second, &[]).status, 200);

    // On the work's own page a writer finds the control; a reader does not.
    let url = format!("{}/owner/shared/work/share/roles", world().worker.url);
    let mut browser = browser::Browser::start();
    browser.sign_in(&world().worker.url, &writer);
    browser.open(&url);
    browser.wait_for("!!document.querySelector('button[data-share]')");
    browser.eval("document.querySelector('button[data-share]').click()");
    browser.wait_for("document.querySelectorAll('[data-shares] [data-revoke]').length === 1");
    browser.eval("document.querySelector('button[data-make-share]').click()");
    browser.wait_for(&format!(
        r#"(document.querySelector('[data-made] input')?.value ?? '').startsWith("{}/-/share/")"#,
        world().worker.url
    ));
    browser.wait_for("document.querySelectorAll('[data-shares] [data-revoke]').length === 2");
    browser.eval("document.querySelector('[data-shares] [data-revoke]').click()");
    browser.wait_for("document.querySelectorAll('[data-shares] [data-revoke]').length === 1");
    let mut reading = browser::Browser::start();
    reading.sign_in(&world().worker.url, &reader);
    reading.open(&url);
    reading.wait_for("!!document.querySelector('[data-standing]')");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        reading.eval("!!document.querySelector('button[data-share]')"),
        json!(false)
    );
}

/// Pictures of a share view, light and dark, wide and at phone width. Part of the ignored
/// `pictures`.
pub fn pictures(shoot: &dyn Fn(&mut browser::Browser, &str)) {
    shared_with_pictures(390, "share/shots");
    let writer = sign_in("writer-code", "/");
    let (path, _) = link(&writer, "share/shots");
    let mut browser = browser::Browser::start();
    browser.open(&format!("{}{path}", world().worker.url));
    for (view, (width, height, mobile)) in
        [("wide", (1280, 900, false)), ("phone", (390, 844, true))]
    {
        for dark in [false, true] {
            browser.view(width, height, mobile, dark);
            browser.wait_for(VIEWS);
            browser.wait_for(&loaded("img", "after"));
            let scheme = if dark { "dark" } else { "light" };
            shoot(&mut browser, &format!("share-{view}-{scheme}"));
        }
    }
    // A writer's share control, open with a link just made.
    let mut writing = browser::Browser::start();
    writing.sign_in(&world().worker.url, &writer);
    writing.open(&format!("{}/owner/shared/pull/390", world().worker.url));
    writing.wait_for("!!document.querySelector('button[data-share]')");
    writing.eval("document.querySelector('button[data-share]').click()");
    writing.eval("document.querySelector('button[data-make-share]').click()");
    writing.wait_for("!!document.querySelector('[data-made] input')");
    for dark in [false, true] {
        writing.view(1280, 900, false, dark);
        let scheme = if dark { "dark" } else { "light" };
        shoot(&mut writing, &format!("share-control-{scheme}"));
    }
}
