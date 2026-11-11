//! Shared settings: pieces carry tool settings, Fearlezz writes the tools' own files, and the
//! settings check fails when a file drifts.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;

const WORKSPACE: &str = "[workspace]\nmembers = [\"crates/*\"]\nresolver = \"3\"\n";
const MEMBER: &str = "[package]\nname = \"app\"\nversion = \"0.1.0\"\n";

/// A piece with lint settings, used by the project, which overrides one level.
const PIECE: &str = r#"
[settings.cargo-lints.clippy]
pedantic = { level = "warn", priority = -1 }
unwrap_used = "deny"
"#;

fn workspace(project: &str) -> Repo {
    Repo::new(
        project,
        &[
            ("Cargo.toml", WORKSPACE),
            ("crates/app/Cargo.toml", MEMBER),
            ("base.toml", PIECE),
        ],
    )
}

const PROJECT: &str =
    "use = [\"./base.toml\"]\n\n[settings.cargo-lints.clippy]\nunwrap_used = \"warn\"\n";

#[test]
fn sync_writes_the_lints_and_members_inherit_them() {
    let repo = workspace(PROJECT);
    let out = repo.fz(&["setup", "sync"]);
    assert!(out.ok, "{}", out.stderr);
    assert!(out.stdout.contains("wrote Cargo.toml"), "{}", out.stdout);
    assert!(
        out.stdout.contains("wrote crates/app/Cargo.toml"),
        "{}",
        out.stdout
    );

    let root = std::fs::read_to_string(repo.path("Cargo.toml")).unwrap();
    assert!(root.starts_with(WORKSPACE), "{root}");
    assert!(root.contains("# Managed by Fearlezz"), "{root}");
    assert!(root.contains("[workspace.lints.clippy]"), "{root}");
    // The project's override wins over the piece, field by field.
    assert!(root.contains("unwrap_used = \"warn\""), "{root}");
    assert!(
        root.contains("pedantic = { level = \"warn\", priority = -1 }"),
        "{root}"
    );
    let member = std::fs::read_to_string(repo.path("crates/app/Cargo.toml")).unwrap();
    assert_eq!(member, format!("{MEMBER}\n[lints]\nworkspace = true\n"));

    assert!(
        repo.fz(&["setup", "sync"])
            .stdout
            .starts_with("settings already in sync\n")
    );
}

