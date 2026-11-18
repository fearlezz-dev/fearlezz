//! Releases: the places a release goes to (`[release.<place>]`), each with the command that
//! publishes to it, how to tell it has the version, and a dry run that proves before merge
//! that publishing will work. `fearlezz run --release` publishes to every place that does
//! not have the version yet and waits until each shows it, so a half done release is
//! finished by running it again.
//!
//! The version is the one in the place's own file (Cargo.toml, package.json,
//! pubspec.yaml), the file release-please bumps on its release pull request.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use serde_json::Value;

use crate::host::{self, Body, Limit};

/// The prefix of the check a place's dry run becomes: `release/<place>`.
pub(crate) const CHECK_PREFIX: &str = "release/";

/// How long a place may take to show a version after it was published, by default.
const DEFAULT_WAIT: u64 = 600;

/// How often a place is asked again while Fearlezz waits for it.
const POLL: Duration = Duration::from_secs(2);

/// One place a release goes to.
#[derive(Clone, Debug)]
pub(crate) struct Place {
    pub name: String,
    pub publish: String,
    pub landed: Landed,
    /// Variables the publish command may see, such as its token.
    pub env: Vec<String>,
    /// The folder of the repository the package lives in; the root without one.
    pub dir: Option<String>,
    /// The file the version comes from, relative to the root; without one, the reader's own
    /// file in `dir`.
    pub version: Option<String>,
    /// Built files the release carries (globs from the root), for a place such as GitHub
    /// Releases. They reach the publish command as its arguments (`"$@"`).
    pub files: Vec<String>,
    /// The tag the release is known by, `{version}` replaced: `v{version}` by default.
    pub tag: String,
    /// Seconds the place may take to show the version once published.
    pub wait: u64,
}

/// How Fearlezz tells a place has the version.
#[derive(Clone, Debug)]
pub(crate) enum Landed {
    /// A command that exits 0 once the place has it.
    Command(String),
    /// A registry Fearlezz asks itself, at `url` when it is not the public one.
    Read { reader: Reader, url: Option<String> },
}

/// The places Fearlezz knows how to ask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reader {
    Crates,
    Npm,
    Pub,
    Github,
}

impl Reader {
    const ALL: [(&'static str, Reader); 4] = [
        ("crates", Reader::Crates),
        ("npm", Reader::Npm),
        ("pub", Reader::Pub),
        ("github", Reader::Github),
    ];

    fn parse(name: &str) -> Result<Reader> {
        Reader::ALL
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, r)| *r)
            .with_context(|| {
                let names: Vec<&str> = Reader::ALL.iter().map(|(n, _)| *n).collect();
                format!(
                    "no landed reader \"{name}\"; readers are {}",
                    names.join(", ")
                )
            })
    }

    /// The file a package of this registry names itself and its version in.
    fn manifest(self) -> Option<&'static str> {
        match self {
            Reader::Crates => Some("Cargo.toml"),
            Reader::Npm => Some("package.json"),
            Reader::Pub => Some("pubspec.yaml"),
            Reader::Github => None,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPlace {
    #[expect(
        dead_code,
        reason = "the dry run becomes a check in the TOML table (add_dry_checks); the field only \
                  lets serde accept the key"
    )]
    dry: Option<String>,
    publish: String,
    landed: RawLanded,
    #[serde(default)]
    env: Vec<String>,
    dir: Option<String>,
    version: Option<String>,
    #[serde(default)]
    files: Vec<String>,
    tag: Option<String>,
    wait: Option<u64>,
    /// Turns off a place a piece from `use` brings in.
    #[serde(default)]
    off: bool,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawLanded {
    Command(String),
    Read { read: String, url: Option<String> },
}

/// The places `[release]` names, in order of name, those turned off left out.
pub(crate) fn parse(table: Option<&toml::Value>) -> Result<Vec<Place>> {
    let Some(table) = table else {
        return Ok(Vec::new());
    };
    let raw: BTreeMap<String, RawPlace> = table.clone().try_into()?;
    let mut places = Vec::new();
    for (name, p) in raw {
        if p.off {
            continue;
        }
        let at = || format!("release.{name}");
        let landed = match p.landed {
            RawLanded::Command(cmd) => Landed::Command(cmd),
            RawLanded::Read { read, url } => Landed::Read {
                reader: Reader::parse(&read).with_context(at)?,
                url,
            },
        };
        let tag = p.tag.unwrap_or_else(|| "v{version}".into());
        if !tag.contains("{version}") {
            bail!("release.{name}: tag needs {{version}}, such as tag = \"v{{version}}\"");
        }
        places.push(Place {
            name,
            publish: p.publish,
            landed,
            env: p.env,
            dir: p.dir,
            version: p.version,
            files: p.files,
            tag,
            wait: p.wait.unwrap_or(DEFAULT_WAIT),
        });
    }
    Ok(places)
}

