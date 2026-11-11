//! `use`: pulling named pieces of config into `fearlezz.toml`. A piece has the same shape as
//! `fearlezz.toml` and may `use` other pieces. Pieces merge in the order they are named,
//! table by table and field by field, and the file that names them wins, so a project tunes
//! a piece by writing only the fields it changes.
//!
//! A piece is one of:
//! - a built in name, such as `rust` or `devenv`;
//! - a path starting with `./` or `../`, relative to the file that names it;
//! - `github:owner/repo//path@rev` or `git:<url>//path@rev`, pinned to a tag or commit and
//!   cached in the shared git folder, so it is fetched once for every worktree, and locked
//!   in `fearlezz.lock` (see [`LOCK`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use sha2::{Digest as _, Sha256};
use toml::{Table, Value};

use crate::config::FILE;

pub(crate) const BUILTIN: &[(&str, &str)] = &[
    ("agents", include_str!("../presets/agents.toml")),
    ("devenv", include_str!("../presets/devenv.toml")),
    ("eslint", include_str!("../presets/eslint.toml")),
    ("flutter", include_str!("../presets/flutter.toml")),
    ("go", include_str!("../presets/go.toml")),
    ("kotlin", include_str!("../presets/kotlin.toml")),
    ("python", include_str!("../presets/python.toml")),
    ("rust", include_str!("../presets/rust.toml")),
    ("typescript", include_str!("../presets/typescript.toml")),
];

/// The file next to `fearlezz.toml` that pins every piece from a repository, nested ones
/// included, to a commit and the hash of its content. It is committed with the project, so a
/// moved tag cannot change the rules on one machine and not another.
pub(crate) const LOCK: &str = "fearlezz.lock";

/// Reads `fearlezz.toml` in `root` with every piece it uses merged in. Pieces from a
/// repository are read at the commit `fearlezz.lock` pins; new ones are added to it and
/// unused ones dropped. A tag that now points elsewhere is an error until `update` accepts it.
/// On CI the lock is never written: one that does not match is an error, so CI reads exactly
/// what was reviewed.
pub(crate) fn resolve(root: &Path, common: &Path) -> Result<Table> {
    let (_, table) = read_config(root)?;
    let locked = read_lock(root)?;
    let mut loader = Loader::new(common, locked.clone(), Mode::Check);
    let table = loader.expand(table, &Origin::Dir(root.to_path_buf()))?;
    if loader.used != locked {
        if on_ci() {
            let missing: Vec<&str> = loader
                .used
                .keys()
                .filter(|id| !locked.contains_key(*id))
                .map(String::as_str)
                .collect();
            let unused: Vec<&str> = locked
                .keys()
                .filter(|id| !loader.used.contains_key(*id))
                .map(String::as_str)
                .collect();
            let mut why = Vec::new();
            if !missing.is_empty() {
                why.push(format!("it lacks {}", missing.join(", ")));
            }
            if !unused.is_empty() {
                why.push(format!("nothing uses {}", unused.join(", ")));
            }
            bail!(
                "{LOCK} does not match the pieces {FILE} uses ({}). On CI it is only read: run \
                 any fearlezz command on a developer machine, review {LOCK}, and commit it",
                why.join("; ")
            );
        }
        write_lock(root, &loader.used)?;
    }
    Ok(table)
}

/// Whether this runs on CI, which sets `CI` (to `true` on the common services).
fn on_ci() -> bool {
    std::env::var("CI").is_ok_and(|v| !matches!(v.as_str(), "" | "0" | "false"))
}

/// What `update` did: each piece `fearlezz.toml` names (its spec before, and the tag after),
/// each locked piece whose tag had moved and now pins where it points, and every setting of
/// the merged config that changed; None when the old pins can no longer be read.
pub(crate) struct Updated {
    pub pieces: Vec<(String, String)>,
    /// Pieces with a newer major version than `update` moves to: their spec and that tag.
    pub majors: Vec<(String, String)>,
    pub moved: Vec<String>,
    pub changes: Option<Vec<String>>,
}