#[test]
fn the_settings_check_fails_on_drift_and_sync_fixes_it() {
    let repo = workspace(PROJECT);
    repo.fz(&["setup", "sync"]);
    repo.commit("synced");
    repo.branch("feat/x");
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    let synced = std::fs::read_to_string(repo.path("Cargo.toml")).unwrap();
    repo.write("Cargo.toml", &synced.replace("\"warn\"\n", "\"allow\"\n"));

    let out = repo.fz(&["run", "settings"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains("gate Cargo.toml:1  settings  differs from the settings in fearlezz.toml"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("fix: fearlezz setup sync"),
        "{}",
        out.stdout
    );

    repo.fz(&["setup", "sync"]);
    assert!(repo.fz(&["run", "settings"]).ok);
}

#[test]
fn the_settings_check_can_be_turned_off() {
    let repo = workspace(&format!("{PROJECT}\n[check.settings]\noff = true\n"));
    let out = repo.fz(&["run", "settings"]);
    assert!(!out.ok);
    assert!(
        out.stderr.contains("no check \"settings\""),
        "{}",
        out.stderr
    );
}

#[test]
fn dart_analysis_options_are_written_whole() {
    let config = r#"
[settings.dart-analysis]
include = "package:lints/recommended.yaml"

[settings.dart-analysis.linter.rules]
prefer_const_constructors = true
"#;
    let repo = Repo::new(config, &[]);
    assert!(repo.fz(&["setup", "sync"]).ok);
    assert_eq!(
        std::fs::read_to_string(repo.path("analysis_options.yaml")).unwrap(),
        "# Managed by Fearlezz from [settings] in fearlezz.toml; change it there.\n\
         include: \"package:lints/recommended.yaml\"\n\
         linter:\n  rules:\n    prefer_const_constructors: true\n"
    );
}

#[test]
fn a_missing_settings_file_is_named_as_missing() {
    let config = "[settings.claude.permissions]\nallow = [\"Bash(ls)\"]\n";
    let repo = Repo::new(config, &[("src/a.txt", "a\n")]);
    repo.branch("feat/x");
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    let out = repo.fz(&["run", "settings"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout
            .contains(".claude/settings.json is missing; the settings in fearlezz.toml write it"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("fix: fearlezz setup sync"),
        "{}",
        out.stdout
    );
    assert!(
        !out.stdout.contains("not files from the repository root"),
        "{}",
        out.stdout
    );
}

#[test]
fn rust_lints_without_a_cargo_manifest_are_a_finding_not_an_error() {
    let config = "[lints.clippy]\nunwrap_used = \"gate\"\n";
    let repo = Repo::new(config, &[("src/a.txt", "a\n")]);
    let sync = repo.fz(&["setup", "sync"]);
    assert!(sync.ok, "{}\n{}", sync.stdout, sync.stderr);
    repo.branch("feat/x");
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    let out = repo.fz(&["run", "settings"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stderr.is_empty(), "{}", out.stderr);
    assert!(
        out.stdout
            .contains("no Cargo.toml to write the [lints.clippy] and [lints.rustc] rules into"),
        "{}",
        out.stdout
    );
}

#[test]
fn agent_permissions_merge_into_the_existing_settings() {
    let config = r#"
[settings.claude.permissions]
allow = ["Bash(cargo test:*)", "Bash(fearlezz:*)"]
"#;
    let existing = "{\n  \"hooks\": {\n    \"Stop\": []\n  }\n}\n";
    let repo = Repo::new(config, &[(".claude/settings.json", existing)]);
    assert!(repo.fz(&["setup", "sync"]).ok);
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(repo.path(".claude/settings.json")).unwrap())
            .unwrap();
    assert_eq!(written["hooks"]["Stop"], serde_json::json!([]));
    assert_eq!(
        written["permissions"]["allow"],
        serde_json::json!(["Bash(cargo test:*)", "Bash(fearlezz:*)"])
    );
}

#[test]
fn clippy_options_are_set_and_other_options_stay() {
    let config = "[settings.clippy]\nallow-unwrap-in-tests = true\n";
    let repo = Repo::new(
        config,
        &[("clippy.toml", "too-many-lines-threshold = 80\n")],
    );
    assert!(repo.fz(&["setup", "sync"]).ok);
    assert_eq!(
        std::fs::read_to_string(repo.path("clippy.toml")).unwrap(),
        "too-many-lines-threshold = 80\nallow-unwrap-in-tests = true\n"
    );
}

#[test]
fn the_agents_piece_writes_the_shared_permissions() {
    let repo = Repo::new("use = [\"agents\"]\n", &[]);
    assert!(repo.fz(&["setup", "sync"]).ok);
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(repo.path(".claude/settings.json")).unwrap())
            .unwrap();
    let allow = written["permissions"]["allow"].as_array().unwrap();
    assert!(allow.contains(&serde_json::json!("Bash(fearlezz:*)")));
    assert_eq!(
        written["permissions"]["deny"],
        serde_json::json!(["Bash(rm:*)"])
    );
}

/// The skill the binary embeds; a project receives this text.
const SHIPPED_SKILL: &str = include_str!("../../../skills/fearlezz/SKILL.md");
const CLAUDE_SKILL: &str = ".claude/skills/fearlezz/SKILL.md";
const AGENTS_SKILL: &str = ".agents/skills/fearlezz/SKILL.md";

#[test]
fn sync_writes_the_shipped_skill_for_every_agent() {
    let repo = Repo::with_skill("", &[]);
    let out = repo.fz(&["setup", "sync"]);
    assert!(out.ok, "{}", out.stderr);
    for path in [CLAUDE_SKILL, AGENTS_SKILL] {
        assert!(
            out.stdout.contains(&format!("wrote {path}")),
            "{}",
            out.stdout
        );
        let written = std::fs::read_to_string(repo.path(path)).unwrap();
        // The frontmatter stays first, or the agent does not see a skill.
        assert!(written.starts_with("---\nname: fearlezz\n"), "{written}");
        assert!(written.contains("fearlezz setup sync"), "{written}");
        let (_, body) = SHIPPED_SKILL.split_once("\n---\n").unwrap();
        assert!(written.ends_with(body), "{written}");
    }
    assert_eq!(
        out.last_line(),
        "next: commit the written files with the change"
    );
}

#[test]
fn a_project_without_the_skill_is_led_to_it() {
    let repo = Repo::with_skill("", &[]);
    let before = repo.fz(&[]);
    assert_eq!(
        before.last_line(),
        "next: write the agent skill for this Fearlezz version: fearlezz setup sync, then commit \
         it"
    );
    assert!(repo.fz(&["setup", "sync"]).ok);
    repo.commit("synced");
    let after = repo.fz(&[]);
    assert!(
        !after.stdout.contains("write the agent skill"),
        "{}",
        after.stdout
    );
}

#[test]
fn a_skill_that_differs_from_this_binary_is_drift() {
    let repo = Repo::with_skill("", &[]);
    repo.fz(&["setup", "sync"]);
    repo.commit("synced");
    repo.branch("feat/x");
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    // What an older Fearlezz left behind, or a hand edit.
    repo.write(
        CLAUDE_SKILL,
        "---\nname: fearlezz\n---\n\nAn older skill.\n",
    );

    assert!(
        repo.fz(&[])
            .last_line()
            .starts_with("next: write the agent skill"),
    );
    let out = repo.fz(&["run", "settings"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains(&format!(
            "gate {CLAUDE_SKILL}:1  settings  differs from the skill this Fearlezz version ships"
        )),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("fix: fearlezz setup sync"),
        "{}",
        out.stdout
    );

    repo.fz(&["setup", "sync"]);
    assert!(repo.fz(&["run", "settings"]).ok);
}

#[test]
fn a_project_can_choose_which_agents_get_the_skill() {
    let repo = Repo::with_skill("[settings.skill]\nagents = [\"claude\"]\n", &[]);
    assert!(repo.fz(&["setup", "sync"]).ok);
    assert!(repo.path(CLAUDE_SKILL).exists());
    assert!(!repo.path(AGENTS_SKILL).exists());
}

#[test]
fn a_project_can_opt_out_of_the_skill() {
    let repo = Repo::with_skill(
        "[settings.skill]\nagents = []\n\n[settings.hooks]\nagents = []\n",
        &[],
    );
    let status = repo.fz(&[]);
    assert!(
        !status.stdout.contains("write the agent skill"),
        "{}",
        status.stdout
    );
    assert!(repo.fz(&["setup", "sync"]).ok);
    assert!(!repo.path(CLAUDE_SKILL).exists());
    assert!(!repo.path(AGENTS_SKILL).exists());
    // No skill or hooks to keep in line and no other settings: nothing for the check to do.
    let out = repo.fz(&["run", "settings"]);
    assert!(
        out.stderr.contains("no check \"settings\""),
        "{}",
        out.stderr
    );
}

#[test]
fn an_unknown_skill_agent_says_which_exist() {
    let repo = Repo::with_skill("[settings.skill]\nagents = [\"cluade\"]\n", &[]);
    let out = repo.fz(&["setup", "sync"]);
    assert!(!out.ok);
    assert!(
        out.stderr
            .contains("[settings.skill]: no agent \"cluade\"; agents are claude, agents"),
        "{}",
        out.stderr
    );
}

#[test]
fn settings_go_into_the_config_file_the_project_already_has() {
    let config = r#"
[lints.ruff]
E501 = "gate"

[lints.golangci]
errcheck = "gate"

[lints.biome]
"suspicious/noDebugger" = "gate"
"#;
    let pyproject = "[project]\nname = \"app\"\n\n[tool.ruff]\nline-length = 100\n";
    let repo = Repo::new(
        config,
        &[
            ("pyproject.toml", pyproject),
            (".golangci.yaml", "run:\n  timeout: 5m\n"),
            (
                "biome.jsonc",
                "{\n  // the project's own\n  \"files\": {}\n}\n",
            ),
        ],
    );
    let out = repo.fz(&["setup", "sync"]);
    assert!(out.ok, "{}", out.stderr);
    for shadow in ["ruff.toml", ".golangci.yml", "biome.json"] {
        assert!(!repo.path(shadow).exists(), "{shadow}: {}", out.stdout);
    }
    let py = std::fs::read_to_string(repo.path("pyproject.toml")).unwrap();
    assert!(py.contains("line-length = 100"), "{py}");
    assert!(py.contains("[tool.ruff.lint]\nselect = [\"E501\"]"), "{py}");
    let go = std::fs::read_to_string(repo.path(".golangci.yaml")).unwrap();
    assert!(go.contains("- \"errcheck\""), "{go}");
    let biome = std::fs::read_to_string(repo.path("biome.jsonc")).unwrap();
    assert!(biome.contains("\"noDebugger\": \"error\""), "{biome}");
    assert!(
        repo.fz(&["setup", "sync"])
            .stdout
            .starts_with("settings already in sync")
    );
}

#[test]
fn a_pyproject_without_a_ruff_table_gets_a_ruff_toml() {
    let repo = Repo::new(
        "[lints.ruff]\nE501 = \"gate\"\n",
        &[("pyproject.toml", "[project]\nname = \"app\"\n")],
    );
    assert!(repo.fz(&["setup", "sync"]).ok);
    assert!(repo.path("ruff.toml").exists());
    assert_eq!(
        std::fs::read_to_string(repo.path("pyproject.toml")).unwrap(),
        "[project]\nname = \"app\"\n"
    );
}

#[test]
fn workspace_members_are_found_the_way_cargo_finds_them() {
    let root = "[package]\nname = \"root\"\nversion = \"0.1.0\"\n\n[workspace]\nmembers = [\".\", \"crates/*\", \"tools/xtask-*\"]\nexclude = [\"crates/old\"]\n";
    let repo = Repo::new(
        "[settings.cargo-lints.clippy]\nunwrap_used = \"warn\"\n",
        &[
            ("Cargo.toml", root),
            ("crates/app/Cargo.toml", MEMBER),
            ("crates/old/Cargo.toml", MEMBER),
            ("tools/xtask-gen/Cargo.toml", MEMBER),
        ],
    );
    let out = repo.fz(&["setup", "sync"]);
    assert!(out.ok, "{}", out.stderr);
    let root = std::fs::read_to_string(repo.path("Cargo.toml")).unwrap();
    assert!(root.contains("[lints]\nworkspace = true"), "{root}");
    assert!(root.contains("[workspace.lints.clippy]"), "{root}");
    let inherits = format!("{MEMBER}\n[lints]\nworkspace = true\n");
    for member in ["crates/app", "tools/xtask-gen"] {
        let text = std::fs::read_to_string(repo.path(&format!("{member}/Cargo.toml"))).unwrap();
        assert_eq!(text, inherits, "{member}");
    }
    assert_eq!(
        std::fs::read_to_string(repo.path("crates/old/Cargo.toml")).unwrap(),
        MEMBER
    );
    // Writing again changes nothing.
    assert!(
        repo.fz(&["setup", "sync"])
            .stdout
            .starts_with("settings already in sync")
    );
}

#[test]
fn a_file_that_cannot_be_read_is_never_written_over() {
    let repo = Repo::new("[settings.clippy]\nallow-unwrap-in-tests = true\n", &[]);
    let bytes = b"too-many-lines-threshold = 80\n# \xff\n";
    std::fs::write(repo.path("clippy.toml"), bytes).unwrap();
    let out = repo.fz(&["setup", "sync"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stderr.contains("reading clippy.toml"), "{}", out.stderr);
    assert_eq!(std::fs::read(repo.path("clippy.toml")).unwrap(), bytes);
}

#[test]
fn an_editorconfig_block_without_its_end_is_not_doubled() {
    let text = "root = true\n\n# Fearlezz: begin\n[*.kt]\nindent_size = 4\n\n[*.md]\ntrim_trailing_whitespace = false\n";
    let repo = Repo::new(
        "[lints.ktlint]\nno-wildcard-imports = \"gate\"\n",
        &[(".editorconfig", text)],
    );
    let out = repo.fz(&["setup", "sync"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(out.stderr.contains("do not pair up"), "{}", out.stderr);
    assert_eq!(
        std::fs::read_to_string(repo.path(".editorconfig")).unwrap(),
        text
    );
}

#[test]
fn a_biome_rule_without_its_group_is_a_config_error() {
    let repo = Repo::new("[lints.biome]\nnoDebugger = \"gate\"\n", &[]);
    let out = repo.fz(&[]);
    assert!(!out.ok);
    assert!(
        out.stderr
            .contains("[lints.biome] noDebugger: name the rule with its group"),
        "{}",
        out.stderr
    );
}

#[test]
fn a_managed_file_git_ignores_still_counts_for_the_check() {
    let config = "[settings.claude.permissions]\nallow = [\"Bash(fearlezz:*)\"]\n";
    let repo = Repo::new(config, &[(".gitignore", ".claude/\n")]);
    repo.fz(&["setup", "sync"]);
    repo.branch("feat/x");
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    assert!(repo.fz(&["run", "settings"]).ok);
    repo.write(".claude/settings.json", "{}\n");
    let out = repo.fz(&["run", "settings"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains(".claude/settings.json"),
        "{}",
        out.stdout
    );
}

#[test]
fn json_arrays_wrap_where_the_projects_formatter_wraps_them() {
    // As Biome 2.6.5 lays it out: a line of exactly 80 columns stays on one line.
    let fits = r#"["bbbb", "bbbb", "bbbb", "bbbb", "bbbb", "bbbb", "bbbb", "bbbbbbbbbbb"]"#;
    let config = format!("[settings.tsconfig.compilerOptions]\nb = {fits}\n");
    let repo = Repo::new(&config, &[]);
    assert!(repo.fz(&["setup", "sync"]).ok);
    let line = format!("    \"b\": {fits}");
    assert_eq!(line.len(), 80);
    let written = std::fs::read_to_string(repo.path("tsconfig.json")).unwrap();
    assert!(written.contains(&format!("{line}\n")), "{written}");

    // One more column wraps, unless the project's Biome allows wider lines.
    let longer = fits.replace("bbbbbbbbbbb", "bbbbbbbbbbbb");
    let config = format!("[settings.tsconfig.compilerOptions]\nb = {longer}\n");
    let repo = Repo::new(&config, &[]);
    assert!(repo.fz(&["setup", "sync"]).ok);
    let written = std::fs::read_to_string(repo.path("tsconfig.json")).unwrap();
    assert!(written.contains("    \"b\": [\n"), "{written}");
    let repo = Repo::new(
        &config,
        &[(
            "biome.json",
            "{\n  \"formatter\": { \"lineWidth\": 100 }\n}\n",
        )],
    );
    assert!(repo.fz(&["setup", "sync"]).ok);
    let written = std::fs::read_to_string(repo.path("tsconfig.json")).unwrap();
    assert!(
        written.contains(&format!("    \"b\": {longer}\n")),
        "{written}"
    );
}

#[test]
fn the_typescript_piece_keeps_biome_off_the_files_fearlezz_writes() {
    let repo = Repo::new("use = [\"typescript\"]\n", &[]);
    let out = repo.fz(&["setup", "sync"]);
    assert!(out.ok, "{}", out.stderr);
    let biome = std::fs::read_to_string(repo.path("biome.json")).unwrap();
    for skipped in ["!.claude", "!.agents", "!.codex", "!.cursor"] {
        assert!(biome.contains(&format!("\"{skipped}\"")), "{biome}");
    }
    // Biome rejects the whole file over one rule it does not know; these started in nursery
    // and are not in Biome 2.6.6.
    assert!(!biome.contains("noComponentHookFactories"), "{biome}");
}

#[test]
fn the_flutter_piece_excludes_the_folders_flutter_adds_to_the_analyzer() {
    let repo = Repo::new("use = [\"flutter\"]\n", &[]);
    let out = repo.fz(&["setup", "sync"]);
    assert!(out.ok, "{}", out.stderr);
    let options = std::fs::read_to_string(repo.path("analysis_options.yaml")).unwrap();
    // `flutter test` appends whichever of these a missing platform folder asks for.
    for folder in [
        "build", "android", "ios", "web", "windows", "macos", "linux",
    ] {
        assert!(
            options.contains(&format!("    - \"{folder}/**\"\n")),
            "{options}"
        );
    }
}

#[test]
fn the_flutter_look_piece_writes_its_test_helpers_and_catches_their_drift() {
    let repo = Repo::new("use = [\"flutter-look\"]\n", &[]);
    let out = repo.fz(&["setup", "sync"]);
    assert!(out.ok, "{}", out.stderr);
    for file in [
        "look_test/fearlezz_look.dart",
        "look_test/flutter_test_config.dart",
    ] {
        assert!(
            out.stdout.contains(&format!("wrote {file}")),
            "{}",
            out.stdout
        );
    }
    let helper = std::fs::read_to_string(repo.path("look_test/fearlezz_look.dart")).unwrap();
    assert!(
        helper.starts_with("// Written by fearlezz setup sync"),
        "{helper}"
    );
    assert!(helper.contains("Future<void> still(WidgetTester tester, String id)"));
    repo.commit("synced");
    repo.branch("feat/x");
    repo.write("look_test/fearlezz_look.dart", "// mine now\n");
    let out = repo.fz(&["run", "settings"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains(
            "gate look_test/fearlezz_look.dart:1  settings  differs from the settings in \
             fearlezz.toml"
        ),
        "{}",
        out.stdout
    );
}