/// Turns each place's dry run into the check `release/<place>` at stage `last`, so it is
/// proven before merge like any check, and the base branch's floor holds it. A project
/// tunes that check under its own name (`[check."release/crates"]`), or turns it off.
/// `default_last` is the last stage when the table orders none of its own.
pub(crate) fn add_dry_checks(table: &mut toml::Table, default_last: Option<&str>) {
    let Some(toml::Value::Table(release)) = table.get("release") else {
        return;
    };
    let Some(last) = table
        .get("stages")
        .and_then(|s| s.get("order"))
        .and_then(toml::Value::as_array)
        .and_then(|o| o.last())
        .and_then(toml::Value::as_str)
        .or(default_last)
        .map(String::from)
    else {
        return;
    };
    let mut dry = Vec::new();
    for (name, place) in release {
        let off = place.get("off").and_then(toml::Value::as_bool) == Some(true);
        if let (Some(run), false) = (place.get("dry").and_then(toml::Value::as_str), off) {
            dry.push((
                format!("{CHECK_PREFIX}{name}"),
                run.to_string(),
                place.get("dir").cloned(),
            ));
        }
    }
    if dry.is_empty() {
        return;
    }
    let checks = table
        .entry("check")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()));
    let Some(checks) = checks.as_table_mut() else {
        return;
    };
    for (id, run, dir) in dry {
        let check = checks
            .entry(id)
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        let Some(check) = check.as_table_mut() else {
            continue;
        };
        check
            .entry("run")
            .or_insert_with(|| toml::Value::String(run));
        check
            .entry("stage")
            .or_insert_with(|| toml::Value::String(last.clone()));
        if let Some(dir) = dir {
            check.entry("dir").or_insert(dir);
        }
    }
}

/// A package as its file names it.
#[derive(Debug, PartialEq, Eq)]
struct Package {
    name: String,
    version: String,
}

/// What one place is asked to publish: the package, the tag and the files.
struct Release {
    package: Package,
    tag: String,
    files: Vec<String>,
}

impl Release {
    /// How the release is named at this place: the tag for GitHub, the version elsewhere.
    fn shown(&self, place: &Place) -> String {
        let tagged = matches!(
            place.landed,
            Landed::Read {
                reader: Reader::Github,
                ..
            }
        );
        let what = if tagged {
            &self.tag
        } else {
            &self.package.version
        };
        format!("{} {what}", self.package.name)
    }
}

/// Where the release runs: the checkout and the `[env]` shell its commands run in.
pub(crate) struct At<'a> {
    pub root: &'a Path,
    pub shell: Option<&'a str>,
    pub github: GitHub,
}

/// The repository on GitHub, from the workflow or the origin remote, and its API.
pub(crate) struct GitHub {
    pub repository: Option<String>,
    pub api: String,
}

/// Publishes every place that does not have its version yet, each after the other, and says
/// what is next. A place that fails leaves the others to go. Returns whether every place has
/// its release.
pub(crate) fn run(places: &[Place], at: &At<'_>) -> bool {
    if places.is_empty() {
        println!("no place to release to in fearlezz.toml");
        println!(
            "next: add one: use = [\"crates\"] (or npm, pub, github), or a [release.<place>] of your own with publish and landed"
        );
        return true;
    }
    let mut ok = true;
    for place in places {
        match release(place, at) {
            Ok(line) => println!("{}: {line}", place.name),
            Err(e) => {
                println!("{}: {e:#}", place.name);
                ok = false;
            }
        }
    }
    if ok {
        println!("next: every place has its release");
    } else {
        println!(
            "next: fix what failed, then fearlezz run --release again; places that have the release are skipped"
        );
    }
    ok
}

/// What the place lacks that only the project can name. A piece may leave it out, so it is
/// asked for when the release runs.
fn lacks(place: &Place) -> Result<()> {
    let reader = match &place.landed {
        Landed::Read { reader, .. } => Some(*reader),
        Landed::Command(_) => None,
    };
    if place.version.is_none() && reader.and_then(Reader::manifest).is_none() {
        bail!(
            "name the file its version comes from in [release.{}], such as version = \"Cargo.toml\"",
            place.name
        );
    }
    if reader == Some(Reader::Github) && place.files.is_empty() {
        bail!(
            "name the built files it carries in [release.{}], such as files = [\"dist/*\"]",
            place.name
        );
    }
    Ok(())
}