/// Moves the repository pieces `fearlezz.toml` names (those whose spec contains `only`, or
/// all) to their newest release tag of the same major version, takes the tag of every locked piece that matches as it
/// now is, wherever it is named, and rewrites `fearlezz.toml` and `fearlezz.lock`. Nothing is
/// written when the new pieces fail to load.
pub(crate) fn update(root: &Path, common: &Path, only: Option<&str>) -> Result<Updated> {
    let (text, table) = read_config(root)?;
    let locked = read_lock(root)?;
    let origin = Origin::Dir(root.to_path_buf());
    // Only for listing the changes: an amended release can leave the old pin unreadable.
    let before = Loader::new(common, locked.clone(), Mode::Trust)
        .expand(table, &origin)
        .ok();

    let mut doc: toml_edit::DocumentMut = text.parse()?;
    let mut picked = Vec::new();
    if let Some(items) = doc.get_mut("use").and_then(|u| u.as_array_mut()) {
        for item in items.iter_mut() {
            let Some(spec) = item.as_str().map(str::to_string) else {
                continue;
            };
            if let Source::Git { url, rev, .. } = Source::parse(&spec, &origin)?
                && only.is_none_or(|o| spec.contains(o))
            {
                picked.push((spec, url, rev, item));
            }
        }
    }
    // Nested pieces are in the lock too, so a moved tag is accepted wherever it is named.
    let mut urls: BTreeSet<String> = locked
        .keys()
        .filter(|id| only.is_none_or(|o| id.contains(o)))
        .filter_map(|id| Some(id.rsplit_once('@')?.0.rsplit_once("//")?.0.to_string()))
        .collect();
    urls.extend(picked.iter().map(|(_, url, ..)| url.clone()));
    if urls.is_empty() {
        bail!(
            "{FILE} uses no piece from a repository{}",
            only.map(|o| format!(" matching \"{o}\""))
                .unwrap_or_default()
        );
    }
    let mut loader = Loader::new(common, locked.clone(), Mode::Refresh(urls));
    let (mut pieces, mut majors) = (Vec::new(), Vec::new());
    for (spec, url, rev, item) in picked {
        let dir = loader.repo(&url)?;
        let Newest { newest, major } = newest_tag(&dir, &rev)?;
        if let Some(major) = major {
            majors.push((spec.clone(), major));
        }
        if newest != rev {
            let (place, _) = spec.rsplit_once('@').context("a git piece has a rev")?;
            let decor = item.decor().clone();
            *item = toml_edit::Value::from(format!("{place}@{newest}"));
            *item.decor_mut() = decor;
        }
        pieces.push((spec, newest));
    }
    let new_text = doc.to_string();
    let table: Table = toml::from_str(&new_text)?;
    let after = loader.expand(table, &origin)?;
    std::fs::write(root.join(FILE), new_text)?;
    write_lock(root, &loader.used)?;
    let moved = loader
        .used
        .iter()
        .filter(|(id, pin)| locked.get(*id).is_some_and(|old| old.commit != pin.commit))
        .map(|(id, _)| id.clone())
        .collect();
    Ok(Updated {
        pieces,
        majors,
        moved,
        changes: before.map(|before| changes(&before, &after)),
    })
}

/// What a piece's tags offer.
struct Newest {
    /// The tag to move to: the newest release of the same major version.
    newest: String,
    /// The newest release of a higher major version, which only a person or agent moves to.
    major: Option<String>,
}

/// The newest release tags of the same family as `current` (the same text before the
/// version, such as `v`); `current` itself when it is not a release version. Prereleases such
/// as `v2-rc1` and unrelated tags are never picked.
fn newest_tag(dir: &Path, current: &str) -> Result<Newest> {
    let mut found = Newest {
        newest: current.to_string(),
        major: None,
    };
    let Some((family, now)) = release(current) else {
        return Ok(found);
    };
    let (mut best, mut best_major) = (now.clone(), now.clone());
    for tag in git(Some(dir), &["tag", "--list"])?.lines() {
        let Some((_, version)) = release(tag).filter(|(f, _)| *f == family) else {
            continue;
        };
        if version.first() == now.first() {
            if version > best {
                best = version;
                found.newest = tag.to_string();
            }
        } else if version > best_major {
            best_major = version;
            found.major = Some(tag.to_string());
        }
    }
    Ok(found)
}

/// A release tag as its family and its version numbers: `v1.10` is `("v", [1, 10])`.
fn release(tag: &str) -> Option<(&str, Vec<u64>)> {
    let start = tag.find(|c: char| c.is_ascii_digit())?;
    let (family, version) = tag.split_at(start);
    let numbers = version
        .split('.')
        .map(|n| n.parse().ok())
        .collect::<Option<Vec<u64>>>()?;
    Some((family, numbers))
}

