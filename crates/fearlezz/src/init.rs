//! `fearlezz setup init`: the starter `fearlezz.toml` for a repository that has none, from the
//! languages the files at its root show (Rust also from Cargo projects in folders), the
//! frameworks its package.json depends on, and the infrastructure tools its files show.

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
    // A Dart package or a Flutter app; `refine` tells which.
    ("flutter", &["pubspec.yaml"]),
    ("lua", &["stylua.toml", ".stylua.toml"]),
    // A folder, not a file: zizmor audits the workflows GitHub runs.
    ("actions", &[".github/workflows"]),
];

/// The framework pieces and the packages in package.json that show each, in the order the
/// starter names them, after the linters: Biome and oxlint run every rule they are given, so a
/// framework's rules come only with its piece. A Next app depends on react too, and gets both.
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

/// The pieces of the linters and formatters other than Biome, and the package in package.json
/// that shows each, in the order the starter names them.
const TOOLS: &[(&str, &str)] = &[
    ("eslint", "eslint"),
    ("oxlint", "oxlint"),
    ("prettier", "prettier"),
];

/// The pieces a TypeScript project in `root` gets after `typescript`: the linters and
/// formatters package.json depends on, or Biome when it names Biome or none of them; the
/// pieces of its frameworks, when Biome or oxlint lints; then its test runner's.
fn javascript(root: &Path) -> Vec<&'static str> {
    let depends = depends(root);
    let tools: Vec<&'static str> = TOOLS
        .iter()
        .filter(|(_, package)| depends(package))
        .map(|(piece, _)| *piece)
        .collect();
    let biome = depends("@biomejs/biome") || tools.is_empty();
    // The framework pieces hold the rules of Biome and oxlint, so either brings them.
    let frameworks = if biome || tools.contains(&"oxlint") {
        frameworks(root)
    } else {
        Vec::new()
    };
    biome
        .then_some("biome")
        .into_iter()
        .chain(tools)
        .chain(frameworks)
        .chain(runner(root))
        .collect()
}

/// The piece of the test runner a TypeScript project in `root` uses: Vitest when package.json
/// depends on it, as a Bun project running Vitest does; otherwise Node's own runner when the
/// test script runs it; otherwise Bun's own runner when a Bun lockfile shows Bun; none without
/// a sign of one.
fn runner(root: &Path) -> Option<&'static str> {
    if depends(root)("vitest") {
        Some("vitest")
    } else if node_test(root) {
        Some("node-test")
    } else if ["bun.lock", "bun.lockb"]
        .iter()
        .any(|f| root.join(f).exists())
    {
        Some("bun")
    } else {
        None
    }
}

/// The piece a marker's language takes in `root`: svelte (svelte-check in place of tsc) for a
/// SvelteKit project, dart for a pubspec.yaml without Flutter.
fn refine(root: &Path, piece: &'static str) -> &'static str {
    match piece {
        // The piece runs `svelte-kit sync` first; a Svelte app without SvelteKit keeps tsc.
        "typescript" if depends(root)("@sveltejs/kit") => "svelte",
        "flutter" => {
            let pubspec = std::fs::read_to_string(root.join("pubspec.yaml")).unwrap_or_default();
            let flutter = pubspec
                .lines()
                .any(|line| line.trim().replace(' ', "") == "sdk:flutter");
            if flutter { "flutter" } else { "dart" }
        }
        _ => piece,
    }
}

/// Whether go.mod in `root` requires `module`.
fn go_requires(root: &Path, module: &str) -> bool {
    std::fs::read_to_string(root.join("go.mod")).is_ok_and(|text| {
        text.lines().any(|line| {
            let line = line.trim().trim_start_matches("require").trim();
            line.split_whitespace().next() == Some(module)
        })
    })
}

/// Whether package.json's test script runs Node's own test runner (`node --test`, also with
/// options such as a loader before `--test`).
fn node_test(root: &Path) -> bool {
    let Some(json) = package_json(root) else {
        return false;
    };
    json.pointer("/scripts/test")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|script| {
            let mut words = script.split_whitespace();
            words.next() == Some("node") && words.any(|w| w == "--test")
        })
}

