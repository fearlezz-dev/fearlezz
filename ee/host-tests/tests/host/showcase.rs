//! Showcase media: the files origin serves the newest proven picture of a push to the default
//! branch of a repository with the app, private or public, at a stable address,
//! `/showcase/<owner>/<repo>/<check>/<picture>`, with no grant, so a README, a docs site or a
//! website can embed what the proof took.

use super::*;

/// The fake GitHub's default branch of every repository.
const DEFAULT: &str = "main";

/// A done run of `job` whose check `check` took one new still `picture` as `bytes`, and came
/// out as `outcome`: a failed check fails its proof. Its id.
fn took(job: &Job, check: &str, picture: &str, bytes: &[u8], outcome: &str) -> String {
    took_all(job, &[(check, picture, bytes)], outcome, &json!({}))
}

/// A done run of `job` whose checks each took one new picture, `(check, picture, bytes)`, every
/// check out as `outcome`, with `extra` laid over its proof. Its id.
fn took_all(job: &Job, pictures: &[(&str, &str, &[u8])], outcome: &str, extra: &Value) -> String {
    let checks: Vec<&str> = pictures.iter().map(|(check, _, _)| *check).collect();
    let run = job.open(&checks);
    let mut entries = Vec::new();
    let mut looks = Vec::new();
    let mut all = Vec::new();
    for (check, picture, bytes) in pictures {
        let path = format!("looks/{check}/after/{picture}");
        let look = look_of(check, &[still(picture, "new", None, Some(&path))]);
        let files = [(path.as_str(), *bytes)];
        let mut ended = entry(check, outcome);
        if outcome == "failed" {
            ended["failure"] =
                json!({"what": "it failed", "why": "a test failed", "fix": "fix it"});
        }
        let answer = job.finished_with_look(&run.id, &ended, &files, &look);
        assert_eq!(answer.status, 200, "{}", answer.text());
        job.send(bytes);
        entries.push(ended);
        looks.push(look);
        all.push((path, *bytes));
    }
    let files: Vec<(&str, &[u8])> = all.iter().map(|(p, b)| (p.as_str(), *b)).collect();
    let answer = job.close_with(&run.id, &entries, &looks, &files, extra);
    assert_eq!(answer.status, 200, "{}", answer.text());
    run.id
}

/// The bytes the showcase address of `picture` of `check` in owner/showcase serves, or its status
/// when it serves none.
fn shows(check: &str, picture: &str) -> Result<Vec<u8>, u16> {
    let answer = get_url(&showcase("owner/showcase", check, picture), &[]);
    match answer.status {
        200 => Ok(answer.body),
        status => Err(status),
    }
}

/// The showcase address of `picture` of `check` in `repository`.
fn showcase(repository: &str, check: &str, picture: &str) -> String {
    format!(
        "{}/showcase/{repository}/{check}/{picture}",
        world().worker.files
    )
}

pub(crate) fn a_showcase_picture_is_the_newest_proven_push_to_the_default_branch() {
    let main = Job::push("owner/showcase", DEFAULT);
    took(
        &main.at(&commit("r1"), 9101),
        "hero",
        "card/light.png",
        b"showcase first",
        "passed",
    );
    took(
        &main.at(&commit("r2"), 9102),
        "hero",
        "card/light.png",
        b"showcase second",
        "passed",
    );
    // A newer push whose proof failed shows nothing of its own.
    took(
        &main.at(&commit("r3"), 9103),
        "hero",
        "card/light.png",
        b"showcase broken",
        "failed",
    );

    let answer = get_url(&showcase("owner/showcase", "hero", "card/light.png"), &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.body, b"showcase second", "the newest proven push");
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
    // A docs site or any website embeds it, also into a canvas or from a script.
    assert_eq!(
        header("access-control-allow-origin"),
        "*",
        "any origin reads it"
    );
    assert_eq!(
        header("cross-origin-resource-policy"),
        "cross-origin",
        "any origin loads it"
    );
    // Asked again with what it has, a cache gets no bytes until a push changes the picture.
    let tag = header("etag");
    assert_eq!(
        tag,
        format!("\"{}\"", sha256(b"showcase second")),
        "tagged by its bytes"
    );
    let answer = get_url(
        &showcase("owner/showcase", "hero", "card/light.png"),
        &[("If-None-Match", &tag)],
    );
    assert_eq!(answer.status, 304, "{}", answer.text());
    assert_eq!(answer.body, b"", "a 304 has no bytes");
}