fn read_config(root: &Path) -> Result<(String, Table)> {
    let path = root.join(FILE);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let table = toml::from_str(&text).with_context(|| format!("in {}", path.display()))?;
    Ok((text, table))
}

/// A piece from a repository as `fearlezz.lock` pins it.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Pin {
    commit: String,
    sha256: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct LockFile {
    #[serde(default, rename = "piece")]
    pieces: Vec<LockEntry>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct LockEntry {
    source: String,
    #[serde(flatten)]
    pin: Pin,
}

/// Pins by piece id (`url//path@rev`).
type Lock = BTreeMap<String, Pin>;

fn read_lock(root: &Path) -> Result<Lock> {
    let path = root.join(LOCK);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Lock::new()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let file: LockFile = toml::from_str(&text).with_context(|| format!("in {}", path.display()))?;
    Ok(file.pieces.into_iter().map(|e| (e.source, e.pin)).collect())
}

fn write_lock(root: &Path, lock: &Lock) -> Result<()> {
    let path = root.join(LOCK);
    if lock.is_empty() {
        return match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            Ok(()) | Err(_) => Ok(()),
        };
    }
    let file = LockFile {
        pieces: lock
            .iter()
            .map(|(source, pin)| LockEntry {
                source: source.clone(),
                pin: pin.clone(),
            })
            .collect(),
    };
    let text = format!(
        "# Written by fearlezz: the commit and content of every piece from a repository.\n\
         # Commit it. Run `fearlezz setup update` to move pieces to newer tags.\n\n{}",
        toml::to_string(&file)?
    );
    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))
}

/// Every setting that differs between two merged configs, by dotted path.
fn changes(before: &Table, after: &Table) -> Vec<String> {
    fn flatten(prefix: &str, table: &Table, out: &mut BTreeMap<String, String>) {
        for (key, value) in table {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            match value {
                Value::Table(t) => flatten(&path, t, out),
                v => {
                    out.insert(path, v.to_string());
                }
            }
        }
    }
    let (mut b, mut a) = (BTreeMap::new(), BTreeMap::new());
    flatten("", before, &mut b);
    flatten("", after, &mut a);
    let keys: BTreeSet<&String> = b.keys().chain(a.keys()).collect();
    keys.into_iter()
        .filter_map(|k| match (b.get(k), a.get(k)) {
            (Some(x), Some(y)) if x != y => Some(format!("{k}: {x} → {y}")),
            (None, Some(y)) => Some(format!("{k}: added {y}")),
            (Some(x), None) => Some(format!("{k}: removed (was {x})")),
            (Some(_), Some(_)) | (None, None) => None,
        })
        .collect()
}

/// Where a piece came from, so the `./` paths inside it resolve next to it.
#[derive(Clone)]
enum Origin {
    Dir(PathBuf),
    Git {
        url: String,
        rev: String,
        dir: String,
    },
}

enum Source {
    Builtin(&'static str, &'static str),
    File(PathBuf),
    Git {
        url: String,
        rev: String,
        path: String,
    },
}

/// How the loader treats `fearlezz.lock`.
enum Mode {
    /// Read at the pinned commits, and refuse a tag that now points elsewhere.
    Check,
    /// Read at the pinned commits without looking at the tags: the config as it was locked.
    Trust,
    /// Fetch these repositories once and pin their pieces as their tags now are.
    Refresh(BTreeSet<String>),
}

struct Loader {
    cache: PathBuf,
    mode: Mode,
    locked: Lock,
    /// The pins of every repository piece read so far: the new lock.
    used: Lock,
    /// Repositories fetched in this run.
    fetched: BTreeSet<String>,
    /// The pieces being expanded, to refuse a piece that uses itself.
    stack: Vec<String>,
}

impl Loader {
    fn new(common: &Path, locked: Lock, mode: Mode) -> Loader {
        Loader {
            cache: common.join("fearlezz/presets"),
            mode,
            locked,
            used: Lock::new(),
            fetched: BTreeSet::new(),
            stack: Vec::new(),
        }
    }

