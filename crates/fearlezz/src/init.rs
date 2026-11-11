//! `fearlezz setup init`: the starter `fearlezz.toml` for a repository that has none, from the
//! languages the files at its root show.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result, bail};

use crate::config::FILE;

/// Each built in piece and the files at a repository's root that show its language, in the
/// order the starter names them. devenv comes first: it gives the others their toolchain.
/// The pieces name their checks after their tools, so any of them combine.
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

/// A piece `init` used and the file at the root that showed it.
pub(crate) struct Found {
    pub piece: &'static str,
    pub file: &'static str,
}

impl std::fmt::Display for Found {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.piece, self.file)
    }
}

/// Writes the starter `fearlezz.toml` in `root`, never over one that is there, and returns
/// the pieces it used.
pub(crate) fn init(root: &Path) -> Result<Vec<Found>> {
    let path = root.join(FILE);
    if path.exists() {
        bail!(
            "{} already has a fearlezz.toml; change it by hand, or move it away to start over",
            root.display()
        );
    }
    let found: Vec<Found> = MARKERS
        .iter()
        .filter_map(|(piece, files)| {
            let file = files.iter().find(|f| root.join(f).is_file())?;
            Some(Found { piece, file })
        })
        .collect();
    std::fs::write(&path, starter(&found)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(found)
}

fn starter(found: &[Found]) -> Result<String, std::fmt::Error> {
    let mut text = String::from(
        "# The proof this project's changes need. `fearlezz --plan` shows which checks a change\n\
         # needs; the agent skill (`fearlezz setup sync` writes it) explains the rest.\n\n",
    );
    if found.is_empty() {
        text.push_str(
            "# No language was found. Declare each check: a command that exits 0 on success, and\n\
             # every file it reads.\n\
             # [check.tests]\n\
             # run = \"make test\"\n\
             # stage = \"working\"\n\
             # inputs = [\"src/**\", \"Makefile\"]\n\n",
        );
    } else {
        let names: Vec<String> = found.iter().map(|p| format!("\"{}\"", p.piece)).collect();
        writeln!(
            text,
            "# Built in pieces; override any field below, or drop a check with `off = true`.\n\
             use = [{}]\n",
            names.join(", ")
        )?;
    }
    writeln!(text, "{NO_PROOF}")?;
    Ok(text)
}
