//! Findings: problems a check reports at a file and line, like a linter's warnings. A check
//! that declares `findings` passes when no gate finding is in scope. By default only findings
//! on lines the change added or modified count, so old code never blocks new work; a finding
//! marked `anywhere` (a compile error) counts wherever it is, because changing one line can
//! break another.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

/// How much a finding matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Level {
    /// Told to the agent only.
    Hint,
    /// Shown in status and the report; does not block.
    Flag,
    /// Fails the check.
    Gate,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Finding {
    /// Relative to the worktree root; empty when the tool named no file.
    pub file: String,
    pub line: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    pub level: Level,
    #[serde(default)]
    pub rule: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub anywhere: bool,
}

/// Where a check's findings come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Reader {
    /// `cargo ... --message-format=json` on stdout.
    Cargo,
    /// SARIF 2.1 in `$FEARLEZZ_OUT/findings.sarif`, or on stdout.
    Sarif,
    /// One finding per line in `$FEARLEZZ_OUT/findings.jsonl`, in this module's shape.
    Jsonl,
    /// `dart analyze --format=machine` on stdout.
    Dart,
    /// `eslint --format json` on stdout.
    Eslint,
}

/// Which findings count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Scope {
    /// Findings on lines the change added or modified.
    #[default]
    Changed,
    Everywhere,
}

impl Reader {
    /// As written in `findings`.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Reader::Cargo => "cargo",
            Reader::Sarif => "sarif",
            Reader::Jsonl => "jsonl",
            Reader::Dart => "dart",
            Reader::Eslint => "eslint",
        }
    }

    /// Whether the check's stdout holds the findings, so it is kept apart from the log.
    pub(crate) fn reads_stdout(self) -> bool {
        matches!(
            self,
            Reader::Cargo | Reader::Sarif | Reader::Dart | Reader::Eslint
        )
    }

    /// Reads the findings a run left. `stdout` is the run's captured stdout, `out` its proof
    /// folder. Returns the findings and what could not be read.
    pub(crate) fn read(
        self,
        stdout: &Path,
        out: &Path,
        root: &Path,
    ) -> (Vec<Finding>, Vec<String>) {
        let text = |p: &Path| std::fs::read_to_string(p).unwrap_or_default();
        let mut found = match self {
            Reader::Cargo => (cargo(&text(stdout)), Vec::new()),
            Reader::Sarif => {
                let file = out.join("findings.sarif");
                let source = if file.exists() {
                    text(&file)
                } else {
                    text(stdout)
                };
                sarif(&source)
            }
            Reader::Jsonl => jsonl(&text(&out.join("findings.jsonl"))),
            Reader::Dart => (dart(&text(stdout)), Vec::new()),
            Reader::Eslint => eslint(&text(stdout)),
        };
        for f in &mut found.0 {
            f.file = relative(&f.file, root);
        }
        // A relative path that names no file here was written from another folder (a tool
        // run in a subfolder). Judging it would hide it as "elsewhere", so it blocks instead.
        let lost: BTreeSet<String> = found
            .0
            .iter()
            .filter(|f| !f.file.is_empty() && !f.file.starts_with('/'))
            .filter(|f| !root.join(&f.file).is_file())
            .map(|f| f.file.clone())
            .collect();
        if !lost.is_empty() {
            found.0.retain(|f| !lost.contains(&f.file));
            let names: Vec<&str> = lost.iter().take(3).map(String::as_str).collect();
            let mut gate = unplaced(&format!(
                "findings name {}{}, which are not files from the repository root",
                names.join(", "),
                if lost.len() > 3 { " and more" } else { "" }
            ));
            gate.fix = Some(
                "run the tool so it names paths from the repository root, or from an absolute \
                 path"
                    .into(),
            );
            found.0.push(gate);
        }
        found
    }
}