    fn expand(&mut self, mut table: Table, origin: &Origin) -> Result<Table> {
        let uses = match table.remove("use") {
            None => Vec::new(),
            Some(Value::Array(items)) => items
                .into_iter()
                .map(|v| match v {
                    Value::String(s) => Ok(s),
                    other => bail!("use takes names, paths or git pieces, not {other}"),
                })
                .collect::<Result<_>>()?,
            Some(other) => bail!("use must be a list, like use = [\"rust\"], not {other}"),
        };
        let mut merged = Table::new();
        for spec in uses {
            let source = Source::parse(&spec, origin)?;
            let id = source.id();
            if self.stack.contains(&id) {
                bail!(
                    "use \"{spec}\" uses itself (through {})",
                    self.stack.join(", ")
                );
            }
            let text = self
                .read(&source, &id)
                .with_context(|| format!("use \"{spec}\""))?;
            let piece: Table =
                toml::from_str(&text).with_context(|| format!("use \"{spec}\": not valid TOML"))?;
            self.stack.push(id);
            let piece = self
                .expand(piece, &source.origin())
                .with_context(|| format!("use \"{spec}\""))?;
            self.stack.pop();
            merge(&mut merged, piece);
        }
        merge(&mut merged, table);
        Ok(merged)
    }

    fn read(&mut self, source: &Source, id: &str) -> Result<String> {
        match source {
            Source::Builtin(_, text) => Ok(text.to_string()),
            Source::File(path) => {
                std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
            }
            Source::Git { url, rev, path } => self.git_file(url, rev, path, id),
        }
    }

    /// The cached bare clone of `url`: cloned when missing, and fetched once per run when
    /// this run refreshes it.
    fn repo(&mut self, url: &str) -> Result<PathBuf> {
        let dir = self.cache.join(short_hash(url));
        if !dir.join("HEAD").exists() {
            std::fs::create_dir_all(&self.cache)?;
            // Cloned beside it and moved into place, so another worktree loading the same
            // piece never sees half a clone.
            let temp = self
                .cache
                .join(format!("{}.{}.tmp", short_hash(url), std::process::id()));
            if temp.exists() {
                std::fs::remove_dir_all(&temp)?;
            }
            let cloned = git(
                None,
                &[
                    "clone",
                    "--quiet",
                    "--bare",
                    "--",
                    url,
                    &temp.to_string_lossy(),
                ],
            )
            .and_then(|_| Ok(std::fs::rename(&temp, &dir)?));
            if let Err(e) = cloned {
                if temp.exists() {
                    std::fs::remove_dir_all(&temp)?;
                }
                // Another command finished the same clone first.
                if !dir.join("HEAD").exists() {
                    return Err(e);
                }
            }
            self.fetched.insert(url.to_string());
        }
        if let Mode::Refresh(urls) = &self.mode
            && urls.contains(url)
            && self.fetched.insert(url.to_string())
        {
            fetch(&dir)?;
        }
        Ok(dir)
    }

    /// A file of a repository piece: at the commit the lock pins, or for a piece not yet
    /// locked (or being refreshed) at the commit its tag or commit names now.
    fn git_file(&mut self, url: &str, rev: &str, path: &str, id: &str) -> Result<String> {
        let dir = self.repo(url)?;
        let refreshing = matches!(&self.mode, Mode::Refresh(urls) if urls.contains(url));
        let pin = if refreshing {
            None
        } else {
            self.locked.get(id).cloned()
        };
        let commit = match &pin {
            Some(pin) => {
                if matches!(self.mode, Mode::Check) {
                    let now = pinned_commit(&dir, url, rev)?;
                    if now != pin.commit {
                        bail!(
                            "{rev} of {url} now points at {}, but {LOCK} pins {}: the tag \
                             moved. Look at what changed, then run `fearlezz setup update` to accept it",
                            now.get(..12).unwrap_or(&now),
                            pin.commit.get(..12).unwrap_or(&pin.commit)
                        );
                    }
                } else if !has_commit(&dir, &pin.commit) {
                    fetch(&dir)?;
                    if !has_commit(&dir, &pin.commit) {
                        bail!(
                            "{LOCK} pins commit {} of {url}, which the repository no longer \
                             has; run `fearlezz setup update` to take {rev} as it is now",
                            pin.commit.get(..12).unwrap_or(&pin.commit)
                        );
                    }
                }
                pin.commit.clone()
            }
            None => pinned_commit(&dir, url, rev)?,
        };
        let text = git(Some(&dir), &["show", &format!("{commit}:{path}")])
            .with_context(|| format!("{path} at {rev}"))?;
        let sha256 = hex::encode(Sha256::digest(text.as_bytes()));
        if let Some(pin) = pin
            && pin.sha256 != sha256
        {
            bail!("{path} at {rev} does not match the content {LOCK} pins");
        }
        self.used.insert(id.to_string(), Pin { commit, sha256 });
        Ok(text)
    }
}

