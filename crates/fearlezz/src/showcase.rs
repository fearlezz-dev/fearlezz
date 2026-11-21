//! `fearlezz setup showcase`: the markup that embeds each still and film of the work's last
//! run from its showcase address on the proof host's files origin,
//! `<files origin>/showcase/<owner>/<repo>/<check>/<picture>`, ready to paste into a README, a
//! docs site or a website. A repository whose proof host is off (`[host] off`) sends no
//! pictures, so it has none.
//!
//! A light and a dark picture, named apart only by one path segment `light` or `dark` (any
//! case, before the extension, as the proof page reads themes), become one `<picture>` whose
//! dark source follows `prefers-color-scheme`, which GitHub renders in a README too. A film is
//! named with `.webp` in place of its extension, the animated copy the host serves for it; a
//! text is never shown.

use crate::config::Config;
use crate::git::Git;
use crate::journal::Journal;
use crate::state::Worktree;
use crate::store::{RunResult, Store};

/// The width each picture is shown at.
const WIDTH: u32 = 720;

/// A still or film a check took, by its name in the check's look.
struct Taken {
    check: String,
    name: String,
    film: bool,
}

/// Prints the markup of every picture the work's last run took, or why there is none, and the
/// `next:` line.
pub(crate) fn print(git: &Git, config: &Config, store: &Store, journal: &Journal, host: &str) {
    if config.host_off {
        println!(
            "the proof host is off for this repository ([host] off = true), so it holds no \
             pictures to show"
        );
        println!(
            "next: remove off = true from [host] in fearlezz.toml to send the proof to the host, \
             then fearlezz setup showcase"
        );
        return;
    }
    let Some(repository) = git.github_repository() else {
        println!(
            "no GitHub repository: origin names none, and a showcase address names the \
             repository on GitHub"
        );
        println!("next: git remote add origin https://github.com/<owner>/<repo>.git");
        return;
    };
    if host != crate::host::DEFAULT {
        println!(
            "this repository sends its proof to {host}, whose files origin Fearlezz does not \
             know, so it cannot write the showcase addresses"
        );
        println!(
            "next: embed <files origin>/showcase/{repository}/<check>/<picture>, with the \
             picture names fearlezz run lists"
        );
        return;
    }
    let taken = taken(git, config, store, journal);
    if taken.is_empty() {
        println!("no check took stills or films in this work's last run");
        println!(
            "next: fearlezz run, with a check that records stills or films, then fearlezz \
             setup showcase"
        );
        return;
    }
    let blocks = markup(crate::host::FILES, &repository, &taken);
    println!(
        "showcase of {repository}: each picture as the newest push to the default branch whose \
         proof did not fail took it"
    );
    for block in blocks {
        println!();
        print!("{block}");
    }
    println!();
    println!(
        "next: paste a block into a README, a docs page or a website; it shows once a push to \
         the default branch proves it with the Fearlezz app installed"
    );
}

/// The stills and films of each check's last run in this work, in the config's order. A check
/// this work never ran counts with its latest run on the inputs it has now, as another work or
/// worktree may have run it.
fn taken(git: &Git, config: &Config, store: &Store, journal: &Journal) -> Vec<Taken> {
    // Read only when a check needs it: it asks git for every file.
    let mut worktree: Option<Option<Worktree>> = None;
    let mut all = Vec::new();
    for check in config.due() {
        let ran = crate::state::latest_ran(journal, &check.id)
            .and_then(|ran| store.get(ran.key, ran.result))
            .or_else(|| {
                let wt = worktree
                    .get_or_insert_with(|| Worktree::read(git.clone(), config).ok())
                    .as_mut()?;
                let inputs = wt.inputs(check).ok()?;
                store.for_key(&inputs.key).pop()
            });
        all.extend(ran.iter().flat_map(|run| pictures(&check.id, run)));
    }
    all
}

/// The stills and films `run` of `check` took.
fn pictures(check: &str, run: &RunResult) -> Vec<Taken> {
    crate::look::media_proofs(run)
        .into_iter()
        .filter(|p| p.kind == "still" || p.kind == "film")
        .map(|p| Taken {
            check: check.to_owned(),
            film: p.kind == "film",
            name: p.name,
        })
        .collect()
}

