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
use serde::{Deserialize, Serialize};

/// Where a check's test runner says how many tests ran.
#[derive(Clone)]
pub(crate) enum Reader {
    /// The test cases, not skipped, in the JUnit reports matching the glob that this run wrote.
    Junit { glob: String, matcher: GlobMatcher },
    /// The tests a pattern finds in the output: the numbers it captures, or one per match.
    Count(Regex),
}

/// Only what was written, since this is part of a check's key: the compiled matcher's form
/// changes with the glob library.
impl std::fmt::Debug for Reader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Reader::Junit { glob, .. } => f.debug_tuple("Junit").field(glob).finish(),
            Reader::Count(re) => f.debug_tuple("Count").field(&re.as_str()).finish(),
        }
    }
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

    /// How many tests ran and, from JUnit reports, each case that ran, or why there is no
    /// count. `outputs` are the files the check's output went to, `out` its `$FEARLEZZ_OUT`,
    /// `root` the checkout, and `since` when the run started.
    pub(crate) fn count(
        &self,
        outputs: &[&Path],
        out: &Path,
        root: &Path,
        since: SystemTime,
    ) -> Result<(u64, Vec<Case>), String> {
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
                found.map(|n| (n, Vec::new())).ok_or_else(|| {
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
                let cases: Vec<Case> = reports.iter().flat_map(|text| junit_cases(text)).collect();
                Ok((u64::try_from(cases.len()).unwrap_or(u64::MAX), cases))
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

/// One test case a JUnit report says ran.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Case {
    /// `classname::name`, or the name alone when there is no class.
    pub name: String,
    /// The `file` attribute, which some runners write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    pub failed: bool,
}

impl Case {
    /// Whether the case comes from the test file `path`: the report names that file, or its
    /// class is the file, the file's module (`tests.test_a`), or ends in the file's stem
    /// (`crate::red_green`, `com.x.FooTest`). Runners that name neither, such as Go's, name
    /// no case.
    pub(crate) fn is_from(&self, path: &str) -> bool {
        if self.file.as_deref() == Some(path) {
            return true;
        }
        let class = self.name.rsplit_once("::").map_or("", |(class, _)| class);
        if class.is_empty() {
            return false;
        }
        let bare = path.rsplit_once('.').map_or(path, |(stem, _)| stem);
        let module = bare.replace('/', ".");
        let stem = bare.rsplit('/').next().unwrap_or(bare);
        let last = class.rsplit(['.', ':', '/', '$']).next().unwrap_or(class);
        class == path || class == module || class.starts_with(&format!("{module}.")) || last == stem
    }
}

/// The test cases of one report that were not skipped: those with a `<failure` or `<error`
/// inside failed.
fn junit_cases(text: &str) -> Vec<Case> {
    let mut cases = Vec::new();
    let mut rest = text;
    while let Some((_, after)) = rest.split_once("<testcase") {
        rest = after;
        if !rest.starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/') {
            continue;
        }
        let (attrs, after) = start_tag(rest);
        rest = after;
        let body = if attrs.ends_with('/') {
            ""
        } else {
            let (body, after) = rest.split_once("</testcase>").unwrap_or((rest, ""));
            rest = after;
            body
        };
        if body.contains("<skipped") {
            continue;
        }
        let name = attr(attrs, "name").unwrap_or_default();
        let name = match attr(attrs, "classname").filter(|c| !c.is_empty()) {
            Some(class) => format!("{class}::{name}"),
            None => name,
        };
        cases.push(Case {
            name,
            file: attr(attrs, "file"),
            failed: body.contains("<failure") || body.contains("<error"),
        });
    }
    cases
}

/// A start tag's attributes and the text after it: split at the first `>` outside quotes.
fn start_tag(text: &str) -> (&str, &str) {
    let mut quote = None;
    for (i, c) in text.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if q == c => quote = None,
            (None, '>') => {
                let (attrs, rest) = text.split_at(i);
                return (attrs, rest.strip_prefix('>').unwrap_or(rest));
            }
            _ => {}
        }
    }
    (text, "")
}

/// The unescaped value of the attribute `name` in a start tag's attributes.
fn attr(attrs: &str, name: &str) -> Option<String> {
    let mut rest = attrs;
    loop {
        let (before, after) = rest.split_once(name)?;
        rest = after;
        if before
            .chars()
            .next_back()
            .is_some_and(|c| !c.is_whitespace())
        {
            continue;
        }
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = value.trim_start();
        let Some((quote, value)) = ['"', '\'']
            .into_iter()
            .find_map(|q| Some((q, value.strip_prefix(q)?)))
        else {
            continue;
        };
        let (value, _) = value.split_once(quote)?;
        return Some(
            value
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&quot;", "\"")
                .replace("&apos;", "'")
                .replace("&amp;", "&"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::Reader;

    #[test]
    fn a_reader_as_part_of_a_key_is_what_was_written() {
        let junit = Reader::parse(&toml::Value::String("junit".into())).unwrap();
        assert_eq!(format!("{junit:?}"), r#"Junit("$FEARLEZZ_OUT/junit.xml")"#);
    }

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
        assert_eq!(
            reader.count(&[&log], dir.path(), dir.path(), since),
            Ok((9, Vec::new()))
        );
    }

    #[test]
    fn a_junit_report_names_its_cases_and_which_failed() {
        let report = r#"<testsuites><testsuite name="s">
<testcase classname="tests.test_a" name="adds &amp; keeps" time="0.1"/>
<testcase name="bare" classname=""><failure message="a > b">boom</failure></testcase>
<testcase classname="a.test.ts" name="errs" file="src/a.test.ts"><error/></testcase>
<testcase classname="x" name="skip"><skipped/></testcase>
</testsuite></testsuites>"#;
        let cases = super::junit_cases(report);
        let names: Vec<(&str, bool)> = cases.iter().map(|c| (c.name.as_str(), c.failed)).collect();
        assert_eq!(
            names,
            [
                ("tests.test_a::adds & keeps", false),
                ("bare", true),
                ("a.test.ts::errs", true)
            ]
        );
        assert!(cases[0].is_from("tests/test_a.py"));
        assert!(cases[2].is_from("src/a.test.ts"));
        assert!(!cases[0].is_from("tests/test_b.py"));
    }
}
