//! Rules: project conventions that tests cannot express, written as `[rule.<name>]` in
//! `fearlezz.toml` or a piece. Together they form the built in `rules` check, whose findings
//! are judged like any other (only lines the change touched count, each at its rule's level).
//!
//! A rule is one of:
//! - `changed = ["engine/**"]`: the change may not touch these paths (deleting included);
//! - `name = "snake_case"`: files matching `files` are named in this style, or match this
//!   regex when it is not a style Fearlezz knows;
//! - `needs = "test/{stem}_test.dart"`: every file matching `files` has this companion file;
//! - one table per language (`[rule.<name>.rust]`) with `pattern`, `any` or an ast-grep
//!   `rule`: code that must not appear, matched by the `ast-grep` tool.
//!
//! `[[rule.<name>.test]]` entries give a bad and a good example per language; a bad example
//! the rule misses, or a good one it matches, is a gate finding of the rule itself.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use regex::Regex;
use serde::Deserialize;
use serde_json::{Value as Json, json};

use crate::findings::{Changed, Finding, Level};
use crate::lints;

/// The id of the check the rules form.
pub(crate) const CHECK: &str = "rules";

#[derive(Clone, Debug, Default)]
pub(crate) struct Rules(Vec<Rule>);

#[derive(Clone, Debug)]
struct Rule {
    id: String,
    level: Level,
    message: Option<String>,
    fix: Option<String>,
    why: Option<String>,
    files: Patterns,
    skip: Patterns,
    kind: Kind,
    tests: Vec<BTreeMap<String, Example>>,
}

#[derive(Clone, Debug)]
enum Kind {
    Changed(Patterns),
    Name(Style),
    Needs(String),
    /// Per language, the ast-grep fields (`rule`, and `utils` or `constraints` when given).
    Code(BTreeMap<String, Json>),
}

#[derive(Clone, Debug)]
enum Style {
    Snake,
    Kebab,
    Pascal,
    Camel,
    Regex(Regex),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Example {
    #[serde(default)]
    bad: Option<String>,
    #[serde(default)]
    good: Option<String>,
}

/// Globs as written (for the fingerprint) and compiled.
#[derive(Clone, Debug, Default)]
struct Patterns {
    text: Vec<String>,
    set: GlobSet,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn into_vec(self) -> Vec<String> {
        match self {
            OneOrMany::One(s) => vec![s],
            OneOrMany::Many(v) => v,
        }
    }
}

/// The fields every rule shares; the rest of the table names its kind.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    level: lints::Level,
    message: Option<String>,
    fix: Option<String>,
    why: Option<String>,
    files: Option<OneOrMany>,
    #[serde(default)]
    skip: Vec<String>,
    changed: Option<OneOrMany>,
    name: Option<String>,
    needs: Option<String>,
    #[serde(default)]
    test: Vec<BTreeMap<String, Example>>,
    #[serde(default)]
    #[expect(
        dead_code,
        reason = "off rules are dropped before parsing; the field only lets serde accept the key"
    )]
    off: bool,
}

/// `pattern`, `any` (patterns, one must match) or `rule` (ast-grep's own rule object, with
/// its `utils` and `constraints` beside it).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMatch {
    pattern: Option<String>,
    any: Option<Vec<String>>,
    rule: Option<toml::Value>,
    utils: Option<toml::Value>,
    constraints: Option<toml::Value>,
}

