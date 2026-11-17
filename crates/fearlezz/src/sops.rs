//! What sops says must be encrypted, and whether a file is: the `path_regex` of every
//! `.sops.yaml` creation rule, matched as sops matches it (against the path from the folder
//! the `.sops.yaml` is in), and the metadata sops writes into every file it encrypts. Nothing
//! is decrypted, and nothing read from a file is kept or shown.

use std::path::Path;

use anyhow::{Context as _, Result};
use regex::Regex;

/// The name sops looks for, from the working folder upwards.
const CONFIG: &str = ".sops.yaml";

/// The files of `files` (the worktree's, from its root) a `.sops.yaml` names: each file is
/// judged by the nearest `.sops.yaml` in its folder or above, as sops run there finds it. A
/// creation rule without `path_regex` names no file: it only says how to encrypt one.
pub(crate) fn named(root: &Path, files: &[String]) -> Result<Vec<String>> {
    let mut configs: Vec<(String, Vec<Regex>)> = Vec::new();
    for file in files {
        let Some(folder) = config_folder(file) else {
            continue;
        };
        let text =
            std::fs::read_to_string(root.join(file)).with_context(|| format!("reading {file}"))?;
        let regexes = path_regexes(&text)
            .iter()
            .map(|r| Regex::new(r).with_context(|| format!("{file}: path_regex {r}")))
            .collect::<Result<Vec<_>>>()?;
        configs.push((folder, regexes));
    }
    // The deepest folder first, so the nearest one decides.
    configs.sort_by_key(|(folder, _)| std::cmp::Reverse(folder.len()));
    Ok(files
        .iter()
        .filter(|f| config_folder(f).is_none())
        .filter(|f| {
            configs
                .iter()
                .find_map(|(folder, regexes)| {
                    let from = if folder.is_empty() {
                        Some(f.as_str())
                    } else {
                        f.strip_prefix(folder.as_str())?.strip_prefix('/')
                    };
                    from.map(|from| regexes.iter().any(|r| r.is_match(from)))
                })
                .unwrap_or(false)
        })
        .cloned()
        .collect())
}

/// The folder of a `.sops.yaml` (empty for the root), none for any other file.
fn config_folder(file: &str) -> Option<String> {
    if file == CONFIG {
        return Some(String::new());
    }
    file.strip_suffix(&format!("/{CONFIG}")).map(String::from)
}

/// The `path_regex` values of a `.sops.yaml`, plain or quoted as YAML writes a string.
fn path_regexes(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let rest = line.trim_start().trim_start_matches('-').trim_start();
            let value = rest.strip_prefix("path_regex:")?.trim();
            Some(unquote(value))
        })
        .collect()
}

fn unquote(value: &str) -> String {
    if let Some(inner) = value.strip_prefix('\'').and_then(|v| v.rsplit_once('\'')) {
        return inner.0.replace("''", "'");
    }
    if let Some(inner) = value.strip_prefix('"').and_then(|v| v.rsplit_once('"')) {
        let mut out = String::new();
        let mut chars = inner.0.chars();
        while let Some(c) = chars.next() {
            match (c, c == '\\') {
                (_, true) => match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(other) => out.push(other),
                    None => {}
                },
                (c, false) => out.push(c),
            }
        }
        return out;
    }
    // A comment starts at a space and a hash.
    value
        .split_once(" #")
        .map_or(value, |(v, _)| v)
        .trim()
        .to_string()
}

/// Whether the file carries the metadata sops writes when it encrypts: a top level `sops`
/// key in YAML and JSON (binary files are stored as JSON), a `[sops]` section in INI, and
/// `sops_` keys in dotenv files. A file that cannot be read is not encrypted.
pub(crate) fn encrypted(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let extension = name.rsplit_once('.').map_or("", |(_, e)| e);
    match extension {
        "yaml" | "yml" => text.lines().any(|l| l.starts_with("sops:")),
        "ini" => text.lines().any(|l| l.trim() == "[sops]"),
        "env" => text.lines().any(|l| l.starts_with("sops_version=")),
        _ => serde_json::from_str::<serde_json::Value>(&text)
            .is_ok_and(|json| json.get("sops").is_some_and(serde_json::Value::is_object)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_regex_is_read_plain_or_quoted() {
        let text = "creation_rules:\n  - path_regex: a/.*\\.yaml$ # secrets\n    age: x\n  - path_regex: 'b/it''s'\n  - age: y\n    path_regex: \"c/.*\\\\.enc\"\n";
        assert_eq!(path_regexes(text), ["a/.*\\.yaml$", "b/it's", "c/.*\\.enc"]);
    }

    #[test]
    fn a_dotenv_file_named_dot_env_is_read_as_dotenv() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        std::fs::write(&path, "A=ENC[x]\nsops_version=3.13.3\n").unwrap();
        assert!(encrypted(&path));
    }
}
