//! Local runs: a developer signed in with `fearlezz setup login` streams each `fearlezz run` to
//! the host with the CLI's token, as CI streams its runs, for repositories they may write. A
//! local run shows unproven and is never reviewed: no verdict or accept on it, and it never
//! moves the GitHub check run or the comment. A person may leave notes on its pictures, which
//! follow the work to later runs, local and CI. One that goes silent is closed after an hour.
//! The CLI's token is the host's own: it lives 30 days from its last use while the host renews
//! the person's GitHub token.

use super::notes::{ask, cli_token, draw_note, feedback};
use super::*;

/// The page's live snapshot of the work at `path`.
fn snapshot(path: &str) -> Value {
    live(path, &[]).json()
}

/// The run `id` as the work's page at `path` gets it.
fn run_on(path: &str, id: &str) -> Value {
    snapshot(path)["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id)
        .cloned()
        .unwrap_or_else(|| panic!("no run {id} on {path}"))
}

pub fn a_cli_token_opens_a_local_run_only_with_write_access() {
    let reader = cli_token("reader");
    let dev = cli_token("dev");
    let installed = |token: &str| {
        api(
            "GET",
            "/api/v2/installed?repository=owner/local",
            token,
            None,
        )
    };
    let answer = installed(&reader);
    assert_eq!(answer.status, 403, "a reader: {}", answer.text());
    assert!(answer.text().contains("write access"), "{}", answer.text());
    let answer = installed(&dev);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.json(), json!({"installed": true}));
    let answer = api("GET", "/api/v2/installed", &dev, None);
    assert_eq!(answer.status, 400, "no repository: {}", answer.text());

    let as_reader = Job::local("owner/local", "local/access", &reader);
    let answer = as_reader.open_answer(&as_reader.opening(&["tests"]));
    assert_eq!(answer.status, 403, "a reader: {}", answer.text());
    let job = Job::local("owner/local", "local/access", &dev);
    let mut named = job.opening(&["tests"]);
    named["pull"] = json!({"number": 5, "title": "Five"});
    let answer = job.open_answer(&named);
    assert_eq!(answer.status, 400, "a pull request: {}", answer.text());

    let run = job.open(&["tests"]);
    assert_eq!(
        run.url,
        format!("{}/owner/local/work/local/access", world().worker.url)
    );
    let answer = job.started(&run.id, "tests");
    assert_eq!(answer.status, 200, "{}", answer.text());
    // Another person's token finds nothing of it, and no CI token writes it.
    let other = Job::local("owner/local", "local/access", &cli_token("writer"));
    let answer = other.started(&run.id, "tests");
    assert_eq!(answer.status, 404, "another person: {}", answer.text());
    let ci = Job::push("owner/local", "local/access");
    let answer = ci.started(&run.id, "tests");
    assert_eq!(answer.status, 403, "a CI token: {}", answer.text());
    let answer = job.done(&run.id, &[entry("tests", "passed")], &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());

    let shown = run_on("/owner/local/work/local/access", &run.id);
    assert_eq!(
        shown["source"],
        json!({"kind": "local", "login": "dev"}),
        "who ran it is the token's word: {shown:#}"
    );
    assert_eq!(shown["state"], "done", "{shown:#}");
    // What dev may do on the repository is asked of GitHub once, then kept.
    assert_eq!(
        world()
            .github
            .asked("GET", "/repos/owner/local/collaborators/dev/permission"),
        1
    );
}

/// A writer's local run of `branch` of `owner/review` whose check `screens` took one changed
/// still, sent in full and done; the job and the run's id.
fn local_with_still(branch: &str, before: &[u8], after: &[u8]) -> (Job, String) {
    let job = Job::local("owner/review", branch, &cli_token("writer"));
    let (look, files) = one_still(before, after);
    let run = proven(&job, &look, &files, &json!({}));
    (job, run)
}