/// The full commit a tag or commit of the cached repository names, fetching once when it is
/// missing. A branch is refused: it moves.
fn pinned_commit(dir: &Path, url: &str, rev: &str) -> Result<String> {
    let find = || -> Option<String> {
        let tag = git(
            Some(dir),
            &[
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/tags/{rev}"),
            ],
        )
        .is_ok();
        let commit = rev.len() >= 7 && rev.chars().all(|c| c.is_ascii_hexdigit());
        if !tag && !commit {
            return None;
        }
        git(
            Some(dir),
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{rev}^{{commit}}"),
            ],
        )
        .ok()
        .map(|s| s.trim().to_string())
    };
    if let Some(commit) = find() {
        return Ok(commit);
    }
    fetch(dir)?;
    find().with_context(|| {
        format!("{rev} is not a tag or commit of {url}; branches move, so pin a tag or commit")
    })
}

fn has_commit(dir: &Path, commit: &str) -> bool {
    git(
        Some(dir),
        &["cat-file", "-e", &format!("{commit}^{{commit}}")],
    )
    .is_ok()
}

/// Brings the cached clone up to date, tags that moved included. One worktree fetches at a
/// time, so two never fight over the clone's refs.
fn fetch(dir: &Path) -> Result<()> {
    let lock = std::fs::File::create(dir.with_extension("lock"))?;
    lock.lock()?;
    git(
        Some(dir),
        &[
            "fetch",
            "--quiet",
            "--force",
            "--tags",
            "origin",
            "+refs/heads/*:refs/heads/*",
        ],
    )
    .map(drop)
}

impl Source {
    fn parse(spec: &str, origin: &Origin) -> Result<Source> {
        if spec.starts_with("./") || spec.starts_with("../") {
            return Ok(match origin {
                Origin::Dir(dir) => Source::File(dir.join(spec)),
                Origin::Git { url, rev, dir } => Source::Git {
                    url: url.clone(),
                    rev: rev.clone(),
                    path: normalize(&format!("{dir}/{spec}"))?,
                },
            });
        }
        let git = if let Some(rest) = spec.strip_prefix("github:") {
            Some(("https://github.com/", rest))
        } else {
            spec.strip_prefix("git:").map(|rest| ("", rest))
        };
        if let Some((host, rest)) = git {
            let (place, rev) = rest
                .rsplit_once('@')
                .filter(|(_, rev)| !rev.is_empty() && !rev.contains('/'))
                .with_context(|| {
                    format!("use \"{spec}\": pin it to a tag or commit, like {spec}@v1.0")
                })?;
            let (repo, path) = match place.rsplit_once("//") {
                Some((repo, path)) if !repo.ends_with(':') => (repo, path.to_string()),
                _ => (place, FILE.to_string()),
            };
            if repo.starts_with('-') {
                bail!("use \"{spec}\": a repository URL cannot start with -");
            }
            return Ok(Source::Git {
                url: format!("{host}{repo}"),
                rev: rev.to_string(),
                path: normalize(&path)?,
            });
        }
        if let Some((name, text)) = BUILTIN.iter().find(|(name, _)| *name == spec) {
            Ok(Source::Builtin(name, text))
        } else {
            let names: Vec<&str> = BUILTIN.iter().map(|(n, _)| *n).collect();
            bail!(
                "use \"{spec}\": no such piece; built in are {}, a path starts with ./, and \
                 a repository is github:owner/repo//file.toml@tag",
                names.join(", ")
            )
        }
    }

    fn id(&self) -> String {
        match self {
            Source::Builtin(name, _) => (*name).to_string(),
            Source::File(path) => path
                .canonicalize()
                .unwrap_or_else(|_| path.clone())
                .display()
                .to_string(),
            Source::Git { url, rev, path } => format!("{url}//{path}@{rev}"),
        }
    }

