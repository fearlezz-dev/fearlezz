//! What a Fearlezz binary ships, as one digest: its own source and the proof model's, the
//! locked dependencies, its built in presets and the agent skill it embeds. The build script computes it into `FEARLEZZ_SHIPPED`; the built in
//! checks key their results on it, since their code and the files they compare against decide
//! their verdicts, and two builds may say the same version while shipping different files.
//!
//! Only std, sha2 and hex, so the build script and the tests include this file as it is.

use sha2::{Digest as _, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The folders and files, relative to the crate's folder, that make what the binary ships.
pub(crate) const SOURCES: &[&str] = &[
    "Cargo.toml",
    "src",
    "presets",
    "../../skills/fearlezz",
    "../../Cargo.lock",
    "../fearlezz-proof/Cargo.toml",
    "../fearlezz-proof/src",
];

/// The SHA-256 hex of every file under `SOURCES` in `crate_dir`: each one's path from the
/// crate's folder and its bytes, in path order, so it is the same on every machine.
pub(crate) fn digest(crate_dir: &Path) -> io::Result<String> {
    let mut files = Vec::new();
    for source in SOURCES {
        collect(crate_dir, &crate_dir.join(source), &mut files)?;
    }
    files.sort();
    let mut hasher = Sha256::new();
    for (name, path) in files {
        let bytes = fs::read(&path)?;
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// The files under `path`. Hidden entries (an editor's lock or backup, `.DS_Store`) are left
/// out, since they ship nothing; a symbolic link counts only when it names a file, so a broken
/// one never fails the build and a linked folder never loops.
fn collect(crate_dir: &Path, path: &Path, files: &mut Vec<(String, PathBuf)>) -> io::Result<()> {
    let hidden = path
        .file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with('.'));
    let kind = fs::symlink_metadata(path)?.file_type();
    if hidden || (kind.is_symlink() && !fs::metadata(path).is_ok_and(|m| m.is_file())) {
        return Ok(());
    }
    if kind.is_dir() {
        for entry in fs::read_dir(path)? {
            collect(crate_dir, &entry?.path(), files)?;
        }
    } else {
        let name = path
            .strip_prefix(crate_dir)
            .unwrap_or(path)
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        files.push((name, path.to_path_buf()));
    }
    Ok(())
}