pub(crate) fn a_showcase_picture_never_comes_from_another_branch_or_a_pull_request() {
    let main = Job::push("owner/showcase", DEFAULT).at(&commit("b1"), 9201);
    took(&main, "branches", "home.png", b"showcase of main", "passed");
    let branch = Job::push("owner/showcase", "feat/showcase").at(&commit("b2"), 9202);
    took(
        &branch,
        "branches",
        "home.png",
        b"showcase of a branch",
        "passed",
    );
    let pull = Job::pull("owner/showcase", 9203, "showcase/pull").at(&commit("b3"), 9203);
    took(
        &pull,
        "branches",
        "home.png",
        b"showcase of a pull",
        "passed",
    );

    let answer = get_url(&showcase("owner/showcase", "branches", "home.png"), &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(
        answer.body, b"showcase of main",
        "only the default branch counts"
    );
}

pub(crate) fn a_showcase_picture_of_a_check_in_a_folder_names_the_check_then_the_picture() {
    let main = Job::push("owner/showcase", DEFAULT).at(&commit("f1"), 9301);
    took(
        &main,
        "look/app",
        "films/in_use.webp",
        b"showcase film",
        "passed",
    );

    let answer = get_url(
        &showcase("owner/showcase", "look/app", "films/in_use.webp"),
        &[],
    );
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.body, b"showcase film", "the check, then the picture");
    assert_eq!(
        answer.header("content-type"),
        Some("image/webp"),
        "its name pins its type"
    );
}