/// One block of markup per picture of `taken`, a light and a dark one of the same check paired
/// into one, each at its address on `origin` for `repository` (`owner/name`).
fn markup(origin: &str, repository: &str, taken: &[Taken]) -> Vec<String> {
    let url = |check: &str, name: &Name| {
        let path: Vec<String> = check
            .split('/')
            .chain(name.full.split('/'))
            .map(crate::host::segment)
            .collect();
        format!("{origin}/showcase/{repository}/{}", path.join("/"))
    };
    let mut left: Vec<(&str, Name)> = taken
        .iter()
        .map(|t| (t.check.as_str(), Name::read(&served(&t.name, t.film))))
        .collect();
    let mut blocks = Vec::new();
    while !left.is_empty() {
        let (check, name) = left.remove(0);
        let partner = name.theme.and_then(|theme| {
            left.iter().position(|(c, other)| {
                *c == check && other.theme == Some(!theme) && other.rest == name.rest
            })
        });
        let partner = partner.map(|at| left.remove(at).1);
        let alt = escape(&name.alt(check));
        let img = |src: &str| format!("<img alt=\"{alt}\" src=\"{src}\" width=\"{WIDTH}\">");
        blocks.push(match partner {
            Some(other) => {
                let (light, dark) = if name.theme == Some(Theme::Light) {
                    (&name, &other)
                } else {
                    (&other, &name)
                };
                format!(
                    "<picture>\n  <source media=\"(prefers-color-scheme: dark)\" \
                     srcset=\"{}\">\n  {}\n</picture>\n",
                    url(check, dark),
                    img(&url(check, light))
                )
            }
            None => format!("{}\n", img(&url(check, &name))),
        });
    }
    blocks
}

/// The name a showcase serves a picture at: a film's animated WebP copy, a still as it is.
fn served(name: &str, film: bool) -> String {
    if film {
        format!("{}.webp", extension(name).0)
    } else {
        name.to_owned()
    }
}

/// `name` without its extension, and the extension: a short last part after a dot, as the
/// proof page reads a picture's name.
fn extension(name: &str) -> (&str, &str) {
    match name.rsplit_once('.') {
        Some((bare, ext)) if !bare.is_empty() && ext.len() <= 4 && !ext.contains('/') => {
            (bare, ext)
        }
        _ => (name, ""),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Theme {
    Light,
    Dark,
}

impl std::ops::Not for Theme {
    type Output = Theme;
    fn not(self) -> Theme {
        match self {
            Theme::Light => Theme::Dark,
            Theme::Dark => Theme::Light,
        }
    }
}

/// A picture's served name, read as the proof page reads it: its segments without the
/// extension, the first `light` or `dark` one its theme.
struct Name {
    full: String,
    theme: Option<Theme>,
    /// Every segment but the theme's, with the extension: what a light and a dark variant
    /// share.
    rest: (Vec<String>, String),
}

impl Name {
    fn read(full: &str) -> Name {
        let (bare, ext) = extension(full);
        let mut theme = None;
        let mut segments = Vec::new();
        for segment in bare.split('/') {
            let word = match segment.to_ascii_lowercase().as_str() {
                "light" => Some(Theme::Light),
                "dark" => Some(Theme::Dark),
                _ => None,
            };
            match word {
                Some(word) if theme.is_none() => theme = Some(word),
                _ => segments.push(segment.to_owned()),
            }
        }
        Name {
            full: full.to_owned(),
            theme,
            rest: (segments, ext.to_owned()),
        }
    }

    /// Words for the picture: its segments but the theme, or the check's name when it has no
    /// others.
    fn alt(&self, check: &str) -> String {
        let words = self.rest.0.join(" ").replace(['-', '_'], " ");
        if words.trim().is_empty() {
            check.replace(['/', '-', '_'], " ")
        } else {
            words
        }
    }
}

/// `text` safe inside an HTML attribute.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
