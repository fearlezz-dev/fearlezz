//! What a project could prove but does not yet, so agents grow the proof: a UI no check takes
//! pictures of, rendered output no check keeps as texts, and pictures the README does not
//! show. Read from the repository's files and the config's checks, never stored.
//!
//! Each is one line: `setup init` lists them for the agent to offer the developer, and while
//! working the built in `settings` check gives one as a hint when the change touches files of
//! its kind. A hint only offers, so it never blocks: `[lints.fearlezz] <rule> = "off"` turns
//! one off for a repository, and any other level leaves it a hint.

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
    /// The folders such a file must be in to count; none means anywhere.
    within: Vec<String>,
    /// The folders such a file must not be in to count.
    outside: Vec<String>,
}

impl Opportunity {
    fn new(kind: Kind, offer: String, touches: fn(&str) -> bool) -> Opportunity {
        Opportunity {
            kind,
            offer,
            touches,
            within: Vec::new(),
            outside: Vec::new(),
        }
    }

    /// Whether a change to `path` makes the offer timely.
    fn touched_by(&self, path: &str) -> bool {
        (self.touches)(path)
            && (self.within.is_empty() || self.within.iter().any(|d| inside(path, d)))
            && !self.outside.iter().any(|d| inside(path, d))
    }
}

/// Whether `path` is inside the folder `dir`, the root being `""`.
fn inside(path: &str, dir: &str) -> bool {
    dir.is_empty()
        || path
            .strip_prefix(dir)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// What the config proves already, read once from its checks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Have {
    /// A check takes stills or films (by glob or by recording a window).
    pub pictures: bool,
    /// A check keeps texts.
    pub texts: bool,
    /// The checks that render output a person could review, such as `kustomize`.
    pub rendered: Vec<Rendering>,
}

/// A check that renders output, and the folder it renders from (`""` for the root).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Rendering {
    pub id: String,
    pub dir: String,
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
                .map(|c| Rendering {
                    id: c.id.clone(),
                    dir: c.command.dir.clone().unwrap_or_default(),
                })
                .collect(),
        }
    }

    /// The kinds [`find`] could offer with what the config proves already.
    pub(crate) fn could(&self) -> Vec<Kind> {
        let mut kinds = Vec::new();
        if !self.pictures {
            kinds.push(Kind::Pictures);
        }
        if !self.texts && !self.rendered.is_empty() {
            kinds.push(Kind::Texts);
        }
        if self.pictures {
            kinds.push(Kind::ReadmeMedia);
        }
        kinds
    }

    /// The folders the rendering checks render from, each once.
    fn rendered_dirs(&self) -> Vec<String> {
        let mut dirs: Vec<String> = self.rendered.iter().map(|r| r.dir.clone()).collect();
        dirs.sort();
        dirs.dedup();
        dirs
    }

    /// Every file whose change can make an offer of `kinds` timely or change what is offered,
    /// as globs: the settings check reads them, so it runs again when one changes.
    pub(crate) fn globs(&self, kinds: &[Kind]) -> Vec<String> {
        let mut globs = Vec::new();
        for kind in kinds {
            match kind {
                Kind::Pictures => globs.extend(
                    [
                        "**/*.tsx",
                        "**/*.jsx",
                        "**/*.vue",
                        "**/*.svelte",
                        "**/*.astro",
                        "**/*.css",
                        "**/*.scss",
                        "**/*.html",
                        "**/lib/**/*.dart",
                        "**/*.swift",
                        "**/project.pbxproj",
                        "package.json",
                        "**/pubspec.yaml",
                    ]
                    .map(String::from),
                ),
                Kind::Texts => {
                    for dir in self.rendered_dirs() {
                        let at = slashed(&dir);
                        for ext in RENDERED {
                            globs.push(format!("{at}**/*.{ext}"));
                        }
                    }
                }
                Kind::ReadmeMedia => globs.push("README.md".into()),
            }
        }
        globs
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

/// A Dart file in a `lib` folder, where a Flutter app's widgets are.
fn dart_file(path: &str) -> bool {
    (path.starts_with("lib/") || path.contains("/lib/")) && has_extension(path, &["dart"])
}

fn swift_file(path: &str) -> bool {
    has_extension(path, &["swift"])
}

/// The extensions of what rendering checks render from.
const RENDERED: [&str; 4] = ["yaml", "yml", "tf", "tpl"];

/// A file a rendering check renders from; GitHub's own files never are.
fn rendered_file(path: &str) -> bool {
    has_extension(path, &RENDERED) && !path.starts_with(".github/")
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
        let ids: Vec<&str> = have.rendered.iter().map(|r| r.id.as_str()).collect();
        found.push(Opportunity {
            within: have.rendered_dirs(),
            ..Opportunity::new(
                Kind::Texts,
                format!(
                    "{} renders output, and no check keeps it as texts: write it into \
                     $FEARLEZZ_OUT and add texts = \"rendered/**\" to the check, so a person \
                     reviews the rendered diff",
                    ids.join(", ")
                ),
                rendered_file,
            )
        });
    }
    // A README shows the proof's pictures through their stable addresses on the files origin,
    // or through any link to the host.
    if have.pictures
        && let Ok(text) = std::fs::read_to_string(root.join("README.md"))
        && !text.contains(host)
        && !text.contains(&format!("{}/showcase/", crate::host::FILES))
    {
        found.push(Opportunity::new(
            Kind::ReadmeMedia,
            format!(
                "checks take stills and films, and README.md shows none: fearlezz setup \
                 showcase prints the markup that shows each from \
                 {}/showcase/<owner>/<repo>/<check>/<picture>, the picture from the newest \
                 passing proof of the default branch (Fearlezz's README, Showcase media)",
                crate::host::FILES
            ),
            readme,
        ));
    }
    found
}

