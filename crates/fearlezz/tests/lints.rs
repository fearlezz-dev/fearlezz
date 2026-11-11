//! Lint levels: `[lints.<tool>]` says what a finding of each rule means, Fearlezz gives
//! findings that level by rule name, and writes the rules on or off into the tool's file.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::Repo;

/// A dart-shaped linter: reports every line with a marker as a finding of that rule, at
/// the tool's own level (warning), on every line of src.
const CHECK: &str = r#"
[check.analyze]
run = '''
grep -n -H -o -e avoid_print -e prefer_const -e sort_keys src/*.dart | while IFS=: read -r file line rule; do
  printf '{"file":"%s","line":%s,"level":"flag","rule":"%s","message":"found %s"}\n' "$file" "$line" "$rule" "$rule"
done > "$FEARLEZZ_OUT/findings.jsonl"
'''
stage = "working"
inputs = ["src/**"]
findings = { read = "jsonl", lints = ["dart"] }

[lints.dart]
avoid_print = "gate"
prefer_const = "hint"
sort_keys = "off"
"#;

fn feature(config: &str) -> Repo {
    let repo = Repo::new(config, &[("src/a.dart", "void main() {}\n")]);
    repo.branch("feat/x");
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    repo
}

#[test]
fn findings_take_their_level_from_the_table() {
    let repo = feature(CHECK);
    repo.write(
        "src/a.dart",
        "void main() {}\n// avoid_print\n// prefer_const\n// sort_keys\n",
    );
    let out = repo.fz(&["run", "analyze"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("gate src/a.dart:2  avoid_print"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("hint src/a.dart:3  prefer_const"),
        "{}",
        out.stdout
    );
    // An off rule is not reported at all.
    assert!(!out.stdout.contains("sort_keys"), "{}", out.stdout);
}

#[test]
fn a_stricter_level_judges_the_same_files_again() {
    let lenient = CHECK.replace("avoid_print = \"gate\"", "avoid_print = \"flag\"");
    let repo = feature(&lenient);
    repo.write("src/a.dart", "void main() {}\n// avoid_print\n");
    assert!(repo.fz(&["run", "analyze"]).ok);

    // Same files, stricter table: the earlier pass must not be reused.
    repo.write("fearlezz.toml", CHECK);
    let out = repo.fz(&["run", "analyze"]);
    assert!(!out.ok, "{}", out.stdout);
    assert!(
        out.stdout.contains("gate src/a.dart:2  avoid_print"),
        "{}",
        out.stdout
    );
}

#[test]
fn a_piece_sets_levels_and_the_project_overrides_one() {
    let piece = "[lints.dart]\navoid_print = \"gate\"\nprefer_const = \"gate\"\n";
    let config = CHECK.split("[lints.dart]").next().unwrap().replace(
        "[check.analyze]",
        "use = [\"./base.toml\"]\n\n[check.analyze]",
    ) + "\n[lints.dart]\nprefer_const = \"off\"\n";
    let repo = Repo::new(
        &config,
        &[("src/a.dart", "void main() {}\n"), ("base.toml", piece)],
    );
    repo.branch("feat/x");
    repo.fz(&["run", "--stage", "working", "--no-run"]);
    repo.write(
        "src/a.dart",
        "void main() {}\n// avoid_print\n// prefer_const\n",
    );
    let out = repo.fz(&["run", "analyze"]);
    assert!(
        out.stdout.contains("gate src/a.dart:2  avoid_print"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("prefer_const"), "{}", out.stdout);
}

#[test]
fn an_unknown_lint_tool_is_refused() {
    let repo = Repo::new(
        &CHECK.replace("lints = [\"dart\"]", "lints = [\"drat\"]"),
        &[],
    );
    let out = repo.fz(&[]);
    assert!(!out.ok);
    assert!(
        out.stderr.contains("no lint tool \"drat\""),
        "{}",
        out.stderr
    );
}

#[test]
fn sync_turns_rules_on_or_off_in_cargo_toml() {
    let config = r#"
[lints.clippy]
wildcard_imports = "hint"
needless_continue = "off"
missing_docs_in_private_items = "gate"

[lints.rustc]
missing_docs = "flag"
"#;
    let repo = Repo::new(
        config,
        &[(
            "Cargo.toml",
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n",
        )],
    );
    let out = repo.fz(&["setup", "sync"]);
    assert!(out.ok, "{}", out.stderr);
    let cargo = std::fs::read_to_string(repo.path("Cargo.toml")).unwrap();
    // Every level that is on warns; Fearlezz gives the finding its level.
    assert!(
        cargo.contains(
            "[lints.clippy]\nmissing_docs_in_private_items = \"warn\"\nneedless_continue = \"allow\"\nwildcard_imports = \"warn\"\n"
        ),
        "{cargo}"
    );
    assert!(
        cargo.contains("[lints.rust]\nmissing_docs = \"warn\"\n"),
        "{cargo}"
    );
}

#[test]
fn sync_writes_dart_rules_with_their_editor_severity() {
    let config = r#"
[settings.dart-analysis.analyzer]
exclude = ["build/**"]

[lints.dart]
avoid_print = "gate"
prefer_const_constructors = "hint"
sort_child_properties_last = "off"
strict-casts = "gate"
"#;
    let repo = Repo::new(config, &[]);
    assert!(repo.fz(&["setup", "sync"]).ok);
    assert_eq!(
        std::fs::read_to_string(repo.path("analysis_options.yaml")).unwrap(),
        "# Managed by Fearlezz from [settings] in fearlezz.toml; change it there.\n\
         analyzer:\n  errors:\n    avoid_print: \"error\"\n    prefer_const_constructors: \"info\"\n  \
         exclude:\n    - \"build/**\"\n  language:\n    strict-casts: true\n\
         linter:\n  rules:\n    avoid_print: true\n    prefer_const_constructors: true\n    sort_child_properties_last: false\n"
    );
}

fn synced(config: &str, files: &[(&str, &str)]) -> Repo {
    let repo = Repo::new(config, files);
    let out = repo.fz(&["setup", "sync"]);
    assert!(out.ok, "{}", out.stderr);
    repo
}

fn read(repo: &Repo, path: &str) -> String {
    std::fs::read_to_string(repo.path(path)).unwrap()
}

#[test]
fn biome_rules_go_under_their_group_and_other_keys_stay() {
    let config = "[lints.biome]\n\"suspicious/noDebugger\" = \"gate\"\n\"style/useConst\" = \"hint\"\n\"style/noVar\" = \"off\"\n";
    // biome.json may hold comments; they go, the keys stay.
    let repo = synced(
        config,
        &[(
            "biome.json",
            "{\n  // formatting\n  \"formatter\": { \"indentStyle\": \"space\", },\n}\n",
        )],
    );
    let doc: serde_json::Value = serde_json::from_str(&read(&repo, "biome.json")).unwrap();
    assert_eq!(doc["formatter"]["indentStyle"], "space");
    let rules = &doc["linter"]["rules"];
    assert_eq!(rules["preset"], "none");
    assert_eq!(rules["suspicious"]["noDebugger"], "error");
    assert_eq!(rules["style"]["useConst"], "info");
    assert_eq!(rules["style"]["noVar"], "off");
}

#[test]
fn a_json_file_keeps_its_indentation_and_key_order() {
    let repo = synced(
        "[settings.tsconfig.compilerOptions]\nstrict = true\n",
        &[("tsconfig.json", "{\n\t\"include\": [\"src\"]\n}\n")],
    );
    assert_eq!(
        read(&repo, "tsconfig.json"),
        "{\n\t\"include\": [\"src\"],\n\t\"compilerOptions\": {\n\t\t\"strict\": true\n\t}\n}\n"
    );
}

#[test]
fn eslint_rules_are_written_to_a_file_the_config_spreads_in() {
    let config = "[lints.eslint]\neqeqeq = \"gate\"\n\"@typescript-eslint/no-explicit-any\" = \"hint\"\nno-console = \"off\"\n";
    let repo = synced(config, &[]);
    let text = read(&repo, "eslint.fearlezz.mjs");
    assert!(text.contains("export default [{"), "{text}");
    assert!(text.contains("\"eqeqeq\": \"error\""), "{text}");
    // ESLint has no info level, so a hint warns.
    assert!(
        text.contains("\"@typescript-eslint/no-explicit-any\": \"warn\""),
        "{text}"
    );
    assert!(text.contains("\"no-console\": \"off\""), "{text}");
}

#[test]
fn ruff_selects_the_rules_that_are_on() {
    let config = "[settings.ruff]\nline-length = 100\n\n[lints.ruff]\nF401 = \"gate\"\nE501 = \"off\"\nB006 = \"hint\"\n";
    let repo = synced(config, &[("ruff.toml", "target-version = \"py312\"\n")]);
    let text = read(&repo, "ruff.toml");
    assert!(text.contains("target-version = \"py312\""), "{text}");
    assert!(text.contains("line-length = 100"), "{text}");
    assert!(text.contains("select = [\"B006\", \"F401\"]"), "{text}");
}

#[test]
fn ktlint_switches_go_in_a_marked_block_of_editorconfig() {
    let config = "[lints.ktlint]\n\"standard:no-wildcard-imports\" = \"gate\"\n\"standard:filename\" = \"off\"\n";
    let mine = "root = true\n\n[*]\nindent_size = 4\n";
    let repo = synced(config, &[(".editorconfig", mine)]);
    let text = read(&repo, ".editorconfig");
    assert!(text.starts_with(mine), "{text}");
    assert!(text.contains("[*.{kt,kts}]\nktlint_standard_filename = disabled\nktlint_standard_no-wildcard-imports = enabled\n"), "{text}");
    // A second sync replaces the block instead of adding another.
    assert!(
        repo.fz(&["setup", "sync"])
            .stdout
            .starts_with("settings already in sync\n")
    );
    std::fs::write(
        repo.path(".editorconfig"),
        text.replace("disabled", "enabled"),
    )
    .unwrap();
    assert!(repo.fz(&["setup", "sync"]).ok);
    assert_eq!(read(&repo, ".editorconfig"), text);
}

#[test]
fn detekt_and_golangci_files_are_written_whole() {
    let config = r#"
[settings.detekt.config]
validation = false

[lints.detekt]
"style/MagicNumber" = "hint"
"naming/FunctionNaming" = "off"

[lints.golangci]
errcheck = "gate"
lll = "off"
"#;
    let repo = synced(config, &[]);
    let detekt = read(&repo, "config/detekt/detekt.yml");
    assert!(
        detekt.contains("config:\n  validation: false\n"),
        "{detekt}"
    );
    assert!(
        detekt.contains("naming:\n  FunctionNaming:\n    active: false\n"),
        "{detekt}"
    );
    assert!(
        detekt.contains("style:\n  MagicNumber:\n    active: true\n"),
        "{detekt}"
    );
    let go = read(&repo, ".golangci.yml");
    assert!(
        go.contains("linters:\n  default: \"none\"\n  enable:\n    - \"errcheck\"\n"),
        "{go}"
    );
    assert!(go.contains("version: \"2\""), "{go}");
}

#[test]
fn tsconfig_options_merge_into_a_file_with_comments() {
    let config =
        "[settings.tsconfig.compilerOptions]\nstrict = true\nallowUnreachableCode = false\n";
    let mine = "{\n  /* base */\n  \"compilerOptions\": {\n    \"target\": \"es2022\", // modern\n    \"url\": \"http://x//y\",\n  },\n  \"include\": [\"src\"],\n}\n";
    let repo = synced(config, &[("tsconfig.json", mine)]);
    let doc: serde_json::Value = serde_json::from_str(&read(&repo, "tsconfig.json")).unwrap();
    let options = &doc["compilerOptions"];
    assert_eq!(options["target"], "es2022");
    assert_eq!(options["url"], "http://x//y");
    assert_eq!(options["strict"], true);
    assert_eq!(options["allowUnreachableCode"], false);
    assert_eq!(doc["include"][0], "src");
}
