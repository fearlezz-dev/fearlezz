//! Coverage: which lines the change touched ran in no test, read from the report a test
//! runner writes while it tests. Each run of such lines is a finding of the rule `uncovered`,
//! a flag unless `[lints.coverage]` says otherwise, so old untested code never counts and new
//! untested code is always shown. A test check declares it with `coverage`; it never decides
//! whether the tests passed.
//!
//! Three report formats cover every language piece: lcov (Rust through cargo-llvm-cov,
//! Vitest, Bun, Flutter, coverage.py), JaCoCo XML (Kotlin through Kover) and Go's
//! coverprofile.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::SystemTime;

use anyhow::{Context as _, Result, bail};
use globset::{GlobBuilder, GlobMatcher};

use crate::findings::{Changed, Finding, Level};
use crate::lints::{Lints, Tool};

/// A check's coverage: where its report is, and the `[lints.coverage]` table.
#[derive(Clone, Debug)]
pub(crate) struct Coverage {
    pub read: Reader,
    pub levels: Lints,
}

impl Coverage {
    /// From the check's `coverage`, if any; `false` turns off what a piece measures.
    pub(crate) fn parse(raw: Option<&toml::Value>, lints: &Lints) -> Result<Option<Coverage>> {
        match raw {
            None | Some(toml::Value::Boolean(false)) => Ok(None),
            Some(raw) => Ok(Some(Coverage {
                read: Reader::parse(raw)?,
                levels: lints.only(&[Tool::Coverage]),
            })),
        }
    }

    /// The changed lines no test ran, at the level `[lints.coverage]` gives them.
    pub(crate) fn findings(
        &self,
        out: &Path,
        root: &Path,
        folder: Option<&str>,
        since: SystemTime,
        changed: &Changed,
    ) -> Result<Vec<Finding>, String> {
        let found = self.read.uncovered(out, root, folder, since, changed)?;
        Ok(self.levels.apply(&[Tool::Coverage], found))
    }
}

/// The rule every coverage finding has, as `[lints.coverage]` names it.
pub(crate) const RULE: &str = "uncovered";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Lcov,
    Jacoco,
    Go,
}

const FORMATS: [(&str, Format, &str); 3] = [
    ("lcov", Format::Lcov, "$FEARLEZZ_OUT/lcov.info"),
    ("jacoco", Format::Jacoco, "$FEARLEZZ_OUT/jacoco.xml"),
    ("go", Format::Go, "$FEARLEZZ_OUT/cover.out"),
];

/// Where a check's test runner wrote its coverage, and in which format.
#[derive(Clone)]
pub(crate) struct Reader {
    format: Format,
    glob: String,
    matcher: GlobMatcher,
}

/// Only what was written, since this is part of a check's key.
impl std::fmt::Debug for Reader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Coverage")
            .field(&self.format)
            .field(&self.glob)
            .finish()
    }
}

/// Covered lines per repository path: how often each measured line ran.
type Hits = BTreeMap<String, BTreeMap<u32, u64>>;

impl Reader {
    /// From `coverage = "lcov"` (the report in `$FEARLEZZ_OUT`) or
    /// `coverage = { lcov = "<glob>" }`, and the same for `jacoco` and `go`.
    pub(crate) fn parse(raw: &toml::Value) -> Result<Reader> {
        let one = raw
            .as_table()
            .filter(|t| t.len() == 1)
            .and_then(|t| t.iter().next());
        let (name, glob) = match (raw.as_str(), one) {
            (Some(name), _) => (name, None),
            (_, Some((name, toml::Value::String(glob)))) => (name.as_str(), Some(glob.as_str())),
            _ => ("", None),
        };
        let Some(&(_, format, default)) = FORMATS.iter().find(|(n, ..)| *n == name) else {
            bail!(
                "coverage = {raw}: no such reader; use \"lcov\", \"jacoco\" or \"go\", \
                 {{ lcov = \"<glob>\" }} for a report the runner writes elsewhere, or false"
            );
        };
        let glob = glob.unwrap_or(default);
        let pattern = glob.strip_prefix("$FEARLEZZ_OUT/").unwrap_or(glob);
        let matcher = GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .with_context(|| format!("coverage {name} \"{glob}\""))?
            .compile_matcher();
        Ok(Reader {
            format,
            glob: glob.to_string(),
            matcher,
        })
    }

