//! Coverage: which lines the change touched ran in no test, read from the report a test
//! runner writes while it tests. Each run of such lines is a finding of the rule `uncovered`,
//! so old untested code never counts and new untested code is always shown. A test check
//! declares it with `coverage`; it never makes a failed test run pass.
//!
//! How strict it is follows the project's coverage, against a target (80% unless the check
//! says otherwise): more tests add little above it, and tests written only to reach a number
//! prove nothing. At or above the target untested lines are hints. Under it, a change that
//! tests the target's share of its lines gets flags; one that does not is blocked. A level in
//! `[lints.coverage]` replaces this. Test files are never measured.
//!
//! Three report formats cover every language piece: lcov (Rust through cargo-llvm-cov,
//! Vitest, Bun, Flutter, coverage.py), JaCoCo XML (Kotlin through Kover) and Go's
//! coverprofile.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::SystemTime;

use anyhow::{Context as _, Result, bail};
use globset::{GlobBuilder, GlobMatcher};

use serde::{Deserialize, Serialize};

use crate::findings::{Changed, Finding, Level};
use crate::lints::{Lints, Tool};

/// The share of lines, in percent, past which more tests add little.
const TARGET: u8 = 80;

/// A check's coverage: where its report is, its target, and the `[lints.coverage]` table.
#[derive(Clone, Debug)]
pub(crate) struct Coverage {
    pub read: Reader,
    pub target: u8,
    pub levels: Lints,
}

/// Lines a test could run, and how many of them ran: of the project, and of the lines the
/// change added or modified. Test files count in neither.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Measured {
    pub lines: u64,
    pub covered: u64,
    pub changed_lines: u64,
    pub changed_covered: u64,
}

impl Measured {
    /// The project's share, rounded down, so it is never shown above what it is.
    pub(crate) fn percent(self) -> u64 {
        (self.covered * 100).checked_div(self.lines).unwrap_or(100)
    }

    /// `coverage 62%, 9 of 12 changed lines`, as a run's line shows it.
    pub(crate) fn summary(self) -> String {
        format!(
            "coverage {}%, {} of {} changed lines",
            self.percent(),
            self.changed_covered,
            self.changed_lines
        )
    }

    /// The level of an untested changed line, and why.
    fn level(self, target: u8) -> (Level, String) {
        let target = u64::from(target);
        let meets = |covered: u64, lines: u64| covered * 100 >= target * lines;
        let project = self.percent();
        let (covered, lines) = (self.changed_covered, self.changed_lines);
        if meets(self.covered, self.lines) {
            (
                Level::Hint,
                format!(
                    "the project is at {project}%; at or above {target}% more tests add little, \
                     so test this only when it matters"
                ),
            )
        } else if meets(covered, lines) {
            (
                Level::Flag,
                format!(
                    "the project is at {project}% and this change tests {covered} of its {lines} \
                     lines; it meets {target}%, which the project does not yet"
                ),
            )
        } else {
            (
                Level::Gate,
                format!(
                    "the project is at {project}% and this change tests {covered} of its {lines} \
                     lines; under {target}%, a change must test {target}% of its lines"
                ),
            )
        }
    }
}

impl Coverage {
    /// From the check's `coverage`, if any: a reader (`"lcov"`, `{ lcov = "<glob>" }`), or
    /// a table with `read` or a reader's glob and a `target`. `false` turns off what a piece
    /// measures.
    pub(crate) fn parse(raw: Option<&toml::Value>, lints: &Lints) -> Result<Option<Coverage>> {
        let (read, target) = match raw {
            None | Some(toml::Value::Boolean(false)) => return Ok(None),
            Some(toml::Value::Table(t)) => {
                let mut t = t.clone();
                let target = match t.remove("target") {
                    None => TARGET,
                    Some(toml::Value::Integer(n)) => u8::try_from(n)
                        .ok()
                        .filter(|n| *n <= 100)
                        .with_context(|| format!("coverage target = {n}: a share from 0 to 100"))?,
                    Some(v) => bail!("coverage target = {v}: a share from 0 to 100"),
                };
                let read = match t.remove("read") {
                    Some(read) if t.is_empty() => read,
                    Some(_) => {
                        let keys: Vec<&str> = t.keys().map(String::as_str).collect();
                        bail!(
                            "coverage: {} beside read; it takes read (or a reader's glob, such \
                             as lcov = \"<glob>\") and target",
                            keys.join(", ")
                        )
                    }
                    // A project that sets only the target of a piece's reader replaces it.
                    None if t.is_empty() => bail!(
                        "coverage = {raw}: names no reader; write read = \"lcov\" (or jacoco, \
                         go) beside the target",
                        raw = toml::Value::Table(t)
                    ),
                    None => toml::Value::Table(t),
                };
                (read, target)
            }
            Some(raw) => (raw.clone(), TARGET),
        };
        Ok(Some(Coverage {
            read: Reader::parse(&read)?,
            target,
            levels: lints.only(&[Tool::Coverage]),
        }))
    }

