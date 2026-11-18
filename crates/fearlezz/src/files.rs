//! Small file and folder facts every module shares: hashes, the per user folders, reads that
//! tell a missing file from a broken one, and writes that never leave half a file.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use sha2::{Digest as _, Sha256};

/// The SHA-256 of `bytes` as lowercase hex.
pub(crate) fn sha256_hex(bytes: impl AsRef<[u8]>) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// A per user folder of Fearlezz.
#[derive(Clone, Copy)]
pub(crate) enum UserDir {
    /// Kept: the proof host's sign in.
    Config,
    /// Can be thrown away: base checkouts, pieces, the machine's film log.
    Cache,
}

impl UserDir {
    /// Below `$XDG_CONFIG_HOME` or `$XDG_CACHE_HOME` when set to an absolute path, else the
    /// platform's (`~/Library/Application Support` or `~/Library/Caches` on macOS,
    /// `~/.config` or `~/.cache` elsewhere).
    pub(crate) fn path(self) -> Result<PathBuf> {
        let (xdg, macos, other, needed) = match self {
            UserDir::Config => (
                "XDG_CONFIG_HOME",
                "Library/Application Support",
                ".config",
                "config folder for the proof host's sign in",
            ),
            UserDir::Cache => (
                "XDG_CACHE_HOME",
                "Library/Caches",
                ".cache",
                "cache folder for base checkouts",
            ),
        };
        let set = std::env::var_os(xdg)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute());
        if let Some(dir) = set {
            return Ok(dir.join("fearlezz"));
        }
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .with_context(|| format!("HOME is not set, so there is no {needed}"))?;
        let base = if cfg!(target_os = "macos") {
            macos
        } else {
            other
        };
        Ok(home.join(base).join("fearlezz"))
    }
}

/// Every file at or below `at`, as a path relative to `base` with forward slashes. A folder
/// that cannot be read counts as empty.
pub(crate) fn walk(base: &Path, at: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut todo = vec![at.to_path_buf()];
    while let Some(path) = todo.pop() {
        if path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&path) {
                todo.extend(entries.flatten().map(|e| e.path()));
            }
        } else if path.is_file()
            && let Ok(rel) = path.strip_prefix(base)
        {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    out
}

/// The text of `path`, or None when there is no such file. Any other read error is an error,
/// so a file that cannot be read is never taken for one that is not there.
pub(crate) fn read_if_exists(path: &Path) -> std::io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Removes `path`; one that is already gone is fine.
pub(crate) fn remove_if_exists(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(e).with_context(|| format!("removing {}", path.display()))
        }
        _ => Ok(()),
    }
}

/// Writes `content` to `path` whole: to a temporary file beside it first, then renamed over
/// it, so a reader or a failed write never sees half of it. Its folder is made when missing.
pub(crate) fn write_atomic(path: &Path, content: impl AsRef<[u8]>) -> Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    // A temporary file of its own for every write, removed when anything fails.
    let mut temp = tempfile::NamedTempFile::new_in(dir)
        .and_then(|mut temp| temp.write_all(content.as_ref()).map(|()| temp))
        .with_context(|| format!("writing {}", path.display()))?;
    temp.persist(path)
        .map(drop)
        .with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_none_and_a_folder_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_if_exists(&dir.path().join("none")).unwrap(), None);
        read_if_exists(dir.path()).unwrap_err();
    }

    #[test]
    fn a_whole_write_leaves_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/b.txt");
        write_atomic(&path, "one").unwrap();
        write_atomic(&path, "two").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        let names: Vec<_> = std::fs::read_dir(dir.path().join("a"))
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["b.txt"]);
        remove_if_exists(&path).unwrap();
        remove_if_exists(&path).unwrap();
    }

    #[test]
    fn a_walk_finds_every_file_below_a_folder_or_the_file_itself() {
        let dir = tempfile::tempdir().unwrap();
        write_atomic(&dir.path().join("a/b/c.txt"), "").unwrap();
        write_atomic(&dir.path().join("a/d.txt"), "").unwrap();
        let mut found = walk(dir.path(), &dir.path().join("a"));
        found.sort();
        assert_eq!(found, ["a/b/c.txt", "a/d.txt"]);
        assert_eq!(walk(dir.path(), &dir.path().join("a/d.txt")), ["a/d.txt"]);
        assert_eq!(
            walk(dir.path(), &dir.path().join("none")),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_hash_is_lowercase_hex() {
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
