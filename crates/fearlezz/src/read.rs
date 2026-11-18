//! Reading what a tool wrote: where a reader finds it (the check's stdout, all it printed, a
//! file in `$FEARLEZZ_OUT` or a glob of reports) and how a path a tool names becomes a
//! repository path. Findings, test counts, coverage, pictures and texts all read through
//! here, so where output comes from is decided once.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context as _, Result};
use globset::{GlobBuilder, GlobMatcher};

/// What a glob starts with to name files in the run's own folder.
const OUT: &str = "$FEARLEZZ_OUT/";

/// The file in a run's folder its stdout goes to when a reader reads it apart from the log.
pub(crate) const STDOUT: &str = "stdout.txt";

/// The file in a run's folder that holds everything it printed but a stdout kept apart.
pub(crate) const LOG: &str = "output.log";

/// Where a run left what its tools wrote.
#[derive(Clone, Copy)]
pub(crate) struct Run<'a> {
    /// Its `$FEARLEZZ_OUT`, which holds its log and kept apart stdout too.
    pub out: &'a Path,
    /// The checkout.
    pub root: &'a Path,
    /// The folder the check ran in, if not the root.
    pub folder: Option<&'a str>,
    /// When it started: a report from before is another run's, never proof of this one.
    pub since: SystemTime,
}

/// Where a reader finds what a tool wrote.
#[derive(Clone, Copy)]
pub(crate) enum Source<'a> {
    /// The check's stdout, kept apart from its log.
    Stdout,
    /// Everything the check printed: its log and a stdout kept apart.
    Output,
    /// A file the tool wrote in `$FEARLEZZ_OUT`; with `else_stdout`, the stdout (kept apart
    /// from the log) when it wrote none there.
    Out {
        file: &'static str,
        else_stdout: bool,
    },
    /// Every report matching a glob that this run wrote.
    Glob(&'a Glob),
}

/// One text a tool wrote, and the file it is in.
pub(crate) struct Report {
    pub path: PathBuf,
    pub text: String,
}

impl Source<'_> {
    /// Whether a run keeps the check's stdout apart from its log, for this source to read it.
    pub(crate) fn reads_stdout(self) -> bool {
        matches!(
            self,
            Source::Stdout
                | Source::Out {
                    else_stdout: true,
                    ..
                }
        )
    }

    /// What the tool wrote there: one text for a stream or a file (empty when it wrote none),
    /// or each report a glob matches. A glob that matches no report this run wrote is an
    /// error saying `what` it looked for.
    pub(crate) fn read(self, run: &Run<'_>, what: &str) -> Result<Vec<Report>, String> {
        // A byte that is not UTF-8 (Latin-1 output, binary data) never empties the text.
        let report = |path: PathBuf| {
            let bytes = std::fs::read(&path).unwrap_or_default();
            let text = String::from_utf8_lossy(&bytes).into_owned();
            Report { path, text }
        };
        let stdout = run.out.join(STDOUT);
        Ok(match self {
            Source::Stdout => vec![report(stdout)],
            Source::Output => {
                let mut log = report(run.out.join(LOG));
                log.text.push_str(&report(stdout).text);
                vec![log]
            }
            Source::Out { file, else_stdout } => {
                let file = run.out.join(file);
                vec![report(if else_stdout && !file.exists() {
                    stdout
                } else {
                    file
                })]
            }
            Source::Glob(glob) => return glob.reports(run, what),
        })
    }
}

/// A glob a check names files by, as written: from `$FEARLEZZ_OUT` when it starts so.
#[derive(Clone)]
pub(crate) struct Glob {
    written: String,
    matcher: GlobMatcher,
}

/// Only what was written, since this is part of a check's key: the compiled matcher's form
/// changes with the glob library.
impl std::fmt::Debug for Glob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.written.fmt(f)
    }
}

