//! Generated code is up to date: a check's `generates` names the files its command writes
//! into the repository, and they must come out the same as the tree had them.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;

/// Writes `gen/<name>.txt` for every line of `schema.txt`, and removes the files of names gone.
const GEN: &str = r#"set -e
mkdir -p gen
for f in gen/*.txt; do [ -e "$f" ] && rm "$f"; done
while read -r name; do
  echo "generated from $name" > "gen/$name.txt"
done < schema.txt
"#;

const CONFIG: &str = r#"
[check.gen]
run = "sh gen.sh"
stage = "working"
generates = ["gen/**"]
"#;

/// A repository whose generated files are up to date, on a branch.
fn repo() -> Repo {
    let repo = Repo::new(
        CONFIG,
        &[
            ("gen.sh", GEN),
            ("schema.txt", "user\nteam\n"),
            ("gen/user.txt", "generated from user\n"),
            ("gen/team.txt", "generated from team\n"),
        ],
    );
    repo.branch("feat/x");
    repo
}

#[test]
fn generated_files_that_match_the_tree_pass() {
    let repo = repo();
    let out = repo.fz(&["run", "gen"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("1 passed, 0 failed"), "{}", out.stdout);
}

#[test]
fn a_generated_file_the_tree_has_otherwise_fails_with_its_diff() {
    let repo = repo();
    repo.write("gen/user.txt", "edited by hand\n");
    let out = repo.fz(&["run", "gen"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("fail gen: generated files are out of date: gen/user.txt"),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("-edited by hand"), "{}", out.stdout);
    assert!(
        out.stdout.contains("+generated from user"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout
            .contains("fix: gen regenerated them; commit them as they are now"),
        "{}",
        out.stdout
    );
    // The regenerated file stays, so committing it is the whole fix.
    let now = std::fs::read_to_string(repo.path("gen/user.txt")).unwrap();
    assert_eq!(now, "generated from user\n");
    let again = repo.fz(&["run", "gen"]);
    assert!(again.ok, "{}", again.stdout);
}

#[test]
fn a_generated_file_the_tree_lacks_fails_as_new() {
    let repo = repo();
    repo.write("schema.txt", "user\nteam\nrole\n");
    let out = repo.fz(&["run", "gen"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("fail gen: generated files are out of date: gen/role.txt (new)"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_generated_file_the_command_no_longer_writes_fails_as_gone() {
    let repo = repo();
    repo.write("schema.txt", "user\n");
    let out = repo.fz(&["run", "gen"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("fail gen: generated files are out of date: gen/team.txt (gone)"),
        "{}",
        out.stdout
    );
}

#[test]
fn editing_a_generated_file_runs_the_check_again_even_when_its_inputs_leave_it_out() {
    let repo = Repo::new(
        &CONFIG.replace(
            "generates",
            "inputs = [\"schema.txt\", \"gen.sh\"]\ngenerates",
        ),
        &[
            ("gen.sh", GEN),
            ("schema.txt", "user\n"),
            ("gen/user.txt", "generated from user\n"),
        ],
    );
    repo.branch("feat/x");
    assert!(repo.fz(&["run", "gen"]).ok);
    repo.write("gen/user.txt", "edited by hand\n");
    let out = repo.fz(&["run", "gen"]);
    assert!(!out.ok, "{}", out.stdout);
}

#[test]
fn ci_names_out_of_date_generated_files_as_the_blocker() {
    let repo = repo();
    repo.write("gen/user.txt", "edited by hand\n");
    repo.commit("edit generated code by hand");
    let out = repo.fz(&["run", "--ci"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("generated files are out of date: gen/user.txt"),
        "{}",
        out.stdout
    );
}

#[test]
fn narrowing_generates_weakens_the_proof() {
    let repo = repo();
    repo.write(
        "fearlezz.toml",
        &format!(
            "{}\n[settings.skill]\nagents = []\n\n[settings.hooks]\nagents = []\n",
            CONFIG.replace("gen/**", "gen/user.txt")
        ),
    );
    repo.commit("check fewer generated files");
    let out = repo.fz(&["run", "--ci"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("it checks fewer generated files (gen/**, now gen/user.txt)"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_check_after_a_generator_that_rewrote_files_is_keyed_on_the_new_files() {
    let repo = Repo::new(
        &format!("{CONFIG}\n[check.reader]\nrun = \"cat gen/user.txt\"\nstage = \"working\"\n"),
        &[
            ("gen.sh", GEN),
            ("schema.txt", "user\n"),
            ("gen/user.txt", "generated from user\n"),
        ],
    );
    repo.branch("feat/x");
    repo.write("gen/user.txt", "edited by hand\n");
    // gen fails and rewrites the file; reader then runs on what gen wrote.
    let out = repo.fz(&["run", "gen", "reader"]);
    assert!(!out.ok, "{}", out.stdout);
    // Back to the hand edit, reader's pass on the regenerated file is not reused.
    repo.write("gen/user.txt", "edited by hand\n");
    let again = repo.fz(&["run", "reader"]);
    assert!(again.stdout.contains("0 reused"), "{}", again.stdout);
}

#[test]
fn generates_that_names_no_file_fails() {
    let repo = Repo::new(
        &CONFIG.replace("gen/**", "generated/**"),
        &[
            ("gen.sh", GEN),
            ("schema.txt", "user\n"),
            ("gen/user.txt", "generated from user\n"),
        ],
    );
    repo.branch("feat/x");
    let out = repo.fz(&["run", "gen"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("fail gen: generates names no file git tracks (generated/**)"),
        "{}",
        out.stdout
    );
}
