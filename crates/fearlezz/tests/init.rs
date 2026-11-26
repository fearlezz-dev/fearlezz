//! Starting out: a repository without `fearlezz.toml` is led to `fearlezz setup init`, which
//! writes a starter from the languages the repository shows.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;

/// A repository with `files` and no `fearlezz.toml`, as a project is before it adopts Fearlezz.
fn project(files: &[(&str, &str)]) -> Repo {
    let repo = Repo::with_skill("", files);
    repo.remove("fearlezz.toml");
    repo.commit("without fearlezz");
    repo
}

#[test]
fn a_repository_without_config_is_led_to_init() {
    let repo = project(&[("README.md", "# app\n")]);
    for args in [&[][..], &["run"], &["setup", "sync"]] {
        let out = repo.fz(args);
        assert!(!out.ok, "{args:?}: {}", out.stdout);
        assert!(
            out.stderr.contains("no fearlezz.toml")
                && out.stderr.contains("fearlezz setup init")
                && out.stderr.contains("fearlezz setup guide"),
            "{args:?}: {}",
            out.stderr
        );
    }
}

#[test]
fn init_uses_the_pieces_of_the_languages_it_finds() {
    let repo = project(&[
        ("devenv.nix", "{ pkgs, ... }: { }\n"),
        (
            "Cargo.toml",
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
        ),
        (
            "web/index.html",
            "<p>not at the root, so not a project</p>\n",
        ),
        ("package.json", "{}\n"),
        ("pyproject.toml", "[project]\nname = \"app\"\n"),
        ("go.mod", "module app\n"),
        ("settings.gradle.kts", "rootProject.name = \"app\"\n"),
        (
            "pubspec.yaml",
            "name: app\ndependencies:\n  flutter:\n    sdk: flutter\n",
        ),
        (".github/workflows/ci.yml", "on: push\njobs: {}\n"),
    ]);
    let out = repo.fz(&["setup", "init"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    assert!(
        config.contains(
            r#"use = ["devenv", "rust", "typescript", "biome", "python", "go", "kotlin", "flutter", "actions", "safety"]"#
        ),
        "{config}"
    );
    // Prose needs no proof from the first commit on, so the base branch has it before any
    // docs only change.
    assert!(
        config.contains(r#"no_proof = ["*.md", "docs/**"]"#),
        "{config}"
    );
    // A Flutter app no check takes pictures of is an offer for the developer.
    assert!(
        out.stdout.contains("could prove: a Flutter app"),
        "{}",
        out.stdout
    );
    assert!(
        out.last_line().ends_with("then fearlezz setup sync"),
        "{}",
        out.stdout
    );

    // What it wrote is a config every command reads.
    let plan = repo.fz(&["--plan"]);
    assert!(plan.ok, "{}{}", plan.stdout, plan.stderr);

    // Each language keeps its own checks, named for their tools, so the pieces combine.
    repo.commit("starter");
    repo.branch("feat/x");
    for file in [
        "src/a.rs",
        "src/a.ts",
        "a.py",
        "a.go",
        "src/A.kt",
        "lib/a.dart",
    ] {
        repo.write(file, "\n");
    }
    let plan = repo.fz(&["--plan"]);
    for check in [
        "clippy",
        "tsc",
        "pytest",
        "go-test",
        "gradle-test",
        "flutter-test",
    ] {
        assert!(plan.stdout.contains(check), "{check}: {}", plan.stdout);
    }
}

#[test]
fn init_adds_the_piece_of_each_framework_package_json_depends_on() {
    // A Next app depends on react too, so it gets the rules of both.
    let repo = project(&[(
        "package.json",
        r#"{ "dependencies": { "next": "15.0.0", "react": "19.0.0" }, "devDependencies": { "vue": "3.5.0" } }"#,
    )]);
    let out = repo.fz(&["setup", "init"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    assert!(
        config.contains(r#"use = ["typescript", "biome", "react", "next", "vue", "safety"]"#),
        "{config}"
    );
    assert!(
        out.stdout.contains("react (package.json)"),
        "{}",
        out.stdout
    );

    // Without a framework, only the language.
    let plain = project(&[(
        "package.json",
        "{ \"dependencies\": { \"zod\": \"3.0.0\" } }\n",
    )]);
    assert!(plain.fz(&["setup", "init"]).ok);
    let config = std::fs::read_to_string(plain.path("fearlezz.toml")).unwrap();
    assert!(
        config.contains(r#"use = ["typescript", "biome", "safety"]"#),
        "{config}"
    );
}

#[test]
fn init_adds_the_piece_of_the_test_runner_the_project_uses() {
    // Vitest when package.json depends on it, even in a Bun project; Bun's own runner when
    // bun.lock shows Bun and nothing else runs the tests; neither without a sign of one.
    for (files, pieces) in [
        (
            &[(
                "package.json",
                r#"{ "devDependencies": { "vitest": "3.0.0" } }"#,
            )][..],
            r#"use = ["typescript", "biome", "vitest", "safety"]"#,
        ),
        (
            &[("package.json", "{}\n"), ("bun.lock", "{}\n")][..],
            r#"use = ["typescript", "biome", "bun", "safety"]"#,
        ),
        (
            &[
                (
                    "package.json",
                    r#"{ "devDependencies": { "vitest": "3.0.0" } }"#,
                ),
                ("bun.lock", "{}\n"),
            ][..],
            r#"use = ["typescript", "biome", "vitest", "safety"]"#,
        ),
        (
            &[("package.json", "{}\n")][..],
            r#"use = ["typescript", "biome", "safety"]"#,
        ),
    ] {
        let repo = project(files);
        let out = repo.fz(&["setup", "init"]);
        assert!(out.ok, "{}{}", out.stdout, out.stderr);
        let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
        assert!(config.contains(pieces), "{files:?}: {config}");
    }
}

#[test]
fn init_picks_the_linter_and_formatter_package_json_depends_on_and_biome_otherwise() {
    // Biome lints and formats a project that names no other tool, or names Biome itself.
    // The framework pieces hold Biome's and oxlint's rules, so they come with either.
    for (package, pieces) in [
        (
            r#"{ "devDependencies": { "@biomejs/biome": "2.6.6", "react": "19.0.0" } }"#,
            r#"use = ["typescript", "biome", "react", "safety"]"#,
        ),
        (
            r#"{ "devDependencies": { "eslint": "9.0.0", "prettier": "3.0.0", "react": "19.0.0" } }"#,
            r#"use = ["typescript", "eslint", "prettier", "safety"]"#,
        ),
        (
            r#"{ "devDependencies": { "oxlint": "1.0.0", "react": "19.0.0" } }"#,
            r#"use = ["typescript", "oxlint", "react", "safety"]"#,
        ),
        (
            r#"{ "devDependencies": { "prettier": "3.0.0" } }"#,
            r#"use = ["typescript", "prettier", "safety"]"#,
        ),
    ] {
        let repo = project(&[("package.json", package)]);
        let out = repo.fz(&["setup", "init"]);
        assert!(out.ok, "{}{}", out.stdout, out.stderr);
        let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
        assert!(config.contains(pieces), "{package}: {config}");
    }
}

#[test]
fn init_adds_node_test_when_the_test_script_runs_it() {
    let repo = project(&[(
        "package.json",
        r#"{ "scripts": { "test": "node --test --test-concurrency=1 'test/**/*.test.ts'" } }"#,
    )]);
    let out = repo.fz(&["setup", "init"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    assert!(
        config.contains(r#"use = ["typescript", "biome", "node-test", "safety"]"#),
        "{config}"
    );
}

#[test]
fn init_finds_lua_by_its_formatter_config() {
    for file in ["stylua.toml", ".stylua.toml"] {
        let repo = project(&[(file, "indent_type = \"Spaces\"\n"), ("main.lua", "\n")]);
        let out = repo.fz(&["setup", "init"]);
        assert!(out.ok, "{}{}", out.stdout, out.stderr);
        let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
        assert!(
            config.contains(r#"use = ["lua", "safety"]"#),
            "{file}: {config}"
        );
    }
}

#[test]
fn init_gives_a_svelte_project_svelte_check_in_place_of_tsc() {
    // As the developer's SvelteKit sites have it: ESLint, Prettier, Vitest.
    let repo = project(&[(
        "package.json",
        r#"{ "devDependencies": { "@sveltejs/kit": "2.0.0", "svelte": "5.0.0", "svelte-check": "4.0.0", "eslint": "9.0.0", "prettier": "3.0.0", "vitest": "3.0.0" } }"#,
    )]);
    let out = repo.fz(&["setup", "init"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    assert!(
        config.contains(r#"use = ["svelte", "eslint", "prettier", "vitest", "safety"]"#),
        "{config}"
    );

    // The piece runs `svelte-kit sync`; a Svelte app without SvelteKit keeps tsc.
    let plain = project(&[(
        "package.json",
        r#"{ "devDependencies": { "svelte": "5.0.0", "prettier": "3.0.0" } }"#,
    )]);
    assert!(plain.fz(&["setup", "init"]).ok);
    let config = std::fs::read_to_string(plain.path("fearlezz.toml")).unwrap();
    assert!(
        config.contains(r#"use = ["typescript", "prettier", "safety"]"#),
        "{config}"
    );
}

#[test]
fn init_tells_a_dart_package_from_a_flutter_app_by_its_sdk() {
    for (pubspec, pieces) in [
        (
            "name: client\nenvironment:\n  sdk: ^3.5.0\n",
            r#"use = ["dart", "safety"]"#,
        ),
        (
            "name: app\ndependencies:\n  flutter:\n    sdk: flutter\n",
            r#"use = ["flutter", "safety"]"#,
        ),
    ] {
        let repo = project(&[("pubspec.yaml", pubspec)]);
        let out = repo.fz(&["setup", "init"]);
        assert!(out.ok, "{}{}", out.stdout, out.stderr);
        let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
        assert!(config.contains(pieces), "{pubspec}: {config}");
    }
}

#[test]
fn init_adds_envtest_to_a_go_module_that_requires_controller_runtime() {
    let repo = project(&[(
        "go.mod",
        "module op\n\nrequire (\n\tk8s.io/api v0.31.2\n\tsigs.k8s.io/controller-runtime v0.19.0\n)\n",
    )]);
    let out = repo.fz(&["setup", "init"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    assert!(
        config.contains(r#"use = ["go", "envtest", "safety"]"#),
        "{config}"
    );
}

#[test]
fn init_finds_cargo_projects_below_the_root_and_the_rust_checks_run_in_each() {
    let manifest = "[package]\nname = \"app\"\nversion = \"0.1.0\"\n";
    let repo = project(&[
        ("shell/Cargo.toml", manifest),
        ("sandbox/Cargo.toml", manifest),
    ]);
    let out = repo.fz(&["setup", "init"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    assert!(config.contains(r#"use = ["rust", "safety"]"#), "{config}");
    assert!(
        out.stdout.contains("rust (sandbox/Cargo.toml)"),
        "{}",
        out.stdout
    );

    repo.commit("starter");
    repo.branch("feat/x");
    repo.write("shell/src/main.rs", "fn main() {}\n");
    let plan = repo.fz(&["--plan"]);
    for check in [
        "fmt/shell",
        "clippy/sandbox",
        "tests/shell",
        "tests/sandbox",
    ] {
        assert!(plan.stdout.contains(check), "{check}: {}", plan.stdout);
    }
}

/// A Flutter app in `app/`, with the Xcode projects Flutter generates for macOS and iOS.
const FLUTTER_IN_A_FOLDER: &[(&str, &str)] = &[
    (
        "app/pubspec.yaml",
        "name: app\ndependencies:\n  flutter:\n    sdk: flutter\n",
    ),
    ("app/lib/main.dart", "void main() {}\n"),
    ("app/test/home_test.dart", "void main() {}\n"),
    ("app/macos/Runner.xcodeproj/project.pbxproj", "// !$*UTF8*$!\n"),
    (
        "app/macos/Runner/AppDelegate.swift",
        "import Cocoa\nimport FlutterMacOS\n",
    ),
    ("app/ios/Runner.xcodeproj/project.pbxproj", "// !$*UTF8*$!\n"),
    (
        "app/ios/Runner/AppDelegate.swift",
        "import Flutter\nimport UIKit\n",
    ),
];

#[test]
fn init_finds_a_flutter_app_below_the_root_and_its_checks_run_there() {
    let repo = project(FLUTTER_IN_A_FOLDER);
    let out = repo.fz(&["setup", "init"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    assert!(config.contains(r#"use = ["flutter", "safety"]"#), "{config}");
    assert!(
        out.stdout.contains("flutter (app/pubspec.yaml)"),
        "{}",
        out.stdout
    );

    repo.commit("starter");
    repo.branch("feat/x");
    repo.write("app/lib/main.dart", "void main() => print('hi');\n");
    let plan = repo.fz(&["--plan"]);
    for check in ["dart-format/app", "dart-analyze/app", "flutter-test/app"] {
        assert!(plan.stdout.contains(check), "{check}: {}", plan.stdout);
    }
}

#[test]
fn init_offers_flutter_pictures_for_a_flutter_app_not_an_apple_app() {
    let repo = project(FLUTTER_IN_A_FOLDER);
    let out = repo.fz(&["setup", "init"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    // Flutter's generated macos/ and ios/ Xcode projects are the Flutter app's, not an
    // Apple app of their own.
    assert!(!out.stdout.contains("an Apple app"), "{}", out.stdout);
    assert!(
        out.stdout
            .contains("could prove: a Flutter app (app/pubspec.yaml)"),
        "{}",
        out.stdout
    );

    // A native Apple app beside it is still one.
    let mut files = FLUTTER_IN_A_FOLDER.to_vec();
    files.push(("mac/Mac.xcodeproj/project.pbxproj", "// !$*UTF8*$!\n"));
    let repo = project(&files);
    let out = repo.fz(&["setup", "init"]);
    assert!(
        out.stdout.contains("could prove: an Apple app"),
        "{}",
        out.stdout
    );
}

#[test]
fn init_sees_a_flutter_app_inside_another_dart_project() {
    let flutter = "name: app\ndependencies:\n  flutter:\n    sdk: flutter\n";
    let xcode = "// !$*UTF8*$!\n";
    // A pub workspace whose root is plain Dart, with the app as a member.
    let repo = project(&[
        (
            "pubspec.yaml",
            "name: root\nworkspace:\n  - apps/mobile\n",
        ),
        ("apps/mobile/pubspec.yaml", flutter),
        ("apps/mobile/ios/Runner.xcodeproj/project.pbxproj", xcode),
    ]);
    let out = repo.fz(&["setup", "init"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    assert!(config.contains(r#"use = ["flutter", "safety"]"#), "{config}");
    assert!(!out.stdout.contains("an Apple app"), "{}", out.stdout);
    assert!(
        out.stdout
            .contains("could prove: a Flutter app (apps/mobile/pubspec.yaml)"),
        "{}",
        out.stdout
    );

    // A plugin with its example app: the example's Xcode projects are Flutter's too.
    let repo = project(&[
        ("pubspec.yaml", flutter),
        ("example/pubspec.yaml", flutter),
        ("example/ios/Runner.xcodeproj/project.pbxproj", xcode),
        ("example/macos/Runner.xcodeproj/project.pbxproj", xcode),
    ]);
    let out = repo.fz(&["setup", "init"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    assert!(!out.stdout.contains("an Apple app"), "{}", out.stdout);
}

#[test]
fn init_without_a_known_language_writes_a_check_to_fill_in() {
    let repo = project(&[("Makefile", "test:\n\ttrue\n")]);
    let out = repo.fz(&["setup", "init"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    assert!(out.stdout.contains("no language"), "{}", out.stdout);
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    // Every project is scanned for leaked secrets, language or not.
    assert!(config.contains(r#"use = ["safety"]"#), "{config}");
    assert!(config.contains("# [check.tests]"), "{config}");
    assert!(repo.fz(&["--plan"]).ok);
}

#[test]
fn init_never_overwrites_a_config() {
    let repo = Repo::new("[check.a]\nrun = \"true\"\ninputs = [\"*\"]\n", &[]);
    let before = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    let out = repo.fz(&["setup", "init"]);
    assert!(!out.ok);
    assert!(
        out.stderr.contains("already has a fearlezz.toml"),
        "{}",
        out.stderr
    );
    assert_eq!(
        std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap(),
        before
    );
}

#[test]
fn init_brings_safety_without_workflows_or_a_language() {
    let repo = project(&[("README.md", "# app\n")]);
    let out = repo.fz(&["setup", "init"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    assert!(config.contains(r#"use = ["safety"]"#), "{config}");
    assert!(!config.contains("actions"), "{config}");
}

/// The `use` line `setup init` writes for a repository with `files`.
fn used(files: &[(&str, &str)]) -> String {
    let repo = project(files);
    let out = repo.fz(&["setup", "init"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    config
        .lines()
        .find(|l| l.starts_with("use = "))
        .unwrap_or_else(|| panic!("{config}"))
        .to_string()
}

#[test]
fn init_adds_the_infra_pieces_from_the_files_each_tool_reads() {
    let use_line = used(&[
        ("flake.nix", "{ outputs = _: { }; }\n"),
        ("deploy/kustomization.yaml", "resources: []\n"),
        (
            "charts/web/Chart.yaml",
            "apiVersion: v2\nname: web\nversion: 0.1.0\n",
        ),
        (
            "infra/.terraform.lock.hcl",
            "provider \"registry.terraform.io/hashicorp/random\" {\n  version = \"3.6.0\"\n}\n",
        ),
        (".yamllint.yaml", "extends: default\n"),
        (
            ".sops.yaml",
            "creation_rules:\n  - path_regex: secrets/.*\n",
        ),
    ]);
    assert_eq!(
        use_line,
        r#"use = ["nix", "kustomize", "helm", "terraform", "yamllint", "sops", "safety"]"#
    );
}

#[test]
fn init_picks_opentofu_when_the_lockfile_names_its_registry() {
    let use_line = used(&[(
        "infra/.terraform.lock.hcl",
        "provider \"registry.opentofu.org/hashicorp/random\" {\n  version = \"3.6.0\"\n}\n",
    )]);
    assert_eq!(use_line, r#"use = ["opentofu", "safety"]"#);
}

#[test]
fn a_flux_repository_gets_flux_instead_of_kustomize() {
    // flate renders every Kustomization of every cluster, so kustomize would build them twice.
    let use_line = used(&[
        (
            "clusters/home/flux-system/gotk-sync.yaml",
            "apiVersion: v1\nkind: List\n",
        ),
        ("clusters/home/kustomization.yaml", "resources: []\n"),
    ]);
    assert_eq!(use_line, r#"use = ["flux", "safety"]"#);
}

/// `project`, on GitHub.
fn on_github(files: &[(&str, &str)]) -> Repo {
    let repo = project(files);
    repo.git(&["remote", "add", "origin", "https://github.com/acme/app.git"]);
    repo
}

const WORKFLOW: &str = ".github/workflows/proof.yml";

#[test]
fn init_on_github_asks_whether_ci_sends_its_proof_to_the_host() {
    let repo = on_github(&[("README.md", "# app\n")]);
    let out = repo.fz(&["setup", "init"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    let next = out.last_line();
    assert!(
        next.starts_with(
            "next: ask the developer whether CI should send its proof to the proof host"
        ) && next.contains("fearlezz setup init --ci host")
            && next.contains("--ci plain")
            && next.ends_with("then fearlezz setup sync"),
        "{}",
        out.stdout
    );

    // A repository whose workflows already prove with Fearlezz is not asked.
    let proven = on_github(&[(
        ".github/workflows/ci.yml",
        "on: push\njobs:\n  proof:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: fearlezz-dev/fearlezz@v0\n",
    )]);
    let out = proven.fz(&["setup", "init"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    assert!(!out.stdout.contains("--ci"), "{}", out.stdout);
}

#[test]
fn init_ci_plain_writes_the_proof_workflow_beside_the_starter() {
    let repo = on_github(&[("README.md", "# app\n")]);
    let out = repo.fz(&["setup", "init", "--ci", "plain"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    assert!(repo.path("fearlezz.toml").exists());
    let workflow = std::fs::read_to_string(repo.path(WORKFLOW)).unwrap();
    for part in [
        "contents: read",
        "fetch-depth: 0",
        "types: [opened, synchronize, reopened, edited]",
        "branches: [main]",
        "uses: fearlezz-dev/fearlezz@v0",
    ] {
        assert!(workflow.contains(part), "{part}: {workflow}");
    }
    assert!(!workflow.contains("id-token"), "{workflow}");
    assert!(!workflow.contains("devenv"), "{workflow}");
    // A push to the base branch is never cancelled; a newer push to a pull request is.
    assert!(
        workflow.contains("cancel-in-progress: ${{ github.event_name == 'pull_request' }}"),
        "{workflow}"
    );
    assert!(!out.stdout.contains("GitHub App"), "{}", out.stdout);
    assert!(
        out.stdout.contains(&format!("wrote {WORKFLOW}")),
        "{}",
        out.stdout
    );
    assert!(out.last_line().starts_with("next: "), "{}", out.stdout);
    // The repository chose no proof host, so runs never send to it or mention it.
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    assert!(config.contains("[host]\noff = true\n"), "{config}");
    let plan = repo.fz(&["--plan"]);
    assert!(plan.ok, "{}{}", plan.stdout, plan.stderr);
}

#[test]
fn init_ci_plain_turns_the_host_off_in_a_config_that_is_there() {
    let repo = on_github(&[]);
    let config =
        "# mine\nbase = \"trunk\"\n\n[check.tests]\nrun = \"true\" # kept\nstage = \"working\"\n";
    repo.write("fearlezz.toml", config);
    repo.commit("config");
    let out = repo.fz(&["setup", "init", "--ci", "plain"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    let now = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    assert!(now.starts_with(config), "{now}");
    assert!(now.ends_with("[host]\noff = true\n"), "{now}");
    assert!(out.stdout.contains("fearlezz.toml"), "{}", out.stdout);
    let next = out.last_line();
    assert!(
        next.contains("fearlezz setup sync") && next.contains(WORKFLOW),
        "{}",
        out.stdout
    );

    // With the host, a new config says nothing about it.
    let host = on_github(&[("README.md", "# app\n")]);
    let out = host.fz(&["setup", "init", "--ci", "host"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    let written = std::fs::read_to_string(host.path("fearlezz.toml")).unwrap();
    assert!(!written.contains("[host]"), "{written}");

    // A [host] table that is there gets off = true, its other keys kept.
    let repo = on_github(&[]);
    let config = "[host]\nurl = \"https://proof.example.com\"\n\n[check.tests]\nrun = \"true\"\nstage = \"working\"\n";
    repo.write("fearlezz.toml", config);
    repo.commit("config");
    let out = repo.fz(&["setup", "init", "--ci", "plain"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    let now = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    assert!(
        now.contains("[host]\nurl = \"https://proof.example.com\"\noff = true\n")
            && now.contains("[check.tests]\nrun = \"true\""),
        "{now}"
    );
}

#[test]
fn init_ci_host_adds_the_workflow_to_a_config_that_is_there() {
    let repo = on_github(&[("devenv.nix", "{ pkgs, ... }: { }\n")]);
    let config = "use = [\"devenv\"]\nbase = \"trunk\"\n";
    repo.write("fearlezz.toml", config);
    repo.commit("config");
    let out = repo.fz(&["setup", "init", "--ci", "host"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    // With the host, the config stays as it was.
    assert_eq!(
        std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap(),
        config
    );
    let workflow = std::fs::read_to_string(repo.path(WORKFLOW)).unwrap();
    for part in [
        "contents: read",
        "id-token: write",
        "devenv: true",
        "branches: [trunk]",
    ] {
        assert!(workflow.contains(part), "{part}: {workflow}");
    }
    // Not signed in, it names where to install the app and how to learn whether it is.
    assert!(
        out.stdout
            .contains("https://github.com/apps/fearlezz-dev/installations/new")
            && out.stdout.contains("fearlezz setup login"),
        "{}",
        out.stdout
    );
    assert!(
        out.last_line().contains("fearlezz setup sync"),
        "{}",
        out.stdout
    );
}

#[test]
fn init_ci_never_writes_over_a_workflow() {
    let repo = on_github(&[(WORKFLOW, "name: mine\n")]);
    let out = repo.fz(&["setup", "init", "--ci", "plain"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stderr.contains(WORKFLOW) && out.stderr.contains("already"),
        "{}",
        out.stderr
    );
    assert_eq!(
        std::fs::read_to_string(repo.path(WORKFLOW)).unwrap(),
        "name: mine\n"
    );
    assert!(!repo.path("fearlezz.toml").exists());
}

#[test]
fn init_ci_host_signed_in_says_whether_the_app_is_installed() {
    let host = common::host::FakeHost::start();
    let repo = on_github(&[]);
    repo.write(
        "fearlezz.toml",
        &format!("[host]\nurl = \"{}\"\n", host.url),
    );
    repo.commit("config");
    let config = tempfile::tempdir().unwrap();
    let folder = config.path().join("fearlezz");
    std::fs::create_dir_all(&folder).unwrap();
    let logins = serde_json::json!({ host.url.clone(): {"login": "writer", "token": common::host::HOST_TOKEN} });
    std::fs::write(folder.join("logins.json"), logins.to_string()).unwrap();
    let out = repo.fz_env(
        &["setup", "init", "--ci", "host"],
        &[("XDG_CONFIG_HOME", config.path().to_str().unwrap())],
    );
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    assert!(
        out.stdout
            .contains("the Fearlezz GitHub App is installed on acme/app"),
        "{}",
        out.stdout
    );
}

#[test]
fn init_names_what_the_project_could_prove_too() {
    let repo = project(&[
        (
            "package.json",
            r#"{ "dependencies": { "react": "19.0.0" } }"#,
        ),
        ("deploy/kustomization.yaml", "resources: []\n"),
    ]);
    let out = repo.fz(&["setup", "init"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    let offers: Vec<&str> = out
        .lines()
        .into_iter()
        .filter(|l| l.starts_with("could prove: "))
        .collect();
    assert_eq!(offers.len(), 2, "{}", out.stdout);
    assert!(
        offers[0].contains("a web UI (react in package.json)"),
        "{}",
        out.stdout
    );
    assert!(
        offers[1].contains("kustomize/deploy renders output"),
        "{}",
        out.stdout
    );
    assert_eq!(
        out.last_line(),
        "next: ask the developer which of these to add and add them to fearlezz.toml, then \
         fearlezz setup sync"
    );

    // Asked together with where CI sends its proof, on GitHub.
    let repo = on_github(&[(
        "pubspec.yaml",
        "name: app\ndependencies:\n  flutter:\n    sdk: flutter\n",
    )]);
    let out = repo.fz(&["setup", "init"]);
    assert!(
        out.stdout.contains("could prove: a Flutter app"),
        "{}",
        out.stdout
    );
    let next = out.last_line();
    assert!(
        next.starts_with("next: ask the developer which of these to add and whether CI should send its proof to the proof host")
            && next.contains("--ci plain"),
        "{}",
        out.stdout
    );
}

#[test]
fn setup_guide_prints_the_whole_guide_and_ends_with_next() {
    let repo = project(&[("README.md", "# app\n")]);
    let out = repo.fz(&["setup", "guide"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    let guide = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../skills/fearlezz/GUIDE.md"
    ))
    .unwrap();
    assert!(out.stdout.starts_with(&guide), "{}", out.stdout);
    for step in [
        "fearlezz setup init --ci host",
        "fearlezz setup init --ci plain",
        "fearlezz setup sync",
        "fearlezz setup login",
        "id-token: write",
        "fearlezz run",
    ] {
        assert!(out.stdout.contains(step), "{step}: {}", out.stdout);
    }
    let last = out.stdout.trim_end().lines().last().unwrap_or_default();
    assert!(last.starts_with("next: "), "{last}");
}