/// Releases to one place, returning the line that says how it ended well.
fn release(place: &Place, at: &At<'_>) -> Result<String> {
    lacks(place)?;
    let package = package(place, at.root)?;
    let release = Release {
        tag: place.tag.replace("{version}", &package.version),
        files: files(&place.files, at.root)?,
        package,
    };
    let shown = release.shown(place);
    if landed(place, &release, at)?.is_ok() {
        return Ok(format!("{shown} is already there"));
    }
    println!("{}: publishing {shown}", place.name);
    let status = command(place, &place.publish, &release, at)
        .status()
        .with_context(|| format!("starting the publish command of {}", place.name))?;
    if !status.success() {
        let code = status
            .code()
            .map_or_else(|| "a signal".to_string(), |c| format!("exit code {c}"));
        bail!("publish ended with {code}");
    }
    let deadline = Instant::now() + Duration::from_secs(place.wait);
    loop {
        // A registry that stumbles once (a 502, a rate limit) is asked again until the end.
        let why = match landed(place, &release, at) {
            Ok(Ok(())) => return Ok(format!("{shown} landed")),
            Ok(Err(why)) => why,
            Err(e) => Some(format!("{e:#}")),
        };
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            let why = why.map(|w| format!(": {w}")).unwrap_or_default();
            bail!("{shown} did not land within {} s{why}", place.wait);
        }
        std::thread::sleep(left.min(POLL));
    }
}

/// A command of the place run as a check runs, with the variables it names and the release's
/// version, tag and files.
fn command(place: &Place, script: &str, release: &Release, at: &At<'_>) -> std::process::Command {
    // Absolute, so a command that runs in the place's folder finds them too.
    let files: Vec<String> = release
        .files
        .iter()
        .map(|f| at.root.join(f).display().to_string())
        .collect();
    let mut cmd = crate::process::Project::new(at.root, at.shell).clean(
        place.dir.as_deref(),
        script,
        &place.env,
        &files,
    );
    cmd.env("FEARLEZZ_VERSION", &release.package.version)
        .env("FEARLEZZ_TAG", &release.tag);
    cmd
}

/// Whether the place has the release: Ok, or Err with why not when the reader can say.
fn landed(place: &Place, release: &Release, at: &At<'_>) -> Result<Result<(), Option<String>>> {
    match &place.landed {
        Landed::Command(script) => {
            let status = command(place, script, release, at)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .with_context(|| format!("starting the landed command of {}", place.name))?;
            Ok(if status.success() { Ok(()) } else { Err(None) })
        }
        Landed::Read { reader, url } => {
            let url = url.as_deref().map(|u| u.trim_end_matches('/'));
            let p = &release.package;
            match reader {
                Reader::Crates => crates_has(url.unwrap_or("https://index.crates.io"), p),
                Reader::Npm => npm_has(url.unwrap_or("https://registry.npmjs.org"), p),
                Reader::Pub => {
                    let url = url.unwrap_or("https://pub.dev");
                    found(&format!(
                        "{url}/api/packages/{}/versions/{}",
                        p.name, p.version
                    ))
                }
                Reader::Github => github_has(url, release, at),
            }
        }
    }
}

/// GET `url`: the status and the answer.
fn get(url: &str, token: &str) -> Result<(u16, Value)> {
    host::curl_within("GET", url, token, &[], Body::None, Some(Limit::within(10)))
}

/// Whether `url` exists: 200 yes, 404 no, anything else an error.
fn found(url: &str) -> Result<Result<(), Option<String>>> {
    match get(url, "")? {
        (200, _) => Ok(Ok(())),
        (404, _) => Ok(Err(None)),
        (code, _) => bail!("GET {url} answered {code}"),
    }
}

/// Whether the package's document on the registry lists the version. The abbreviated
/// document is the one npm install reads; every registry serves it for scoped names too.
fn npm_has(registry: &str, p: &Package) -> Result<Result<(), Option<String>>> {
    let url = format!("{registry}/{}", p.name.replace('/', "%2f"));
    let answer = match host::curl_within(
        "GET",
        &url,
        "",
        &["Accept: application/vnd.npm.install-v1+json"],
        Body::None,
        Some(Limit::within(10)),
    )? {
        (200, answer) => answer,
        (404, _) => return Ok(Err(None)),
        (code, _) => bail!("GET {url} answered {code}"),
    };
    let listed = answer
        .get("versions")
        .and_then(|v| v.get(&p.version))
        .is_some();
    Ok(if listed { Ok(()) } else { Err(None) })
}

