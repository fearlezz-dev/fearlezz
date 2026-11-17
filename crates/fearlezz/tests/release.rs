//! Releases: `[release.<place>]` names a place a release goes to. Its dry run is a check
//! before merge; `fearlezz run --release` publishes to each place that does not have the
//! version yet, and waits until the place shows it.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;
use common::registry::FakeRegistry;
use sha2::Digest as _;

const CARGO: &str = "[package]\nname = \"widget\"\nversion = \"1.2.0\"\nedition = \"2024\"\n";

/// The line crates.io's sparse index holds for widget 1.2.0, at its path.
const INDEX_PATH: &str = "wi/dg/widget";
const INDEX_LINE: &str = "{\"name\":\"widget\",\"vers\":\"1.2.0\",\"deps\":[]}";

/// A crates place on the fake registry whose publish runs `publish`.
fn crates_place(reg: &FakeRegistry, publish: &str) -> String {
    format!(
        "[release.crates]\npublish = '''{publish}'''\nlanded = {{ read = \"crates\", url = \"{}\" }}\nwait = 2\n",
        reg.url
    )
}

/// A publish command that makes widget 1.2.0 appear on the fake registry and counts its runs.
fn publish_to(reg: &FakeRegistry) -> String {
    let file = reg.dir.join(INDEX_PATH);
    format!(
        "echo run >> .publishes; mkdir -p '{}' && echo '{INDEX_LINE}' >> '{}'",
        file.parent().unwrap().display(),
        file.display()
    )
}

fn publishes(repo: &Repo) -> usize {
    std::fs::read_to_string(repo.path(".publishes")).map_or(0, |s| s.lines().count())
}

/// The checks Fearlezz knows, as its error for an unknown check lists them.
fn checks(repo: &Repo) -> String {
    let out = repo.fz(&["run", "no-such-check"]);
    assert!(!out.ok);
    out.stderr
        .split("checks are ")
        .nth(1)
        .unwrap_or_else(|| panic!("{}", out.stderr))
        .trim()
        .to_string()
}

#[test]
fn a_place_with_a_dry_run_brings_a_check_at_the_last_stage() {
    let repo = Repo::new(
        "[release.crates]\ndry = \"echo dry\"\npublish = \"false\"\nlanded = \"false\"\nversion = \"Cargo.toml\"\n",
        &[("Cargo.toml", CARGO)],
    );
    assert_eq!(checks(&repo), "release/crates");
    repo.branch("feat/x");
    repo.write("Cargo.toml", &CARGO.replace("1.2.0", "1.3.0"));
    let plan = repo.fz(&["--plan"]);
    assert!(plan.ok, "{}", plan.stderr);
    let merge = plan.stdout.split("merge").nth(1).unwrap_or_default();
    assert!(merge.contains("release/crates"), "{}", plan.stdout);
}