pub fn a_local_run_takes_notes_but_never_a_verdict_or_an_accept() {
    let (_, run) = local_with_still("local/notes", b"local notes before", b"local notes after");
    let writer = sign_in("writer-code", "/");
    let admin = sign_in("admin-code", "/");
    let answer = say(
        &run,
        "verdicts",
        Some(&writer),
        &[],
        &looks_right("card/light.png"),
    );
    assert_eq!(answer.status, 409, "looks right: {}", answer.text());
    assert!(answer.text().contains("local run"), "{}", answer.text());

    let body = ask("card/light.png", "after", [0.1, 0.6, 0.3, 0.2], "Too tight");
    let answer = say(&run, "notes", Some(&writer), &[], &body);
    assert_eq!(answer.status, 201, "a note: {}", answer.text());
    assert_eq!(answer.json()["note"]["run"], run.as_str());

    let answer = say(&run, "accept", Some(&admin), &[], &json!({}));
    assert_eq!(answer.status, 409, "an accept: {}", answer.text());
    assert!(answer.text().contains("local run"), "{}", answer.text());
}

pub fn a_local_run_never_touches_the_check_run_or_the_comment() {
    let head = commit("l130");
    webhook(
        "pull_request",
        &pull_request("opened", 130, "local/check", &head),
    );
    let before = world().github.check_runs("owner/review", &head);
    assert_eq!(before.len(), 1, "the pull request waits for CI");
    let writes = before[0].writes.len();
    let review = review_of(130);

    // The developer's local run of the same head, with a picture, from open to done.
    let job = Job::local("owner/review", "local/check", &cli_token("writer")).at(&head, 0);
    let (look, files) = one_still(b"untouched before", b"untouched after");
    proven(&job, &look, &files, &json!({}));

    let after = world().github.check_runs("owner/review", &head);
    assert_eq!(after.len(), 1, "{after:?}");
    assert_eq!(after[0].writes.len(), writes, "{:?}", after[0].writes);
    assert!(
        world().github.comments("owner/review", 130).is_empty(),
        "no comment"
    );
    assert_eq!(review_of(130), review, "the review stands where CI left it");
    assert_eq!(review["state"], "ci", "{review:#}");
}

