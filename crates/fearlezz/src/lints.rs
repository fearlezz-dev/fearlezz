//! Lint levels: one table per tool, `[lints.<tool>]`, saying for each rule what a finding of
//! it means. The meaning is the same for every tool:
//!
//! - `gate`: this must never happen; the check fails.
//! - `flag`: there is little or no reason for it; shown, and fixed unless there is a reason.
//! - `hint`: there can be a good reason; an agent looks at it again.
//! - `off`: the rule does not run.
//!
//! Fearlezz turns the rules on or off in each tool's own file (see `settings`) and gives
//! every finding its level from this table by rule name, so a tool with fewer levels (or
//! none) still reports all four.

use std::collections::BTreeMap;

use anyhow::{Result, bail};
use serde::Deserialize;

use crate::findings::{self, Finding};

/// What a finding of one rule means.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Level {
    Off,
    Hint,
    Flag,
    Gate,
}

impl Level {
    /// The finding level, or `None` for a rule that is off.
    pub(crate) fn finding(self) -> Option<findings::Level> {
        match self {
            Level::Off => None,
            Level::Hint => Some(findings::Level::Hint),
            Level::Flag => Some(findings::Level::Flag),
            Level::Gate => Some(findings::Level::Gate),
        }
    }

    pub(crate) fn is_on(self) -> bool {
        self != Level::Off
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Level::Off => "off",
            Level::Hint => "hint",
            Level::Flag => "flag",
            Level::Gate => "gate",
        }
    }
}

/// A linter whose rules Fearlezz knows how to name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Tool {
    Clippy,
    Rustc,
    Dart,
    /// Rules named `group/rule`, as `linter.rules` in biome.json groups them.
    Biome,
    /// Core rules by name, plugin rules with their prefix (`@typescript-eslint/no-explicit-any`).
    Eslint,
    /// Rule codes such as `F401`.
    Ruff,
    /// Rule ids such as `standard:no-wildcard-imports`.
    Ktlint,
    /// Rules named `ruleset/Rule`, such as `style/MagicNumber`.
    Detekt,
    /// One row per linter, such as `errcheck`.
    Golangci,
}

const TOOLS: [(&str, Tool); 9] = [
    ("clippy", Tool::Clippy),
    ("rustc", Tool::Rustc),
    ("dart", Tool::Dart),
    ("biome", Tool::Biome),
    ("eslint", Tool::Eslint),
    ("ruff", Tool::Ruff),
    ("ktlint", Tool::Ktlint),
    ("detekt", Tool::Detekt),
    ("golangci", Tool::Golangci),
];

impl Tool {
    pub(crate) fn parse(name: &str) -> Result<Tool> {
        if let Some((_, t)) = TOOLS.iter().find(|(n, _)| *n == name) {
            Ok(*t)
        } else {
            let names: Vec<&str> = TOOLS.iter().map(|(n, _)| *n).collect();
            bail!("no lint tool \"{name}\"; tools are {}", names.join(", "))
        }
    }

    pub(crate) fn name(self) -> &'static str {
        TOOLS
            .iter()
            .find(|(_, t)| *t == self)
            .map_or("", |(n, _)| *n)
    }

    /// The rule's name in this tool's table, when a reported rule id belongs to this tool.
    fn rule(self, reported: &str) -> Option<String> {
        let name = match self {
            Tool::Clippy => reported.strip_prefix("clippy::")?.to_string(),
            Tool::Rustc if reported.is_empty() || reported.contains("::") => return None,
            // Biome reports `lint/style/useConst`.
            Tool::Biome => reported
                .strip_prefix("lint/")
                .unwrap_or(reported)
                .to_string(),
            // detekt reports `detekt.style.MagicNumber`.
            Tool::Detekt => reported
                .strip_prefix("detekt.")
                .unwrap_or(reported)
                .replacen('.', "/", 1),
            Tool::Rustc
            | Tool::Dart
            | Tool::Eslint
            | Tool::Ruff
            | Tool::Ktlint
            | Tool::Golangci => reported.to_string(),
        };
        Some(name)
    }
}

/// Every tool's table, after merging pieces and overrides.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Lints(BTreeMap<Tool, BTreeMap<String, Level>>);

impl Lints {
    pub(crate) fn parse(raw: BTreeMap<String, BTreeMap<String, Level>>) -> Result<Lints> {
        let mut tools = BTreeMap::new();
        for (name, rules) in raw {
            let tool = Tool::parse(&name).map_err(|e| anyhow::anyhow!("[lints.{name}]: {e}"))?;
            // Their files list each rule under its group, so a bare name would turn nothing on.
            let example = match tool {
                Tool::Biome => Some("style/useConst"),
                Tool::Detekt => Some("style/MagicNumber"),
                Tool::Clippy
                | Tool::Rustc
                | Tool::Dart
                | Tool::Eslint
                | Tool::Ruff
                | Tool::Ktlint
                | Tool::Golangci => None,
            };
            if let Some(example) = example
                && let Some(bare) = rules.keys().find(|r| !r.contains('/'))
            {
                bail!("[lints.{name}] {bare}: name the rule with its group, such as {example}");
            }
            tools.insert(tool, rules);
        }
        Ok(Lints(tools))
    }

