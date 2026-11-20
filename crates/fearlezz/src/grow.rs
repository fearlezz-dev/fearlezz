//! What a project could prove but does not yet, so agents grow the proof: a UI no check takes
//! pictures of, rendered output no check keeps as texts, and pictures the README does not
//! show. Read from the repository's files and the config's checks, never stored.
//!
//! Each is one line: `setup init` lists them for the agent to offer the developer, and while
//! working the built in `settings` check gives one as a hint when the change touches files of
//! its kind. `[lints.fearlezz] <rule> = "off"` turns one off for a repository.

use std::path::Path;

use crate::config::Check;
use crate::files::has_extension;
use crate::findings::{Finding, Level};
use crate::lints::{Lints, Tool};

/// One kind of proof a project could add, by its rule name in `[lints.fearlezz]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// A UI and no check that takes stills or films.
    Pictures,
    /// Rendered output (manifests, charts) and no check that keeps texts.
    Texts,
    /// Checks take pictures, and the README shows none of them.
    ReadmeMedia,
}

impl Kind {
    pub(crate) fn rule(self) -> &'static str {
        match self {
            Kind::Pictures => "pictures",
            Kind::Texts => "texts",
            Kind::ReadmeMedia => "readme-media",
        }
    }
}

/// A kind of proof the project could add, with what shows it.
#[derive(Clone, Debug)]
pub(crate) struct Opportunity {
    pub kind: Kind,
    /// What is there and what to add, in one line.
    pub offer: String,
    /// Whether a change to a file makes the offer timely while working.
    touches: fn(&str) -> bool,
}

/// What the config proves already, read once from its checks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Have {
    /// A check takes stills or films (by glob or by recording a window).
    pub pictures: bool,
    /// A check keeps texts.
    pub texts: bool,
    /// The checks that render output a person could review, such as `kustomize`.
    pub rendered: Vec<String>,
}

/// The checks of the pieces whose tools render output: manifests, charts, a plan.
const RENDERING: &[&str] = &[
    "kustomize",
    "helm",
    "flate",
    "terraform-validate",
    "tofu-validate",
];

impl Have {
    pub(crate) fn of(checks: &[Check]) -> Have {
        Have {
            pictures: checks.iter().any(|c| c.pictures.names_media()),
            texts: checks.iter().any(|c| c.pictures.texts.is_some()),
            rendered: checks
                .iter()
                .filter(|c| {
                    let piece = c.id.split('/').next().unwrap_or_default();
                    RENDERING.contains(&piece)
                })
                .map(|c| c.id.clone())
                .collect(),
        }
    }
}

fn web_file(path: &str) -> bool {
    has_extension(
        path,
        &[
            "tsx", "jsx", "vue", "svelte", "astro", "css", "scss", "html",
        ],
    )
}

fn dart_file(path: &str) -> bool {
    path.starts_with("lib/") && has_extension(path, &["dart"])
}

fn swift_file(path: &str) -> bool {
    has_extension(path, &["swift"])
}

fn rendered_file(path: &str) -> bool {
    has_extension(path, &["yaml", "yml", "tf", "tpl"])
}

fn readme(path: &str) -> bool {
    path == "README.md"
}

/// The packages in package.json that show a web UI.
const WEB: &[&str] = &[
    "react",
    "next",
    "vue",
    "svelte",
    "@sveltejs/kit",
    "solid-js",
    "@builder.io/qwik",
    "@qwik.dev/core",
];

/// What the project in `root`, with `files` (every file git lists), could prove but does not.
/// `host` is the proof host's URL, which a README showing the proof's pictures names.
pub(crate) fn find(root: &Path, files: &[String], have: &Have, host: &str) -> Vec<Opportunity> {
    let mut found = Vec::new();
    if !have.pictures {
        found.extend(uis(root, files));
    }
    if !have.texts && !have.rendered.is_empty() {
        found.push(Opportunity {
            kind: Kind::Texts,
            offer: format!(
                "{} renders output, and no check keeps it as texts: write it into \
                 $FEARLEZZ_OUT and add texts = \"rendered/**\" to the check, so a person reviews \
                 the rendered diff",
                have.rendered.join(", ")
            ),
            touches: rendered_file,
        });
    }
    // The proof host will give each picture a stable URL for READMEs; until then, any link to
    // the host counts as showing the proof. Once it exists, name its pattern here and in the
    // offer.
    if have.pictures
        && let Ok(text) = std::fs::read_to_string(root.join("README.md"))
        && !text.contains(host)
    {
        found.push(Opportunity {
            kind: Kind::ReadmeMedia,
            offer: "checks take stills and films, and README.md shows none: offer to show the \
                    proof's stills and films in README.md"
                .into(),
            touches: readme,
        });
    }
    found
}

