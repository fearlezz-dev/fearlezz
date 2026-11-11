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
        ("pubspec.yaml", "name: app\n"),
    ]);
    let out = repo.fz(&["setup", "init"]);
    assert!(out.ok, "{}{}", out.stdout, out.stderr);
    let config = std::fs::read_to_string(repo.path("fearlezz.toml")).unwrap();
    assert!(
        config.contains(
            r#"use = ["devenv", "rust", "typescript", "python", "go", "kotlin", "flutter"]"#
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
        "vitest",
        "pytest",
        "go-test",
        "gradle-test",
        "flutter-test",
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
    assert!(!config.contains("use ="), "{config}");
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