pub(crate) fn a_showcase_picture_of_a_private_repository_with_the_app_is_served() {
    let main = Job::push("owner/unseen", DEFAULT).at(&commit("p1"), 9401);
    took(&main, "hero", "home.png", b"showcase private", "passed");

    // GitHub's image proxy loads a README's pictures without a sign in, private or not.
    let answer = get_url(&showcase("owner/unseen", "hero", "home.png"), &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(answer.body, b"showcase private", "the picture itself");
}

pub(crate) fn an_unknown_showcase_picture_is_not_found() {
    let main = Job::push("owner/showcase", DEFAULT).at(&commit("u1"), 9501);
    took(&main, "known", "home.png", b"showcase known", "passed");

    for (repository, check, picture) in [
        ("owner/showcase", "known", "away.png"),
        ("owner/showcase", "unknown", "home.png"),
        // Installed and public, but no run.
        ("owner/open", "known", "home.png"),
        // Public, without the app.
        ("owner/bare", "known", "home.png"),
        // Private, without the app.
        ("owner/hidden", "known", "home.png"),
        ("owner/nowhere", "known", "home.png"),
    ] {
        let answer = get_url(&showcase(repository, check, picture), &[]);
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
        "/showcase/owner/showcase/known",
        "/showcase/owner/showcase",
        "/showcase",
    ] {
        let answer = get_url(&format!("{files}{path}"), &[]);
        assert_eq!(answer.status, 404, "{path}: {}", answer.text());
    }
}

pub(crate) fn a_showcase_picture_comes_only_from_a_run_a_push_started() {
    let main = Job::push("owner/showcase", DEFAULT);
    took(
        &main.at(&commit("e1"), 9601),
        "events",
        "home.png",
        b"showcase pushed",
        "passed",
    );
    // A pull_request_target run carries the default branch's ref while it checks out a fork's
    // pull request: what it took never reaches the showcase.
    took(
        &main.at(&commit("e2"), 9602).by("pull_request_target"),
        "events",
        "home.png",
        b"showcase of a fork",
        "passed",
    );
    // Nor does a push run that proves another commit than the one pushed.
    took(
        &main.at(&commit("e3"), 9603).naming(&commit("e1")),
        "events",
        "home.png",
        b"showcase of another commit",
        "passed",
    );

    assert_eq!(
        shows("events", "home.png").as_deref(),
        Ok(&b"showcase pushed"[..]),
        "only the push's own commit counts"
    );
}

pub(crate) fn a_rerun_of_an_older_push_never_takes_the_showcase_picture_back() {
    let main = Job::push("owner/showcase", DEFAULT);
    let older = main.at(&commit("o1"), 9701);
    took(&older, "rerun", "home.png", b"showcase older", "passed");
    took(
        &main.at(&commit("o2"), 9702),
        "rerun",
        "home.png",
        b"showcase newer",
        "passed",
    );
    // The older push's job runs again, after a flake or an accept: its run opens last.
    took(
        &older.again(),
        "rerun",
        "home.png",
        b"showcase older again",
        "passed",
    );

    assert_eq!(
        shows("rerun", "home.png").as_deref(),
        Ok(&b"showcase newer"[..]),
        "the newest push, not the newest run"
    );
}

pub(crate) fn a_showcase_picture_counts_a_weakening_only_once_an_admin_accepted_it() {
    let main = Job::push("owner/showcase", DEFAULT);
    // Two checks, one asked before the accept and one after, as each address is kept a while.
    let pictures = |at: &'static [u8]| {
        [
            ("weak-before", "home.png", at),
            ("weak-after", "home.png", at),
        ]
    };
    took_all(
        &main.at(&commit("w1"), 9801),
        &pictures(b"showcase strong"),
        "passed",
        &json!({}),
    );
    let job = main.at(&commit("w2"), 9802);
    world()
        .github
        .job_ended("owner/showcase", 9802, 98021, "proof", RUNNER, "failure");
    let run = took_all(&job, &pictures(b"showcase weakened"), "passed", &weakened());
    assert_eq!(
        shows("weak-before", "home.png").as_deref(),
        Ok(&b"showcase strong"[..]),
        "what a push weakens fails it until an admin accepts it"
    );

    let admin = sign_in("admin-code", "/");
    let answer = say(&run, "accept", Some(&admin), &[], &json!({}));
    assert_eq!(answer.status, 201, "{}", answer.text());
    assert_eq!(
        shows("weak-after", "home.png").as_deref(),
        Ok(&b"showcase weakened"[..]),
        "an accepted weakening does not fail it"
    );
}

pub(crate) fn a_picture_the_newest_proven_push_no_longer_takes_is_not_found() {
    let main = Job::push("owner/showcase", DEFAULT);
    took(
        &main.at(&commit("g1"), 9901),
        "gone",
        "old.png",
        b"showcase old",
        "passed",
    );
    took(
        &main.at(&commit("g2"), 9902),
        "gone",
        "new.png",
        b"showcase new",
        "passed",
    );

    assert_eq!(
        shows("gone", "old.png"),
        Err(404),
        "the showcase shows what main proves now"
    );
    assert_eq!(
        shows("gone", "new.png").as_deref(),
        Ok(&b"showcase new"[..]),
        "what it takes now is served"
    );
}

pub(crate) fn a_showcase_picture_names_the_longest_check_of_its_run() {
    let main = Job::push("owner/showcase", DEFAULT).at(&commit("l1"), 10001);
    took_all(
        &main,
        &[
            ("long", "app/home.png", b"showcase of long"),
            ("long/app", "home.png", b"showcase of long/app"),
            ("long", "menu.png", b"showcase menu of long"),
        ],
        "passed",
        &json!({}),
    );

    assert_eq!(
        shows("long/app", "home.png").as_deref(),
        Ok(&b"showcase of long/app"[..]),
        "long/app/home.png is long/app's home.png, not long's app/home.png"
    );
    assert_eq!(
        shows("long", "menu.png").as_deref(),
        Ok(&b"showcase menu of long"[..]),
        "a picture only the shorter check spells is its own"
    );
}