/// The UIs the project has, each with the picture proof that fits it.
fn uis(root: &Path, files: &[String]) -> Vec<Opportunity> {
    let pictures =
        |offer: String, touches: fn(&str) -> bool| Opportunity::new(Kind::Pictures, offer, touches);
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
    // Each Flutter app's folder, the root as `""`, and those no other app holds.
    let apps: Vec<String> = crate::init::flutter_apps(root, files)
        .into_iter()
        .map(|f| if f == "." { String::new() } else { f })
        .collect();
    let outer: Vec<String> = apps
        .iter()
        .filter(|d| !apps.iter().any(|o| o != *d && inside(d, o)))
        .cloned()
        .collect();
    if !outer.is_empty() {
        let pubspecs: Vec<String> = outer
            .iter()
            .map(|d| format!("{}pubspec.yaml", slashed(d)))
            .collect();
        // The flutter-look checks and helpers are the root's alone for now.
        let folder = outer
            .iter()
            .find(|d| !d.is_empty())
            .map(|d| {
                format!(
                    " (its checks run at the root, so an app in a folder needs dir = \"{d}\" \
                     on them and the look_test helpers in {d})"
                )
            })
            .unwrap_or_default();
        found.push(Opportunity {
            within: outer.clone(),
            ..pictures(
                format!(
                    "a Flutter app ({}), and no check takes stills or films of it: offer \
                     use = [\"flutter-look\"]{folder}, stills of its widgets, and flutter-film \
                     to film its flows",
                    pubspecs.join(", ")
                ),
                dart_file,
            )
        });
    }
    // Flutter generates an Xcode project in each Apple platform folder of its app; those are
    // the Flutter app's, not an Apple app of their own.
    let generated: Vec<String> = apps
        .iter()
        .flat_map(|d| ["macos", "ios"].map(|p| format!("{}{p}", slashed(d))))
        .collect();
    let own = |f: &&String| !generated.iter().any(|d| inside(f, d));
    let xcode = files
        .iter()
        .filter(own)
        .any(|f| f.ends_with(".xcodeproj/project.pbxproj"));
    let swiftui = || {
        files.iter().filter(own).filter(|f| swift_file(f)).any(|f| {
            std::fs::read_to_string(root.join(f)).is_ok_and(|t| t.contains("import SwiftUI"))
        })
    };
    if xcode || swiftui() {
        found.push(Opportunity {
            outside: generated.clone(),
            ..pictures(
                "an Apple app (Xcode or SwiftUI), and no check takes stills or films of it: offer \
                 a check with record = { still = \"home\", film = \"home\" } on its macOS \
                 window, or needs = [\"ios\"] with record for the simulator"
                    .into(),
                swift_file,
            )
        });
    }
    found
}

/// The folder `dir` as a path prefix: `""` for the root, `"<dir>/"` otherwise.
fn slashed(dir: &str) -> String {
    if dir.is_empty() {
        String::new()
    } else {
        format!("{dir}/")
    }
}

/// A hint for each opportunity the change makes timely, touching `changed` files of its kind;
/// one `[lints.fearlezz]` turns off is left out, and any other level stays a hint.
pub(crate) fn hints(found: &[Opportunity], changed: &[&str], lints: &Lints) -> Vec<Finding> {
    let hints = found
        .iter()
        .filter_map(|o| {
            let file = changed.iter().find(|f| o.touched_by(f))?;
            Some(Finding {
                file: (*file).to_string(),
                line: 1,
                end_line: None,
                level: Level::Hint,
                rule: o.kind.rule().into(),
                message: format!("this change could prove more: {}", o.offer),
                fix: Some(format!(
                    "the developer decides whether to add it; [lints.fearlezz] {} = \"off\" \
                     stops this hint",
                    o.kind.rule()
                )),
                why: Some("proof that people can see grows with the project".into()),
                anywhere: true,
            })
        })
        .collect();
    let mut hints = lints.apply(&[Tool::Fearlezz], hints);
    for hint in &mut hints {
        hint.level = Level::Hint;
    }
    hints
}
