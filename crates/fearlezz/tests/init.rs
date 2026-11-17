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
            out.stderr.contains("no fearlezz.toml") && out.stderr.contains("fearlezz setup init"),
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
    assert_eq!(out.last_line(), "next: fearlezz setup sync");

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