pub(crate) fn a_showcase_film_answers_a_range() {
    let film: &[u8] = b"showcase film bytes";
    let main = Job::push("owner/showcase", DEFAULT).at(&commit("m1"), 10101);
    took(&main, "films", "in_use.mp4", film, "passed");
    let address = showcase("owner/showcase", "films", "in_use.mp4");

    let answer = get_url(&address, &[("Range", "bytes=0-5")]);
    assert_eq!(answer.status, 206, "{}", answer.text());
    assert_eq!(answer.body.get(..), film.get(..6), "the first six bytes");
    assert_eq!(
        answer.header("content-range"),
        Some(format!("bytes 0-5/{}", film.len()).as_str()),
        "the range it sends"
    );
    assert_eq!(
        answer.header("content-type"),
        Some("video/mp4"),
        "its name pins its type"
    );
    let tag = answer.header("etag").unwrap_or_default().to_owned();

    // A range of the version a cache holds comes as a range only while that version is served.
    let answer = get_url(&address, &[("Range", "bytes=6-"), ("If-Range", &tag)]);
    assert_eq!(answer.status, 206, "{}", answer.text());
    assert_eq!(answer.body.get(..), film.get(6..), "the rest of the film");
    let answer = get_url(
        &address,
        &[("Range", "bytes=6-"), ("If-Range", "\"older\"")],
    );
    assert_eq!(answer.status, 200, "another version gets the whole film");
    assert_eq!(answer.body, film, "the whole film");
}

pub(crate) fn a_showcase_picture_waits_for_a_newer_push_to_finish() {
    let main = Job::push("owner/showcase", DEFAULT);
    took(
        &main.at(&commit("d1"), 10201),
        "open",
        "home.png",
        b"showcase finished",
        "passed",
    );
    // A newer push whose run is still open has no proof yet.
    let open = main.at(&commit("d2"), 10202);
    let run = open.open(&["open"]);
    let path = "looks/open/after/home.png";
    let look = look_of("open", &[still("home.png", "new", None, Some(path))]);
    let answer = open.finished_with_look(
        &run.id,
        &entry("open", "passed"),
        &[(path, b"showcase open")],
        &look,
    );
    assert_eq!(answer.status, 200, "{}", answer.text());
    open.send(b"showcase open");

    assert_eq!(
        shows("open", "home.png").as_deref(),
        Ok(&b"showcase finished"[..]),
        "a run counts once it is done with its proof"
    );
}

pub(crate) fn a_showcase_serves_no_text_and_no_name_github_could_not_have() {
    let main = Job::push("owner/showcase", DEFAULT).at(&commit("t1"), 10301);
    let path = "looks/texts/after/out.txt";
    let mut text = still("out.txt", "new", None, Some(path));
    text["kind"] = json!("text");
    let run = main.open(&["texts"]);
    let look = look_of("texts", &[text]);
    let answer = main.finished_with_look(
        &run.id,
        &entry("texts", "passed"),
        &[(path, b"showcase text")],
        &look,
    );
    assert_eq!(answer.status, 200, "{}", answer.text());
    main.send(b"showcase text");
    let answer = main.close(
        &run.id,
        &[entry("texts", "passed")],
        &[look],
        &[(path, b"showcase text")],
    );
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(shows("texts", "out.txt"), Err(404), "only stills and films");

    let files = &world().worker.files;
    let github = &world().github;
    // A slash inside a segment would name another path of GitHub's API.
    for (owner, name, asked) in [
        (
            "own%2Fer",
            "showcase",
            "/repos/own/er/showcase/installation",
        ),
        ("owner", "read%2Fme", "/repos/owner/read/me/installation"),
    ] {
        let answer = get_url(
            &format!("{files}/showcase/{owner}/{name}/known/home.png"),
            &[],
        );
        assert_eq!(answer.status, 404, "{owner}/{name}: {}", answer.text());
        assert_eq!(
            github.asked("GET", asked),
            0,
            "{asked} is never asked of GitHub"
        );
    }
}

