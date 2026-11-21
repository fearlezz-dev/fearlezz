//! `fearlezz setup showcase`: the markup that embeds each picture of the work's last run from
//! its showcase address on the proof host's files origin, ready for a README, docs or a website.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;

/// A check that takes a light and a dark home screen, a lone about screen, a film and a text.
/// Its inputs are its own script, so a change elsewhere leaves its pictures as on the base and
/// nothing is compared.
const SHOTS: &str = r#"set -e
mkdir -p "$FEARLEZZ_OUT/home"
for file in home/light.png home/Dark.png about.png intro.webm menu.txt; do
  printf '%s' "$file" > "$FEARLEZZ_OUT/$file"
done
cat > "$FEARLEZZ_OUT/proof.jsonl" <<'EOF'
{"kind":"still","file":"home/light.png"}
{"kind":"still","file":"home/Dark.png"}
{"kind":"still","file":"about.png"}
{"kind":"film","file":"intro.webm"}
{"kind":"text","file":"menu.txt"}
EOF
"#;

const CONFIG: &str = r#"
[check.shots]
run = "sh shots.sh"
stage = "look"
inputs = ["shots.sh"]
"#;

const FILES: &str = "https://fearlezz.seamlezz.net/showcase/acme/app/shots";

/// A repository on GitHub as acme/app whose `shots` check ran on a branch.
fn ran() -> Repo {
    let repo = Repo::new(CONFIG, &[("shots.sh", SHOTS), ("src/app.txt", "one")]);
    repo.git(&["remote", "add", "origin", "git@github.com:acme/app.git"]);
    repo.branch("feat/two");
    repo.write("src/app.txt", "two");
    repo.commit("two");
    let out = repo.fz(&["run", "--stage", "look"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    repo
}

#[test]
fn a_light_and_a_dark_picture_become_one_picture_that_follows_the_theme() {
    let repo = ran();
    let out = repo.fz(&["setup", "showcase"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    let block = format!(
        "<picture>\n  <source media=\"(prefers-color-scheme: dark)\" srcset=\"{FILES}/home/Dark.png\">\n  <img alt=\"home\" src=\"{FILES}/home/light.png\" width=\"720\">\n</picture>\n"
    );
    assert!(out.stdout.contains(&block), "{}", out.stdout);
    assert!(out.last_line().starts_with("next: "), "{}", out.stdout);
}

#[test]
fn a_picture_without_a_theme_is_a_plain_image() {
    let repo = ran();
    let out = repo.fz(&["setup", "showcase"]);
    assert!(
        out.stdout.contains(&format!(
            "<img alt=\"about\" src=\"{FILES}/about.png\" width=\"720\">\n"
        )),
        "{}",
        out.stdout
    );
}

#[test]
fn a_film_shows_as_its_animated_webp_and_a_text_not_at_all() {
    let repo = ran();
    let out = repo.fz(&["setup", "showcase"]);
    assert!(
        out.stdout.contains(&format!(
            "<img alt=\"intro\" src=\"{FILES}/intro.webp\" width=\"720\">\n"
        )),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("intro.webm"), "{}", out.stdout);
    assert!(!out.stdout.contains("menu"), "{}", out.stdout);
}

#[test]
fn a_work_without_pictures_says_so() {
    let repo = Repo::new(
        "[check.unit]\nrun = \"true\"\nstage = \"exploring\"\n",
        &[("src/app.txt", "one")],
    );
    repo.git(&["remote", "add", "origin", "https://github.com/acme/app.git"]);
    repo.branch("feat/two");
    repo.write("src/app.txt", "two");
    repo.commit("two");
    assert!(repo.fz(&["run"]).ok);
    let out = repo.fz(&["setup", "showcase"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("no check took stills or films"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("<img"), "{}", out.stdout);
    assert!(out.last_line().starts_with("next: "), "{}", out.stdout);
}

#[test]
fn a_repository_not_on_github_has_no_showcase_address() {
    let repo = ran();
    repo.git(&["remote", "remove", "origin"]);
    let out = repo.fz(&["setup", "showcase"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("no GitHub repository"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("<img"), "{}", out.stdout);
    assert!(out.last_line().starts_with("next: "), "{}", out.stdout);
}

/// A repository that turned the proof host off sends no pictures, so it has no showcase.
#[test]
fn a_repository_with_the_proof_host_off_has_no_showcase() {
    let repo = ran();
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    repo.write("fearlezz.toml", &format!("{config}\n[host]\noff = true\n"));
    let out = repo.fz(&["setup", "showcase"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("the proof host is off"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("<img"), "{}", out.stdout);
    assert!(
        out.last_line().starts_with("next: ") && out.last_line().contains("[host]"),
        "{}",
        out.stdout
    );
}
