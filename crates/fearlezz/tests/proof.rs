//! The proof bundle: `fearlezz run --ci --proof <dir>` writes everything one proving run
//! showed (each check with its output, tests and findings, what the change weakens, red before
//! green, and every picture with its media) as `proof.json` beside the files it names, so a
//! page can show all of it without the machine that ran it.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use std::path::{Path, PathBuf};

use common::Repo;
use serde_json::Value;

/// `tests` prints a line and passes; `lint` fails while `src/broken` exists.
const CHECKS: &str = r#"
[check.tests]
run = "echo all tests passed"
stage = "working"
inputs = ["src/**"]

[check.lint]
run = "echo looking; test ! -e src/broken"
stage = "working"
inputs = ["src/**"]
"#;

fn repo(config: &str, files: &[(&str, &str)]) -> Repo {
    let mut all = vec![(".gitignore", ".runs/\n")];
    all.extend_from_slice(files);
    let repo = Repo::new(config, &all);
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    repo
}

/// `fearlezz run --ci --proof` with `env`, and the bundle's folder and `proof.json`.
fn prove(repo: &Repo, env: &[(&str, &str)]) -> (common::Output, PathBuf, Value) {
    let dir = repo.path(".runs/proof");
    let out = repo.fz_env(&["run", "--ci", "--proof", dir.to_str().unwrap()], env);
    let text = std::fs::read_to_string(dir.join("proof.json"))
        .unwrap_or_else(|_| panic!("no proof.json\n{}\n{}", out.stdout, out.stderr));
    (out, dir, serde_json::from_str(&text).unwrap())
}