pub fn a_note_on_a_local_run_shows_on_a_later_ci_run_of_the_work() {
    // Before any pull request: the work exists by its branch.
    let (_, local) = local_with_still("local/carry", b"carry before", b"carry after");
    let writer = sign_in("writer-code", "/");
    let body = ask(
        "card/light.png",
        "after",
        [0.2, 0.2, 0.3, 0.3],
        "The title wraps",
    );
    let answer = say(&local, "notes", Some(&writer), &[], &body);
    assert_eq!(answer.status, 201, "{}", answer.text());
    let note = answer.json()["note"]["id"].as_str().unwrap().to_owned();
    let token = cli_token("reader");
    let read = feedback(&token, "owner/review", "local/carry").json();
    assert_eq!(read["notes"][0]["note"]["id"], note.as_str(), "{read:#}");
    assert_eq!(read["notes"][0]["note"]["run"], local.as_str(), "{read:#}");

    // The pull request comes, then CI proves it: the same work, and the note shows there.
    let head = commit("c131");
    webhook(
        "pull_request",
        &pull_request("opened", 131, "local/carry", &head),
    );
    let job = Job::pull("owner/review", 131, "local/carry").at(&head, 1131);
    let (look, files) = one_still(b"carry before", b"carry after, on CI");
    proven(&job, &look, &files, &json!({}));
    let review = review_of(131);
    assert_eq!(review["notes"][0]["id"], note.as_str(), "{review:#}");
    let page = snapshot("/owner/review/pull/131");
    let sources: Vec<&str> = page["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["source"]["kind"].as_str().unwrap())
        .collect();
    assert_eq!(sources, ["ci", "local"], "{page:#}");
    let summary = check_run(&head).field("output")["summary"].clone();
    assert!(
        summary.as_str().unwrap().contains("The title wraps"),
        "{summary:#}"
    );
}

pub fn a_silent_local_run_is_cancelled_after_an_hour() {
    let token = cli_token("writer");
    let silent = Job::local("owner/review", "local/silent", &token);
    let quiet = Job::local("owner/review", "local/quiet", &token);
    let ci = Job::push("owner/review", "local/ci-silent");
    let (silent_run, quiet_run, ci_run) = (
        silent.open(&["tests"]),
        quiet.open(&["tests"]),
        ci.open(&["tests"]),
    );
    // Time passes: the first said nothing for two hours, the second for half an hour. A CI
    // run is closed by its job's end, never by the clock.
    world().worker.sql(&format!(
        "UPDATE runs SET touched = touched - 7200 WHERE id IN ('{}', '{}')",
        silent_run.id, ci_run.id
    ));
    world().worker.sql(&format!(
        "UPDATE runs SET touched = touched - 1800 WHERE id = '{}'",
        quiet_run.id
    ));
    let answer = get("/__scheduled?cron=*/10+*+*+*+*", &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    let state = |path: &str, id: &str| run_on(path, id)["state"].clone();
    assert_eq!(
        state("/owner/review/work/local/silent", &silent_run.id),
        "cancelled"
    );
    assert_eq!(
        state("/owner/review/work/local/quiet", &quiet_run.id),
        "running"
    );
    assert_eq!(
        state("/owner/review/work/local/ci-silent", &ci_run.id),
        "running"
    );
    let answer = silent.started(&silent_run.id, "tests");
    assert_eq!(answer.status, 409, "closed: {}", answer.text());
}

pub fn a_local_run_uploading_its_files_is_not_silent() {
    let token = cli_token("writer");
    let job = Job::local("owner/review", "local/uploading", &token);
    let run = job.open(&["screens"]);
    let film = b"local/uploading: a long film";
    let answer = job.finished(
        &run.id,
        &entry("screens", "passed"),
        &[("looks/screens/after/long.webm", film)],
    );
    assert_eq!(answer.status, 200, "{}", answer.text());
    // The check ended two hours ago; the run has been sending its films since.
    world().worker.sql(&format!(
        "UPDATE runs SET touched = touched - 7200 WHERE id = '{}'",
        run.id
    ));
    let answer = job.send(film);
    assert_eq!(answer.status, 204, "{}", answer.text());
    let answer = get("/__scheduled?cron=*/10+*+*+*+*", &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(
        run_on("/owner/review/work/local/uploading", &run.id)["state"],
        "running"
    );
}

pub fn the_host_renews_an_expired_github_token() {
    let token = cli_token("refresher");
    // Eight hours later: GitHub's user token expired, its refresh token still holds.
    world().github.expire("refresher");
    world()
        .worker
        .sql("UPDATE cli_tokens SET token_expires = 0 WHERE login = 'refresher'");
    let answer = api(
        "GET",
        "/api/v2/installed?repository=owner/local",
        &token,
        None,
    );
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.json(), json!({"installed": true}));
    let job = Job::local("owner/local", "local/renewed", &token);
    let run = job.open(&["tests"]);
    let answer = job.done(&run.id, &[entry("tests", "passed")], &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
}

pub fn a_cli_token_unused_for_30_days_is_refused() {
    let idle = cli_token("idle");
    let active = cli_token("active");
    world()
        .worker
        .sql("UPDATE cli_tokens SET used = used - 31 * 86400 WHERE login = 'idle'");
    world()
        .worker
        .sql("UPDATE cli_tokens SET used = used - 29 * 86400 WHERE login = 'active'");
    // An unknown work of a public repository: 404 to a token that holds, 401 to one that
    // does not.
    let answer = feedback(&idle, "owner/local", "local/nothing");
    assert_eq!(answer.status, 401, "unused 31 days: {}", answer.text());
    assert!(
        answer.text().contains("fearlezz setup login"),
        "{}",
        answer.text()
    );
    let answer = feedback(&active, "owner/local", "local/nothing");
    assert_eq!(answer.status, 404, "used 29 days ago: {}", answer.text());
    // That use keeps it alive for 30 more days.
    world()
        .worker
        .sql("UPDATE cli_tokens SET used = used - 2 * 86400 WHERE login = 'active'");
    let answer = feedback(&active, "owner/local", "local/nothing");
    assert_eq!(answer.status, 404, "used again: {}", answer.text());
}

/// A CI run of pull request `number` with the pictures of `shots`, then a writer's finished
/// local run of the same branch with them too, which the page shows first; the local run's id.
fn ci_then_local(number: u64, branch: &str) -> String {
    review_with_pictures(number, branch, &json!({}));
    let job = Job::local("owner/review", branch, &cli_token("writer"))
        .at(&commit(&format!("l{number}")), 0);
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
    run.id
}

pub fn the_page_marks_a_local_run_unproven_and_offers_only_notes() {
    let local = ci_then_local(132, "local/page");
    let url = format!("{}/owner/review/pull/132", world().worker.url);
    let mut browser = browser::Browser::start();
    browser.sign_in(&world().worker.url, &sign_in("writer-code", "/"));
    browser.open(&url);
    browser.wait_for(&format!(
        r#"document.querySelector('section.run[data-shown="true"]')?.dataset.run === "{local}""#
    ));
    browser.wait_for(&loaded("img", "after"));
    // Marked in the runs list and over its pictures.
    assert_eq!(
        browser.eval(&format!(
            r#"document.querySelector('section.run[data-run="{local}"]').dataset.source"#
        )),
        json!("local")
    );
    assert_eq!(
        browser.eval(&format!(
            r#"!!document.querySelector('section.run[data-run="{local}"] [data-unproven]')"#
        )),
        json!(true)
    );
    assert_eq!(
        browser.eval(
            "document.querySelector('[data-pictures] [data-unproven]')?.textContent.includes('writer')"
        ),
        json!(true)
    );
    assert_eq!(
        browser.eval("document.querySelector('[data-standing]').dataset.standing"),
        json!("local")
    );
    // No verdict and no accept; notes still.
    assert_eq!(
        browser.eval("!!document.querySelector('button[data-say], button[data-accept]')"),
        json!(false)
    );
    assert_eq!(
        browser.eval("document.querySelector('[data-said]').dataset.said"),
        json!("never")
    );
    draw_note(
        &mut browser,
        "after",
        [0.1, 0.6, 0.4, 0.8],
        "The button text is cut off.",
        false,
    );
    let notes = snapshot("/owner/review/pull/132")["notes"].clone();
    assert_eq!(notes[0]["run"], local.as_str(), "{notes:#}");
    // The CI run beside it still offers the verdict.
    browser.eval("document.querySelector('section.run[data-source=\"ci\"] .run-head').click()");
    browser.wait_for(r#"!!document.querySelector('button[data-say="looks_right"]')"#);
}

/// Pictures of a local run beside a CI run of the same pull request, light and dark, wide and
/// at phone width, with a note on the local run. Part of the ignored `pictures`.
pub fn pictures(shoot: &dyn Fn(&mut browser::Browser, &str)) {
    let local = ci_then_local(192, "local/shots");
    let writer = sign_in("writer-code", "/");
    let body = ask(
        "card/light.png",
        "after",
        [0.1, 0.62, 0.42, 0.2],
        "The button text is cut off: it should fit on one line.",
    );
    let answer = say(&local, "notes", Some(&writer), &[], &body);
    assert_eq!(answer.status, 201, "{}", answer.text());
    let url = format!("{}/owner/review/pull/192", world().worker.url);
    let mut browser = browser::Browser::start();
    browser.sign_in(&world().worker.url, &writer);
    browser.open(&url);
    for (view, (width, height, mobile)) in
        [("wide", (1280, 900, false)), ("phone", (390, 844, true))]
    {
        for dark in [false, true] {
            browser.view(width, height, mobile, dark);
            browser.wait_for(&format!(
                r#"document.querySelector('section.run[data-shown="true"]')?.dataset.run === "{local}""#
            ));
            browser.wait_for(&loaded("img", "after"));
            let scheme = if dark { "dark" } else { "light" };
            shoot(&mut browser, &format!("local-{view}-{scheme}"));
        }
    }
}