/// The UIs the project has, each with the picture proof that fits it.
fn uis(root: &Path, files: &[String]) -> Vec<Opportunity> {
    let pictures = |offer: String, touches: fn(&str) -> bool| Opportunity {
        kind: Kind::Pictures,
        offer,
        touches,
    };
    let mut found = Vec::new();
    let depends = crate::init::depends(root);
    if root.join("src-tauri").is_dir() {
        found.push(pictures(
            "a Tauri app (src-tauri), and no check takes stills or films of it: offer a check \
             that starts it with record = { still = \"home\", film = \"home\" }, or \
             use = [\"playwright\"] for its web view"
                .into(),
            web_file,
        ));
    } else if let Some(package) = WEB.iter().find(|p| depends(p)) {
        found.push(pictures(
            format!(
                "a web UI ({package} in package.json), and no check takes stills or films of \
                 it: offer use = [\"playwright\"], whose toHaveScreenshot calls are stills and \
                 videos films of its flows"
            ),
            web_file,
        ));
    }
    if crate::init::flutter_app(root) {
        found.push(pictures(
            "a Flutter app (pubspec.yaml), and no check takes stills or films of it: offer \
             use = [\"flutter-look\"], stills of its widgets, and flutter-film to film its flows"
                .into(),
            dart_file,
        ));
    }
    let xcode = files
        .iter()
        .any(|f| f.ends_with(".xcodeproj/project.pbxproj"));
    let swiftui = || {
        files.iter().filter(|f| swift_file(f)).any(|f| {
            std::fs::read_to_string(root.join(f)).is_ok_and(|t| t.contains("import SwiftUI"))
        })
    };
    if xcode || swiftui() {
        found.push(pictures(
            "an Apple app (Xcode or SwiftUI), and no check takes stills or films of it: offer a \
             check with record = { still = \"home\", film = \"home\" } on its macOS window, or \
             needs = [\"ios\"] with record for the simulator"
                .into(),
            swift_file,
        ));
    }
    found
}

/// A hint for each opportunity the change makes timely, touching `changed` files of its kind,
/// at the level `[lints.fearlezz]` gives its rule; one that is off is left out.
pub(crate) fn hints(found: &[Opportunity], changed: &[&str], lints: &Lints) -> Vec<Finding> {
    let hints = found
        .iter()
        .filter_map(|o| {
            let file = changed.iter().find(|f| (o.touches)(f))?;
            Some(Finding {
                file: (*file).to_string(),
                line: 1,
                end_line: None,
                level: Level::Hint,
                rule: o.kind.rule().into(),
                message: format!("this change could prove more: {}", o.offer),
                fix: Some(format!(
                    "ask the developer whether to add it (fearlezz say ask); [lints.fearlezz] {} \
                     = \"off\" stops this hint",
                    o.kind.rule()
                )),
                why: Some("proof that people can see grows with the project".into()),
                anywhere: true,
            })
        })
        .collect();
    lints.apply(&[Tool::Fearlezz], hints)
}

/// Every file whose change can make an opportunity timely, as globs: the settings check reads
/// them, so it runs again when one changes.
pub(crate) const GLOBS: &[&str] = &[
    "**/*.tsx",
    "**/*.jsx",
    "**/*.vue",
    "**/*.svelte",
    "**/*.astro",
    "**/*.css",
    "**/*.scss",
    "**/*.html",
    "lib/**/*.dart",
    "**/*.swift",
    "**/*.yaml",
    "**/*.yml",
    "**/*.tf",
    "**/*.tpl",
    "README.md",
    "package.json",
    "pubspec.yaml",
];