impl Rules {
    /// From the `[rule]` table of the merged config. Rules turned off, or set to level
    /// `off`, are left out.
    pub(crate) fn parse(table: BTreeMap<String, toml::Table>) -> Result<Rules> {
        let mut rules = Vec::new();
        for (id, mut fields) in table {
            if fields.get("off").and_then(toml::Value::as_bool) == Some(true) {
                continue;
            }
            // Language tables name the code a rule matches; everything else is a field.
            let mut code = BTreeMap::new();
            for key in fields.keys().cloned().collect::<Vec<_>>() {
                if fields.get(&key).is_some_and(toml::Value::is_table)
                    && let Some(toml::Value::Table(t)) = fields.remove(&key)
                {
                    code.insert(key, t);
                }
            }
            let rule = Rule::parse(&id, fields, code).with_context(|| format!("rule {id}"))?;
            if let Some(rule) = rule {
                rules.push(rule);
            }
        }
        Ok(Rules(rules))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// How `head` enforces less than these rules: a rule gone or off, at a lower level,
    /// covering fewer of `files`, or checking something else. Each as `rule.<id>` and how.
    pub(crate) fn weakenings(&self, head: &Rules, files: &[String]) -> Vec<(String, String)> {
        let mut found = Vec::new();
        for b in &self.0 {
            let mut say = |how: String| found.push((format!("rule.{}", b.id), how));
            let Some(h) = head.0.iter().find(|h| h.id == b.id) else {
                say("it is gone".into());
                continue;
            };
            if h.level < b.level {
                say(format!(
                    "its level goes from {} to {}",
                    b.level.name(),
                    h.level.name()
                ));
            }
            let lost: Vec<&String> = files
                .iter()
                .filter(|f| b.covers(f) && !h.covers(f))
                .collect();
            if let Some(first) = lost.first() {
                say(match lost.len() {
                    1 => format!("it no longer covers {first}"),
                    n => format!("it no longer covers {first} and {} more files", n - 1),
                });
            }
            if !b.kind.same(&h.kind) {
                say("it checks something else".into());
            }
        }
        found
    }

    /// A stable text of every rule, part of the check's inputs.
    pub(crate) fn fingerprint(&self) -> String {
        format!("{:?}", self.0)
    }

    /// The findings of every rule over `files` (the worktree's files) at `root`.
    pub(crate) fn check(
        &self,
        root: &Path,
        files: &[String],
        changed: &Changed,
    ) -> Result<Vec<Finding>> {
        let mut found = Vec::new();
        for rule in &self.0 {
            match &rule.kind {
                Kind::Changed(paths) => {
                    for path in changed.paths().filter(|p| paths.set.is_match(p)) {
                        found.push(rule.finding(path, 1, None, true, || {
                            format!("this change touches {path}, which {} keeps as is", rule.id)
                        }));
                    }
                }
                Kind::Name(style) => {
                    for path in files.iter().filter(|p| rule.covers(p)) {
                        let stem = stem(path);
                        if !style.allows(stem) {
                            found.push(rule.finding(path, 1, None, false, || {
                                format!("{stem} is not {style}")
                            }));
                        }
                    }
                }
                Kind::Needs(template) => {
                    for path in files.iter().filter(|p| rule.covers(p)) {
                        let target = fill(template, path);
                        if !root.join(&target).is_file() {
                            // Deleting the companion is a change to judge even though the
                            // file that needs it did not change.
                            let deleted = changed.paths().any(|p| p == target);
                            found.push(rule.finding(path, 1, None, deleted, || {
                                format!("{path} needs {target}")
                            }));
                        }
                    }
                }
                Kind::Code(_) => {}
            }
        }
        found.extend(self.code(root)?);
        found.extend(self.tests()?);
        Ok(found)
    }

    /// Runs every code rule through one `ast-grep scan` of the worktree.
    fn code(&self, root: &Path) -> Result<Vec<Finding>> {
        let mut docs = Vec::new();
        // The ast-grep id of each rule and language, mapped back exactly.
        let mut by_id: BTreeMap<String, &Rule> = BTreeMap::new();
        for rule in &self.0 {
            if let Kind::Code(langs) = &rule.kind {
                for (lang, matcher) in langs {
                    let id = format!("{}.{lang}", rule.id);
                    docs.push(sg_rule(&id, lang, matcher));
                    by_id.insert(id, rule);
                }
            }
        }
        if docs.is_empty() {
            return Ok(Vec::new());
        }
        let mut found = Vec::new();
        for m in ast_grep(root, &docs, None)? {
            let Some(rule) = by_id.get(&m.rule) else {
                continue;
            };
            if rule.covers(&m.file) {
                found.push(rule.finding(&m.file, m.line, Some(m.end_line), false, || {
                    format!("matches rule {}", rule.id)
                }));
            }
        }
        Ok(found)
    }

    /// Checks each rule's examples: a bad one must match, a good one must not.
    fn tests(&self) -> Result<Vec<Finding>> {
        let mut found = Vec::new();
        for rule in &self.0 {
            for (n, test) in rule.tests.iter().enumerate() {
                for (lang, example) in test {
                    let Kind::Code(langs) = &rule.kind else {
                        bail!("rule {}: only code rules have tests", rule.id);
                    };
                    let matcher = langs.get(lang).with_context(|| {
                        format!(
                            "rule {}: test {} is in {lang}, which it does not match",
                            rule.id,
                            n + 1
                        )
                    })?;
                    let doc = sg_rule(&rule.id, lang, matcher);
                    for (code, should_match, word) in [
                        (&example.bad, true, "misses its bad"),
                        (&example.good, false, "matches its good"),
                    ] {
                        let Some(code) = code else { continue };
                        let matched =
                            !ast_grep(Path::new("."), std::slice::from_ref(&doc), Some(code))?
                                .is_empty();
                        if matched != should_match {
                            found.push(Finding {
                                file: String::new(),
                                line: 0,
                                end_line: None,
                                level: Level::Gate,
                                rule: rule.id.clone(),
                                message: format!(
                                    "rule {} {word} {lang} example {}: {code}",
                                    rule.id,
                                    n + 1
                                ),
                                fix: Some("fix the rule or its example".into()),
                                why: None,
                                anywhere: true,
                            });
                        }
                    }
                }
            }
        }
        Ok(found)
    }
}

impl Rule {
    fn parse(
        id: &str,
        fields: toml::Table,
        code: BTreeMap<String, toml::Table>,
    ) -> Result<Option<Rule>> {
        let raw: RawRule = toml::Value::Table(fields).try_into()?;
        let Some(level) = raw.level.finding() else {
            return Ok(None);
        };
        let files = raw.files.map(OneOrMany::into_vec).unwrap_or_default();
        let mut kinds = Vec::new();
        if let Some(paths) = raw.changed {
            kinds.push(Kind::Changed(Patterns::new(paths.into_vec())?));
        }
        if let Some(name) = raw.name {
            kinds.push(Kind::Name(Style::parse(&name)?));
        }
        if let Some(needs) = raw.needs {
            kinds.push(Kind::Needs(needs));
        }
        if !code.is_empty() {
            let mut langs = BTreeMap::new();
            for (lang, table) in code {
                let m: RawMatch = toml::Value::Table(table)
                    .try_into()
                    .with_context(|| format!("[rule.{id}.{lang}]"))?;
                let rule = match (m.pattern, m.any, m.rule) {
                    (Some(p), None, None) => json!({ "pattern": p }),
                    (None, Some(any), None) => {
                        json!({ "any": any.into_iter().map(|p| json!({ "pattern": p })).collect::<Vec<_>>() })
                    }
                    (None, None, Some(rule)) => serde_json::to_value(rule)?,
                    _ => bail!("[rule.{id}.{lang}] needs exactly one of pattern, any or rule"),
                };
                let mut fields = serde_json::Map::new();
                fields.insert("rule".into(), rule);
                for (key, value) in [("utils", m.utils), ("constraints", m.constraints)] {
                    if let Some(v) = value {
                        fields.insert(key.into(), serde_json::to_value(v)?);
                    }
                }
                langs.insert(lang, Json::Object(fields));
            }
            kinds.push(Kind::Code(langs));
        }
        let kind = match kinds.len() {
            1 => kinds.remove(0),
            0 => bail!(
                "say what it checks: changed = [paths], name = \"snake_case\", \
                 needs = \"path\", or a language table like [rule.{id}.rust]"
            ),
            _ => bail!(
                "give it one of changed, name, needs or language tables; split it into two rules"
            ),
        };
        if matches!(kind, Kind::Name(_) | Kind::Needs(_)) && files.is_empty() {
            bail!("name and needs rules say which files they cover: files = [\"src/**/*.rs\"]");
        }
        Ok(Some(Rule {
            id: id.to_string(),
            level,
            message: raw.message,
            fix: raw.fix,
            why: raw.why,
            files: Patterns::new(files)?,
            skip: Patterns::new(raw.skip)?,
            kind,
            tests: raw.test,
        }))
    }