#[test]
fn dropping_a_dry_run_weakens_the_proof() {
    let repo = Repo::new(
        "[release.crates]\ndry = \"echo dry\"\npublish = \"false\"\nlanded = \"false\"\nversion = \"Cargo.toml\"\n",
        &[("Cargo.toml", CARGO)],
    );
    repo.branch("feat/x");
    repo.write(
        "fearlezz.toml",
        "[release.crates]\npublish = \"false\"\nlanded = \"false\"\nversion = \"Cargo.toml\"\n\n[settings.skill]\nagents = []\n\n[settings.hooks]\nagents = []\n",
    );
    repo.commit("drop the dry run");
    let out = repo.fz(&["run", "--ci"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("weakens") && out.stdout.contains("release/crates"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_release_publishes_what_has_not_landed_and_waits_until_it_has() {
    let reg = FakeRegistry::start();
    let repo = Repo::new(
        &crates_place(&reg, &publish_to(&reg)),
        &[("Cargo.toml", CARGO), (".gitignore", ".publishes\n")],
    );
    let out = repo.fz(&["run", "--release"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("crates: publishing widget 1.2.0"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("crates: widget 1.2.0 landed"),
        "{}",
        out.stdout
    );
    assert_eq!(out.last_line(), "next: every place has its release");
    assert_eq!(publishes(&repo), 1);

    // Run again, as after a half done release: a place that has it is skipped.
    let again = repo.fz(&["run", "--release"]);
    assert!(again.ok, "{}\n{}", again.stdout, again.stderr);
    assert!(
        again
            .stdout
            .contains("crates: widget 1.2.0 is already there"),
        "{}",
        again.stdout
    );
    assert_eq!(publishes(&repo), 1);
}

#[test]
fn a_place_that_never_shows_the_version_fails_the_release() {
    let reg = FakeRegistry::start();
    let repo = Repo::new(&crates_place(&reg, "true"), &[("Cargo.toml", CARGO)]);
    let out = repo.fz(&["run", "--release"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("crates: widget 1.2.0 did not land within 2 s"),
        "{}",
        out.stdout
    );
    assert!(
        out.last_line().contains("fearlezz run --release"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_failed_publish_leaves_the_other_places_to_go() {
    let reg = FakeRegistry::start();
    let config = format!(
        "{}\n[release.other]\npublish = \"exit 3\"\nlanded = \"false\"\nversion = \"Cargo.toml\"\n",
        crates_place(&reg, &publish_to(&reg))
    );
    let repo = Repo::new(
        &config,
        &[("Cargo.toml", CARGO), (".gitignore", ".publishes\n")],
    );
    let out = repo.fz(&["run", "--release"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("crates: widget 1.2.0 landed"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("other: publish ended with exit code 3"),
        "{}",
        out.stdout
    );
}

#[test]
fn publish_sees_only_its_env_and_the_version() {
    let config = "[release.shop]\npublish = 'echo \"token=$SHOP_TOKEN other=$OTHER version=$FEARLEZZ_VERSION\" > .published'\nlanded = \"test -f .published\"\nenv = [\"SHOP_TOKEN\"]\nversion = \"package.json\"\n";
    let repo = Repo::new(
        config,
        &[
            (
                "package.json",
                "{\"name\": \"widget\", \"version\": \"0.9.1\"}\n",
            ),
            (".gitignore", ".published\n"),
        ],
    );
    let out = repo.fz_env(
        &["run", "--release"],
        &[("SHOP_TOKEN", "s3cret"), ("OTHER", "leak")],
    );
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert_eq!(
        std::fs::read_to_string(repo.path(".published")).unwrap(),
        "token=s3cret other= version=0.9.1\n"
    );
    assert!(!out.stdout.contains("s3cret"), "{}", out.stdout);
}

#[test]
fn the_npm_reader_asks_the_registry_for_a_scoped_package() {
    let reg = FakeRegistry::start();
    // The scope's slash goes encoded, as the registry takes it.
    reg.put(
        "@acme%2fwidget",
        "{\"name\": \"@acme/widget\", \"versions\": {\"1.0.0\": {}, \"2.0.0\": {}}}",
    );
    let config = format!(
        "[release.npm]\npublish = \"false\"\nlanded = {{ read = \"npm\", url = \"{}\" }}\n",
        reg.url
    );
    let repo = Repo::new(
        &config,
        &[(
            "package.json",
            "{\"name\": \"@acme/widget\", \"version\": \"2.0.0\"}\n",
        )],
    );
    let out = repo.fz(&["run", "--release"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("npm: @acme/widget 2.0.0 is already there"),
        "{}",
        out.stdout
    );
}

#[test]
fn the_pub_reader_asks_pub_dev_for_the_version() {
    let reg = FakeRegistry::start();
    reg.put("api/packages/widget/versions/0.3.0+1", "{}");
    let config = format!(
        "[release.pub]\npublish = \"false\"\nlanded = {{ read = \"pub\", url = \"{}\" }}\n",
        reg.url
    );
    let repo = Repo::new(
        &config,
        &[(
            "pubspec.yaml",
            "name: widget\ndescription: A widget.\nversion: 0.3.0+1\n",
        )],
    );
    let out = repo.fz(&["run", "--release"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("pub: widget 0.3.0+1 is already there"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_package_in_a_folder_reads_its_own_manifest() {
    let reg = FakeRegistry::start();
    reg.put(INDEX_PATH, INDEX_LINE);
    let config = format!(
        "[release.crates]\ndir = \"crates/widget\"\npublish = \"false\"\nlanded = {{ read = \"crates\", url = \"{}\" }}\n",
        reg.url
    );
    let repo = Repo::new(&config, &[("crates/widget/Cargo.toml", CARGO)]);
    let out = repo.fz(&["run", "--release"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("crates: widget 1.2.0 is already there"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_workspace_version_is_read_from_the_workspace() {
    let reg = FakeRegistry::start();
    reg.put(INDEX_PATH, INDEX_LINE);
    let config = format!(
        "[release.crates]\ndir = \"crates/widget\"\npublish = \"false\"\nlanded = {{ read = \"crates\", url = \"{}\" }}\n",
        reg.url
    );
    let repo = Repo::new(
        &config,
        &[
            (
                "Cargo.toml",
                "[workspace]\nmembers = [\"crates/*\"]\n\n[workspace.package]\nversion = \"1.2.0\"\n",
            ),
            (
                "crates/widget/Cargo.toml",
                "[package]\nname = \"widget\"\nversion.workspace = true\n",
            ),
        ],
    );
    let out = repo.fz(&["run", "--release"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("crates: widget 1.2.0 is already there"),
        "{}",
        out.stdout
    );
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(sha2::Sha256::digest(bytes))
}

/// A GitHub release of widget's tag v1.2.0 on the fake API, with assets of these names and
/// digests.
fn github_release(assets: &[(&str, &str)]) -> String {
    let assets: Vec<String> = assets
        .iter()
        .map(|(name, digest)| format!("{{\"name\": \"{name}\", \"digest\": \"sha256:{digest}\"}}"))
        .collect();
    format!(
        "{{\"tag_name\": \"v1.2.0\", \"assets\": [{}]}}",
        assets.join(", ")
    )
}

const RELEASE_PATH: &str = "repos/acme/widget/releases/tags/v1.2.0";

fn github_place(reg: &FakeRegistry, publish: &str) -> String {
    format!(
        "[release.github]\nversion = \"Cargo.toml\"\nfiles = [\"dist/*.tar.gz\"]\npublish = '''{publish}'''\nlanded = {{ read = \"github\", url = \"{}\" }}\nwait = 2\n",
        reg.url
    )
}

#[test]
fn a_github_release_has_landed_when_every_file_is_there_with_its_digest() {
    let reg = FakeRegistry::start();
    let tar = "the binary";
    reg.put(
        RELEASE_PATH,
        &github_release(&[("widget.tar.gz", &sha256(tar.as_bytes()))]),
    );
    let repo = Repo::new(
        &github_place(&reg, "false"),
        &[("Cargo.toml", CARGO), ("dist/widget.tar.gz", tar)],
    );
    let out = repo.fz_env(
        &["run", "--release"],
        &[("GITHUB_REPOSITORY", "acme/widget")],
    );
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("github: widget v1.2.0 is already there"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_github_file_with_other_bytes_is_published_again_with_its_path_whole() {
    let reg = FakeRegistry::start();
    let tar = "the binary";
    reg.put(
        RELEASE_PATH,
        &github_release(&[("my widget.tar.gz", &sha256(b"an old build"))]),
    );
    let good = reg.dir.join("good.json");
    std::fs::write(
        &good,
        github_release(&[("my widget.tar.gz", &sha256(tar.as_bytes()))]),
    )
    .unwrap();
    let publish = format!(
        "printf '%s\\n' \"$FEARLEZZ_TAG\" \"$@\" > .uploaded && cp '{}' '{}'",
        good.display(),
        reg.dir.join(RELEASE_PATH).display()
    );
    let repo = Repo::new(
        &github_place(&reg, &publish),
        &[
            ("Cargo.toml", CARGO),
            // A space in a file name reaches the command whole.
            ("dist/my widget.tar.gz", tar),
            (".gitignore", ".uploaded\n"),
        ],
    );
    let out = repo.fz_env(
        &["run", "--release"],
        &[("GITHUB_REPOSITORY", "acme/widget")],
    );
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("github: widget v1.2.0 landed"),
        "{}",
        out.stdout
    );
    assert_eq!(
        std::fs::read_to_string(repo.path(".uploaded")).unwrap(),
        format!("v1.2.0\n{}\n", repo.path("dist/my widget.tar.gz").display())
    );
}

#[test]
fn a_github_place_without_its_files_fails_before_publishing() {
    let reg = FakeRegistry::start();
    let repo = Repo::new(
        &github_place(&reg, "echo ran > .uploaded"),
        &[("Cargo.toml", CARGO), (".gitignore", ".uploaded\n")],
    );
    let out = repo.fz_env(
        &["run", "--release"],
        &[("GITHUB_REPOSITORY", "acme/widget")],
    );
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("dist/*.tar.gz matches no file"),
        "{}",
        out.stdout
    );
    assert!(!repo.path(".uploaded").exists());
}

#[test]
fn the_tag_follows_the_place_pattern() {
    let reg = FakeRegistry::start();
    let tar = "the binary";
    reg.put(
        "repos/acme/widget/releases/tags/widget-v1.2.0",
        &github_release(&[("widget.tar.gz", &sha256(tar.as_bytes()))]),
    );
    let config = format!(
        "{}tag = \"widget-v{{version}}\"\n",
        github_place(&reg, "false")
    );
    let repo = Repo::new(
        &config,
        &[("Cargo.toml", CARGO), ("dist/widget.tar.gz", tar)],
    );
    let out = repo.fz_env(
        &["run", "--release"],
        &[("GITHUB_REPOSITORY", "acme/widget")],
    );
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("github: widget widget-v1.2.0 is already there"),
        "{}",
        out.stdout
    );
}

#[test]
fn without_places_a_release_says_how_to_add_one() {
    let repo = Repo::new("", &[]);
    let out = repo.fz(&["run", "--release"]);
    assert!(out.ok, "{}", out.stderr);
    assert!(out.last_line().starts_with("next: "), "{}", out.stdout);
    assert!(out.stdout.contains("[release."), "{}", out.stdout);
}

#[test]
fn the_registry_pieces_bring_their_dry_runs() {
    for (piece, files) in [
        ("crates", vec![("Cargo.toml", CARGO), ("src/lib.rs", "")]),
        (
            "npm",
            vec![(
                "package.json",
                "{\"name\": \"w\", \"version\": \"1.0.0\"}\n",
            )],
        ),
        ("pub", vec![("pubspec.yaml", "name: w\nversion: 1.0.0\n")]),
    ] {
        let repo = Repo::new(&format!("use = [\"{piece}\"]\n"), &files);
        assert_eq!(checks(&repo), format!("release/{piece}"), "{piece}");
    }
}

#[test]
fn the_github_piece_asks_the_project_for_its_version_and_files() {
    let repo = Repo::new("use = [\"github\"]\n", &[("Cargo.toml", CARGO)]);
    // A piece alone is a valid config: what only the project knows is asked for at release.
    let plan = repo.fz(&["--plan"]);
    assert!(plan.ok, "{}", plan.stderr);
    let out = repo.fz(&["run", "--release"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("github: name the file its version comes from in [release.github]"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_place_a_piece_brings_can_be_turned_off() {
    let repo = Repo::new(
        "use = [\"crates\"]\n\n[release.crates]\noff = true\n",
        &[("Cargo.toml", CARGO)],
    );
    let out = repo.fz(&["run", "no-such-check"]);
    assert!(!out.ok);
    assert!(!out.stderr.contains("release/crates"), "{}", out.stderr);
    let release = repo.fz(&["run", "--release"]);
    assert!(release.ok, "{}", release.stderr);
    assert!(
        release.stdout.contains("no place to release to"),
        "{}",
        release.stdout
    );
}

#[test]
fn a_dry_run_is_tuned_under_its_check_name() {
    let repo = Repo::new(
        "use = [\"crates\"]\n\n[check.\"release/crates\"]\nstage = \"working\"\n",
        &[("Cargo.toml", CARGO)],
    );
    repo.branch("feat/x");
    repo.write("Cargo.toml", &CARGO.replace("1.2.0", "1.3.0"));
    let plan = repo.fz(&["--plan"]);
    assert!(plan.ok, "{}", plan.stderr);
    let working = plan.stdout.split("look").next().unwrap_or_default();
    assert!(working.contains("release/crates"), "{}", plan.stdout);
}

#[test]
fn a_registry_that_stumbles_while_waiting_is_asked_again() {
    let reg = FakeRegistry::start();
    let file = reg.dir.join(INDEX_PATH);
    let status = reg.dir.join(format!("{INDEX_PATH}.status"));
    // The publish works, but the registry answers 502 for a moment before it lists it.
    let publish = format!(
        "mkdir -p '{dir}' && echo 502 > '{status}' && (sleep 1; echo '{INDEX_LINE}' > '{file}'; rm '{status}') &",
        dir = file.parent().unwrap().display(),
        status = status.display(),
        file = file.display(),
    );
    let config = crates_place(&reg, &publish).replace("wait = 2", "wait = 10");
    let repo = Repo::new(&config, &[("Cargo.toml", CARGO)]);
    let out = repo.fz(&["run", "--release"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("crates: widget 1.2.0 landed"),
        "{}",
        out.stdout
    );
}

#[test]
fn two_files_of_one_name_are_refused() {
    let reg = FakeRegistry::start();
    let config =
        github_place(&reg, "echo ran > .uploaded").replace("dist/*.tar.gz", "dist/**/*.tar.gz");
    let repo = Repo::new(
        &config,
        &[
            ("Cargo.toml", CARGO),
            ("dist/linux/widget.tar.gz", "a"),
            ("dist/macos/widget.tar.gz", "b"),
            (".gitignore", ".uploaded\n"),
        ],
    );
    let out = repo.fz_env(
        &["run", "--release"],
        &[("GITHUB_REPOSITORY", "acme/widget")],
    );
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("two files are called widget.tar.gz"),
        "{}",
        out.stdout
    );
    assert!(!repo.path(".uploaded").exists());
}

#[test]
fn a_root_package_reads_its_workspace_version_from_its_own_file() {
    let reg = FakeRegistry::start();
    reg.put(INDEX_PATH, INDEX_LINE);
    let config = format!(
        "[release.crates]\npublish = \"false\"\nlanded = {{ read = \"crates\", url = \"{}\" }}\n",
        reg.url
    );
    let repo = Repo::new(
        &config,
        &[(
            "Cargo.toml",
            "[package]\nname = \"widget\"\nversion.workspace = true\n\n[workspace]\n\n[workspace.package]\nversion = \"1.2.0\"\n",
        )],
    );
    let out = repo.fz(&["run", "--release"]);
    assert!(out.ok, "{}\n{}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("crates: widget 1.2.0 is already there"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_workspace_root_without_a_package_says_how_to_name_the_crate() {
    let reg = FakeRegistry::start();
    let config = format!(
        "[release.crates]\npublish = \"false\"\nlanded = {{ read = \"crates\", url = \"{}\" }}\n",
        reg.url
    );
    let repo = Repo::new(
        &config,
        &[("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n")],
    );
    let out = repo.fz(&["run", "--release"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stdout.contains("with dir or version"), "{}", out.stdout);
}