    /// The changed lines no test ran, at the level the project's coverage gives them (or
    /// `[lints.coverage]` does), and what was measured. `counts` says which files are the
    /// project's code: not tests, not generated or ignored files.
    pub(crate) fn findings(
        &self,
        at: &At<'_>,
        changed: &Changed,
        counts: impl Fn(&str) -> bool,
    ) -> Result<(Vec<Finding>, Measured), String> {
        let (mut hits, unplaced) = self.read.hits(at, changed)?;
        hits.retain(|file, _| counts(file));
        let mut measured = measure(&hits, changed);
        measured.lines += unplaced.lines;
        measured.covered += unplaced.covered;
        let (level, why) = measured.level(self.target);
        let mut found = findings(&hits, changed);
        for f in &mut found {
            f.level = level;
            f.why = Some(why.clone());
        }
        Ok((self.levels.apply(&[Tool::Coverage], found), measured))
    }
}

/// Where a run left its report: its `$FEARLEZZ_OUT`, the checkout, the folder the check ran
/// in if not the root, and when the run started, since a report from an earlier run is stale.
pub(crate) struct At<'a> {
    pub out: &'a Path,
    pub root: &'a Path,
    pub folder: Option<&'a str>,
    pub since: SystemTime,
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

    /// How often each measured line ran, and the lines of JaCoCo source files no changed file
    /// is, or why there is no coverage.
    fn hits(&self, at: &At<'_>, changed: &Changed) -> Result<(Hits, Measured), String> {
        let At {
            out,
            root,
            folder,
            since,
        } = *at;
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
        let mut unplaced = Measured::default();
        let mut named = 0;
        for (path, text) in &reports {
            named += match self.format {
                Format::Lcov => lcov(text, &places, &mut hits),
                Format::Go => go(text, &places, &mut hits),
                // It looks up only changed files, so a miss says nothing about its paths.
                Format::Jacoco => {
                    jacoco(text, path, root, changed, &mut hits, &mut unplaced);
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
        Ok((hits, unplaced))
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
/// covered when any block on it ran; blank and comment lines inside a block are not code.
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
    let mut sources: BTreeMap<String, Vec<bool>> = BTreeMap::new();
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
        let code = sources.entry(path.clone()).or_insert_with(|| {
            let text = std::fs::read_to_string(places.root.join(&path)).unwrap_or_default();
            text.lines()
                .map(|l| {
                    let l = l.trim();
                    !l.is_empty() && !l.starts_with("//")
                })
                .collect()
        });
        // A block spans lines that are not code: blank lines and comments.
        for n in start..=end {
            let index = usize::try_from(n).unwrap_or(usize::MAX).saturating_sub(1);
            if code.get(index).copied().unwrap_or(true) {
                mark(hits, path.clone(), n, count);
            }
        }
    }
    named
}

/// JaCoCo XML: `<sourcefile>` by name inside its `<package>`, with a `<line>` per line that
/// counts missed (`mi`) and covered (`ci`) instructions. A package names no source folder, so
/// the file is the changed one whose path ends in `<package>/<name>`; of several, the one in
/// the module the report sits in.
fn jacoco(
    text: &str,
    report: &Path,
    root: &Path,
    changed: &Changed,
    hits: &mut Hits,
    unplaced: &mut Measured,
) {
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
            file = source(&suffix, &package, report, root, changed);
        } else if tag == "/sourcefile" {
            file = None;
        } else if tag.starts_with("line ") {
            let number = |name: &str| attr(name)?.parse::<u64>().ok();
            let (Some(nr), Some(mi), Some(ci)) = (number("nr"), number("mi"), number("ci")) else {
                continue;
            };
            let Ok(nr) = u32::try_from(nr) else {
                continue;
            };
            if mi + ci == 0 {
                continue;
            }
            // A file the change did not touch counts toward the project only.
            if let Some(f) = &file {
                mark(hits, f.clone(), nr, ci);
            } else {
                unplaced.lines += 1;
                unplaced.covered += u64::from(ci > 0);
            }
        }
    }
}

/// The changed file a JaCoCo source file is, by its package path, or by its name alone when
/// one changed file has it and declares that package (Kotlin may leave the common root
/// package out of its folders).
fn source(
    suffix: &str,
    package: &str,
    report: &Path,
    root: &Path,
    changed: &Changed,
) -> Option<String> {
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
        let declared = format!("package {}", package.replace('/', "."));
        matches = ending(name);
        matches.retain(|p| {
            std::fs::read_to_string(root.join(p)).is_ok_and(|text| {
                text.lines()
                    .any(|l| l.trim().trim_end_matches(';') == declared)
            })
        });
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

/// What a report measured of the project and of the change.
fn measure(hits: &Hits, changed: &Changed) -> Measured {
    let mut m = Measured::default();
    for (file, lines) in hits {
        for (&n, &count) in lines {
            let ran = u64::from(count > 0);
            m.lines += 1;
            m.covered += ran;
            if changed.has_line(file, n) {
                m.changed_lines += 1;
                m.changed_covered += ran;
            }
        }
    }
    m
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
        level: Level::Gate,
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
