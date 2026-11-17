//! Preview deploys: `[env] preview` deploys the checkout and prints its URL, once per tree, and a
//! check that needs the preview gets that URL. The deploy here is a shell command that notes each
//! deploy in a log and prints a URL built from the name it was given.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;

/// Notes each deploy with its name in `$DEPLOYS` and prints the URL among other words, as a
/// real deploy tool does.
const PREVIEW: &str = r#"preview = 'echo "deploy $FEARLEZZ_PREVIEW_NAME" >> "$DEPLOYS"; echo "uploading (see https://docs.example.dev/deploys)"; echo "Version preview URL: https://$FEARLEZZ_PREVIEW_NAME.example.dev/"'"#;

/// `site` needs the preview and keeps the URL it got; `unit` does not.
const CHECKS: &str = r#"
[check.site]
run = 'echo "$FEARLEZZ_PREVIEW_URL" > .runs/url; echo run >> .runs/site'
stage = "working"
inputs = ["src/**"]
needs = ["preview"]

[check.unit]
run = "echo run >> .runs/unit"
stage = "working"
inputs = ["src/**"]
"#;

struct Setup {
    repo: Repo,
    deploys: String,
}

impl Setup {
    /// A repository whose config is `[env]` with `env` and then `checks`, with a change on the
    /// branch `branch`.
    fn new(env: &str, checks: &str, branch: &str) -> Setup {
        let repo = Repo::new(
            // The deploy runs in a clean env, so it sees the log of deploys only as declared.
            &format!("[env]\nenv = [\"DEPLOYS\"]\n{env}\n{checks}"),
            &[
                (".gitignore", ".runs/\n"),
                ("src/lib.txt", "a"),
                ("secretspec.toml", SECRETSPEC),
            ],
        );
        std::fs::create_dir_all(repo.path(".runs")).unwrap();
        let deploys = repo.path(".runs/deploys").to_string_lossy().to_string();
        repo.branch(branch);
        repo.write("src/lib.txt", "b");
        repo.commit("change");
        Setup { repo, deploys }
    }

    fn fz(&self, args: &[&str]) -> common::Output {
        self.fz_env(args, &[])
    }

    fn fz_env(&self, args: &[&str], env: &[(&str, &str)]) -> common::Output {
        let mut all = vec![("DEPLOYS", self.deploys.as_str())];
        all.extend_from_slice(env);
        self.repo.fz_env(args, &all)
    }