    fn origin(&self) -> Origin {
        match self {
            // A built in piece names only other built in pieces.
            Source::Builtin(..) => Origin::Dir(PathBuf::new()),
            Source::File(path) => {
                Origin::Dir(path.parent().map(Path::to_path_buf).unwrap_or_default())
            }
            Source::Git { url, rev, path } => Origin::Git {
                url: url.clone(),
                rev: rev.clone(),
                dir: path.rsplit_once('/').map_or("", |(d, _)| d).to_string(),
            },
        }
    }
}

/// The files in `root` that make up the config, as repository paths: `fearlezz.toml`, the
/// lock, and every piece used by a path, however deep.
pub(crate) fn config_files(root: &Path) -> Result<Vec<String>> {
    let mut files = vec![FILE.to_string(), LOCK.to_string()];
    let mut todo = vec![FILE.to_string()];
    while let Some(file) = todo.pop() {
        let Ok(text) = std::fs::read_to_string(root.join(&file)) else {
            continue;
        };
        let table: Table = toml::from_str(&text).with_context(|| format!("reading {file}"))?;
        let dir = file.rsplit_once('/').map_or("", |(d, _)| d);
        let named = table.get("use").and_then(Value::as_array);
        for spec in named.into_iter().flatten().filter_map(Value::as_str) {
            if !(spec.starts_with("./") || spec.starts_with("../")) {
                continue;
            }
            let path = normalize(&format!("{dir}/{spec}"))?;
            if !files.contains(&path) {
                files.push(path.clone());
                todo.push(path);
            }
        }
    }
    Ok(files)
}

/// Merges `from` into `into`: tables merge key by key, anything else replaces. Pieces merge
/// this way, and so does each tool's settings over its lint tables.
pub(crate) fn merge(into: &mut Table, from: Table) {
    for (key, value) in from {
        match (into.get_mut(&key), value) {
            (Some(Value::Table(a)), Value::Table(b)) => merge(a, b),
            (_, value) => {
                into.insert(key, value);
            }
        }
    }
}

/// A path inside a repository, with `.` and `..` resolved; it may not leave the repository.
fn normalize(path: &str) -> Result<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts
                    .pop()
                    .with_context(|| format!("{path} points outside its repository"))?;
            }
            p => parts.push(p),
        }
    }
    Ok(parts.join("/"))
}

fn short_hash(text: &str) -> String {
    hex::encode(&Sha256::digest(text.as_bytes())[..8])
}

fn git(dir: Option<&Path>, args: &[&str]) -> Result<String> {
    let mut cmd = Command::new("git");
    if let Some(dir) = dir {
        cmd.arg("-C").arg(dir);
    }
    let out = cmd.args(args).output().context("running git")?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(text: &str) -> Table {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn the_naming_file_wins_field_by_field() {
        let mut merged = table("[check.tests]\nrun = \"cargo test\"\nstage = \"working\"\n");
        merge(&mut merged, table("[check.tests]\nstage = \"look\"\n"));
        assert_eq!(
            merged,
            table("[check.tests]\nrun = \"cargo test\"\nstage = \"look\"\n")
        );
    }

    #[test]
    fn git_pieces_parse_with_and_without_a_path() {
        let here = Origin::Dir(PathBuf::from("/repo"));
        let Source::Git { url, rev, path } =
            Source::parse("github:acme/presets//web/fearlezz.toml@v1.2", &here).unwrap()
        else {
            panic!("not a git piece")
        };
        assert_eq!(
            (url.as_str(), rev.as_str(), path.as_str()),
            (
                "https://github.com/acme/presets",
                "v1.2",
                "web/fearlezz.toml"
            )
        );
        let Source::Git { url, path, .. } =
            Source::parse("git:https://example.com/p.git@abc1234", &here).unwrap()
        else {
            panic!("not a git piece")
        };
        assert_eq!(
            (url.as_str(), path.as_str()),
            ("https://example.com/p.git", FILE)
        );
        assert!(Source::parse("github:acme/presets//web.toml", &here).is_err());
    }

    #[test]
    fn a_relative_piece_inside_a_repository_stays_in_it() {
        let inside = Origin::Git {
            url: "u".into(),
            rev: "v1".into(),
            dir: "web".into(),
        };
        let Source::Git { path, .. } = Source::parse("../base.toml", &inside).unwrap() else {
            panic!("not a git piece")
        };
        assert_eq!(path, "base.toml");
        assert!(Source::parse("../../x.toml", &inside).is_err());
    }
}
