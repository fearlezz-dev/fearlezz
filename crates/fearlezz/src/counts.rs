//! Test counts: how many tests a check ran, read from what its test runner wrote. A check with
//! a reader that ran no tests fails, so a test command that proves nothing cannot pass.
//!
//! Two readers cover every runner: JUnit reports, which most runners write, and a pattern for
//! the summary line a runner prints. The language pieces set the right one.

use anyhow::{Context as _, Result, bail};
use quick_xml::events::{BytesStart, Event};
use regex::Regex;

use crate::read::{Glob, Run, Source};

/// Where a check's test runner says how many tests ran.
#[derive(Clone)]
pub(crate) enum Reader {
    /// The test cases, not skipped, in the JUnit reports matching the glob that this run wrote.
    Junit(Glob),
    /// The tests a pattern finds in the output: the numbers it captures, or one per match;
    /// and the failed tests another pattern names, by its first capture.
    Count { count: Regex, failed: Option<Regex> },
}

/// Only what was written, since this is part of a check's key.
impl std::fmt::Debug for Reader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Reader::Junit(glob) => f.debug_tuple("Junit").field(glob).finish(),
            Reader::Count { count, failed } => {
                let mut tuple = f.debug_tuple("Count");
                tuple.field(&count.as_str());
                if let Some(failed) = failed {
                    tuple.field(&failed.as_str());
                }
                tuple.finish()
            }
        }
    }
}

/// Where a runner's report goes when the check names none.
const OUT_REPORT: &str = "$FEARLEZZ_OUT/junit.xml";

impl Reader {
    /// From `tests = "junit"`, `tests = { junit = "<glob>" }` or
    /// `tests = { count = "<pattern>", failed = "<pattern>" }`, `failed` optional.
    pub(crate) fn parse(raw: &toml::Value) -> Result<Reader> {
        let table = raw.as_table();
        let text = |key: &str| table.and_then(|t| t.get(key)).and_then(toml::Value::as_str);
        let regex = |key: &str, pattern: &str| {
            Regex::new(pattern).with_context(|| format!("tests {key} \"{pattern}\""))
        };
        let junit = |glob: &str| Glob::parse(glob, "tests junit").map(Reader::Junit);
        let len = table.map_or(0, toml::Table::len);
        match (raw.as_str(), text("junit"), text("count"), text("failed")) {
            (Some("junit"), ..) => junit(OUT_REPORT),
            (_, Some(glob), None, None) if len == 1 => junit(glob),
            (_, None, Some(count), failed) if len == 1 + usize::from(failed.is_some()) => {
                Ok(Reader::Count {
                    count: regex("count", count)?,
                    failed: failed.map(|f| regex("failed", f)).transpose()?,
                })
            }
            _ => bail!(
                "tests = {raw}: no such reader; use \"junit\", {{ junit = \"<glob>\" }} or \
                 {{ count = \"<pattern>\" }}, with failed = \"<pattern>\" to name the failed tests"
            ),
        }
    }

    /// How many tests ran and the cases it names (from JUnit reports each case that ran, from
    /// a `failed` pattern each failed test once), or why there is no count. A JUnit glob
    /// names reports from the repository root, whatever folder the check runs in.
    pub(crate) fn count(&self, run: &Run<'_>) -> Result<(u64, Vec<Case>), String> {
        match self {
            Reader::Count { count: re, failed } => {
                let text: String = Source::Output
                    .read(run, "output")?
                    .into_iter()
                    .map(|r| r.text)
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
                let mut cases: Vec<Case> = Vec::new();
                for caps in failed.iter().flat_map(|f| f.captures_iter(&text)) {
                    let Some(name) = caps.get(1).map(|m| m.as_str()) else {
                        continue;
                    };
                    if !cases.iter().any(|c| c.name == name) {
                        cases.push(Case {
                            name: name.to_string(),
                            file: None,
                            failed: true,
                            flaky: false,
                        });
                    }
                }
                found.map(|n| (n, cases)).ok_or_else(|| {
                    format!(
                        "no line of its output matches {}; a runner that prints another summary \
                         needs its own tests reader",
                        re.as_str()
                    )
                })
            }
            Reader::Junit(glob) => {
                let at_root = Run {
                    folder: None,
                    ..*run
                };
                let cases: Vec<Case> = Source::Glob(glob)
                    .read(&at_root, "JUnit report")?
                    .iter()
                    .flat_map(|r| junit_cases(&r.text))
                    .collect();
                Ok((u64::try_from(cases.len()).unwrap_or(u64::MAX), cases))
            }
        }
    }
}

pub(crate) use fearlezz_proof::Case;