/// Lines the change added or modified, per file.
#[derive(Default)]
pub(crate) struct Changed {
    files: BTreeMap<String, Lines>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Lines {
    /// A file the base does not have.
    All,
    Ranges(Vec<(u32, u32)>),
}

impl Changed {
    /// From `git diff -U0` against the base, the files the base does not have, and every
    /// changed path. A changed path the diff text did not yield (an unusual name git quotes)
    /// counts as changed everywhere, so it can never hide a finding.
    pub(crate) fn from_diff(diff: &str, new_files: &[String], all: &BTreeSet<String>) -> Changed {
        let mut files: BTreeMap<String, Lines> = BTreeMap::new();
        let mut current: Option<String> = None;
        let mut old: Option<String> = None;
        for line in diff.lines() {
            // Git ends a header path that holds a space with a tab.
            if let Some(path) = line.strip_prefix("--- ") {
                old = path
                    .trim_end_matches('\t')
                    .strip_prefix("a/")
                    .map(String::from);
            } else if let Some(path) = line.strip_prefix("+++ ") {
                current = path
                    .trim_end_matches('\t')
                    .strip_prefix("b/")
                    .map(String::from);
                // A deleted file has no lines left, but the change still touched it.
                if let Some(p) = current.as_ref().or(old.as_ref()) {
                    files.entry(p.clone()).or_insert(Lines::Ranges(Vec::new()));
                }
            } else if let (Some(hunk), Some(path)) = (line.strip_prefix("@@ "), &current) {
                let Some((start, count)) = added_range(hunk) else {
                    continue;
                };
                if count == 0 {
                    continue;
                }
                if let Some(Lines::Ranges(r)) = files.get_mut(path) {
                    r.push((start, start + count - 1));
                }
            }
        }
        for f in new_files {
            files.insert(f.clone(), Lines::All);
        }
        for f in all {
            files.entry(f.clone()).or_insert(Lines::All);
        }
        Changed { files }
    }

    /// Every path the change touched: added, modified or deleted.
    pub(crate) fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    fn touches(&self, f: &Finding) -> bool {
        let end = f.end_line.unwrap_or(f.line).max(f.line);
        match self.files.get(&f.file) {
            None => false,
            Some(Lines::All) => true,
            // A finding that names no line (a vulnerable dependency in a lockfile) belongs to
            // its whole file, so it counts when the change touched the file at all.
            Some(Lines::Ranges(_)) if f.line == 0 => true,
            Some(Lines::Ranges(r)) => r.iter().any(|&(a, b)| f.line <= b && a <= end),
        }
    }

    /// A digest of the changed lines in the files `reads` accepts, so a result is reused only
    /// when it was judged against the same lines.
    pub(crate) fn digest(&self, reads: impl Fn(&str) -> bool) -> String {
        let mut h = Sha256::new();
        for (path, lines) in self.files.iter().filter(|(p, _)| reads(p)) {
            h.update(path.as_bytes());
            h.update(format!("{lines:?}\0").as_bytes());
        }
        hex::encode(h.finalize())
    }
}

/// `-a,b +c,d @@` gives the added range `(c, d)`; a missing count means one line.
fn added_range(hunk: &str) -> Option<(u32, u32)> {
    let plus = hunk.split_whitespace().find(|p| p.starts_with('+'))?;
    let mut parts = plus[1..].splitn(2, ',');
    let start = parts.next()?.parse().ok()?;
    let count = match parts.next() {
        Some(c) => c.parse().ok()?,
        None => 1,
    };
    Some((start, count))
}

/// What a run's findings mean for the check.
pub(crate) struct Verdict {
    pub passed: bool,
    /// The findings in scope, worst first.
    pub counted: Vec<Finding>,
    /// Findings outside the scope, not shown.
    pub elsewhere: usize,
}

/// Judges a run: it fails on a gate finding in scope, or when the tool failed without
/// reporting any finding (a crash, not a lint).
pub(crate) fn judge(
    found: Vec<Finding>,
    scope: Scope,
    changed: &Changed,
    exit_ok: bool,
) -> Verdict {
    let any = !found.is_empty();
    // A tool may report one problem once per build target.
    let mut unique: Vec<Finding> = Vec::with_capacity(found.len());
    for f in found {
        if !unique.contains(&f) {
            unique.push(f);
        }
    }
    let (mut counted, other): (Vec<Finding>, Vec<Finding>) = unique
        .into_iter()
        .partition(|f| scope == Scope::Everywhere || f.anywhere || changed.touches(f));
    counted.sort_by(|a, b| (b.level, &a.file, a.line).cmp(&(a.level, &b.file, b.line)));
    let gated = counted.iter().any(|f| f.level == Level::Gate);
    Verdict {
        passed: !gated && (exit_ok || any),
        counted,
        elsewhere: other.len(),
    }
}

impl Level {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Level::Gate => "gate",
            Level::Flag => "flag",
            Level::Hint => "hint",
        }
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let level = self.level.name();
        let place = if self.file.is_empty() {
            "(no file)".to_string()
        } else if self.line == 0 {
            self.file.clone()
        } else {
            format!("{}:{}", self.file, self.line)
        };
        write!(f, "{level} {place}")?;
        if !self.rule.is_empty() {
            write!(f, "  {}", self.rule)?;
        }
        write!(f, "  {}", self.message)?;
        if let Some(fix) = &self.fix {
            write!(f, "\n    fix: {fix}")?;
        }
        if let Some(why) = &self.why {
            write!(f, "\n    why: {why}")?;
        }
        Ok(())
    }
}