    /// Whether `path` is in the rule's files (all files when it names none) and not skipped.
    fn covers(&self, path: &str) -> bool {
        (self.files.text.is_empty() || self.files.set.is_match(path))
            && !self.skip.set.is_match(path)
    }

    fn finding(
        &self,
        file: &str,
        line: u32,
        end_line: Option<u32>,
        anywhere: bool,
        default: impl FnOnce() -> String,
    ) -> Finding {
        Finding {
            file: file.to_string(),
            line,
            end_line: end_line.filter(|&e| e != line),
            level: self.level,
            rule: self.id.clone(),
            message: self.message.clone().unwrap_or_else(default),
            fix: self.fix.clone(),
            why: self.why.clone(),
            anywhere,
        }
    }
}

impl Kind {
    /// Whether both check the same thing, as written.
    fn same(&self, other: &Kind) -> bool {
        match (self, other) {
            (Kind::Changed(a), Kind::Changed(b)) => a.text == b.text,
            (Kind::Name(a), Kind::Name(b)) => a.to_string() == b.to_string(),
            (Kind::Needs(a), Kind::Needs(b)) => a == b,
            (Kind::Code(a), Kind::Code(b)) => a == b,
            _ => false,
        }
    }
}

impl Patterns {
    fn new(text: Vec<String>) -> Result<Patterns> {
        let mut set = GlobSetBuilder::new();
        for p in &text {
            set.add(GlobBuilder::new(p).literal_separator(true).build()?);
        }
        Ok(Patterns {
            set: set.build()?,
            text,
        })
    }
}

impl Style {
    fn parse(text: &str) -> Result<Style> {
        Ok(match text {
            "snake_case" => Style::Snake,
            "kebab-case" => Style::Kebab,
            "PascalCase" => Style::Pascal,
            "camelCase" => Style::Camel,
            regex => Style::Regex(Regex::new(&format!("^(?:{regex})$")).with_context(|| {
                format!(
                    "name \"{regex}\" is not snake_case, kebab-case, PascalCase or \
                         camelCase, nor a valid regex"
                )
            })?),
        })
    }