    /// Only the tables of `tools`.
    pub(crate) fn only(&self, tools: &[Tool]) -> Lints {
        Lints(
            self.0
                .iter()
                .filter(|(t, _)| tools.contains(t))
                .map(|(t, r)| (*t, r.clone()))
                .collect(),
        )
    }

    /// The rules of one tool, by name.
    pub(crate) fn of(&self, tool: Tool) -> Option<&BTreeMap<String, Level>> {
        self.0.get(&tool)
    }

    /// How `head` judges the rules of `tool` more mildly than these tables: a rule at a lower
    /// level, or no longer set. Each as `lints.<tool>.<rule>` and how it weakened.
    pub(crate) fn weakenings(&self, head: &Lints, tool: Tool) -> Vec<(String, String)> {
        let Some(base) = self.of(tool) else {
            return Vec::new();
        };
        let head = head.of(tool);
        base.iter()
            .filter_map(|(rule, &was)| {
                let how = match head.and_then(|h| h.get(rule)) {
                    None => "it is no longer set".to_string(),
                    Some(&now) if now < was => {
                        format!("it goes from {} to {}", was.name(), now.name())
                    }
                    Some(_) => return None,
                };
                Some((format!("lints.{}.{rule}", tool.name()), how))
            })
            .collect()
    }

    /// A stable text of the tables, part of the inputs of checks that read them.
    pub(crate) fn fingerprint(&self, tools: &[Tool]) -> String {
        let mut out = String::new();
        for t in tools {
            out = out + t.name() + &format!("{:?}\0", self.0.get(t));
        }
        out
    }

    /// Gives each finding the level its rule has in the first of `tools` that names it, and
    /// drops findings of rules that are off. Findings of rules no table names keep the level
    /// the tool gave them.
    pub(crate) fn apply(&self, tools: &[Tool], found: Vec<Finding>) -> Vec<Finding> {
        found
            .into_iter()
            .filter_map(|mut f| {
                let level = tools.iter().find_map(|t| {
                    let name = t.rule(&f.rule)?;
                    self.0.get(t)?.get(&name).copied()
                });
                match level {
                    None => Some(f),
                    Some(l) => {
                        f.level = l.finding()?;
                        Some(f)
                    }
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(rule: &str) -> Finding {
        Finding {
            file: "src/a.rs".into(),
            line: 1,
            end_line: None,
            level: findings::Level::Flag,
            rule: rule.into(),
            message: String::new(),
            fix: None,
            why: None,
            anywhere: false,
        }
    }

    #[test]
    fn findings_take_the_table_level_and_off_rules_vanish() {
        let raw = toml::from_str(
            "[clippy]\nwildcard_imports = \"hint\"\nneedless_continue = \"off\"\n[rustc]\nmissing_docs = \"gate\"\n",
        )
        .unwrap();
        let lints = Lints::parse(raw).unwrap();
        let found = vec![
            finding("clippy::wildcard_imports"),
            finding("clippy::needless_continue"),
            finding("missing_docs"),
            finding("clippy::not_in_the_table"),
            finding("E0308"),
        ];
        let levels: Vec<(String, findings::Level)> = lints
            .apply(&[Tool::Clippy, Tool::Rustc], found)
            .into_iter()
            .map(|f| (f.rule, f.level))
            .collect();
        assert_eq!(
            levels,
            vec![
                ("clippy::wildcard_imports".into(), findings::Level::Hint),
                ("missing_docs".into(), findings::Level::Gate),
                ("clippy::not_in_the_table".into(), findings::Level::Flag),
                ("E0308".into(), findings::Level::Flag),
            ]
        );
    }

    #[test]
    fn an_unknown_tool_says_which_exist() {
        let raw = toml::from_str("[clipy]\nx = \"gate\"\n").unwrap();
        let e = Lints::parse(raw).unwrap_err().to_string();
        assert!(e.contains("tools are clippy, rustc, dart, biome"), "{e}");
    }

    #[test]
    fn reported_rule_ids_map_to_table_names() {
        assert_eq!(
            Tool::Biome.rule("lint/suspicious/noDebugger").as_deref(),
            Some("suspicious/noDebugger")
        );
        assert_eq!(
            Tool::Detekt
                .rule("detekt.potential-bugs.UnsafeCast")
                .as_deref(),
            Some("potential-bugs/UnsafeCast")
        );
        assert_eq!(Tool::Rustc.rule("clippy::x"), None);
        assert_eq!(Tool::Clippy.rule("dead_code"), None);
    }
}
