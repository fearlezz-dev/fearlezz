//! Notes and the feedback loop: a person marks what should change on a picture, which then
//! needs change until its notes are resolved; the agent reads the notes with its CLI token, which the CLI gets through
//! GitHub's device flow run by the host and which can never write; open notes follow the work
//! to later runs until a person resolves them.

use super::*;

/// What the page sends to make a note on `picture` of check `screens`.
pub fn ask(picture: &str, side: &str, region: [f64; 4], words: &str) -> Value {
    let [x, y, w, h] = region;
    json!({
        "check": "screens", "picture": picture, "side": side,
        "region": {"x": x, "y": y, "w": w, "h": h}, "words": words,
    })
}

/// A writer's note on the after of `card/light.png` of `run`; its id.
fn noted(run: &str, words: &str) -> String {
    let writer = sign_in("writer-code", "/");
    let body = ask("card/light.png", "after", [0.1, 0.6, 0.3, 0.2], words);
    let answer = say(run, "notes", Some(&writer), &[], &body);
    assert_eq!(answer.status, 201, "{}", answer.text());
    answer.json()["note"]["id"].as_str().unwrap().to_owned()
}

/// Resolves note `id` as the page does, with the session `cookie` and `headers` besides.
fn resolve(id: &str, cookie: Option<&str>, headers: &[(&str, &str)]) -> Answer {
    let origin = world().worker.url.clone();
    let mut all: Vec<(&str, &str)> =
        vec![("Origin", &origin), ("Content-Type", "application/json")];
    all.extend_from_slice(headers);
    if let Some(cookie) = cookie {
        all.push(("Cookie", cookie));
    }
    let path = format!("/api/v2/notes/{id}/resolve");
    request("POST", &path, &all, Some(b"{}".to_vec()))
}

/// The CLI's sign in as `login`: the device flow through the host, approved on the fake
/// GitHub. The host token it ends in.
pub fn cli_token(login: &str) -> String {
    let answer = request("POST", "/api/v2/login/device", &[], Some(b"{}".to_vec()));
    assert_eq!(answer.status, 200, "{}", answer.text());
    let device = answer.json();
    world()
        .github
        .approve(device["user_code"].as_str().unwrap(), login);
    let body = json!({"device_code": device["device_code"]}).to_string();
    let answer = request(
        "POST",
        "/api/v2/login/token",
        &[("Content-Type", "application/json")],
        Some(body.into_bytes()),
    );
    assert_eq!(answer.status, 200, "{}", answer.text());
    answer.json()["token"].as_str().unwrap().to_owned()
}

/// What waits on the host for a work, as the CLI reads it with `token`.
pub fn feedback(token: &str, repository: &str, branch: &str) -> Answer {
    api(
        "GET",
        &format!("/api/v2/work/{repository}/{branch}"),
        token,
        None,
    )
}