impl Glob {
    /// From the glob as written in the check's key `what`.
    pub(crate) fn parse(written: &str, what: &str) -> Result<Glob> {
        let matcher = GlobBuilder::new(Glob::pattern_of(written))
            .literal_separator(true)
            .build()
            .with_context(|| format!("{what} \"{written}\""))?
            .compile_matcher();
        Ok(Glob {
            written: written.to_string(),
            matcher,
        })
    }

    fn pattern_of(written: &str) -> &str {
        written.strip_prefix(OUT).unwrap_or(written)
    }

    /// The glob as written.
    pub(crate) fn written(&self) -> &str {
        &self.written
    }

    /// Every file below `base` it matches, as a sorted path from there. `whole` walks every
    /// folder; otherwise hidden folders and installed packages are skipped, as no report is
    /// there.
    /// A linked folder is not followed then either (a `result` link into the nix store).
    pub(crate) fn matches(&self, base: &Path, whole: bool) -> Vec<String> {
        let skip = |e: &std::fs::DirEntry| {
            let name = e.file_name();
            let name = name.to_string_lossy();
            !whole
                && (name.starts_with('.')
                    || name == "node_modules"
                    || e.file_type().is_ok_and(|t| t.is_symlink()))
        };
        let mut found: Vec<String> = crate::files::walk_skipping(base, base, skip)
            .into_iter()
            .filter(|rel| self.matcher.is_match(rel))
            .collect();
        found.sort();
        found
    }

    /// A match's path below the glob's fixed folders (those before the first segment with a
    /// wildcard), without its extension: `stills/desktop/home.png` matched by
    /// `stills/**/*.png` is `desktop/home`.
    pub(crate) fn id<'r>(&self, rel: &'r str) -> &'r str {
        let pattern = Glob::pattern_of(&self.written);
        // The file name segment is never a folder, so only the ones before it count.
        let folders = pattern.rsplit_once('/').map_or("", |(dirs, _)| dirs);
        let fixed: String = folders
            .split('/')
            .filter(|s| !s.is_empty())
            .take_while(|s| !s.contains(['*', '?', '[', '{']))
            .flat_map(|s| [s, "/"])
            .collect();
        let below = rel.strip_prefix(fixed.as_str()).unwrap_or(rel);
        below.rsplit_once('.').map_or(below, |(stem, _)| stem)
    }

    /// Every report it matches that `run` wrote: in `$FEARLEZZ_OUT` for a glob that starts
    /// there, otherwise from the check's folder (or the root), changed since the run began.
    /// None is an error saying `what` it looked for.
    fn reports(&self, run: &Run<'_>, what: &str) -> Result<Vec<Report>, String> {
        let folder = run.folder.map(|f| run.root.join(f));
        let base = if self.written.starts_with(OUT) {
            run.out
        } else {
            folder.as_deref().unwrap_or(run.root)
        };
        let reports: Vec<Report> = self
            .matches(base, false)
            .into_iter()
            .map(|rel| base.join(rel))
            .filter(|path| {
                std::fs::metadata(path)
                    .and_then(|m| m.modified())
                    .is_ok_and(|m| m >= run.since)
            })
            .filter_map(|path| {
                let text = std::fs::read_to_string(&path).ok()?;
                Some(Report { path, text })
            })
            .collect();
        if reports.is_empty() {
            return Err(format!(
                "no {what} matching {} written by this run; the runner wrote none, or wrote it \
                 elsewhere",
                self.written
            ));
        }
        Ok(reports)
    }
}

/// Where a path a tool names is.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Place {
    /// A file of the checkout, by its path from the root.
    Repo(String),
    /// An absolute path outside the checkout (a dependency, the toolchain).
    Outside(String),
    /// A path from the checkout that names no file there: written from another folder.
    Lost(String),
}

