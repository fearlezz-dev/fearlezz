//! Text looks: a check's rendered text output (manifests, a plan, generated output it does not
//! commit) compared line by line with the same check run on the base commit, for a person to
//! review like a changed picture.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;
use serde_json::Value;

/// Renders one manifest per file in `src/`, as `kustomize build` or `flate` would.
const RENDER: &str = r#"set -e
mkdir -p "$FEARLEZZ_OUT/rendered"
for f in src/*; do
  name=$(basename "$f")
  printf 'kind: ConfigMap\nname: %s\ncolor: %s\n' "$name" "$(cat "$f")" > "$FEARLEZZ_OUT/rendered/$name.yaml"
done
"#;

fn config(texts: &str) -> String {
    format!(
        "\n[check.render]\nrun = \"sh render.sh\"\nstage = \"look\"\ninputs = [\"src/**\", \
         \"render.sh\"]\n{texts}\n"
    )
}

/// A repository whose `render` check writes a manifest per file in `src/`, on a branch that
/// changes `app` and adds `extra`.
fn repo() -> Repo {
    let repo = Repo::new(
        &config("texts = \"rendered/**/*.yaml\""),
        &[
            (".gitignore", ".runs/\n"),
            ("render.sh", RENDER),
            ("src/app", "red"),
            ("src/db", "white"),
        ],
    );
    std::fs::create_dir_all(repo.path(".runs")).unwrap();
    repo.branch("feat/blue");
    repo.write("src/app", "blue");
    repo.write("src/extra", "green");
    repo.fz(&["run", "--stage", "look", "--no-run"]);
    repo
}

#[test]
fn texts_are_compared_with_the_base_and_show_their_diff() {
    let repo = repo();
    let out = repo.fz(&["run"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("look render: 1 changed, 1 new, 1 same"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("  app: 1 line added, 1 removed"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("    -color: red"), "{}", out.stdout);
    assert!(out.stdout.contains("    +color: blue"), "{}", out.stdout);
    assert!(out.stdout.contains("  extra: new"), "{}", out.stdout);
    // `fearlezz` names them too, with the files to open.
    let status = repo.fz(&[]);
    assert!(status.stdout.contains("+color: blue"), "{}", status.stdout);
    let after = status
        .lines()
        .iter()
        .find_map(|l| l.trim().strip_prefix("after: ").map(String::from))
        .unwrap_or_else(|| panic!("{}", status.stdout));
    assert!(
        std::fs::read_to_string(after)
            .unwrap()
            .contains("color: blue"),
        "{}",
        status.stdout
    );
}

#[test]
fn texts_can_be_listed_in_the_proof_jsonl() {
    let repo = Repo::new(
        &config(""),
        &[
            (
                "render.sh",
                "echo \"plan: $(cat src/app)\" > \"$FEARLEZZ_OUT/plan.txt\"\necho \
                 '{\"kind\":\"text\",\"file\":\"plan.txt\",\"id\":\"plan\"}' > \
                 \"$FEARLEZZ_OUT/proof.jsonl\"\n",
            ),
            ("src/app", "red"),
        ],
    );
    repo.branch("feat/blue");
    repo.write("src/app", "blue");
    repo.fz(&["run", "--stage", "look", "--no-run"]);
    let out = repo.fz(&["run"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("look render: 1 changed"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("+plan: blue"), "{}", out.stdout);
}

#[test]
fn ci_shows_changed_texts_in_the_job_summary_and_the_bundle() {
    let repo = repo();
    repo.commit("blue");
    let summary = repo.path(".runs/summary.md");
    let bundle = repo.path(".runs/proof");
    let out = repo.fz_env(
        &["run", "--ci", "--proof", bundle.to_str().unwrap()],
        &[("GITHUB_STEP_SUMMARY", summary.to_str().unwrap())],
    );
    assert!(out.ok, "{}", out.stdout);
    let text = std::fs::read_to_string(&summary).unwrap();
    assert!(text.contains("#### Changed looks"), "{text}");
    assert!(
        text.contains("- **app** changed: 1 line added, 1 removed"),
        "{text}"
    );
    assert!(text.contains("```diff"), "{text}");
    assert!(text.contains("+color: blue"), "{text}");

    let proof: Value =
        serde_json::from_str(&std::fs::read_to_string(bundle.join("proof.json")).unwrap()).unwrap();
    let items = proof["looks"][0]["items"].as_array().unwrap();
    let app = items.iter().find(|i| i["name"] == "app").unwrap();
    assert_eq!(app["kind"], "text", "{app:#}");
    // Kept as plain text, the one kind of text the proof host serves.
    let kept = std::path::Path::new(app["after"].as_str().unwrap());
    assert_eq!(kept.extension().unwrap(), "txt", "{app:#}");
    assert_eq!(app["change"], "changed", "{app:#}");
    let after = bundle.join(app["after"].as_str().unwrap());
    assert!(
        std::fs::read_to_string(after)
            .unwrap()
            .contains("color: blue"),
        "{app:#}"
    );
}

#[test]
fn dropping_the_texts_glob_weakens_the_proof() {
    let repo = repo();
    repo.write(
        "fearlezz.toml",
        &format!(
            "{}\n[settings.skill]\nagents = []\n\n[settings.hooks]\nagents = []\n",
            config("")
        ),
    );
    repo.commit("no more texts");
    let out = repo.fz(&["run", "--ci"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("it no longer takes texts by rendered/**/*.yaml"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_long_changed_line_is_cut_where_it_is_shown() {
    let repo = repo();
    repo.write("src/app", &"b".repeat(1000));
    let out = repo.fz(&["run"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let line = out
        .lines()
        .into_iter()
        .find(|l| l.trim_start().starts_with("+color: bbb"))
        .unwrap_or_else(|| panic!("{}", out.stdout));
    assert!(line.len() < 400, "{line}");
    assert!(line.ends_with("more characters)"), "{line}");
}