/// Cargo's JSON messages. Lints (a code like `clippy::unwrap_used` or `unused_variables`)
/// count on changed lines; hard errors (`E0308`, or no code) count anywhere.
/// A gate finding with no file, counted wherever the change is.
fn unplaced(message: &str) -> Finding {
    Finding {
        file: String::new(),
        line: 0,
        end_line: None,
        level: Level::Gate,
        rule: String::new(),
        message: message.to_string(),
        fix: None,
        why: None,
        anywhere: true,
    }
}

/// Cargo names files from its workspace folder, which may be a subfolder of the repository:
/// the file is looked up from the crate's folder upwards, and made absolute when found.
fn cargo_path(file: &str, manifest: Option<&str>) -> String {
    if file.starts_with('/') {
        return file.to_string();
    }
    let Some(manifest) = manifest else {
        return file.to_string();
    };
    Path::new(manifest)
        .ancestors()
        .skip(1)
        .map(|dir| dir.join(file))
        .find(|p| p.is_file())
        .map_or_else(|| file.to_string(), |p| p.to_string_lossy().to_string())
}

fn cargo(text: &str) -> Vec<Finding> {
    let mut found = Vec::new();
    let mut build_failed = false;
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v["reason"] == "build-finished" && v["success"] == false {
            build_failed = true;
        }
        if v["reason"] != "compiler-message" {
            continue;
        }
        let m = &v["message"];
        let level = match m["level"].as_str() {
            Some("error") => Level::Gate,
            Some("warning") => Level::Flag,
            _ => continue,
        };
        let Some(span) = m["spans"]
            .as_array()
            .and_then(|s| s.iter().find(|s| s["is_primary"] == true))
        else {
            // An error without a place (a linker or dependency failure) still counts, except
            // the summaries like "aborting due to 2 previous errors".
            let message = m["message"].as_str().unwrap_or("");
            if level == Level::Gate && !message.starts_with("aborting due to") {
                found.push(unplaced(message));
            }
            continue;
        };
        let code = m["code"]["code"].as_str().unwrap_or("");
        let hard = code.is_empty()
            || (code.len() == 5
                && code.starts_with('E')
                && code[1..].bytes().all(|b| b.is_ascii_digit()));
        // Clippy's help is either a suggestion or a link to the lint's page.
        let helps: Vec<&Value> = m["children"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c["level"] == "help")
            .collect();
        let text = |c: &Value| c["message"].as_str().unwrap_or("").to_string();
        let link = |c: &Value| text(c).starts_with("for further information");
        let fix = helps.iter().find(|c| !link(c)).map(|c| {
            // A suggestion spread over several places reads wrong in one line; name it only.
            let parts: Vec<&str> = c["spans"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| s["suggested_replacement"].as_str())
                .collect();
            let replacement = (parts.len() == 1).then(|| parts[0]);
            match replacement {
                Some(r) if !r.is_empty() => format!("{}: `{r}`", text(c)),
                _ => text(c),
            }
        });
        let why = helps.iter().find(|c| link(c)).map(|c| {
            text(c)
                .trim_start_matches("for further information visit ")
                .to_string()
        });
        found.push(Finding {
            file: cargo_path(
                span["file_name"].as_str().unwrap_or(""),
                v["manifest_path"].as_str(),
            ),
            line: span["line_start"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(0),
            end_line: span["line_end"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok()),
            level,
            rule: code.to_string(),
            message: m["message"].as_str().unwrap_or("").to_string(),
            fix,
            why,
            anywhere: level == Level::Gate && hard,
        });
    }
    // A failed build script or a crash leaves no compiler message; it must not pass because
    // some unrelated warning exists.
    if build_failed && !found.iter().any(|f| f.level == Level::Gate) {
        found.push(unplaced(
            "cargo reported a failed build without saying where; see the log",
        ));
    }
    found
}

