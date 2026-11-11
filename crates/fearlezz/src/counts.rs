//! Test counts: how many tests a check ran, read from what its test runner wrote. A check with
//! a reader that ran no tests fails, so a test command that proves nothing cannot pass.

use std::path::Path;

use serde::Deserialize;

/// Where a check's test runner says how many tests ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Reader {
    /// The `test result: ok. N passed; M failed; ...` lines of `cargo test` in the output.
    Cargo,
    /// The test cases in `$FEARLEZZ_OUT/junit.xml` that were not skipped.
    Junit,
}

impl Reader {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Reader::Cargo => "cargo",
            Reader::Junit => "junit",
        }
    }

    /// How many tests ran, or why there is no count. `outputs` are the files the check's
    /// output went to, `dir` is its `$FEARLEZZ_OUT`.
    pub(crate) fn count(self, outputs: &[&Path], dir: &Path) -> Result<u64, String> {
        match self {
            Reader::Cargo => {
                let text: String = outputs
                    .iter()
                    .filter_map(|p| std::fs::read(p).ok())
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .collect();
                let mut found = None;
                for line in text.lines() {
                    if let Some((_, rest)) = line.split_once("test result:") {
                        *found.get_or_insert(0) += cargo_ran(rest);
                    }
                }
                found.ok_or_else(|| {
                    "no test result line in its output; another test runner needs its own \
                     tests reader"
                        .into()
                })
            }
            Reader::Junit => {
                let text = std::fs::read_to_string(dir.join("junit.xml"))
                    .ok()
                    .ok_or("no $FEARLEZZ_OUT/junit.xml: the test runner wrote no report")?;
                let cases = u64::try_from(text.matches("<testcase").count()).unwrap_or(u64::MAX);
                let skipped = u64::try_from(text.matches("<skipped").count()).unwrap_or(u64::MAX);
                Ok(cases.saturating_sub(skipped))
            }
        }
    }
}

/// The passed and failed tests of one `test result` line.
fn cargo_ran(line: &str) -> u64 {
    let words: Vec<&str> = line
        .split(|c: char| c.is_whitespace() || c == ';')
        .filter(|w| !w.is_empty())
        .collect();
    words
        .windows(2)
        .filter(|w| matches!(w.get(1), Some(&("passed" | "failed"))))
        .filter_map(|w| w.first()?.parse::<u64>().ok())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::cargo_ran;

    #[test]
    fn a_cargo_line_counts_passed_and_failed() {
        assert_eq!(
            cargo_ran(" FAILED. 3 passed; 2 failed; 1 ignored; 0 measured; 4 filtered out"),
            5
        );
    }
}
