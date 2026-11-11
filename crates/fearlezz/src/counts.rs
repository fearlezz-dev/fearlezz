//! Test counts: how many tests a check ran, read from what its test runner wrote. A check with
//! a reader that ran no tests fails, so a test command that proves nothing cannot pass.
//!
//! Two readers cover every runner: JUnit reports, which most runners write, and a pattern for
//! the summary line a runner prints. The language pieces set the right one.

use std::path::Path;
use std::time::SystemTime;

use anyhow::{Context as _, Result, bail};
use globset::{GlobBuilder, GlobMatcher};
use regex::Regex;

/// Where a check's test runner says how many tests ran.
#[derive(Clone, Debug)]
pub(crate) enum Reader {
    /// The test cases, not skipped, in the JUnit reports matching the glob that this run wrote.
    Junit { glob: String, matcher: GlobMatcher },
    /// The tests a pattern finds in the output: the numbers it captures, or one per match.
    Count(Regex),
}

/// Where a runner's report goes when the check names none.
const OUT_REPORT: &str = "$FEARLEZZ_OUT/junit.xml";

impl Reader {
    /// From `tests = "junit"`, `tests = { junit = "<glob>" }` or
    /// `tests = { count = "<pattern>" }`.
    pub(crate) fn parse(raw: &toml::Value) -> Result<Reader> {
        let one = raw
            .as_table()
            .filter(|t| t.len() == 1)
            .and_then(|t| t.iter().next());
        match (raw.as_str(), one) {
            (Some("junit"), _) => Reader::junit(OUT_REPORT),
            (_, Some((key, toml::Value::String(glob)))) if key == "junit" => Reader::junit(glob),
            (_, Some((key, toml::Value::String(count)))) if key == "count" => Ok(Reader::Count(
                Regex::new(count).with_context(|| format!("tests count \"{count}\""))?,
            )),
            _ => bail!(
                "tests = {raw}: no such reader; use \"junit\", {{ junit = \"<glob>\" }} or \
                 {{ count = \"<pattern>\" }}"
            ),
        }
    }

    fn junit(glob: &str) -> Result<Reader> {
        let pattern = glob.strip_prefix("$FEARLEZZ_OUT/").unwrap_or(glob);
        let matcher = GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .with_context(|| format!("tests junit \"{glob}\""))?
            .compile_matcher();
        Ok(Reader::Junit {
            glob: glob.to_string(),
            matcher,
        })
    }

    /// A stable text of the reader, part of the check's inputs.
    pub(crate) fn fingerprint(&self) -> String {
        match self {
            Reader::Junit { glob, .. } => format!("junit {glob}"),
            Reader::Count(re) => format!("count {}", re.as_str()),
        }
    }

    /// How many tests ran, or why there is no count. `outputs` are the files the check's
    /// output went to, `out` its `$FEARLEZZ_OUT`, `root` the checkout, and `since` when the
    /// run started.
    pub(crate) fn count(
        &self,
        outputs: &[&Path],
        out: &Path,
        root: &Path,
        since: SystemTime,
    ) -> Result<u64, String> {
        match self {
            Reader::Count(re) => {
                let text: String = outputs
                    .iter()
                    .filter_map(|p| std::fs::read(p).ok())
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .collect();
                let mut found = None;
                for caps in re.captures_iter(&text) {
                    let numbers: Vec<u64> = caps
                        .iter()
                        .skip(1)
                        .flatten()
                        .filter_map(|m| m.as_str().parse().ok())
                        .collect();
                    let n = if numbers.is_empty() {
                        1
                    } else {
                        numbers.iter().sum()
                    };
                    *found.get_or_insert(0) += n;
                }
                found.ok_or_else(|| {
                    format!(
                        "no line of its output matches {}; a runner that prints another summary \
                         needs its own tests reader",
                        re.as_str()
                    )
                })
            }
            Reader::Junit { glob, matcher } => {
                let base = if glob.starts_with("$FEARLEZZ_OUT/") {
                    out
                } else {
                    root
                };
                let mut reports = Vec::new();
                find(base, base, matcher, since, &mut reports);
                if reports.is_empty() {
                    return Err(format!(
                        "no JUnit report matching {glob} written by this run; the runner wrote \
                         none, or wrote it elsewhere"
                    ));
                }
                Ok(reports.iter().map(|text| junit_ran(text)).sum())
            }
        }
    }
}

/// The text of every file under `dir` matching `matcher` (from `base`) and changed after
/// `since`. Hidden folders and installed packages are skipped.
fn find(base: &Path, dir: &Path, matcher: &GlobMatcher, since: SystemTime, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            if !name.starts_with('.') && name != "node_modules" {
                find(base, &path, matcher, since, out);
            }
            continue;
        }
        let rel = path.strip_prefix(base).unwrap_or(&path);
        let fresh = meta.modified().is_ok_and(|m| m >= since);
        if fresh
            && matcher.is_match(rel)
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            out.push(text);
        }
    }
}

/// The test cases of one report that were not skipped.
fn junit_ran(text: &str) -> u64 {
    let count = |tag: &str| u64::try_from(text.matches(tag).count()).unwrap_or(u64::MAX);
    count("<testcase").saturating_sub(count("<skipped"))
}

#[cfg(test)]
mod tests {
    use super::Reader;

    #[test]
    fn a_pattern_sums_the_numbers_it_captures() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("log");
        std::fs::write(
            &log,
            "test result: FAILED. 3 passed; 2 failed\ntest result: ok. 4 passed; 0 failed\n",
        )
        .unwrap();
        let raw: toml::Value = toml::from_str(r"count = '(\d+) passed; (\d+) failed'").unwrap();
        let reader = Reader::parse(&raw).unwrap();
        let since = std::time::SystemTime::UNIX_EPOCH;
        assert_eq!(reader.count(&[&log], dir.path(), dir.path(), since), Ok(9));
    }
}