pub fn a_note_needs_a_session_write_access_and_a_picture_of_the_run() {
    let job = Job::pull("owner/review", 120, "notes/guarded").at(&commit("a120"), 1120);
    let (look, files) = one_still(b"noted before", b"noted after");
    let run = proven(&job, &look, &files, &json!({}));
    let writer = sign_in("writer-code", "/");
    let reader = sign_in("reader-code", "/");
    let good = ask("card/light.png", "after", [0.1, 0.6, 0.3, 0.2], "Too tight");

    let answer = say(&run, "notes", None, &[], &good);
    assert_eq!(answer.status, 401, "without a session: {}", answer.text());
    let elsewhere = [("Origin", "https://elsewhere.test")];
    let answer = say(&run, "notes", Some(&writer), &elsewhere, &good);
    assert_eq!(answer.status, 403, "from another site: {}", answer.text());
    let answer = say(&run, "notes", Some(&reader), &[], &good);
    assert_eq!(answer.status, 403, "a reader: {}", answer.text());
    assert!(answer.text().contains("write access"), "{}", answer.text());
    for (why, body) in [
        (
            "no words",
            ask("card/light.png", "after", [0.1, 0.6, 0.3, 0.2], "  "),
        ),
        (
            "an empty region",
            ask("card/light.png", "after", [0.1, 0.6, 0.0, 0.2], "x"),
        ),
        (
            "a region off the picture",
            ask("card/light.png", "after", [0.9, 0.6, 0.3, 0.2], "x"),
        ),
        ("a moment of a still", {
            let mut body = good.clone();
            body["time"] = json!(1.0);
            body
        }),
    ] {
        let answer = say(&run, "notes", Some(&writer), &[], &body);
        assert_eq!(answer.status, 400, "{why}: {}", answer.text());
    }
    let missing = ask("nothing.png", "after", [0.1, 0.1, 0.1, 0.1], "x");
    let answer = say(&run, "notes", Some(&writer), &[], &missing);
    assert_eq!(
        answer.status,
        409,
        "a picture the run lacks: {}",
        answer.text()
    );

    let answer = say(&run, "notes", Some(&writer), &[], &good);
    assert_eq!(answer.status, 201, "{}", answer.text());
    let note = answer.json()["note"].clone();
    assert_eq!(note["by"], "writer", "{note:#}");
    assert_eq!(note["state"], "open", "{note:#}");
    assert_eq!(note["run"], run.as_str(), "{note:#}");
    assert_eq!(note["head"], commit("a120"), "{note:#}");
    let snapshot = live("/owner/review/pull/120", &[]).json();
    assert_eq!(snapshot["notes"][0]["words"], "Too tight", "{snapshot:#}");
    assert_eq!(review_of(120)["notes"][0]["id"], note["id"]);
}