    fn deploys(&self) -> Vec<String> {
        std::fs::read_to_string(&self.deploys)
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    fn url(&self) -> String {
        std::fs::read_to_string(self.repo.path(".runs/url"))
            .unwrap_or_default()
            .trim()
            .to_string()
    }
}

const SECRETSPEC: &str = r#"[project]
name = "demo"
revision = "1.0"

[profiles.default]
CLOUDFLARE_API_TOKEN = { description = "deploys previews", required = true }
"#;

const TOKEN: &str = "cf-test-0123456789";

#[test]
fn a_check_that_needs_the_preview_gets_its_url_named_for_the_branch() {
    let s = Setup::new(PREVIEW, CHECKS, "feat/Site_Look--New");
    let out = s.fz(&["run", "--stage", "working"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert_eq!(s.url(), "https://feat-site-look-new.example.dev/");
    assert_eq!(s.deploys(), ["deploy feat-site-look-new"]);
    assert!(
        out.stdout
            .contains("preview deployed: https://feat-site-look-new.example.dev/"),
        "{}",
        out.stdout
    );
    assert!(out.last_line().starts_with("next:"), "{}", out.stdout);
}

#[test]
fn a_long_branch_name_is_cut_to_forty_characters() {
    let s = Setup::new(
        PREVIEW,
        CHECKS,
        "feature/a-very-long-branch-name-that-goes-on-and-on",
    );
    let out = s.fz(&["run", "--stage", "working"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert_eq!(
        s.deploys(),
        ["deploy feature-a-very-long-branch-name-that-goe"]
    );
}

#[test]
fn on_a_detached_head_the_preview_is_named_for_the_commit() {
    let s = Setup::new(PREVIEW, CHECKS, "feat/x");
    s.repo.git(&["switch", "-q", "--detach"]);
    let short = s.repo.git(&["rev-parse", "--short", "HEAD"]);
    let out = s.fz(&["run", "--ci"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert_eq!(s.deploys(), [format!("deploy commit-{}", short.trim())]);
}

#[test]
fn the_same_tree_deploys_once() {
    let s = Setup::new(
        PREVIEW,
        &format!(
            "{CHECKS}\n[check.other]\nrun = 'test -n \"$FEARLEZZ_PREVIEW_URL\"'\nstage = \"working\"\ninputs = [\"src/**\"]\nneeds = [\"preview\"]\n"
        ),
        "feat/x",
    );
    let out = s.fz(&["run", "--stage", "working"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert_eq!(s.deploys().len(), 1, "two checks, one deploy");
    // The final stage runs every check again, fresh, on the same tree.
    let out = s.fz(&["run", "--stage", "merge"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert_eq!(
        common::runs(&s.repo.path(".runs/site")),
        2,
        "{}",
        out.stdout
    );
    assert_eq!(s.deploys().len(), 1, "{}", out.stdout);
    assert_eq!(s.url(), "https://feat-x.example.dev/");
}

#[test]
fn a_new_tree_deploys_again() {
    let s = Setup::new(PREVIEW, CHECKS, "feat/x");
    let out = s.fz(&["run", "--stage", "working"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    s.repo.write("src/lib.txt", "c");
    s.repo.commit("again");
    let out = s.fz(&["run", "--stage", "working"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert_eq!(s.deploys().len(), 2, "{}", out.stdout);
}

#[test]
fn a_check_that_does_not_need_the_preview_deploys_nothing() {
    let s = Setup::new(PREVIEW, CHECKS, "feat/x");
    let out = s.fz(&["run", "unit"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert_eq!(
        common::runs(&s.repo.path(".runs/unit")),
        1,
        "{}",
        out.stdout
    );
    assert!(s.deploys().is_empty(), "{}", out.stdout);
}

#[test]
fn a_preview_that_prints_no_url_fails_the_check_without_running_it() {
    let s = Setup::new(
        "preview = 'echo \"deploying to nowhere\"'",
        CHECKS,
        "feat/x",
    );
    let out = s.fz(&["run", "--stage", "working"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("fail site: the preview printed no URL"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("deploying to nowhere"),
        "{}",
        out.stdout
    );
    assert!(
        out.lines()
            .iter()
            .any(|l| l.trim_start().starts_with("fix:") && l.contains("[env] preview")),
        "{}",
        out.stdout
    );
    assert_eq!(common::runs(&s.repo.path(".runs/site")), 0, "never ran");
    assert_eq!(
        common::runs(&s.repo.path(".runs/unit")),
        1,
        "{}",
        out.stdout
    );
}

#[test]
fn a_failing_preview_fails_the_check_without_running_it() {
    let s = Setup::new(
        "preview = 'echo \"quota exceeded\"; echo https://half.example.dev; exit 3'",
        CHECKS,
        "feat/x",
    );
    let out = s.fz(&["run", "--stage", "working"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("fail site: the preview failed (exit 3)"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("quota exceeded"), "{}", out.stdout);
    assert_eq!(common::runs(&s.repo.path(".runs/site")), 0, "never ran");
}

#[test]
fn the_env_secrets_reach_the_preview_and_are_masked_in_its_log() {
    let s = Setup::new(
        &format!(
            "preview = 'echo \"token $CLOUDFLARE_API_TOKEN\"; test \"$CLOUDFLARE_API_TOKEN\" = {TOKEN} && echo https://x.example.dev/'\nsecrets = [\"CLOUDFLARE_API_TOKEN\"]"
        ),
        CHECKS,
        "feat/x",
    );
    let env = [
        ("SECRETSPEC_PROVIDER", "env"),
        ("CLOUDFLARE_API_TOKEN", TOKEN),
    ];
    let out = s.fz_env(&["run", "--stage", "working"], &env);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert_eq!(s.url(), "https://x.example.dev/");
    let mut kept = vec![s.repo.path(".git/fearlezz"), s.repo.cache.clone()];
    let mut logs = 0;
    while let Some(path) = kept.pop() {
        if path.is_dir() {
            kept.extend(std::fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
            continue;
        }
        let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).to_string();
        assert!(!text.contains(TOKEN), "{} holds the secret", path.display());
        logs += usize::from(text.contains("token ***"));
    }
    assert_eq!(logs, 1, "the preview's log, masked");
}

#[test]
fn a_secret_the_preview_cannot_get_fails_the_check_with_the_fix() {
    let s = Setup::new(
        "preview = 'echo https://x.example.dev/'\nsecrets = [\"CLOUDFLARE_API_TOKEN\"]",
        CHECKS,
        "feat/x",
    );
    let out = s.fz_env(
        &["run", "--stage", "working"],
        &[("SECRETSPEC_PROVIDER", "env")],
    );
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("fail site: SecretSpec could not give the preview CLOUDFLARE_API_TOKEN"),
        "{}",
        out.stdout
    );
    assert!(
        out.lines()
            .iter()
            .any(|l| l.trim_start().starts_with("fix:") && l.contains("secretspec.toml")),
        "{}",
        out.stdout
    );
    assert_eq!(common::runs(&s.repo.path(".runs/site")), 0, "never ran");
}

#[test]
fn needing_the_preview_without_a_preview_command_is_a_config_error() {
    let s = Setup::new("", CHECKS, "feat/x");
    let out = s.fz(&["run", "--stage", "working"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stderr
            .contains("check site needs the preview, but [env] has no preview command"),
        "{}\n{}",
        out.stdout,
        out.stderr
    );
}

#[test]
fn ci_names_the_preview_in_the_job_summary_and_the_proof() {
    let s = Setup::new(PREVIEW, CHECKS, "feat/x");
    let summary = s.repo.path(".runs/summary.md");
    let bundle = s.repo.path(".runs/proof");
    let out = s.fz_env(
        &["run", "--ci", "--proof", bundle.to_str().unwrap()],
        &[
            ("GITHUB_SERVER_URL", "https://github.com"),
            ("GITHUB_REPOSITORY", "owner/repo"),
            ("GITHUB_RUN_ID", "42"),
            ("GITHUB_STEP_SUMMARY", summary.to_str().unwrap()),
        ],
    );
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let text = std::fs::read_to_string(&summary).unwrap();
    assert!(
        text.contains("Preview: https://feat-x.example.dev/"),
        "{text}"
    );
    let proof: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(bundle.join("proof.json")).unwrap()).unwrap();
    assert_eq!(proof["preview"], "https://feat-x.example.dev/");
}

#[test]
fn changing_the_preview_or_its_secrets_weakens_the_proof() {
    let s = Setup::new(PREVIEW, CHECKS, "feat/x");
    let config = std::fs::read_to_string(s.repo.path("fearlezz.toml")).unwrap();
    s.repo.write(
        "fearlezz.toml",
        &config
            .replace("[env]\n", "[env]\nsecrets = [\"CLOUDFLARE_API_TOKEN\"]\n")
            .replace("Version preview URL:", "URL:"),
    );
    s.repo.commit("config");
    let out = s.fz(&["run", "--ci"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("weakens env.preview: it changes from"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout
            .contains("weakens env.secrets: it changes (+CLOUDFLARE_API_TOKEN)"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_look_deploys_the_base_too_named_for_its_tree() {
    let s = Setup::new(
        PREVIEW,
        r#"
[check.screens]
run = '''ffmpeg -v error -nostdin -f lavfi -i "color=c=$(cat src/lib.txt | sed 's/a/red/;s/b/blue/'):s=64x48" -frames:v 1 "$FEARLEZZ_OUT/home.png" && echo '{"kind":"still","file":"home.png"}' >> "$FEARLEZZ_OUT/proof.jsonl"'''
stage = "working"
inputs = ["src/**"]
needs = ["preview"]
"#,
        "feat/x",
    );
    // Named by the base's tree, so a red before green checkout of the same commit, whose tree
    // holds the change's tests, deploys to a name of its own.
    let base = s.repo.git(&["rev-parse", "main^{tree}"]);
    let out = s.fz(&["run", "--stage", "working"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let mut deploys = s.deploys();
    deploys.sort();
    assert_eq!(
        deploys,
        [
            format!("deploy base-{}", &base.trim()[..7]),
            "deploy feat-x".to_string()
        ],
        "{}",
        out.stdout
    );
}

#[test]
fn a_pull_request_in_ci_names_its_preview_for_its_number() {
    let s = Setup::new(PREVIEW, CHECKS, "feat/site");
    // GitHub checks out the merge commit with no branch.
    s.repo.git(&["switch", "-q", "--detach"]);
    let event = s.repo.path(".runs/event.json");
    std::fs::write(
        &event,
        r#"{"pull_request": {"number": 7, "title": "feat: site", "head": {"ref": "feat/site", "sha": "abc"}}}"#,
    )
    .unwrap();
    let out = s.fz_env(
        &["run", "--ci"],
        &[("GITHUB_EVENT_PATH", event.to_str().unwrap())],
    );
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert_eq!(s.deploys(), ["deploy pr-7"]);
}

#[test]
fn the_preview_sees_only_the_base_variables_its_env_and_its_secrets() {
    let s = Setup::new(
        "preview = 'echo \"token [$OP_SERVICE_ACCOUNT_TOKEN]\"; echo https://x.example.dev/'",
        CHECKS,
        "feat/x",
    );
    let out = s.fz_env(
        &["run", "--stage", "working"],
        &[("OP_SERVICE_ACCOUNT_TOKEN", "ops-test-0123456789")],
    );
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let mut kept = vec![s.repo.path(".git/fearlezz"), s.repo.cache.clone()];
    let mut seen = 0;
    while let Some(path) = kept.pop() {
        if path.is_dir() {
            kept.extend(std::fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
            continue;
        }
        let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).to_string();
        assert!(
            !text.contains("ops-test"),
            "{} holds the token",
            path.display()
        );
        seen += usize::from(text.contains("token []"));
    }
    assert_eq!(seen, 1, "the preview ran without the provider's token");
}
