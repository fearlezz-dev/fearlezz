//! README pictures: the files origin serves the newest proven picture of a push to a public
//! repository's default branch at a stable address, `/readme/<owner>/<repo>/<check>/<picture>`,
//! with no grant, so a README can embed what the proof took.

use super::*;

/// The fake GitHub's default branch of every repository.
const DEFAULT: &str = "main";

/// A done run of `job` whose check `check` took one new still `picture` as `bytes`, and came
/// out as `outcome`: a failed check fails its proof. Its id.
fn took(job: &Job, check: &str, picture: &str, bytes: &[u8], outcome: &str) -> String {
    let path = format!("looks/{check}/after/{picture}");
    let look = look_of(check, &[still(picture, "new", None, Some(&path))]);
    let files = [(path.as_str(), bytes)];
    let run = job.open(&[check]);
    let mut ended = entry(check, outcome);
    if outcome == "failed" {
        ended["failure"] = json!({"what": "it failed", "why": "a test failed", "fix": "fix it"});
    }
    let answer = job.finished_with_look(&run.id, &ended, &files, &look);
    assert_eq!(answer.status, 200, "{}", answer.text());
    job.send(bytes);
    let answer = job.close(&run.id, &[ended], std::slice::from_ref(&look), &files);
    assert_eq!(answer.status, 200, "{}", answer.text());
    run.id
}

/// The README address of `picture` of `check` in `repository`.
fn readme(repository: &str, check: &str, picture: &str) -> String {
    format!(
        "{}/readme/{repository}/{check}/{picture}",
        world().worker.files
    )
}

pub(crate) fn a_readme_picture_is_the_newest_proven_push_to_the_default_branch() {
    let main = Job::push("owner/readme", DEFAULT);
    took(
        &main.at(&commit("r1"), 9101),
        "hero",
        "card/light.png",
        b"readme first",
        "passed",
    );
    took(
        &main.at(&commit("r2"), 9102),
        "hero",
        "card/light.png",
        b"readme second",
        "passed",
    );
    // A newer push whose proof failed shows nothing of its own.
    took(
        &main.at(&commit("r3"), 9103),
        "hero",
        "card/light.png",
        b"readme broken",
        "failed",
    );

    let answer = get_url(&readme("owner/readme", "hero", "card/light.png"), &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.body, b"readme second", "the newest proven push");
    let header = |name: &str| answer.header(name).unwrap_or_default().to_owned();
    assert_eq!(
        header("content-type"),
        "image/png",
        "its name pins its type"
    );
    assert_eq!(header("x-content-type-options"), "nosniff", "never sniffed");
    assert_eq!(
        header("cache-control"),
        "public, max-age=300",
        "kept five minutes"
    );
    // Asked again with what it has, a cache gets no bytes until a push changes the picture.
    let tag = header("etag");
    assert_eq!(
        tag,
        format!("\"{}\"", sha256(b"readme second")),
        "tagged by its bytes"
    );
    let answer = get_url(
        &readme("owner/readme", "hero", "card/light.png"),
        &[("If-None-Match", &tag)],
    );
    assert_eq!(answer.status, 304, "{}", answer.text());
    assert_eq!(answer.body, b"", "a 304 has no bytes");
}

pub(crate) fn a_readme_picture_never_comes_from_another_branch_or_a_pull_request() {
    let main = Job::push("owner/readme", DEFAULT).at(&commit("b1"), 9201);
    took(&main, "branches", "home.png", b"readme of main", "passed");
    let branch = Job::push("owner/readme", "feat/readme").at(&commit("b2"), 9202);
    took(
        &branch,
        "branches",
        "home.png",
        b"readme of a branch",
        "passed",
    );
    let pull = Job::pull("owner/readme", 9203, "readme/pull").at(&commit("b3"), 9203);
    took(&pull, "branches", "home.png", b"readme of a pull", "passed");

    let answer = get_url(&readme("owner/readme", "branches", "home.png"), &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(
        answer.body, b"readme of main",
        "only the default branch counts"
    );
}

pub(crate) fn a_readme_picture_of_a_check_in_a_folder_names_the_check_then_the_picture() {
    let main = Job::push("owner/readme", DEFAULT).at(&commit("f1"), 9301);
    took(
        &main,
        "look/app",
        "films/in_use.webp",
        b"readme film",
        "passed",
    );

    let answer = get_url(
        &readme("owner/readme", "look/app", "films/in_use.webp"),
        &[],
    );
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.body, b"readme film", "the check, then the picture");
    assert_eq!(
        answer.header("content-type"),
        Some("image/webp"),
        "its name pins its type"
    );
}

pub(crate) fn a_readme_picture_of_a_private_repository_is_not_found() {
    let main = Job::push("owner/unseen", DEFAULT).at(&commit("p1"), 9401);
    took(&main, "hero", "home.png", b"readme private", "passed");

    let answer = get_url(&readme("owner/unseen", "hero", "home.png"), &[]);
    assert_eq!(answer.status, 404, "{}", answer.text());
}

pub(crate) fn an_unknown_readme_picture_is_not_found() {
    let main = Job::push("owner/readme", DEFAULT).at(&commit("u1"), 9501);
    took(&main, "known", "home.png", b"readme known", "passed");

    for (repository, check, picture) in [
        ("owner/readme", "known", "away.png"),
        ("owner/readme", "unknown", "home.png"),
        // Installed and public, but no run.
        ("owner/open", "known", "home.png"),
        // Public, without the app.
        ("owner/bare", "known", "home.png"),
        ("owner/nowhere", "known", "home.png"),
    ] {
        let answer = get_url(&readme(repository, check, picture), &[]);
        assert_eq!(
            answer.status,
            404,
            "{repository} {check} {picture}: {}",
            answer.text()
        );
    }
    // Neither the check alone nor a path without a picture names anything.
    let files = &world().worker.files;
    for path in [
        "/readme/owner/readme/known",
        "/readme/owner/readme",
        "/readme",
    ] {
        let answer = get_url(&format!("{files}{path}"), &[]);
        assert_eq!(answer.status, 404, "{path}: {}", answer.text());
    }
}