    /// As written in `coverage`.
    pub(crate) fn name(&self) -> &'static str {
        FORMATS
            .iter()
            .find(|(_, f, _)| *f == self.format)
            .map_or("", |(n, ..)| n)
    }

    /// The changed lines no test ran, as flag findings, or why there is no coverage. `out` is
    /// the run's `$FEARLEZZ_OUT`, `root` the checkout, `folder` the one the check ran in, if
    /// not the root, and `since` when the run started: a report from an earlier run is stale.
    pub(crate) fn uncovered(
        &self,
        out: &Path,
        root: &Path,
        folder: Option<&str>,
        since: SystemTime,
        changed: &Changed,
    ) -> Result<Vec<Finding>, String> {
        // A check in a project folder looks there, so a sibling's report is never read.
        let folder_root = folder.map(|f| root.join(f));
        let base = if self.glob.starts_with("$FEARLEZZ_OUT/") {
            out
        } else {
            folder_root.as_deref().unwrap_or(root)
        };
        let mut reports = Vec::new();
        crate::counts::find(base, base, &self.matcher, since, &mut reports);
        if reports.is_empty() {
            return Err(format!(
                "no {} report matching {} written by this run; the runner wrote none, or wrote \
                 it elsewhere",
                self.name(),
                self.glob
            ));
        }
        let places = Places { root, folder };
        let mut hits = Hits::new();
        let mut named = 0;
        for (path, text) in &reports {
            named += match self.format {
                Format::Lcov => lcov(text, &places, &mut hits),
                Format::Go => go(text, &places, &mut hits),
                // It looks up only changed files, so a miss says nothing about its paths.
                Format::Jacoco => {
                    jacoco(text, path, root, changed, &mut hits);
                    0
                }
            };
        }
        // A report whose every path misses the repository would hide all untested code.
        if named > 0 && hits.is_empty() {
            return Err(format!(
                "the {} report names no file of the repository; the runner names files from \
                 another folder",
                self.name()
            ));
        }
        Ok(findings(&hits, changed))
    }
}

/// Where the files a report names are looked for.
struct Places<'a> {
    root: &'a Path,
    folder: Option<&'a str>,
}

impl Places<'_> {
    /// The repository path of a file a report names: an absolute path inside the checkout,
    /// or a relative one from the check's folder, then from the root. `None` for a file
    /// outside the repository (a dependency) or one that is not there.
    fn resolve(&self, file: &str) -> Option<String> {
        let file = file.trim();
        let path = Path::new(file);
        if path.is_absolute() {
            return path
                .strip_prefix(self.root)
                .ok()
                .map(|p| p.to_string_lossy().to_string());
        }
        let file = file.trim_start_matches("./");
        self.folder
            .map(|f| format!("{f}/{file}"))
            .into_iter()
            .chain([file.to_string()])
            .find(|p| self.root.join(p).is_file())
    }
}

fn mark(hits: &mut Hits, file: String, line: u32, count: u64) {
    let seen = hits.entry(file).or_default().entry(line).or_insert(0);
    *seen = (*seen).max(count);
}

/// lcov's `SF:` and `DA:<line>,<count>` records. Returns how many files it named.
fn lcov(text: &str, places: &Places<'_>, hits: &mut Hits) -> usize {
    let mut named = 0;
    let mut file = None;
    for line in text.lines() {
        if let Some(path) = line.strip_prefix("SF:") {
            named += 1;
            file = places.resolve(path);
        } else if line == "end_of_record" {
            file = None;
        } else if let (Some(da), Some(f)) = (line.strip_prefix("DA:"), &file) {
            let mut parts = da.split(',');
            let (Some(Ok(n)), Some(Ok(count))) = (
                parts.next().map(str::parse::<u32>),
                parts.next().map(|c| c.trim().parse::<f64>()),
            ) else {
                continue;
            };
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "some tools write counts as floats; a count only says whether it ran"
            )]
            mark(hits, f.clone(), n, count.max(0.0) as u64);
        }
    }
    named
}

/// Go's coverprofile: `<import path>/<file>:<line>.<col>,<line>.<col> <statements> <count>`.
/// Files are named by import path, from the module the nearest `go.mod` names. A line is
/// covered when any block on it ran.
fn go(text: &str, places: &Places<'_>, hits: &mut Hits) -> usize {
    let modules: Vec<(String, String)> = places
        .folder
        .into_iter()
        .chain([""])
        .filter_map(|dir| {
            let manifest = places.root.join(dir).join("go.mod");
            let text = std::fs::read_to_string(manifest).ok()?;
            let module = text
                .lines()
                .find_map(|l| l.trim().strip_prefix("module "))?
                .trim()
                .trim_matches('"')
                .to_string();
            Some((module, dir.to_string()))
        })
        .collect();
    let mut named = 0;
    for line in text.lines().filter(|l| !l.starts_with("mode:")) {
        let Some((file, block)) = line.rsplit_once(':') else {
            continue;
        };
        let mut fields = block.split_whitespace();
        let (Some(span), Some(_), Some(Ok(count))) = (
            fields.next(),
            fields.next(),
            fields.next().map(str::parse::<u64>),
        ) else {
            continue;
        };
        let Some((start, end)) = span.split_once(',') else {
            continue;
        };
        let line_of = |p: &str| p.split('.').next()?.parse::<u32>().ok();
        let (Some(start), Some(end)) = (line_of(start), line_of(end)) else {
            continue;
        };
        named += 1;
        let local = modules.iter().find_map(|(module, dir)| {
            let rest = file.strip_prefix(module.as_str())?.strip_prefix('/')?;
            Some(if dir.is_empty() {
                rest.to_string()
            } else {
                format!("{dir}/{rest}")
            })
        });
        let Some(path) = places.resolve(local.as_deref().unwrap_or(file)) else {
            continue;
        };
        for n in start..=end {
            mark(hits, path.clone(), n, count);
        }
    }
    named
}