/// `SEVERITY|TYPE|CODE|FILE|LINE|COLUMN|LENGTH|MESSAGE` lines. Every issue blocks, as
/// `flutter analyze` fails on any; compile errors count anywhere.
fn dart(text: &str) -> Vec<Finding> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.splitn(8, '|').collect();
            let [severity, kind, code, file, line, _, _, message] = f[..] else {
                return None;
            };
            if !matches!(severity, "ERROR" | "WARNING" | "INFO") {
                return None;
            }
            Some(Finding {
                file: file.to_string(),
                line: line.parse().ok()?,
                end_line: None,
                level: Level::Gate,
                rule: code.to_lowercase(),
                message: message.to_string(),
                fix: None,
                why: None,
                anywhere: matches!(kind, "COMPILE_TIME_ERROR" | "SYNTACTIC_ERROR"),
            })
        })
        .collect()
}

/// ESLint's JSON: one entry per file with its messages. Severity 2 blocks; a message without
/// a rule is a parse error, which counts anywhere.
fn eslint(text: &str) -> (Vec<Finding>, Vec<String>) {
    if text.trim().is_empty() {
        return (Vec::new(), Vec::new());
    }
    let files: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => return (Vec::new(), vec![format!("ESLint JSON: {e}")]),
    };
    let mut found = Vec::new();
    for file in files.as_array().into_iter().flatten() {
        for m in file["messages"].as_array().into_iter().flatten() {
            let rule = m["ruleId"].as_str().unwrap_or("");
            found.push(Finding {
                file: file["filePath"].as_str().unwrap_or("").to_string(),
                line: m["line"]
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .unwrap_or(0),
                end_line: m["endLine"].as_u64().and_then(|n| u32::try_from(n).ok()),
                level: if m["severity"] == 2 {
                    Level::Gate
                } else {
                    Level::Flag
                },
                rule: rule.to_string(),
                message: m["message"].as_str().unwrap_or("").to_string(),
                fix: None,
                why: None,
                anywhere: rule.is_empty(),
            });
        }
    }
    (found, Vec::new())
}

/// A result's file. Some tools (ktlint) name it relative to a base folder the run declares.
fn sarif_file(run: &Value, location: &Value) -> String {
    let uri = location["uri"].as_str().unwrap_or("");
    let Some(base) = location["uriBaseId"]
        .as_str()
        .and_then(|id| run["originalUriBaseIds"][id]["uri"].as_str())
    else {
        return uri.to_string();
    };
    let base = base.strip_prefix("file://").unwrap_or(base);
    let mut parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    for part in uri.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    format!("/{}", parts.join("/"))
}

