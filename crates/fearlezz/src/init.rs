//! `fearlezz setup init`: the starter `fearlezz.toml` for a repository that has none, from the
//! languages the files at its root show.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result, bail};

use crate::config::FILE;

/// Pieces that bring no checks of their own names, so any number of them combine.
const TOOLCHAINS: &[&str] = &["devenv"];

/// Each built in piece and the files at a repository's root that show its language, in the
/// order the starter names them. devenv comes first: it gives the others their toolchain.
const MARKERS: &[(&str, &[&str])] = &[
    ("devenv", &["devenv.nix"]),
    ("rust", &["Cargo.toml"]),
    ("typescript", &["package.json"]),
    (
        "python",
        &["pyproject.toml", "setup.py", "requirements.txt"],
    ),
    ("go", &["go.mod"]),
    (
        "kotlin",
        &[
            "settings.gradle.kts",
            "settings.gradle",
            "build.gradle.kts",
            "build.gradle",
        ],
    ),
    ("flutter", &["pubspec.yaml"]),
];

/// Prose needs no proof. Written from the first commit on, so the base branch already has it
/// when the first change to docs alone comes.
const NO_PROOF: &str = r#"no_proof = ["*.md", "docs/**"]"#;

/// What `init` wrote: the pieces it used, and the languages it found but left to the developer.
pub(crate) struct Started {
    pub pieces: Vec<Found>,
    pub others: Vec<Found>,
}

/// A piece and the file at the root that showed it.
pub(crate) struct Found {
    pub piece: &'static str,
    pub file: &'static str,
}

impl std::fmt::Display for Found {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.piece, self.file)
    }
}

/// Writes the starter `fearlezz.toml` in `root`; never over one that is there.
pub(crate) fn init(root: &Path) -> Result<Started> {
    let path = root.join(FILE);
    if path.exists() {
        bail!(
            "{} already has a fearlezz.toml; change it by hand, or move it away to start over",
            root.display()
        );
    }
    let mut started = Started {
        pieces: Vec::new(),
        others: Vec::new(),
    };
    for (piece, files) in MARKERS {
        let Some(file) = files.iter().find(|f| root.join(f).is_file()) else {
            continue;
        };
        let found = Found { piece, file };
        // Language pieces name their checks alike (tests, lint, format), so a second one
        // would merge into the first's checks instead of adding its own.
        let language = !TOOLCHAINS.contains(piece);
        if language
            && started
                .pieces
                .iter()
                .any(|p| !TOOLCHAINS.contains(&p.piece))
        {
            started.others.push(found);
        } else {
            started.pieces.push(found);
        }
    }
    std::fs::write(&path, starter(&started)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(started)
}

fn starter(started: &Started) -> Result<String, std::fmt::Error> {
    let pieces = &started.pieces;
    let mut text = String::from(
        "# The proof this project's changes need. `fearlezz --plan` shows which checks a change\n\
         # needs; the agent skill (`fearlezz setup sync` writes it) explains the rest.\n\n",
    );
    if pieces.is_empty() {
        text.push_str(
            "# No language was found. Declare each check: a command that exits 0 on success, and\n\
             # every file it reads.\n\
             # [check.tests]\n\
             # run = \"make test\"\n\
             # stage = \"working\"\n\
             # inputs = [\"src/**\", \"Makefile\"]\n\n",
        );
    } else {
        let names: Vec<String> = pieces.iter().map(|p| format!("\"{}\"", p.piece)).collect();
        writeln!(
            text,
            "# Built in pieces; override any field below, or drop a check with `off = true`.\n\
             use = [{}]\n",
            names.join(", ")
        )?;
    }
    if !started.others.is_empty() {
        let others: Vec<String> = started.others.iter().map(ToString::to_string).collect();
        writeln!(
            text,
            "# Also found: {}.\n\
             # Language pieces name their checks alike (tests, lint, format), so these need\n\
             # checks of their own names, written here, until pieces can be combined.\n",
            others.join(", ")
        )?;
    }
    writeln!(text, "{NO_PROOF}")?;
    Ok(text)
}