    fn allows(&self, name: &str) -> bool {
        let lower = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
        let alnum = |c: char| c.is_ascii_alphanumeric();
        let first = name.chars().next();
        match self {
            Style::Snake => {
                first.is_some_and(|c| c.is_ascii_lowercase())
                    && name
                        .split('_')
                        .all(|w| !w.is_empty() && w.chars().all(lower))
            }
            Style::Kebab => {
                first.is_some_and(|c| c.is_ascii_lowercase())
                    && name
                        .split('-')
                        .all(|w| !w.is_empty() && w.chars().all(lower))
            }
            Style::Pascal => {
                first.is_some_and(|c| c.is_ascii_uppercase()) && name.chars().all(alnum)
            }
            Style::Camel => {
                first.is_some_and(|c| c.is_ascii_lowercase()) && name.chars().all(alnum)
            }
            Style::Regex(re) => re.is_match(name),
        }
    }
}

impl std::fmt::Display for Style {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Style::Snake => f.write_str("snake_case"),
            Style::Kebab => f.write_str("kebab-case"),
            Style::Pascal => f.write_str("PascalCase"),
            Style::Camel => f.write_str("camelCase"),
            Style::Regex(re) => write!(f, "named like {}", re.as_str()),
        }
    }
}

/// The file name up to its first dot: `widget.test.dart` gives `widget`.
fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.split('.').next().unwrap_or(name)
}