fn sarif(text: &str) -> (Vec<Finding>, Vec<String>) {
    if text.trim().is_empty() {
        return (Vec::new(), Vec::new());
    }
    let doc: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => return (Vec::new(), vec![format!("SARIF: {e}")]),
    };
    let mut found = Vec::new();
    for run in doc["runs"].as_array().into_iter().flatten() {
        for r in run["results"].as_array().into_iter().flatten() {
            let level = match r["level"].as_str().unwrap_or("warning") {
                "error" => Level::Gate,
                "warning" => Level::Flag,
                _ => Level::Hint,
            };
            let place = &r["locations"][0]["physicalLocation"];
            found.push(Finding {
                file: sarif_file(run, &place["artifactLocation"]),
                line: place["region"]["startLine"]
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .unwrap_or(0),
                end_line: place["region"]["endLine"]
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok()),
                level,
                rule: r["ruleId"].as_str().unwrap_or("").to_string(),
                message: r["message"]["text"].as_str().unwrap_or("").to_string(),
                fix: r["fixes"][0]["description"]["text"]
                    .as_str()
                    .map(String::from),
                why: None,
                anywhere: false,
            });
        }
    }
    (found, Vec::new())
}

fn jsonl(text: &str) -> (Vec<Finding>, Vec<String>) {
    let mut found = Vec::new();
    let mut errors = Vec::new();
    for (n, line) in text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
    {
        match serde_json::from_str(line) {
            Ok(f) => found.push(f),
            Err(e) => errors.push(format!("findings.jsonl line {}: {e}", n + 1)),
        }
    }
    (found, errors)
}

/// A tool's path made relative to the worktree root (`file://` URIs and absolute paths).
fn relative(file: &str, root: &Path) -> String {
    let decoded;
    let path = match file.strip_prefix("file://") {
        Some(uri) => {
            decoded = percent_decode(uri);
            decoded.as_str()
        }
        None => file,
    };
    let root = root.to_string_lossy();
    path.strip_prefix(&format!("{root}/"))
        .unwrap_or(path)
        .trim_start_matches("./")
        .to_string()
}

