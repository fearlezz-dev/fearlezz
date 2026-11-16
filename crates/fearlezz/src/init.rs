//! `fearlezz setup init`: the starter `fearlezz.toml` for a repository that has none, from the
//! languages the files at its root show, and the frameworks its package.json depends on.

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
    // A folder, not a file: zizmor audits the workflows GitHub runs.
    ("actions", &[".github/workflows"]),
];

/// The framework pieces and the packages in package.json that show each, in the order the
/// starter names them, after `typescript`: Biome runs every rule it is given, so a framework's
/// rules come only with its piece. A Next app depends on react too, and gets both.
const FRAMEWORKS: &[(&str, &[&str])] = &[
    ("react", &["react"]),
    ("next", &["next"]),
    ("qwik", &["@builder.io/qwik", "@qwik.dev/core"]),
    ("solid", &["solid-js"]),
    ("vue", &["vue"]),
];

/// The framework pieces whose packages `package.json` in `root` depends on. The settings check
/// names one whose rules a project lacks.
pub(crate) fn frameworks(root: &Path) -> Vec<&'static str> {
    let depends = depends(root);
    FRAMEWORKS
        .iter()
        .filter(|(_, packages)| packages.iter().any(|p| depends(p)))
        .map(|(piece, _)| *piece)
        .collect()
}

/// The piece of the test runner a TypeScript project in `root` uses: Vitest when package.json
/// depends on it, as a Bun project running Vitest does; otherwise Bun's own runner when a Bun
/// lockfile shows Bun; none without a sign of either.
fn runner(root: &Path) -> Option<&'static str> {
    if depends(root)("vitest") {
        Some("vitest")
    } else if ["bun.lock", "bun.lockb"]
        .iter()
        .any(|f| root.join(f).exists())
    {
        Some("bun")
    } else {
        None
    }
}

/// Whether `package.json` in `root` depends on a package (its dependencies or
/// devDependencies). A file that cannot be read as JSON depends on none.
fn depends(root: &Path) -> impl Fn(&str) -> bool {
    let json = std::fs::read_to_string(root.join("package.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
    move |package| {
        json.as_ref().is_some_and(|json| {
            ["dependencies", "devDependencies"]
                .iter()
                .any(|key| json.get(key).and_then(|d| d.get(package)).is_some())
        })
    }
}

/// The piece every project gets, whatever it is written in: a leaked credential is never
/// acceptable. Last in the starter, after the pieces that give the others their toolchain.
const SAFETY: &str = "safety";

/// Prose needs no proof. Written from the first commit on, so the base branch already has it
/// when the first change to docs alone comes.
const NO_PROOF: &str = r#"no_proof = ["*.md", "docs/**"]"#;

/// A piece `init` used and the file at the root that showed it, none for a piece every
/// project gets.
pub(crate) struct Found {
    pub piece: &'static str,
    pub file: Option<&'static str>,
}

impl std::fmt::Display for Found {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.file {
            Some(file) => write!(f, "{} ({file})", self.piece),
            None => write!(f, "{} (every project)", self.piece),
        }
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
            let file = files.iter().find(|f| root.join(f).exists())?;
            Some((*piece, *file))
        })
        .flat_map(|(piece, file)| {
            // A TypeScript project also gets its frameworks' pieces and its test runner's.
            let more = if piece == "typescript" {
                frameworks(root).into_iter().chain(runner(root)).collect()
            } else {
                Vec::new()
            };
            std::iter::once(piece).chain(more).map(move |piece| Found {
                piece,
                file: Some(file),
            })
        })
        .chain([Found {
            piece: SAFETY,
            file: None,
        }])
        .collect();
    std::fs::write(&path, starter(&found)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(found)
}

fn starter(found: &[Found]) -> Result<String, std::fmt::Error> {
    let mut text = String::from(
        "# The proof this project's changes need. `fearlezz --plan` shows where each check stands;\n\
         # the agent skill (`fearlezz setup sync` writes it) explains the rest.\n\n",
    );
    if found.iter().all(|p| p.file.is_none()) {
        text.push_str(
            "# No language was found. Declare each check: a command that exits 0 on success.\n\
             # [check.tests]\n\
             # run = \"make test\"\n\
             # stage = \"working\"\n\n",
        );
    }
    let names: Vec<String> = found.iter().map(|p| format!("\"{}\"", p.piece)).collect();
    writeln!(
        text,
        "# Built in pieces; override any field below, or drop a check with `off = true`.\n\
         # `safety` scans for leaked secrets with gitleaks; pick `trufflehog` or `betterleaks`\n\
         # instead, or add `osv-scanner` for vulnerable dependencies.\n\
         use = [{}]\n",
        names.join(", ")
    )?;
    writeln!(text, "{NO_PROOF}")?;
    Ok(text)
}
