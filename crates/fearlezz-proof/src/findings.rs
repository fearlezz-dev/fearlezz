//! Findings: problems a check reports at a file and line, like a linter's warnings.

use std::fmt;

use serde::{Deserialize, Serialize};

/// How much a finding matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Told to the agent only.
    Hint,
    /// Shown in status and the report; does not block.
    Flag,
    /// Fails the check.
    Gate,
}

impl Level {
    /// The level as config and output name it.
    pub fn name(self) -> &'static str {
        match self {
            Level::Gate => "gate",
            Level::Flag => "flag",
            Level::Hint => "hint",
        }
    }
}

/// One problem a check reported.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    /// Relative to the worktree root; empty when the tool named no file.
    pub file: String,
    /// The line, or 0 when the tool named none.
    pub line: u32,
    /// The last line it spans, when it spans more than one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    /// How much it matters.
    pub level: Level,
    /// The rule the tool names it by.
    #[serde(default)]
    pub rule: String,
    /// What the tool says.
    pub message: String,
    /// What to do about it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    /// Why it matters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// It counts wherever it is, not only on lines the change touched.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub anywhere: bool,
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