/// Whether `case` comes from the test file `path`: the report names that file, or its class
/// is the file, the file's module (`tests.test_a`), or ends in the file's stem
/// (`crate::red_green`, `com.x.FooTest`). Runners that name neither, such as Go's, name no
/// case.
pub(crate) fn is_from(case: &Case, path: &str) -> bool {
    if case.file.as_deref() == Some(path) {
        return true;
    }
    let class = case.name.rsplit_once("::").map_or("", |(class, _)| class);
    if class.is_empty() {
        return false;
    }
    let bare = path.rsplit_once('.').map_or(path, |(stem, _)| stem);
    let module = bare.replace('/', ".");
    let stem = bare.rsplit('/').next().unwrap_or(bare);
    let last = class.rsplit(['.', ':', '/', '$']).next().unwrap_or(class);
    class == path || class == module || class.starts_with(&format!("{module}.")) || last == stem
}

/// The value of the attribute `name` of an XML element, unescaped.
pub(crate) fn attr(tag: &BytesStart<'_>, name: &str) -> Option<String> {
    let a = tag.try_get_attribute(name).ok()??;
    Some(
        match a.normalized_value(quick_xml::XmlVersion::Implicit1_0) {
            Ok(v) => v.into_owned(),
            // An entity XML does not define (such as `&nbsp;`) is left as written.
            Err(_) => a.value.into_owned(),
        },
    )
}

/// The test cases of one report that were not skipped: those with a `<failure>` or `<error>`
/// inside failed, and those with a `<flakyFailure>` or `<flakyError>` (Surefire's word for a
/// failed attempt before a pass, which Playwright and nextest write too) passed only on a
/// rerun. A `<rerunFailure>` comes after a `<failure>`, so a test that failed its reruns too
/// is a failure. A report cut short (a runner that crashed) gives the cases up to the cut.
fn junit_cases(text: &str) -> Vec<Case> {
    #[derive(Default)]
    struct Open {
        case: Option<Case>,
        skipped: bool,
        failed: bool,
    }
    let mut reader = quick_xml::Reader::from_str(text);
    reader.config_mut().check_end_names = false;
    let mut cases = Vec::new();
    let mut open = Open::default();
    loop {
        let event = reader.read_event();
        let (tag, empty) = match event {
            Ok(Event::Start(tag)) => (tag, false),
            Ok(Event::Empty(tag)) => (tag, true),
            Ok(Event::End(ref tag)) if tag.local_name().as_ref() != "testcase" => continue,
            Ok(Event::End(_) | Event::Eof) | Err(_) => {
                let Open {
                    case,
                    skipped,
                    failed,
                } = std::mem::take(&mut open);
                if let (Some(mut case), false) = (case, skipped) {
                    // nextest, by default, writes a `<failure>` beside the flaky attempts as
                    // well (and exits non zero, which fails the check on its own).
                    case.failed = failed && !case.flaky;
                    cases.push(case);
                }
                if matches!(event, Ok(Event::End(_))) {
                    continue;
                }
                break;
            }
            Ok(_) => continue,
        };
        match tag.local_name().as_ref() {
            "testcase" => {
                let name = attr(&tag, "name").unwrap_or_default();
                let name = match attr(&tag, "classname").filter(|c| !c.is_empty()) {
                    Some(class) => format!("{class}::{name}"),
                    None => name,
                };
                let case = Case {
                    name,
                    file: attr(&tag, "file"),
                    failed: false,
                    flaky: false,
                };
                if empty {
                    cases.push(case);
                } else {
                    open = Open {
                        case: Some(case),
                        ..Open::default()
                    };
                }
            }
            "skipped" => open.skipped = true,
            "failure" | "error" => open.failed = true,
            "flakyFailure" | "flakyError" => {
                if let Some(case) = &mut open.case {
                    case.flaky = true;
                }
            }
            _ => {}
        }
    }
    cases
}

#[cfg(test)]
mod tests {
    use super::{Reader, is_from};

    #[test]
    fn a_reader_as_part_of_a_key_is_what_was_written() {
        let junit = Reader::parse(&toml::Value::String("junit".into())).unwrap();
        assert_eq!(format!("{junit:?}"), r#"Junit("$FEARLEZZ_OUT/junit.xml")"#);
    }

    #[test]
    fn a_pattern_sums_the_numbers_it_captures() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join(crate::read::LOG);
        std::fs::write(
            &log,
            "test result: FAILED. 3 passed; 2 failed\ntest result: ok. 4 passed; 0 failed\n",
        )
        .unwrap();
        let raw: toml::Value = toml::from_str(r"count = '(\d+) passed; (\d+) failed'").unwrap();
        let reader = Reader::parse(&raw).unwrap();
        let run = crate::read::Run {
            out: dir.path(),
            root: dir.path(),
            folder: None,
            since: std::time::SystemTime::UNIX_EPOCH,
        };
        assert_eq!(reader.count(&run), Ok((9, Vec::new())));
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
        assert!(is_from(&cases[0], "tests/test_a.py"));
        assert!(is_from(&cases[2], "src/a.test.ts"));
        assert!(!is_from(&cases[0], "tests/test_b.py"));
    }
}