impl Run<'_> {
    /// Where a path a tool names is: a `file://` URI or absolute path inside the checkout,
    /// or a relative path from the check's folder first (the root may hold a file of the same
    /// name), then from the root.
    pub(crate) fn place(&self, file: &str) -> Place {
        let decoded;
        let file = match file.trim().strip_prefix("file://") {
            Some(uri) => {
                decoded = percent_decode(uri);
                decoded.as_str()
            }
            None => file.trim(),
        };
        let path = Path::new(file);
        if path.is_absolute() {
            return match path.strip_prefix(self.root) {
                Ok(rel) => {
                    let rel = rel.to_string_lossy().to_string();
                    if self.root.join(&rel).is_file() {
                        Place::Repo(rel)
                    } else {
                        Place::Lost(rel)
                    }
                }
                Err(_) => Place::Outside(file.to_string()),
            };
        }
        let file = file.trim_start_matches("./");
        self.folder
            .map(|f| format!("{f}/{file}"))
            .into_iter()
            .chain([file.to_string()])
            .find(|p| self.root.join(p).is_file())
            .map_or_else(|| Place::Lost(file.to_string()), Place::Repo)
    }
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
        if let (b'%', Some(v)) = (b, hex) {
            out.push(v);
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run<'a>(root: &'a Path, folder: Option<&'a str>) -> Run<'a> {
        Run {
            out: root,
            root,
            folder,
            since: SystemTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn paths_are_placed_from_the_folder_then_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("app/src")).unwrap();
        std::fs::create_dir_all(root.join("my dir")).unwrap();
        for file in ["app/src/a.rs", "src.rs", "app/src.rs", "my dir/a.rs"] {
            std::fs::write(root.join(file), "").unwrap();
        }
        let at = run(root, Some("app"));
        let repo = |p: &str| Place::Repo(p.into());
        assert_eq!(at.place("src.rs"), repo("app/src.rs"));
        assert_eq!(at.place("./src/a.rs"), repo("app/src/a.rs"));
        assert_eq!(run(root, None).place("src.rs"), repo("src.rs"));
        let uri = format!("file://{}/my%20dir/a.rs", root.display());
        assert_eq!(at.place(&uri), repo("my dir/a.rs"));
        assert_eq!(
            at.place("/rustc/x.rs"),
            Place::Outside("/rustc/x.rs".into())
        );
        assert_eq!(at.place("gone.rs"), Place::Lost("gone.rs".into()));
        let missing = format!("{}/gone.rs", root.display());
        assert_eq!(at.place(&missing), Place::Lost("gone.rs".into()));
    }

    #[test]
    fn a_match_is_named_below_the_fixed_folders() {
        let glob = Glob::parse("$FEARLEZZ_OUT/stills/**/*.png", "stills").unwrap();
        assert_eq!(glob.id("stills/desktop/home.png"), "desktop/home");
        assert_eq!(glob.written(), "$FEARLEZZ_OUT/stills/**/*.png");
        let flat = Glob::parse("*.txt", "texts").unwrap();
        assert_eq!(flat.id("plan.txt"), "plan");
    }

    #[test]
    fn a_bad_glob_names_the_key_and_the_glob() {
        let Err(e) = Glob::parse("[", "tests junit") else {
            panic!("a lone [ is no glob");
        };
        assert!(format!("{e:#}").starts_with("tests junit \"[\": "), "{e:#}");
    }

    #[test]
    fn a_glob_reads_only_reports_this_run_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("node_modules/x")).unwrap();
        std::fs::write(root.join("node_modules/x/junit.xml"), "theirs").unwrap();
        std::fs::write(root.join("junit.xml"), "ours").unwrap();
        let glob = Glob::parse("**/junit.xml", "tests junit").unwrap();
        let at = run(root, None);
        let reports = Source::Glob(&glob).read(&at, "JUnit report").unwrap();
        let texts: Vec<&str> = reports.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(texts, ["ours"]);
        let later = Run {
            since: SystemTime::now() + std::time::Duration::from_secs(60),
            ..at
        };
        let Err(why) = Source::Glob(&glob).read(&later, "JUnit report") else {
            panic!("a report from before the run is not read");
        };
        assert_eq!(
            why,
            "no JUnit report matching **/junit.xml written by this run; the runner wrote none, \
             or wrote it elsewhere"
        );
    }
}