/// A URI path with `%XX` escapes (such as `%20` for a space) decoded.
fn percent_decode(uri: &str) -> String {
    let bytes = uri.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (b, hex) {
            (b'%', Some(v)) => {
                out.push(v);
                i += 3;
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_paths_are_found_from_the_crate_folder_upwards() {
        let dir = std::env::temp_dir().join(format!("fz-cargo-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("backend/app/src")).unwrap();
        std::fs::write(dir.join("backend/app/src/lib.rs"), "").unwrap();
        let manifest = dir.join("backend/app/Cargo.toml");
        let manifest = manifest.to_str();
        // From the workspace folder above the crate, and from the crate folder itself.
        let found = cargo_path("app/src/lib.rs", manifest);
        assert_eq!(found, dir.join("backend/app/src/lib.rs").to_string_lossy());
        assert_eq!(cargo_path("src/lib.rs", manifest), found);
        assert_eq!(cargo_path("missing.rs", manifest), "missing.rs");
        assert_eq!(cargo_path("/rustc/x.rs", manifest), "/rustc/x.rs");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sarif_uris_are_decoded() {
        assert_eq!(
            relative("file:///r/my%20dir/a.rs", Path::new("/r")),
            "my dir/a.rs"
        );
    }

    fn finding(file: &str, line: u32, level: Level) -> Finding {
        Finding {
            file: file.into(),
            line,
            end_line: None,
            level,
            rule: "r".into(),
            message: "m".into(),
            fix: None,
            why: None,
            anywhere: false,
        }
    }

    const DIFF: &str = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -3,0 +4,2 @@ fn x\n+one\n+two\n@@ -9 +11 @@\n-old\n+new\n@@ -20,3 +22,0 @@\n-gone\n";

    #[test]
    fn only_added_and_modified_lines_count() {
        let changed = Changed::from_diff(DIFF, &["src/new.rs".into()], &BTreeSet::new());
        let touched = |file: &str, line| changed.touches(&finding(file, line, Level::Gate));
        assert!(touched("src/a.rs", 4) && touched("src/a.rs", 5) && touched("src/a.rs", 11));
        assert!(!touched("src/a.rs", 6) && !touched("src/a.rs", 22));
        assert!(!touched("src/other.rs", 4));
        assert!(touched("src/new.rs", 900));
    }

    #[test]
    fn a_gate_on_old_lines_does_not_fail_but_a_hard_error_does() {
        let changed = Changed::from_diff(DIFF, &[], &BTreeSet::new());
        let old = vec![finding("src/a.rs", 30, Level::Gate)];
        let v = judge(old.clone(), Scope::Changed, &changed, false);
        assert!(v.passed);
        assert_eq!((v.counted.len(), v.elsewhere), (0, 1));
        assert!(!judge(old, Scope::Everywhere, &changed, false).passed);

        let mut broken_caller = finding("src/a.rs", 30, Level::Gate);
        broken_caller.anywhere = true;
        assert!(!judge(vec![broken_caller], Scope::Changed, &changed, false).passed);
    }

    #[test]
    fn a_failing_tool_without_findings_fails() {
        let changed = Changed::default();
        assert!(!judge(Vec::new(), Scope::Changed, &changed, false).passed);
        assert!(judge(Vec::new(), Scope::Changed, &changed, true).passed);
    }

    #[test]
    fn cargo_lints_count_on_changed_lines_and_errors_anywhere() {
        let lint = r#"{"reason":"compiler-message","message":{"level":"error","message":"used `unwrap()` on an `Option` value","code":{"code":"clippy::unwrap_used"},"spans":[{"file_name":"src/a.rs","line_start":4,"line_end":4,"is_primary":true}],"children":[{"level":"help","message":"for further information visit https://lints/unwrap"},{"level":"help","message":"consider using `expect()`"}]}}"#;
        let hard = r#"{"reason":"compiler-message","message":{"level":"error","message":"mismatched types","code":{"code":"E0308"},"spans":[{"file_name":"src/b.rs","line_start":9,"line_end":9,"is_primary":true}],"children":[]}}"#;
        let summary = r#"{"reason":"compiler-message","message":{"level":"error","message":"aborting due to 2 previous errors","code":null,"spans":[],"children":[]}}"#;
        let found = cargo(&format!("{lint}\nCompiling x\n{hard}\n{summary}\n"));
        assert_eq!(found.len(), 2);
        let twice = judge(
            cargo(&format!("{lint}\n{lint}\n")),
            Scope::Everywhere,
            &Changed::default(),
            false,
        );
        assert_eq!(twice.counted.len(), 1);
        assert_eq!(
            (found[0].rule.as_str(), found[0].anywhere),
            ("clippy::unwrap_used", false)
        );
        assert_eq!(found[0].fix.as_deref(), Some("consider using `expect()`"));
        assert_eq!(found[0].why.as_deref(), Some("https://lints/unwrap"));
        assert!(found[1].anywhere);
    }

    #[test]
    fn a_failed_cargo_build_without_a_placed_error_still_fails() {
        let old_warning = r#"{"reason":"compiler-message","message":{"level":"warning","message":"unused","code":{"code":"unused_variables"},"spans":[{"file_name":"src/a.rs","line_start":30,"line_end":30,"is_primary":true}],"children":[]}}"#;
        let linker = r#"{"reason":"compiler-message","message":{"level":"error","message":"linking with `cc` failed","code":null,"spans":[],"children":[]}}"#;
        let failed = r#"{"reason":"build-finished","success":false}"#;
        let changed = Changed::from_diff(DIFF, &[], &BTreeSet::new());
        let script = cargo(&format!("{old_warning}\n{failed}\n"));
        assert!(!judge(script, Scope::Changed, &changed, false).passed);
        let link = cargo(&format!("{old_warning}\n{linker}\n{failed}\n"));
        assert_eq!(link.iter().filter(|f| f.level == Level::Gate).count(), 1);
        assert!(!judge(link, Scope::Changed, &changed, false).passed);
    }

    #[test]
    fn a_changed_path_the_diff_text_misses_counts_everywhere() {
        let all: BTreeSet<String> = ["odd \"name\".rs".to_string()].into();
        let changed = Changed::from_diff("", &[], &all);
        assert!(changed.touches(&finding("odd \"name\".rs", 7, Level::Gate)));
        let spaced = "--- a/my file.rs\t\n+++ b/my file.rs\t\n@@ -1 +1 @@\n-a\n+b\n";
        let changed = Changed::from_diff(spaced, &[], &BTreeSet::new());
        assert!(changed.touches(&finding("my file.rs", 1, Level::Gate)));
    }

    #[test]
    fn dart_issues_block_and_compile_errors_count_anywhere() {
        let out = "ERROR|COMPILE_TIME_ERROR|INVALID_ASSIGNMENT|/r/lib/a.dart|3|11|3|A value of type 'String' can't be assigned.\nINFO|LINT|PREFER_DOUBLE_QUOTES|/r/lib/a.dart|2|20|3|Unnecessary use of single quotes.\nAnalyzing try...\n";
        let found = dart(out);
        assert_eq!(found.len(), 2);
        assert!(found[0].anywhere && !found[1].anywhere);
        assert_eq!(
            (found[1].rule.as_str(), found[1].line),
            ("prefer_double_quotes", 2)
        );
        assert_eq!(relative(&found[1].file, Path::new("/r")), "lib/a.dart");
    }

    #[test]
    fn sarif_results_become_findings() {
        let doc = r#"{"runs":[{"results":[{"ruleId":"no-eval","level":"error","message":{"text":"eval is evil"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"file:///repo/web/a.js"},"region":{"startLine":7}}}]}]}]}"#;
        let (found, errors) = sarif(doc);
        assert!(errors.is_empty(), "{errors:?}");
        let f = &found[0];
        assert_eq!(relative(&f.file, Path::new("/repo")), "web/a.js");
        assert_eq!(
            (f.line, f.level, f.rule.as_str()),
            (7, Level::Gate, "no-eval")
        );
    }

    #[test]
    fn sarif_paths_resolve_against_the_base_the_run_declares() {
        // ktlint names files relative to the home folder.
        let doc = r#"{"runs":[{"originalUriBaseIds":{"%SRCROOT%":{"uri":"file:///Users/me/"}},"results":[{"ruleId":"standard:filename","level":"error","message":{"text":"x"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"code/app/../app/src/A.kt","uriBaseId":"%SRCROOT%"},"region":{"startLine":1}}}]}]}]}"#;
        let (found, _) = sarif(doc);
        assert_eq!(
            relative(&found[0].file, Path::new("/Users/me/code/app")),
            "src/A.kt"
        );
    }

    #[test]
    fn eslint_messages_become_findings() {
        let doc = r#"[{"filePath":"/repo/src/a.ts","messages":[
            {"ruleId":"eqeqeq","severity":2,"message":"use ===","line":3,"endLine":3},
            {"ruleId":"no-console","severity":1,"message":"console","line":5},
            {"ruleId":null,"severity":2,"message":"Parsing error","line":9}]}]"#;
        let (found, errors) = eslint(doc);
        assert!(errors.is_empty(), "{errors:?}");
        let got: Vec<_> = found
            .iter()
            .map(|f| (f.rule.as_str(), f.line, f.level, f.anywhere))
            .collect();
        assert_eq!(
            got,
            vec![
                ("eqeqeq", 3, Level::Gate, false),
                ("no-console", 5, Level::Flag, false),
                // A parse error has no rule and fails wherever it is.
                ("", 9, Level::Gate, true),
            ]
        );
        assert_eq!(relative(&found[0].file, Path::new("/repo")), "src/a.ts");
    }
}