/// Fills `{stem}`, `{name}` and `{dir}` in a `needs` template from `path`.
fn fill(template: &str, path: &str) -> String {
    let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
    template
        .replace("{stem}", stem(path))
        .replace("{name}", name)
        .replace("{dir}", dir)
}

/// One ast-grep rule document. Severity `info` keeps ast-grep's exit code clean; the level
/// comes from the Fearlezz rule.
fn sg_rule(id: &str, lang: &str, fields: &Json) -> String {
    let mut doc = json!({ "id": id, "language": lang, "severity": "info", "message": id });
    if let (Some(doc), Some(fields)) = (doc.as_object_mut(), fields.as_object()) {
        doc.extend(fields.clone());
    }
    doc.to_string()
}

struct Match {
    rule: String,
    file: String,
    line: u32,
    end_line: u32,
}

fn text(v: &Json, key: &str) -> String {
    v.get(key)
        .and_then(Json::as_str)
        .unwrap_or_default()
        .to_string()
}

/// Runs `ast-grep scan` with `docs` over `root`, or over `stdin` when given.
fn ast_grep(root: &Path, docs: &[String], stdin: Option<&str>) -> Result<Vec<Match>> {
    let mut cmd = Command::new("ast-grep");
    cmd.current_dir(root)
        .args(["scan", "--json=stream", "--inline-rules"])
        .arg(docs.join("\n---\n"))
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if stdin.is_some() {
        cmd.arg("--stdin");
    } else {
        cmd.arg(".");
    }
    let mut child = cmd.spawn().map_err(|e| {
        anyhow::anyhow!(
            "code rules run the ast-grep tool, which did not start ({e}); add pkgs.ast-grep to \
             devenv.nix"
        )
    })?;
    if let (Some(code), Some(mut input)) = (stdin, child.stdin.take()) {
        input.write_all(code.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if err.is_empty() {
            bail!("ast-grep failed with exit {:?}", out.status.code());
        }
        bail!("ast-grep failed: {err}");
    }
    let mut matches = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let v: Json = serde_json::from_str(line).context("reading ast-grep's output")?;
        let line_at = |key: &str| {
            v.pointer(&format!("/range/{key}/line"))
                .and_then(Json::as_u64)
                .and_then(|l| u32::try_from(l + 1).ok())
                .unwrap_or(1)
        };
        matches.push(Match {
            rule: text(&v, "ruleId"),
            file: text(&v, "file"),
            line: line_at("start"),
            end_line: line_at("end"),
        });
    }
    Ok(matches)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_accept_their_own_names_only() {
        let ok = |s: &str, name: &str| Style::parse(s).unwrap().allows(name);
        assert!(ok("snake_case", "user_profile2"));
        assert!(!ok("snake_case", "userProfile"));
        assert!(ok("kebab-case", "user-profile"));
        assert!(!ok("kebab-case", "user_profile"));
        assert!(ok("PascalCase", "UserProfile"));
        assert!(!ok("PascalCase", "userProfile"));
        assert!(ok("camelCase", "userProfile"));
        assert!(ok("[a-z]+_screen", "home_screen"));
        assert!(!ok("[a-z]+_screen", "home_screen_old"));
    }

    #[test]
    fn needs_templates_fill_from_the_path() {
        assert_eq!(
            fill("test/{dir}/{stem}_test.dart", "lib/widgets/button.g.dart"),
            "test/lib/widgets/button_test.dart"
        );
    }
}