/// JaCoCo XML: `<sourcefile>` by name inside its `<package>`, with a `<line>` per line that
/// counts missed (`mi`) and covered (`ci`) instructions. A package names no source folder, so
/// the file is the changed one whose path ends in `<package>/<name>`; of several, the one in
/// the module the report sits in.
fn jacoco(text: &str, report: &Path, root: &Path, changed: &Changed, hits: &mut Hits) {
    let mut package = String::new();
    let mut file: Option<String> = None;
    let mut rest = text;
    while let Some((_, after)) = rest.split_once('<') {
        let (tag, after) = after.split_once('>').unwrap_or((after, ""));
        rest = after;
        let attr = |name: &str| crate::counts::attr(tag, name);
        if let Some(t) = tag.strip_prefix("package") {
            if t.starts_with(char::is_whitespace) {
                package = attr("name").unwrap_or_default();
            }
        } else if tag.starts_with("sourcefile") {
            let name = attr("name").unwrap_or_default();
            let suffix = if package.is_empty() {
                name
            } else {
                format!("{package}/{name}")
            };
            file = source(&suffix, report, root, changed);
        } else if tag == "/sourcefile" {
            file = None;
        } else if let (Some(f), true) = (&file, tag.starts_with("line ")) {
            let number = |name: &str| attr(name)?.parse::<u64>().ok();
            let (Some(nr), Some(mi), Some(ci)) = (number("nr"), number("mi"), number("ci")) else {
                continue;
            };
            if mi + ci > 0
                && let Ok(nr) = u32::try_from(nr)
            {
                mark(hits, f.clone(), nr, ci);
            }
        }
    }
}

/// The changed file a JaCoCo source file is, by its package path, or by its name alone when
/// one changed file has it (Kotlin may leave the common root package out of its folders).
fn source(suffix: &str, report: &Path, root: &Path, changed: &Changed) -> Option<String> {
    let ending = |end: &str| -> Vec<&str> {
        changed
            .paths()
            .filter(|p| *p == end || p.ends_with(&format!("/{end}")))
            .filter(|p| root.join(p).is_file())
            .collect()
    };
    let mut matches = ending(suffix);
    if matches.is_empty() {
        let name = suffix.rsplit('/').next().unwrap_or(suffix);
        matches = ending(name);
        if matches.len() > 1 {
            return None;
        }
    }
    if matches.len() <= 1 {
        return matches.first().map(|p| (*p).to_string());
    }
    // The deepest folder holding the report and one of the files is the report's module.
    let module = report.strip_prefix(root).ok().and_then(|rel| {
        rel.ancestors()
            .filter(|dir| !dir.as_os_str().is_empty())
            .find(|dir| matches.iter().any(|p| Path::new(p).starts_with(dir)))
    });
    module
        .and_then(|m| matches.iter().find(|p| Path::new(p).starts_with(m)))
        .or(matches.first())
        .map(|p| (*p).to_string())
}

/// A finding per run of changed lines no test ran. Lines that are not code (blank, a
/// comment) neither break a run nor join one; a covered or unchanged line ends it.
fn findings(hits: &Hits, changed: &Changed) -> Vec<Finding> {
    let mut found = Vec::new();
    for (file, lines) in hits {
        let mut run: Option<(u32, u32)> = None;
        for (&n, &count) in lines {
            if count == 0 && changed.has_line(file, n) {
                run = Some(run.map_or((n, n), |(a, _)| (a, n)));
            } else if let Some((a, b)) = run.take() {
                found.push(uncovered(file, a, b));
            }
        }
        if let Some((a, b)) = run {
            found.push(uncovered(file, a, b));
        }
    }
    found
}

fn uncovered(file: &str, from: u32, to: u32) -> Finding {
    Finding {
        file: file.to_string(),
        line: from,
        end_line: (to > from).then_some(to),
        level: Level::Flag,
        rule: RULE.into(),
        message: if to > from {
            format!("changed lines {from} to {to} ran in no test")
        } else {
            "this changed line ran in no test".into()
        },
        fix: Some(
            "add a test that reaches this code through what it should do, or delete the code"
                .into(),
        ),
        why: None,
        anywhere: false,
    }
}