/// `package.json` in `root` as JSON; none when it is missing or not JSON.
fn package_json(root: &Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(root.join("package.json")).ok()?;
    serde_json::from_str(&text).ok()
}

/// Whether `package.json` in `root` depends on a package (its dependencies or
/// devDependencies). A file that cannot be read as JSON depends on none.
fn depends(root: &Path) -> impl Fn(&str) -> bool {
    let json = package_json(root);
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

/// A piece `init` used and the file that showed it (at the root, or a Cargo project's in a
/// folder), none for a piece every project gets.
pub(crate) struct Found {
    pub piece: &'static str,
    pub file: Option<String>,
}

impl std::fmt::Display for Found {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.file {
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
    // The rust piece's checks run in every Cargo project (`each`), so one below the root
    // shows Rust too, as an app with a native shell and a sandbox in their own folders has.
    let files = crate::git::Git::discover(root)
        .and_then(|git| git.files())
        .unwrap_or_default();
    let below = |piece: &str| -> Option<String> {
        if piece != "rust" {
            return None;
        }
        let none = crate::config::Globs::none();
        let folder = crate::config::projects(root, &files, "Cargo.toml", &none)
            .into_iter()
            .next()?;
        Some(format!("{folder}/Cargo.toml"))
    };
    let found: Vec<Found> = MARKERS
        .iter()
        .filter_map(|(piece, files)| {
            let file = files
                .iter()
                .find(|f| root.join(f).exists())
                .map(|f| (*f).to_string())
                .or_else(|| below(piece))?;
            Some((*piece, file))
        })
        .flat_map(|(piece, file)| {
            // A TypeScript project also gets its linters', formatters' and test runner's pieces,
            // a Go operator envtest.
            let more = if piece == "typescript" {
                javascript(root)
            } else if piece == "go" && go_requires(root, "sigs.k8s.io/controller-runtime") {
                // An operator's controller tests run against envtest's API server.
                vec!["envtest"]
            } else {
                Vec::new()
            };
            let piece = refine(root, piece);
            std::iter::once(piece).chain(more).map(move |piece| Found {
                piece,
                file: Some(file.clone()),
            })
        })
        .chain(infra(root, &files))
        .chain([Found {
            piece: SAFETY,
            file: None,
        }])
        .collect();
    std::fs::write(&path, starter(&found)?)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(found)
}

/// The infrastructure pieces, after the languages, each from a file its tool reads anywhere
/// in the repository: the kustomize, helm and terraform checks run in every folder that has
/// one (`each`). A Flux repository gets flux instead of kustomize, since flate renders every
/// Kustomization; a lockfile naming OpenTofu's registry gets opentofu instead of terraform.
fn infra(root: &Path, files: &[String]) -> Vec<Found> {
    let none = crate::config::Globs::none();
    let first = |marker: &str| -> Option<String> {
        let folder = crate::config::projects(root, files, marker, &none)
            .into_iter()
            .next()?;
        Some(if folder == "." {
            marker.to_string()
        } else {
            format!("{folder}/{marker}")
        })
    };
    let at_root = |names: &[&str]| {
        names
            .iter()
            .find(|n| root.join(n).exists())
            .map(|n| (*n).to_string())
    };
    let flux = ["gotk-sync.yaml", "gotk-components.yaml"]
        .iter()
        .find_map(|m| first(m));
    let lock = first(".terraform.lock.hcl");
    let tofu = lock.as_ref().is_some_and(|l| {
        std::fs::read_to_string(root.join(l)).is_ok_and(|t| t.contains("registry.opentofu.org"))
    });
    [
        ("nix", at_root(&["flake.nix"])),
        ("flux", flux.clone()),
        (
            "kustomize",
            flux.is_none()
                .then(|| first("kustomization.yaml"))
                .flatten(),
        ),
        ("helm", first("Chart.yaml")),
        (if tofu { "opentofu" } else { "terraform" }, lock),
        (
            "yamllint",
            at_root(&[".yamllint", ".yamllint.yaml", ".yamllint.yml"]),
        ),
        ("sops", first(".sops.yaml")),
    ]
    .into_iter()
    .filter_map(|(piece, file)| {
        Some(Found {
            piece,
            file: Some(file?),
        })
    })
    .collect()
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