pub fn an_open_note_makes_its_picture_need_change_and_fails_the_check_run() {
    let head = commit("a121");
    webhook(
        "pull_request",
        &pull_request("synchronize", 121, "notes/check", &head),
    );
    let job = Job::pull("owner/review", 121, "notes/check").at(&head, 1121);
    let (look, files) = one_still(b"check before", b"check after");
    let run = proven(&job, &look, &files, &json!({}));
    let writer = sign_in("writer-code", "/");
    // Said to look right first; a note then says what should change, which outweighs it.
    let answer = say(
        &run,
        "verdicts",
        Some(&writer),
        &[],
        &looks_right("card/light.png"),
    );
    assert_eq!(answer.status, 201, "{}", answer.text());
    published(121);
    assert_eq!(check_run(&head).state(), "success");

    let id = noted(&run, "The button text is cut off");
    let review = published(121);
    assert_eq!(review["state"], "needs_change", "{review:#}");
    assert_eq!(
        review["pictures"][0]["standing"], "needs_change",
        "{review:#}"
    );
    let check = check_run(&head);
    assert_eq!(check.state(), "failure", "{:?}", check.writes);
    let summary = check.field("output")["summary"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(summary.contains("card/light.png"), "{summary}");
    assert!(summary.contains("The button text is cut off"), "{summary}");

    // Resolving the last note: the looks right verdict on its content holds again.
    let answer = resolve(&id, Some(&writer), &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    let review = published(121);
    assert_eq!(review["state"], "approved", "{review:#}");
    assert!(review["notes"].as_array().is_none_or(Vec::is_empty));
    let check = check_run(&head);
    assert_eq!(check.state(), "success", "{:?}", check.writes);
    let summary = check.field("output")["summary"].clone();
    assert!(
        !summary.as_str().unwrap().contains("cut off"),
        "{summary:#}"
    );
}

pub fn resolving_the_last_note_brings_a_picture_back_to_not_looked_at() {
    let job = Job::pull("owner/review", 128, "notes/back").at(&commit("a128"), 1128);
    let (look, files) = one_still(b"back before", b"back after");
    let run = proven(&job, &look, &files, &json!({}));
    let first = noted(&run, "Too dark");
    let second = noted(&run, "Too tight");
    let writer = sign_in("writer-code", "/");
    resolve(&first, Some(&writer), &[]);
    assert_eq!(
        review_of(128)["state"],
        "needs_change",
        "one note is still open"
    );
    resolve(&second, Some(&writer), &[]);
    let review = review_of(128);
    assert_eq!(review["state"], "waits", "{review:#}");
    assert_eq!(review["pictures"][0]["standing"], "waits", "{review:#}");
}

pub fn an_open_note_follows_the_work_until_a_writer_resolves_it() {
    let first = Job::pull("owner/review", 122, "notes/follow").at(&commit("a122"), 1122);
    let (look, files) = one_still(b"follow before", b"follow after");
    let run = proven(&first, &look, &files, &json!({}));
    let id = noted(&run, "More room around the button");

    // The agent changed the picture: the note still shows, from the run it was made on, and
    // the picture needs change until a person resolves it.
    let (fixed, fixed_files) = one_still(b"follow before", b"follow after, with room");
    proven(
        &first.at(&commit("b122"), 2122),
        &fixed,
        &fixed_files,
        &json!({}),
    );
    let review = review_of(122);
    assert_eq!(review["head"], commit("b122"), "{review:#}");
    assert_eq!(review["state"], "needs_change", "{review:#}");
    assert!(review["pictures"][0]["verdict"].is_null(), "{review:#}");
    assert_eq!(review["notes"][0]["id"], id.as_str(), "{review:#}");
    let snapshot = live("/owner/review/pull/122", &[]).json();
    assert_eq!(snapshot["notes"][0]["run"], run.as_str(), "{snapshot:#}");

    let reader = sign_in("reader-code", "/");
    let answer = resolve(&id, Some(&reader), &[]);
    assert_eq!(answer.status, 403, "a reader: {}", answer.text());
    let answer = resolve(&id, None, &[]);
    assert_eq!(answer.status, 401, "{}", answer.text());
    let writer = sign_in("writer-code", "/");
    let answer = resolve(&id, Some(&writer), &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.json()["note"]["resolved_by"], "writer");
    assert_eq!(
        review_of(122)["state"],
        "waits",
        "its new content waits for a look"
    );
    let snapshot = live("/owner/review/pull/122", &[]).json();
    assert!(
        snapshot["notes"].as_array().is_none_or(Vec::is_empty),
        "{snapshot:#}"
    );
}

pub fn a_cli_token_reads_but_never_writes() {
    let job = Job::pull("owner/review", 123, "notes/token").at(&commit("a123"), 1123);
    let (look, files) = one_still(b"token before", b"token after");
    let run = proven(&job, &look, &files, &weakened());
    let id = noted(&run, "A note");
    // An admin's token: what they may do in a browser, their token can never do.
    let token = cli_token("admin");
    let auth = format!("Bearer {token}");
    let bearer = [("Authorization", auth.as_str())];
    let admin = sign_in("admin-code", "/");
    for cookie in [None, Some(admin.as_str())] {
        let answer = say(
            &run,
            "verdicts",
            cookie,
            &bearer,
            &looks_right("card/light.png"),
        );
        assert_eq!(answer.status, 403, "a verdict: {}", answer.text());
        let body = ask("card/light.png", "after", [0.1, 0.1, 0.2, 0.2], "x");
        let answer = say(&run, "notes", cookie, &bearer, &body);
        assert_eq!(answer.status, 403, "a note: {}", answer.text());
        let answer = resolve(&id, cookie, &bearer);
        assert_eq!(answer.status, 403, "a resolve: {}", answer.text());
        let answer = say(&run, "accept", cookie, &bearer, &json!({}));
        assert_eq!(answer.status, 403, "an accept: {}", answer.text());
    }
    // Nor does it open a CI run: it opens only its person's local runs.
    let answer = api(
        "POST",
        "/api/v2/runs",
        &token,
        Some(job.opening(&["screens"]).to_string().into_bytes()),
    );
    assert_eq!(answer.status, 400, "{}", answer.text());
    assert!(answer.text().contains("local run"), "{}", answer.text());
    let review = review_of(123);
    assert!(review["pictures"][0]["verdict"].is_null(), "{review:#}");
    assert!(review["accept"].is_null(), "{review:#}");
    assert_eq!(review["notes"].as_array().map(Vec::len), Some(1));

    // It reads what waits.
    let answer = feedback(&token, "owner/review", "notes/token");
    assert_eq!(answer.status, 200, "{}", answer.text());
}

pub fn the_cli_signs_in_through_the_host_with_githubs_device_flow() {
    let job = Job::push("owner/secret", "notes/device");
    let run = job.open(&["tests"]);
    job.done(&run.id, &[entry("tests", "passed")], &[]);

    let answer = request("POST", "/api/v2/login/device", &[], Some(b"{}".to_vec()));
    assert_eq!(answer.status, 200, "{}", answer.text());
    let device = answer.json();
    assert_eq!(
        device["verification_uri"],
        "https://github.com/login/device"
    );
    let code = device["user_code"].as_str().unwrap();
    let body = json!({"device_code": device["device_code"]}).to_string();
    let poll = || {
        request(
            "POST",
            "/api/v2/login/token",
            &[("Content-Type", "application/json")],
            Some(body.clone().into_bytes()),
        )
    };
    let answer = poll();
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.json()["pending"], true, "{}", answer.text());
    world().github.approve(code, "reader");
    let answer = poll();
    assert_eq!(answer.status, 200, "{}", answer.text());
    let signed = answer.json();
    assert_eq!(signed["login"], "reader");
    let token = signed["token"].as_str().unwrap().to_owned();
    assert!(token.starts_with("fzh_"), "{token}");
    // The device code signs in once.
    assert_eq!(poll().status, 400);

    // It reads the works of repositories its person may read, and no other.
    let answer = feedback(&token, "owner/secret", "notes/device");
    assert_eq!(answer.status, 200, "{}", answer.text());
    let read = answer.json();
    assert_eq!(read["work"]["branch"], "notes/device", "{read:#}");
    assert_eq!(
        read["url"],
        format!("{}/owner/secret/work/notes/device", world().worker.url)
    );
    let stranger = cli_token("stranger");
    let answer = feedback(&stranger, "owner/secret", "notes/device");
    assert_eq!(answer.status, 404, "{}", answer.text());
    let answer = get("/api/v2/work/owner/secret/notes/device", &[]);
    assert_eq!(answer.status, 401, "{}", answer.text());
    let answer = feedback("fzh_forged", "owner/secret", "notes/device");
    assert_eq!(answer.status, 401, "{}", answer.text());
    let answer = feedback(&token, "owner/secret", "notes/unknown");
    assert_eq!(answer.status, 404, "{}", answer.text());

    // Logging out ends it on the host.
    let answer = api("DELETE", "/api/v2/login", &token, None);
    assert_eq!(answer.status, 204, "{}", answer.text());
    let answer = feedback(&token, "owner/secret", "notes/device");
    assert_eq!(answer.status, 401, "{}", answer.text());
}

pub fn the_cli_reads_open_notes_with_their_pictures() {
    let job = Job::pull("owner/review", 124, "notes/read").at(&commit("a124"), 1124);
    let (look, files) = one_still(b"read before", b"read after");
    let run = proven(&job, &look, &files, &json!({}));
    let id = noted(&run, "The title wraps");
    let token = cli_token("reader");
    let answer = feedback(&token, "owner/review", "notes/read");
    assert_eq!(answer.status, 200, "{}", answer.text());
    let read = answer.json();
    let noted = &read["notes"][0];
    assert_eq!(noted["note"]["id"], id.as_str(), "{read:#}");
    assert_eq!(noted["note"]["words"], "The title wraps", "{read:#}");
    assert_eq!(noted["kind"], "still", "{read:#}");
    assert_eq!(read["review"]["notes"][0]["id"], id.as_str(), "{read:#}");
    for (side, bytes) in [
        ("before", b"read before".as_slice()),
        ("after", b"read after".as_slice()),
    ] {
        let path = noted[side].as_str().unwrap();
        let answer = api(
            "GET",
            &format!("/api/v2/runs/{run}/files/{path}"),
            &token,
            None,
        );
        assert_eq!(answer.status, 200, "{side}: {}", answer.text());
        assert_eq!(answer.body, bytes, "{side}");
        assert_eq!(answer.header("content-type"), Some("image/png"));
    }
    // Only pictures and films, and only with a token.
    let answer = api(
        "GET",
        &format!("/api/v2/runs/{run}/files/outputs/screens.log"),
        &token,
        None,
    );
    assert_eq!(answer.status, 404, "{}", answer.text());
    let path = noted["after"].as_str().unwrap();
    let answer = get(&format!("/api/v2/runs/{run}/files/{path}"), &[]);
    assert_eq!(answer.status, 401, "{}", answer.text());
}

/// The point at fractions `(fx, fy)` of the drawing layer on `side` of the stage, in page
/// pixels.
pub fn point_on(browser: &mut browser::Browser, side: &str, fx: f64, fy: f64) -> (f64, f64) {
    let rect = browser.eval(&format!(
        r#"(() => {{ const r = document.querySelector('[data-draw="{side}"]').getBoundingClientRect(); return [r.left, r.top, r.width, r.height]; }})()"#
    ));
    let n = |i: usize| rect[i].as_f64().unwrap();
    (n(0) + n(2) * fx, n(1) + n(3) * fy)
}

/// Draws a region on `side` from `(x0, y0)` to `(x1, y1)` in fractions, writes `words` and
/// adds the note, `both` ticking "on both sides" first.
pub fn draw_note(
    browser: &mut browser::Browser,
    side: &str,
    region: [f64; 4],
    words: &str,
    both: bool,
) {
    browser.wait_for(&format!(
        r#"!!document.querySelector('[data-draw="{side}"].can')"#
    ));
    // A mouse reaches only what is on the screen.
    browser.eval(&format!(
        r#"document.querySelector('[data-draw="{side}"]').scrollIntoView({{block: "center"}})"#
    ));
    let [x0, y0, x1, y1] = region;
    let from = point_on(browser, side, x0, y0);
    let to = point_on(browser, side, x1, y1);
    browser.drag(from, to);
    browser.wait_for("!!document.querySelector('form[data-compose]')");
    if both {
        browser.eval("document.querySelector('[data-both]').click()");
    }
    browser.eval("document.querySelector('[data-words]').focus()");
    browser.type_text(words);
    browser.eval("document.querySelector('[data-add-note]').click()");
    browser.wait_for(
        "!document.querySelector('form[data-compose]') || !!document.querySelector('.verdict .error')",
    );
    let error = browser.eval("document.querySelector('.verdict .error')?.textContent ?? ''");
    assert_eq!(error, json!(""), "the note was refused");
}

/// The open notes of the work at `path`, as its page gets them.
fn notes_of(path: &str) -> Vec<Value> {
    live(path, &[]).json()["notes"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn close(a: &Value, b: f64) -> bool {
    (a.as_f64().unwrap() - b).abs() < 0.02
}

pub fn a_note_drawn_on_the_page_is_saved_and_shows_in_another_tab() {
    review_with_pictures(125, "notes/draw", &json!({}));
    let url = format!("{}/owner/review/pull/125", world().worker.url);
    let mut browser = browser::Browser::start();
    browser.sign_in(&world().worker.url, &sign_in("writer-code", "/"));
    let first = browser.open(&url);
    browser.wait_for(&verdict_shows("none"));
    let second = browser.open(&url);
    browser.wait_for(&verdict_shows("none"));
    browser.eval("window.stayed = true");
    // Needs change is not a button: a note says it.
    assert_eq!(
        browser.eval(r#"!!document.querySelector('button[data-say="needs_change"]')"#),
        json!(false)
    );

    browser.on(&first);
    draw_note(
        &mut browser,
        "after",
        [0.1, 0.6, 0.4, 0.8],
        "The button text is cut off.",
        false,
    );
    let notes = notes_of("/owner/review/pull/125");
    assert_eq!(notes.len(), 1, "{notes:#?}");
    let note = &notes[0];
    assert_eq!(note["side"], "after", "{note:#}");
    assert_eq!(note["picture"], "card/light.png", "{note:#}");
    assert_eq!(note["words"], "The button text is cut off.", "{note:#}");
    let region = &note["region"];
    assert!(
        close(&region["x"], 0.1)
            && close(&region["y"], 0.6)
            && close(&region["w"], 0.3)
            && close(&region["h"], 0.2),
        "{region:#}"
    );

    // The picture needs change now. The other tab shows the note numbered, on its side, and in
    // the list, without a reload.
    browser.wait_for(&verdict_shows("needs_change"));
    browser.on(&second);
    browser
        .wait_for(r#"!!document.querySelector('[data-draw="after"] [data-region][data-n="1"]')"#);
    assert_eq!(
        browser.eval(r#"!!document.querySelector('[data-draw="before"] [data-region]')"#),
        json!(false)
    );
    browser.wait_for("document.querySelector('[data-notes]')?.textContent.includes('cut off')");

    // On both sides, drawn on the before.
    draw_note(
        &mut browser,
        "before",
        [0.5, 0.1, 0.9, 0.3],
        "The title moved.",
        true,
    );
    browser.on(&first);
    browser.wait_for(r#"document.querySelectorAll('[data-region][data-n="2"]').length === 2"#);
    let both = notes_of("/owner/review/pull/125")
        .into_iter()
        .find(|n| n["words"] == "The title moved.")
        .unwrap();
    assert_eq!(both["side"], "both", "{both:#}");

    browser.on(&second);
    browser.wait_for(&verdict_shows("needs_change"));
    browser.wait_for(r#"document.querySelector('[data-picture="screens/card/light.png"]').dataset.verdict === "needs_change""#);

    // Resolve in one tab; it leaves the other, where the other note still says needs change.
    browser.eval(
        "document.querySelector('[data-notes] li[data-n=\"1\"] button[data-resolve]').click()",
    );
    browser.on(&first);
    browser.wait_for(
        r#"!document.querySelector('[data-region][data-says="The button text is cut off."]')"#,
    );
    browser.wait_for("!document.querySelector('[data-notes]').textContent.includes('cut off')");
    browser.wait_for(&verdict_shows("needs_change"));
    browser.on(&second);
    assert_eq!(
        browser.eval("window.stayed === true"),
        json!(true),
        "no reload"
    );
}

pub fn a_film_note_takes_the_moment_of_the_film_clock() {
    review_with_pictures(126, "notes/film", &json!({}));
    let url = format!("{}/owner/review/pull/126", world().worker.url);
    let mut browser = browser::Browser::start();
    browser.sign_in(&world().worker.url, &sign_in("writer-code", "/"));
    browser.open(&url);
    browser.wait_for(&verdict_shows("none"));
    browser.eval(r#"document.querySelector('[data-picture="screens/checkout.webm"]').click()"#);
    browser.wait_for(&loaded("video", "after"));
    browser.eval(r#"document.querySelector('[data-seek]').click()"#);
    browser.wait_for("document.querySelector('[data-clock]').textContent.startsWith('1.00')");
    draw_note(
        &mut browser,
        "after",
        [0.2, 0.6, 0.6, 0.85],
        "The total jumps here.",
        false,
    );
    let note = notes_of("/owner/review/pull/126").remove(0);
    assert_eq!(note["picture"], "checkout.webm", "{note:#}");
    assert!(close(&note["time"], 1.0), "{note:#}");
    browser.wait_for("document.querySelector('[data-notes]').textContent.includes('1.00 s')");
}

pub fn an_old_note_shows_dashed_with_its_run_on_a_later_run() {
    let (job, run) = review_with_pictures(127, "notes/old", &json!({}));
    noted(&run, "Spacing grew between the rows");
    // A later push with the same pictures: the note is from the earlier run.
    let later = job.at(&commit("b127"), 2127);
    let next = later.open(&["screens"]);
    run_with_pictures(&later, &next.id);
    let url = format!("{}/owner/review/pull/127", world().worker.url);
    let mut browser = browser::Browser::start();
    browser.sign_in(&world().worker.url, &sign_in("writer-code", "/"));
    browser.open(&url);
    browser.wait_for(&format!(
        r#"document.querySelector('section.run')?.dataset.run === "{}""#,
        next.id
    ));
    browser.wait_for(
        r#"!!document.querySelector('[data-draw="after"] [data-region][data-old="true"]')"#,
    );
    browser.wait_for(r#"document.querySelector('[data-notes] li[data-old="true"]')?.textContent.includes('a127000')"#);
    // A note can be started from the keyboard too: on the whole picture. The control shows
    // once the page knows the viewer may write, which it asks for after the snapshot.
    browser.wait_for("!!document.querySelector('button[data-whole]')");
    browser.eval("document.querySelector('button[data-whole]').focus()");
    browser.press(" ");
    browser.wait_for("!!document.querySelector('form[data-compose]')");
}

/// Pictures of notes on the page for a person to look at: an old note from an earlier push,
/// a region being written, and the notes with the picture said to need change, light, dark
/// and at phone width. Part of the ignored `pictures`.
pub fn pictures(shoot: &dyn Fn(&mut browser::Browser, &str)) {
    let (job, run) = review_with_pictures(191, "notes/shots", &json!({}));
    noted(
        &run,
        "The button text is cut off: it should fit on one line.",
    );
    let later = job.at(&commit("b191"), 2191);
    let next = later.open(&["screens"]);
    run_with_pictures(&later, &next.id);
    let url = format!("{}/owner/review/pull/191", world().worker.url);
    let mut writer = browser::Browser::start();
    writer.sign_in(&world().worker.url, &sign_in("writer-code", "/"));
    writer.open(&url);
    writer.view(1280, 900, false, false);
    writer.wait_for(&format!(
        r#"document.querySelector('section.run')?.dataset.run === "{}""#,
        next.id
    ));
    writer.wait_for(&loaded("img", "after"));
    writer.wait_for(r#"!!document.querySelector('[data-draw="after"].can')"#);
    shoot(&mut writer, "notes-light-old");
    writer.eval(
        r#"document.querySelector('[data-draw="before"]').scrollIntoView({block: "center"})"#,
    );
    let from = point_on(&mut writer, "before", 0.55, 0.12);
    let to = point_on(&mut writer, "before", 0.92, 0.32);
    writer.drag(from, to);
    writer.wait_for("!!document.querySelector('form[data-compose]')");
    writer.eval("document.querySelector('[data-both]').click()");
    writer.type_text("The title sits lower than before; keep it where it was.");
    shoot(&mut writer, "notes-light-compose");
    writer.eval("document.querySelector('[data-add-note]').click()");
    writer.wait_for(r#"document.querySelectorAll('[data-region][data-n="2"]').length === 2"#);
    writer.wait_for(&verdict_shows("needs_change"));
    for dark in [false, true] {
        writer.view(1280, 900, false, dark);
        let scheme = if dark { "dark" } else { "light" };
        shoot(&mut writer, &format!("notes-{scheme}-needs-change"));
    }
    writer.view(390, 844, true, false);
    shoot(&mut writer, "notes-phone-light");
    writer.view(390, 844, true, true);
    shoot(&mut writer, "notes-phone-dark");
}