/// The folder of crates.io's sparse index a crate's file is in.
fn index_prefix(name: &str) -> String {
    // Crate names are ASCII; chars keep a stray other name from panicking.
    let part =
        |from: usize, to: usize| -> String { name.chars().skip(from).take(to - from).collect() };
    match name.chars().count() {
        1 => "1".to_string(),
        2 => "2".to_string(),
        3 => format!("3/{}", part(0, 1)),
        _ => format!("{}/{}", part(0, 2), part(2, 4)),
    }
}

/// Whether crates.io's sparse index lists the version, the index cargo itself reads.
fn crates_has(index: &str, p: &Package) -> Result<Result<(), Option<String>>> {
    let name = p.name.to_lowercase();
    let url = format!("{index}/{}/{name}", index_prefix(&name));
    let text = match get(&url, "")? {
        (200, Value::String(text)) => text,
        // One line of JSON comes back parsed.
        (200, value) => value.to_string(),
        (404, _) => return Ok(Err(None)),
        (code, _) => bail!("GET {url} answered {code}"),
    };
    let listed = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .any(|v| v.get("vers").and_then(Value::as_str) == Some(p.version.as_str()));
    Ok(if listed { Ok(()) } else { Err(None) })
}

/// Whether the GitHub release of the tag carries every file with the same bytes, by the
/// SHA-256 digest GitHub keeps of each asset (by size where it keeps none).
fn github_has(
    url: Option<&str>,
    release: &Release,
    at: &At<'_>,
) -> Result<Result<(), Option<String>>> {
    let root = at.root;
    let api = url.unwrap_or(&at.github.api);
    let repository = at.github.repository.as_deref().context(
        "no GitHub repository: set GITHUB_REPOSITORY, or give the checkout a github.com origin remote",
    )?;
    let token = ["GITHUB_TOKEN", "GH_TOKEN"]
        .iter()
        .find_map(|n| std::env::var(n).ok().filter(|v| !v.is_empty()))
        .unwrap_or_default();
    let url = format!(
        "{}/repos/{repository}/releases/tags/{}",
        api.trim_end_matches('/'),
        release.tag
    );
    let answer = match get(&url, &token)? {
        (200, answer) => answer,
        (404, _) => return Ok(Err(Some(format!("no release of {}", release.tag)))),
        (code, _) => bail!("GET {url} answered {code}"),
    };
    let assets = answer
        .get("assets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for file in &release.files {
        let path = root.join(file);
        let name = Path::new(file)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let Some(asset) = assets
            .iter()
            .find(|a| a.get("name").and_then(Value::as_str) == Some(name.as_str()))
        else {
            return Ok(Err(Some(format!("{name} is not there"))));
        };
        let same = match asset.get("digest").and_then(Value::as_str) {
            Some(digest) => {
                let bytes =
                    std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
                digest.strip_prefix("sha256:") == Some(crate::files::sha256_hex(&bytes).as_str())
            }
            None => {
                asset.get("size").and_then(Value::as_u64) == Some(std::fs::metadata(&path)?.len())
            }
        };
        if !same {
            return Ok(Err(Some(format!("{name} there has other bytes"))));
        }
    }
    Ok(Ok(()))
}

/// The file the place's version comes from, relative to the root.
fn manifest_path(place: &Place) -> PathBuf {
    if let Some(file) = &place.version {
        return PathBuf::from(file);
    }
    let file = match &place.landed {
        Landed::Read { reader, .. } => reader.manifest().unwrap_or_default(),
        Landed::Command(_) => "",
    };
    match &place.dir {
        Some(dir) => Path::new(dir).join(file),
        None => PathBuf::from(file),
    }
}

/// The package the place's file names.
fn package(place: &Place, root: &Path) -> Result<Package> {
    let rel = manifest_path(place);
    let path = root.join(&rel);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {} for the version", rel.display()))?;
    let file = rel
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let found = match file.as_str() {
        "Cargo.toml" => cargo_package(&text, &path, root),
        "package.json" => json_package(&text),
        "pubspec.yaml" => pubspec_package(&text),
        _ => bail!(
            "{}: Fearlezz reads a version from Cargo.toml, package.json or pubspec.yaml",
            rel.display()
        ),
    };
    found.with_context(|| format!("reading {}", rel.display()))
}

