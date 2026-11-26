//! Keeping the CLI up to date: anyone may ask the host for the newest release of Fearlezz
//! (from GitHub, kept an hour) and the lowest version whose runs it takes, and the host refuses
//! to open a run for a CLI older than that, by the version its `User-Agent` names.

use super::notes::cli_token;
use super::worker::CLI_MINIMUM;
use super::*;

pub(crate) fn the_cli_is_told_the_newest_release_and_the_lowest_version_taken() {
    let github = &world().github;
    let releases = "/repos/fearlezz-dev/fearlezz/releases/latest";
    let before = github.asked("GET", releases);
    for _ in 0..2 {
        let answer = get("/api/v2/cli", &[]);
        assert_eq!(answer.status, 200, "no sign in needed: {}", answer.text());
        assert_eq!(
            answer.json(),
            json!({"latest": github::LATEST, "minimum": CLI_MINIMUM}),
            "the newest release without its v, and the lowest version taken"
        );
    }
    assert!(
        github.asked("GET", releases) <= before + 1,
        "the newest release is kept, not asked of GitHub each time"
    );
}

pub(crate) fn a_run_from_a_cli_older_than_the_host_takes_is_refused() {
    let dev = cli_token("dev");
    let job = Job::local("owner/local", "local/versions", &dev);
    let body = job.opening(&["tests"]).to_string().into_bytes();
    let auth = format!("Bearer {dev}");
    let open = |agent: &str| {
        request(
            "POST",
            "/api/v2/runs",
            &[("Authorization", &auth), ("User-Agent", agent)],
            Some(body.clone()),
        )
    };

    let answer = open("fearlezz/0.5.3");
    assert_eq!(answer.status, 426, "{}", answer.text());
    assert_eq!(
        answer.json().get("error"),
        Some(&json!(format!(
            "Fearlezz 0.5.3 is older than this proof host takes ({CLI_MINIMUM} or newer)"
        ))),
        "the refusal names the caller's version and the lowest taken"
    );
    let answer = open("fearlezz");
    assert_eq!(answer.status, 426, "{}", answer.text());
    assert!(
        answer.text().contains("a Fearlezz that names no version"),
        "{}",
        answer.text()
    );

    let answer = open(&format!("fearlezz/{CLI_MINIMUM}"));
    assert_eq!(
        answer.status,
        201,
        "the lowest version taken: {}",
        answer.text()
    );
}