pub(crate) fn a_showcase_picture_outlasts_many_newer_pushes_that_did_not_prove_it() {
    let main = Job::push("owner/showcase", DEFAULT);
    took(
        &main.at(&commit("ol0"), 10601),
        "lasting",
        "home.png",
        b"showcase lasting",
        "passed",
    );
    // Many pushes reuse the check's earlier result, so they take no picture of it, and many
    // others fail: neither replaces the picture, however many there are.
    for n in 1..=24u64 {
        took(
            &main.at(&commit(&format!("ol{n:02}")), 10601 + n),
            "elsewhere",
            "home.png",
            format!("showcase elsewhere {n}").as_bytes(),
            "passed",
        );
        took(
            &main.at(&commit(&format!("olf{n:02}")), 10701 + n),
            "lasting",
            "home.png",
            format!("showcase broken {n}").as_bytes(),
            "failed",
        );
    }

    assert_eq!(
        shows("lasting", "home.png").as_deref(),
        Ok(&b"showcase lasting"[..]),
        "the newest proven push that took it, however old"
    );
}

/// A done, passed run of `job` whose check `check` took `items`, with `files` its pictures.
fn took_look(job: &Job, check: &str, items: &[Value], files: &[(&str, &[u8])]) {
    let run = job.open(&[check]);
    let look = look_of(check, items);
    let ended = entry(check, "passed");
    let answer = job.finished_with_look(&run.id, &ended, files, &look);
    assert_eq!(answer.status, 200, "{}", answer.text());
    for (_, bytes) in files {
        job.send(bytes);
    }
    let answer = job.close(&run.id, &[ended], &[look], files);
    assert_eq!(answer.status, 200, "{}", answer.text());
}

/// A new film `name` whose after is at `after`, with its animated WebP copy at `webp`.
fn film_with_copy(name: &str, after: &str, webp: &str) -> Value {
    let mut film = still(name, "new", None, Some(after));
    film["kind"] = json!("film");
    film["webp"] = json!(webp);
    film
}

pub(crate) fn a_films_webp_address_serves_its_animated_copy() {
    let main = Job::push("owner/readme", DEFAULT).at(&commit("w1"), 10401);
    let (film, copy) = (
        "looks/films/after/home.webm",
        "looks/films/after/home.anim.webp",
    );
    let (bare, bare_copy) = (
        "looks/films/after/intro",
        "looks/films/after/intro.anim.webp",
    );
    took_look(
        &main,
        "films",
        &[
            film_with_copy("home.webm", film, copy),
            film_with_copy("intro", bare, bare_copy),
        ],
        &[
            (film, b"readme webm film"),
            (copy, b"readme webm copy"),
            (bare, b"readme bare film"),
            (bare_copy, b"readme bare copy"),
        ],
    );

    let answer = get_url(&readme("owner/readme", "films", "home.webp"), &[]);
    assert_eq!(answer.status, 200, "{}", answer.text());
    assert_eq!(
        answer.body, b"readme webm copy",
        "the film's extension swapped"
    );
    assert_eq!(answer.header("content-type"), Some("image/webp"));
    assert_eq!(
        shows("films", "intro.webp").as_deref(),
        Ok(&b"readme bare copy"[..]),
        "a film named without an extension gets one"
    );
    assert_eq!(
        shows("films", "home.webm").as_deref(),
        Ok(&b"readme webm film"[..]),
        "the film itself is still served as recorded"
    );
    assert_eq!(shows("films", "home.gif"), Err(404), "only a WebP copy");
}

pub(crate) fn a_still_named_like_a_films_copy_wins() {
    let main = Job::push("owner/readme", DEFAULT).at(&commit("w2"), 10501);
    let (film, copy) = (
        "looks/both/after/home.webm",
        "looks/both/after/home.anim.webp",
    );
    let real = "looks/both/after/home.webp";
    took_look(
        &main,
        "both",
        &[
            film_with_copy("home.webm", film, copy),
            still("home.webp", "new", None, Some(real)),
        ],
        &[
            (film, b"readme both film"),
            (copy, b"readme both copy"),
            (real, b"readme both still"),
        ],
    );

    assert_eq!(
        shows("both", "home.webp").as_deref(),
        Ok(&b"readme both still"[..]),
        "the still the check took, not the film's copy"
    );
}