fn cargo_package(text: &str, path: &Path, root: &Path) -> Result<Package> {
    let table: toml::Table = toml::from_str(text)?;
    let package = table
        .get("package")
        .and_then(toml::Value::as_table)
        .context("it has no [package]; name the crate's own Cargo.toml with dir or version in its [release] place")?;
    let name = package
        .get("name")
        .and_then(toml::Value::as_str)
        .context("its package has no name")?
        .to_string();
    let version = match package.get("version") {
        Some(toml::Value::String(v)) => v.clone(),
        Some(toml::Value::Table(t))
            if t.get("workspace").and_then(toml::Value::as_bool) == Some(true) =>
        {
            workspace_version(path, root)?
        }
        _ => bail!("its package has no version"),
    };
    Ok(Package { name, version })
}

/// The version of the workspace above `path` (`[workspace.package] version`).
fn workspace_version(path: &Path, root: &Path) -> Result<String> {
    let mut dir = path.parent();
    while let Some(d) = dir {
        let manifest = d.join("Cargo.toml");
        // The package's own file comes first: a root Cargo.toml can hold both.
        if let Ok(text) = std::fs::read_to_string(&manifest) {
            let table: toml::Table = toml::from_str(&text)?;
            if let Some(ws) = table.get("workspace") {
                return ws
                    .get("package")
                    .and_then(|p| p.get("version"))
                    .and_then(toml::Value::as_str)
                    .map(String::from)
                    .with_context(|| {
                        format!("{} has no [workspace.package] version", manifest.display())
                    });
            }
        }
        if d == root {
            break;
        }
        dir = d.parent();
    }
    bail!("it takes its version from a workspace, but no workspace is above it")
}

fn json_package(text: &str) -> Result<Package> {
    let v: Value = serde_json::from_str(text)?;
    let field = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .map(String::from)
            .with_context(|| format!("it has no {k}"))
    };
    Ok(Package {
        name: field("name")?,
        version: field("version")?,
    })
}

/// The top level `name:` and `version:` of a pubspec.
fn pubspec_package(text: &str) -> Result<Package> {
    let field = |k: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(k)?.strip_prefix(':'))
            .map(|v| {
                let v = v.split(" #").next().unwrap_or(v).trim();
                v.trim_matches(|c| c == '"' || c == '\'').to_string()
            })
            .filter(|v| !v.is_empty())
            .with_context(|| format!("it has no {k}"))
    };
    Ok(Package {
        name: field("name")?,
        version: field("version")?,
    })
}

/// The files the globs name under the root, sorted; each glob must name at least one.
fn files(globs: &[String], root: &Path) -> Result<Vec<String>> {
    let mut all = Vec::new();
    for glob in globs {
        let matcher = globset::GlobBuilder::new(glob)
            .literal_separator(true)
            .build()
            .with_context(|| format!("files: {glob}"))?
            .compile_matcher();
        // Only the folder before the first pattern character is walked.
        let base: PathBuf = Path::new(glob)
            .components()
            .take_while(|c| {
                !c.as_os_str()
                    .to_string_lossy()
                    .contains(['*', '?', '[', '{'])
            })
            .collect();
        let found: Vec<String> = crate::files::walk(root, &root.join(&base))
            .into_iter()
            .filter(|rel| matcher.is_match(rel))
            .collect();
        if found.is_empty() {
            bail!("files: {glob} matches no file; build them before the release");
        }
        all.extend(found);
    }
    all.sort();
    all.dedup();
    // A release names its files by their names alone, so two of one name would overwrite.
    let mut names = std::collections::BTreeSet::new();
    for file in &all {
        let name = Path::new(file).file_name().unwrap_or_default();
        if !names.insert(name.to_os_string()) {
            bail!(
                "files: two files are called {}; a release names its files by name alone",
                name.to_string_lossy()
            );
        }
    }
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sparse_index_folder_follows_the_length_of_the_name() {
        assert_eq!(index_prefix("a"), "1");
        assert_eq!(index_prefix("ab"), "2");
        assert_eq!(index_prefix("abc"), "3/a");
        assert_eq!(index_prefix("serde"), "se/rd");
    }

    #[test]
    fn a_pubspec_version_keeps_its_build_and_drops_quotes_and_comments() {
        let p =
            pubspec_package("name: 'w'\nversion: 1.0.0+2 # bumped\ndependencies:\n  name: other\n")
                .unwrap();
        assert_eq!(
            p,
            Package {
                name: "w".into(),
                version: "1.0.0+2".into()
            }
        );
    }
}