fn check<'a>(proof: &'a Value, id: &str) -> &'a Value {
    proof["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("no check {id} in {proof:#}"))
}

/// A file the bundle names, relative to its folder.
fn file(dir: &Path, rel: &Value) -> String {
    let rel = rel.as_str().unwrap_or_else(|| panic!("not a path: {rel}"));
    assert!(!rel.starts_with('/'), "absolute: {rel}");
    std::fs::read_to_string(dir.join(rel)).unwrap_or_else(|_| panic!("missing {rel}"))
}

#[test]
fn a_ci_run_writes_every_check_with_its_output() {
    let repo = repo(CHECKS, &[("src/a.txt", "a\n")]);
    repo.branch("feat/x");
    repo.write("src/broken", "x\n");
    repo.commit("break");
    let (out, dir, proof) = prove(&repo, &[]);
    assert!(!out.ok, "{}", out.stdout);
    assert_eq!(proof["schema"], 1, "{proof:#}");
    let head = repo.git(&["rev-parse", "HEAD"]);
    assert_eq!(proof["head"], head.trim(), "{proof:#}");
    assert_eq!(proof["base"], "main", "{proof:#}");
    let tests = check(&proof, "tests");
    assert_eq!(tests["outcome"], "passed", "{proof:#}");
    assert_eq!(tests["run"], "echo all tests passed", "{proof:#}");
    assert!(tests["seconds"].is_number(), "{proof:#}");
    assert!(file(&dir, &tests["output"]).contains("all tests passed"));
    let lint = check(&proof, "lint");
    assert_eq!(lint["outcome"], "failed", "{proof:#}");
    assert_eq!(lint["exit_code"], 1, "{proof:#}");
    assert!(file(&dir, &lint["output"]).contains("looking"));
}

#[test]
fn the_proof_bundle_needs_ci() {
    let repo = repo(CHECKS, &[("src/a.txt", "a\n")]);
    repo.branch("feat/x");
    let out = repo.fz(&["run", "--proof", "x"]);
    assert!(!out.ok, "{}", out.stdout);
}

#[test]
fn the_bundle_names_the_pull_request_by_its_title() {
    let repo = repo(CHECKS, &[("src/a.txt", "a\n")]);
    repo.branch("feat/x");
    repo.write("src/a.txt", "b\n");
    repo.commit("change");
    let head = repo.git(&["rev-parse", "HEAD"]);
    let event = repo.path(".runs/event.json");
    std::fs::write(
        &event,
        serde_json::json!({"pull_request": {
            "number": 17,
            "title": "feat: settings save on enter",
            "body": "a long description the page leaves out",
            "html_url": "https://github.com/owner/repo/pull/17",
            "head": {"sha": head.trim()},
        }})
        .to_string(),
    )
    .unwrap();
    let (out, _, proof) = prove(
        &repo,
        &[
            ("GITHUB_EVENT_PATH", event.to_str().unwrap()),
            ("GITHUB_REPOSITORY", "owner/repo"),
        ],
    );
    assert!(out.ok, "{}", out.stdout);
    assert_eq!(proof["repository"], "owner/repo", "{proof:#}");
    assert_eq!(proof["pull"]["number"], 17, "{proof:#}");
    assert_eq!(
        proof["pull"]["title"], "feat: settings save on enter",
        "{proof:#}"
    );
    assert_eq!(
        proof["pull"]["url"], "https://github.com/owner/repo/pull/17",
        "{proof:#}"
    );
    assert!(
        !proof.to_string().contains("long description"),
        "only the title: {proof:#}"
    );
}

/// A linter that flags every line with TODO (gate) or XXX (flag) in `src`.
const LINT: &str = r#"
[check.lint]
run = '''
grep -n -H -e TODO -e XXX src/*.txt | while IFS=: read -r file line text; do
  case "$text" in
    *TODO*) level=gate ;;
    *) level=flag ;;
  esac
  printf '{"file":"%s","line":%s,"level":"%s","rule":"no-markers","message":"marker left in"}\n' "$file" "$line" "$level"
done > "$FEARLEZZ_OUT/findings.jsonl"
'''
stage = "working"
inputs = ["src/**"]
findings = "jsonl"
"#;

#[test]
fn the_bundle_keeps_the_findings_of_passing_checks_too() {
    let repo = repo(LINT, &[("src/a.txt", "one\nTODO from long ago\n")]);
    repo.branch("feat/x");
    repo.write("src/a.txt", "one\nTODO from long ago\nXXX new\n");
    repo.commit("change");
    let (out, _, proof) = prove(&repo, &[]);
    assert!(out.ok, "a flag does not block: {}", out.stdout);
    let lint = check(&proof, "lint");
    let findings = lint["findings"].as_array().unwrap();
    assert_eq!(findings.len(), 1, "{proof:#}");
    assert_eq!(findings[0]["file"], "src/a.txt", "{proof:#}");
    assert_eq!(findings[0]["line"], 3, "{proof:#}");
    assert_eq!(findings[0]["level"], "flag", "{proof:#}");
    assert_eq!(findings[0]["rule"], "no-markers", "{proof:#}");
    assert_eq!(lint["findings_elsewhere"], 1, "{proof:#}");
}

/// A runner that writes a JUnit case per line of `src/a.txt`.
const JUNIT: &str = r#"
[check.tests]
run = '''
{
  echo '<testsuites><testsuite name="all">'
  while read -r line; do echo "<testcase name=\"$line\"/>"; done < src/a.txt
  echo '</testsuite></testsuites>'
} > "$FEARLEZZ_OUT/junit.xml"
'''
stage = "working"
inputs = ["src/**"]
tests = "junit"
"#;

#[test]
fn the_bundle_names_every_test_that_ran() {
    let repo = repo(JUNIT, &[("src/a.txt", "saves\n")]);
    repo.branch("feat/x");
    repo.write("src/a.txt", "saves\nsaves on enter\n");
    repo.commit("change");
    let (out, _, proof) = prove(&repo, &[]);
    assert!(out.ok, "{}", out.stdout);
    let tests = check(&proof, "tests");
    assert_eq!(tests["tests"], 2, "{proof:#}");
    let names: Vec<&str> = tests["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["saves", "saves on enter"], "{proof:#}");
}

#[test]
fn the_bundle_says_what_the_change_weakens() {
    let repo = repo(CHECKS, &[("src/a.txt", "a\n")]);
    repo.branch("feat/x");
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    repo.write(
        "fearlezz.toml",
        &config.replace("echo looking; test ! -e src/broken", "true"),
    );
    repo.commit("weaker lint");
    let (out, _, proof) = prove(&repo, &[]);
    assert!(!out.ok, "{}", out.stdout);
    let weakened = proof["weakened"].as_array().unwrap();
    assert_eq!(weakened.len(), 1, "{proof:#}");
    assert_eq!(weakened[0]["what"], "check `lint` is weaker", "{proof:#}");
    assert!(weakened[0]["why"].is_string(), "{proof:#}");
    assert!(weakened[0]["fix"].is_string(), "{proof:#}");
}

/// Every shell script in `tests/` must pass; they are the check's test files.
const SCRIPTS: &str = r#"
[check.tests]
run = "for t in tests/*.sh; do sh \"$t\" || exit 1; done"
stage = "working"
inputs = ["src/**", "tests/**"]
test_files = ["tests/**"]
"#;

#[test]
fn the_bundle_shows_red_before_green_per_check() {
    let repo = repo(
        SCRIPTS,
        &[
            ("src/value", "1\n"),
            ("tests/old.sh", "test -e src/value\n"),
        ],
    );
    repo.branch("fix/value");
    repo.write("src/value", "2\n");
    repo.write("tests/new.sh", "test \"$(cat src/value)\" = 2\n");
    repo.commit("fix: the value is two");
    let (out, _, proof) = prove(&repo, &[]);
    assert!(out.ok, "{}", out.stdout);
    let red = &check(&proof, "tests")["red"];
    assert_eq!(red["verdict"], "red", "{proof:#}");
    assert_eq!(
        red["files"],
        serde_json::json!(["tests/new.sh"]),
        "{proof:#}"
    );
}

/// Records two stills: `home` in the color `src/color.txt` names, `about` in white.
const RECORD: &str = r#"set -e
still() {
  ffmpeg -v error -nostdin -f lavfi -i "color=c=$2:s=64x48" -frames:v 1 "$FEARLEZZ_OUT/$1.png"
  echo "{\"kind\":\"still\",\"file\":\"$1.png\"}" >> "$FEARLEZZ_OUT/proof.jsonl"
}
still home "$(cat src/color.txt)"
still about white
"#;

const SCREENS: &str = r#"
[check.screens]
run = "sh record.sh"
stage = "look"
inputs = ["src/**", "record.sh"]
"#;

#[test]
fn the_bundle_holds_every_picture_with_its_media() {
    let repo = repo(SCREENS, &[("record.sh", RECORD), ("src/color.txt", "red")]);
    repo.branch("feat/look");
    repo.write("src/color.txt", "blue");
    repo.commit("color");
    let (out, dir, proof) = prove(&repo, &[]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let looks = proof["looks"].as_array().unwrap();
    assert_eq!(looks.len(), 1, "{proof:#}");
    let look = &looks[0];
    assert_eq!(look["check"], "screens", "{proof:#}");
    let item = |name: &str| {
        look["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["name"] == name)
            .unwrap_or_else(|| panic!("no picture {name} in {proof:#}"))
    };
    let home = item("home.png");
    assert_eq!(home["change"], "changed", "{proof:#}");
    assert!(home["comparison"].is_object(), "{proof:#}");
    let about = item("about.png");
    assert_eq!(about["change"], "same", "unchanged ones too: {proof:#}");
    for picture in [home, about] {
        for side in ["before", "after"] {
            let rel = picture[side].as_str().unwrap();
            assert!(
                dir.join(rel).is_file(),
                "{side} of {} missing: {rel}",
                picture["name"]
            );
        }
    }
}

// The page

fn page(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("index.html")).expect("the bundle has its page")
}

#[test]
fn the_page_leads_with_the_verdict_and_opens_what_broke() {
    let repo = repo(CHECKS, &[("src/a.txt", "a\n")]);
    repo.branch("feat/x");
    repo.write("src/broken", "x\n");
    repo.commit("break");
    let (_, dir, _) = prove(&repo, &[]);
    let html = page(&dir);
    assert!(html.contains("Not ready: 1 thing to fix</h1>"), "{html}");
    assert!(html.contains("Did anything break?"), "{html}");
    // The failed check comes first and open, with why, what to do and its output.
    let lint = html.find("id=\"check-lint\" open").expect("lint open");
    let tests = html.find("id=\"check-tests\"").expect("tests listed");
    assert!(lint < tests, "the failed check comes first");
    assert!(
        html.contains("Its command ended with exit code 1"),
        "{html}"
    );
    assert!(html.contains("looking"), "its output: {html}");
    assert!(html.contains("href=\"outputs/lint.log\""), "{html}");
}

#[test]
fn the_page_says_ready_when_everything_holds() {
    let repo = repo(CHECKS, &[("src/a.txt", "a\n")]);
    repo.branch("feat/x");
    repo.write("src/a.txt", "b\n");
    repo.commit("change");
    let (out, dir, _) = prove(&repo, &[]);
    assert!(out.ok, "{}", out.stdout);
    let html = page(&dir);
    assert!(html.contains("Ready to merge</h1>"), "{html}");
    assert!(!html.contains("Waits for you"), "{html}");
}

#[test]
fn the_page_names_the_pull_request_and_fetches_nothing() {
    let repo = repo(CHECKS, &[("src/a.txt", "a\n")]);
    repo.branch("feat/x");
    repo.write("src/a.txt", "b\n");
    repo.commit("change");
    let event = repo.path(".runs/event.json");
    std::fs::write(
        &event,
        serde_json::json!({"pull_request": {
            "number": 17,
            "title": "feat: settings <save> on enter",
            "html_url": "https://github.com/owner/repo/pull/17",
        }})
        .to_string(),
    )
    .unwrap();
    let (_, dir, _) = prove(&repo, &[("GITHUB_EVENT_PATH", event.to_str().unwrap())]);
    let html = page(&dir);
    assert!(
        html.contains("#17</span> feat: settings &lt;save&gt; on enter"),
        "escaped title: {html}"
    );
    assert!(!html.contains("src=\"http"), "{html}");
    assert!(!html.contains("<link"), "{html}");
    assert!(html.contains("prefers-reduced-motion"));
    assert!(html.contains("prefers-color-scheme: dark"));
}

#[test]
fn the_page_puts_weaker_checks_with_what_waits_for_a_person() {
    let repo = repo(CHECKS, &[("src/a.txt", "a\n")]);
    repo.branch("feat/x");
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    repo.write(
        "fearlezz.toml",
        &config.replace("echo looking; test ! -e src/broken", "true"),
    );
    repo.commit("weaker lint");
    let (_, dir, _) = prove(&repo, &[]);
    let html = page(&dir);
    assert!(html.contains("Not ready: 1 thing wait"), "{html}");
    let waits = html.find("Waits for you").expect("waits");
    let weak = html
        .find("Check <code>lint</code> is weaker")
        .expect("the weakening");
    assert!(waits < weak && weak < html.find("Did anything break?").unwrap());
    // Outside a pull request nothing can accept it, so the page offers no accept.
    assert!(!html.contains("data-post=\"accept\""), "{html}");
}

#[test]
fn the_page_offers_an_accept_where_a_weakening_waits() {
    let repo = repo(CHECKS, &[("src/a.txt", "a\n")]);
    repo.branch("feat/x");
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    repo.write(
        "fearlezz.toml",
        &config.replace("echo looking; test ! -e src/broken", "true"),
    );
    repo.commit("weaker lint");
    let head = repo.git(&["rev-parse", "HEAD"]).trim().to_string();
    let gh = common::github::FakeGitHub::start();
    gh.state().pulls.insert(7, (head.clone(), "open".into()));
    let event = repo.path(".runs/event.json");
    let pull = serde_json::json!({
        "action": "synchronize",
        "number": 7,
        "pull_request": {
            "number": 7,
            "title": "feat: a weaker lint",
            "head": {"sha": head, "repo": {"full_name": "owner/repo"}},
        },
        "repository": {"full_name": "owner/repo"},
    });
    std::fs::write(&event, pull.to_string()).unwrap();
    let graphql = format!("{}/graphql", gh.url);
    let temp = repo.path(".runs");
    let (out, dir, proof) = prove(
        &repo,
        &[
            ("GITHUB_ACTIONS", "true"),
            ("GITHUB_EVENT_NAME", "pull_request"),
            ("GITHUB_EVENT_PATH", event.to_str().unwrap()),
            ("GITHUB_TOKEN", common::github::WORKFLOW_TOKEN),
            ("GITHUB_API_URL", &gh.url),
            ("GITHUB_GRAPHQL_URL", &graphql),
            ("GITHUB_SERVER_URL", "https://github.com"),
            ("GITHUB_REPOSITORY", "owner/repo"),
            ("GITHUB_RUN_ID", "42"),
            ("GITHUB_RUN_ATTEMPT", "1"),
            ("RUNNER_TEMP", temp.to_str().unwrap()),
        ],
    );
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert_eq!(proof["weakenings_wait"], true, "{proof:#}");
    let html = page(&dir);
    let weak = html
        .find("Check <code>lint</code> is weaker")
        .expect("the weakening");
    let accept = html.find("data-post=\"accept\"").expect("an accept");
    assert!(weak < accept, "the accept sits with the weakening");
    // Locally the comment is there to copy; framed, a button has the host post it.
    assert!(html.contains("/fearlezz accept &lt;why the weaker proof is right&gt;"));
    assert!(html.contains("<span>Accept</span>"), "{html}");
    assert!(html.contains("<span>Copy accept comment</span>"), "{html}");
}

#[test]
fn the_page_lets_a_person_approve_in_their_own_words() {
    let repo = repo(SCREENS, &[("record.sh", RECORD), ("src/color.txt", "red")]);
    repo.branch("feat/look");
    repo.write("src/color.txt", "blue");
    repo.commit("color");
    let (_, dir, _) = prove(&repo, &[]);
    let html = page(&dir);
    let row = &html[html.find("class=\"row approve\"").expect("the approve row")..];
    let row = &row[..row.find("</section>").unwrap()];
    assert!(
        row.contains("data-post=\"approve\" data-check=\"screens\""),
        "{row}"
    );
    // The words are the person's own: nothing is filled in for them.
    assert!(row.contains("<input required"), "{row}");
    assert!(!row.contains("value=\"looks good\""), "{row}");
    assert!(row.contains("<span>Approve</span>"), "{row}");
    // Locally the command stays; framed, the page asks its parent, the host's shell, and
    // fetches nothing itself.
    assert!(row.contains("fearlezz say approve screens --said"), "{row}");
    assert!(html.contains("var FRAMED = window.parent !== window;"));
    assert!(html.contains("window.parent.postMessage({ kind: 'fearlezz:' + act.dataset.post"));
    assert!(html.contains("e.source !== window.parent"));
    assert!(!html.contains("fetch("), "the page fetches nothing");
}

#[test]
fn the_page_shows_every_picture_and_how_to_approve_them() {
    let repo = repo(SCREENS, &[("record.sh", RECORD), ("src/color.txt", "red")]);
    repo.branch("feat/look");
    repo.write("src/color.txt", "blue");
    repo.commit("color");
    let (_, dir, _) = prove(&repo, &[]);
    let html = page(&dir);
    assert!(html.contains("Does it look right to you?"), "{html}");
    assert!(
        html.contains("fearlezz say approve screens --said"),
        "{html}"
    );
    assert!(html.contains("data-name=\"home.png\" data-change=\"changed\""));
    assert!(html.contains("data-name=\"about.png\" data-change=\"same\""));
    for src in html.split("src=\"").skip(1) {
        let rel = src.split('"').next().unwrap();
        assert!(
            dir.join(rel).is_file(),
            "the page shows {rel}, which is missing"
        );
    }
}
