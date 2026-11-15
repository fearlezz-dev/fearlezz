//! The repository page: `/<owner>/<repo>` lists every work of the repository, newest activity
//! first, with where its latest run stands and what waits on it, live, for the people who may
//! see the repository's work pages.

use super::notes::{ask, cli_token};
use super::*;

/// The works of `owner/listing` as its page's live connection lists them, for `cookie`.
fn listing(cookie: &str) -> Value {
    let answer = live("/owner/listing", &[("Cookie", cookie)]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    answer.json()
}

/// The listed work whose page is at `path`.
fn listed<'a>(listing: &'a Value, path: &str) -> &'a Value {
    listing["works"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["path"] == path)
        .unwrap_or_else(|| panic!("{path} is not listed: {listing:#}"))
}

pub fn the_repository_page_lists_its_works_with_what_waits() {
    // A pull request whose pictures wait for a look and whose weakening waits for an admin.
    let job = Job::pull("owner/listing", 401, "list/pictures").at(&commit("l401"), 401);
    let pictures = job.open(&["screens"]);
    run_with_pictures(&job, &pictures.id);
    let shots = shots();
    let files: Vec<(&str, &[u8])> = shots
        .files
        .iter()
        .map(|(p, b)| (p.as_str(), b.as_slice()))
        .collect();
    let answer = job.close_with(
        &pictures.id,
        &[entry("screens", "passed")],
        std::slice::from_ref(&shots.look),
        &files,
        &weakened(),
    );
    assert_eq!(answer.status, 200, "{}", answer.text());
    // A branch whose proof failed.
    let failing = Job::push("owner/listing", "list/failed");
    let run = failing.open(&["tests"]);
    let answer = failing.done(&run.id, &[entry("tests", "failed")], &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    // A developer's local run, still going.
    let local = Job::local("owner/listing", "list/local", &cli_token("writer"));
    local.open(&["tests"]);
    // A CI run still going.
    Job::push("owner/listing", "list/running").open(&["tests"]);
    // Last, a note on the pull request's pictures: its newest activity.
    let writer = sign_in("writer-code", "/");
    let body = ask(
        "card/light.png",
        "after",
        [0.1, 0.6, 0.3, 0.2],
        "Too tight.",
    );
    let answer = say(&pictures.id, "notes", Some(&writer), &[], &body);
    assert_eq!(answer.status, 201, "{}", answer.text());

    let reader = sign_in("reader-code", "/");
    let all = listing(&reader);
    assert_eq!(all["kind"], "snapshot");
    assert_eq!(all["repository"], "owner/listing");
    let order: Vec<&str> = all["works"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|w| w["path"].as_str())
        .filter(|p| {
            [
                "/owner/listing/pull/401",
                "/owner/listing/work/list/failed",
                "/owner/listing/work/list/local",
                "/owner/listing/work/list/running",
            ]
            .contains(p)
        })
        .collect();
    assert_eq!(
        order,
        [
            "/owner/listing/pull/401",
            "/owner/listing/work/list/running",
            "/owner/listing/work/list/local",
            "/owner/listing/work/list/failed",
        ],
        "newest activity first"
    );

    let pull = listed(&all, "/owner/listing/pull/401");
    assert_eq!(pull["work"]["branch"], "list/pictures");
    assert_eq!(pull["work"]["pull"]["number"], 401);
    assert_eq!(pull["work"]["pull"]["title"], "Pull request 401");
    assert_eq!(pull["latest"]["state"], "done");
    assert_eq!(pull["latest"]["source"]["kind"], "ci");
    // Two changed stills, a new one and a changed film; the flaky and the same one never wait,
    // and the one with a note needs change.
    assert_eq!(pull["pictures"], 3, "{pull:#}");
    assert_eq!(pull["notes"], 1, "{pull:#}");
    assert_eq!(pull["weakened"], true, "{pull:#}");

    let failed = listed(&all, "/owner/listing/work/list/failed");
    assert_eq!(failed["latest"]["state"], "done");
    assert_eq!(failed["latest"]["failed"], 1);
    assert_eq!(failed["pictures"], 0);
    assert_eq!(failed["weakened"], false);

    let local = listed(&all, "/owner/listing/work/list/local");
    assert_eq!(local["latest"]["state"], "running");
    assert_eq!(local["latest"]["source"]["kind"], "local");
    assert_eq!(local["latest"]["source"]["login"], "writer");
}

pub fn a_private_repository_page_needs_a_reader() {
    Job::push("owner/listing", "list/access").open(&["tests"]);
    let answer = navigate("/owner/listing", &[]);
    assert_eq!(answer.status, 302, "{}", answer.text());
    assert_eq!(
        answer.header("location").unwrap(),
        "/login?next=%2Fowner%2Flisting"
    );
    let answer = live("/owner/listing", &[]);
    assert_eq!(answer.status, 401, "{}", answer.text());
    assert!(!answer.text().contains("list/"), "{}", answer.text());
    let stranger = sign_in("stranger-code", "/");
    assert_eq!(
        navigate("/owner/listing", &[("Cookie", &stranger)]).status,
        404
    );
    assert_eq!(live("/owner/listing", &[("Cookie", &stranger)]).status, 404);
    let reader = sign_in("reader-code", "/");
    let answer = navigate("/owner/listing", &[("Cookie", &reader)]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.header("cache-control"), Some("no-store"));
    // A repository the host holds no work of is not found.
    let answer = navigate("/owner/nothing", &[("Cookie", &reader)]);
    assert_eq!(answer.status, 404, "{}", answer.text());
}

pub fn the_repository_page_updates_live() {
    let job = Job::push("owner/listing", "list/live");
    Job::push("owner/listing", "list/before").open(&["tests"]);
    let reader = sign_in("reader-code", "/");
    let mut browser = browser::Browser::start();
    browser.sign_in(&world().worker.url, &reader);
    browser.open(&format!("{}/owner/listing", world().worker.url));
    browser.wait_for("!!document.querySelector('[data-listed]')");
    browser.eval("window.stayed = true");
    let row = r#"document.querySelector('[data-listed="/owner/listing/work/list/live"]')"#;
    let run = job.open(&["tests"]);
    browser.wait_for(&format!("{row}?.dataset.state === 'running'"));
    let answer = job.done(&run.id, &[entry("tests", "passed")], &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    browser.wait_for(&format!("{row}?.dataset.state === 'passed'"));
    // Each row leads to its work's page.
    assert_eq!(
        browser.eval(&format!("{row}.querySelector('a').getAttribute('href')")),
        json!("/owner/listing/work/list/live")
    );
    assert_eq!(
        browser.eval("window.stayed === true"),
        json!(true),
        "no reload"
    );
}

/// Pictures of the repository page, light and dark, wide and at phone width. Part of the
/// ignored `pictures`.
pub fn pictures(shoot: &dyn Fn(&mut browser::Browser, &str)) {
    the_repository_page_lists_its_works_with_what_waits();
    let mut browser = browser::Browser::start();
    browser.sign_in(&world().worker.url, &sign_in("reader-code", "/"));
    browser.open(&format!("{}/owner/listing", world().worker.url));
    for (view, (width, height, mobile)) in
        [("wide", (1280, 900, false)), ("phone", (390, 844, true))]
    {
        for dark in [false, true] {
            browser.view(width, height, mobile, dark);
            browser.wait_for("document.querySelectorAll('[data-listed]').length >= 4");
            let scheme = if dark { "dark" } else { "light" };
            shoot(&mut browser, &format!("repository-{view}-{scheme}"));
        }
    }
}
