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

/// What a file shows of sops.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum State {
    /// No sops metadata: it was never encrypted, or was decrypted.
    Plain,
    /// Encrypted, but with values sops would encrypt in plain text, at these lines (0 where
    /// the format gives none).
    Leaks(Vec<u32>),
    Encrypted,
}

/// Which values sops encrypts in a file, as its metadata records the creation rule: keys
/// matching `encrypted_regex` or ending in `encrypted_suffix` (and all below them), or every
/// key but those matching `unencrypted_regex` or ending in `unencrypted_suffix`.
enum Mode {
    Only(Box<dyn Fn(&str) -> bool>),
    Except(Box<dyn Fn(&str) -> bool>),
    /// By comments, which this reader does not follow: values are not judged.
    Unknown,
}

impl Mode {
    fn from(field: impl Fn(&str) -> Option<String>) -> Mode {
        let regex = |key: &str| Regex::new(&field(key)?).ok();
        if let Some(r) = regex("encrypted_regex") {
            return Mode::Only(Box::new(move |k| r.is_match(k)));
        }
        if let Some(r) = regex("unencrypted_regex") {
            return Mode::Except(Box::new(move |k| r.is_match(k)));
        }
        if let Some(s) = field("encrypted_suffix") {
            return Mode::Only(Box::new(move |k| k.ends_with(&s)));
        }
        if field("encrypted_comment_regex").is_some()
            || field("unencrypted_comment_regex").is_some()
        {
            return Mode::Unknown;
        }
        let s = field("unencrypted_suffix").unwrap_or_else(|| "_unencrypted".into());
        Mode::Except(Box::new(move |k| k.ends_with(&s)))
    }

    /// Whether sops encrypts a value at this path of keys.
    fn encrypts(&self, path: &[String]) -> bool {
        match self {
            Mode::Only(f) => path.iter().any(|k| f(k)),
            Mode::Except(f) => !path.iter().any(|k| f(k)),
            Mode::Unknown => false,
        }
    }
}

fn is_ciphertext(value: &str) -> bool {
    value
        .trim_matches(|c| c == '"' || c == '\'')
        .starts_with("ENC[")
}

/// The file as sops left it: its metadata, a top level `sops` key in YAML and JSON (binary
/// files are stored as JSON), `sops_` keys in dotenv files, a `[sops]` section in INI; and
/// every value sops would encrypt in ciphertext (INI values are not judged). Nothing read is
/// kept but line numbers. A file that cannot be read is plain.
pub(crate) fn state(path: &Path) -> State {
    let Ok(text) = std::fs::read_to_string(path) else {
        return State::Plain;
    };
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let extension = name.rsplit_once('.').map_or("", |(_, e)| e);
    let leaks = match extension {
        "yaml" | "yml" => yaml(&text),
        "ini" => text.lines().any(|l| l.trim() == "[sops]").then(Vec::new),
        "env" => dotenv(&text),
        _ => json(&text),
    };
    match leaks {
        None => State::Plain,
        Some(lines) if lines.is_empty() => State::Encrypted,
        Some(lines) => State::Leaks(lines),
    }
}

/// The lines of plain values sops would encrypt, none without metadata. YAML is read by
/// indentation, as sops writes it: `key: value`, `key:` above deeper lines, and `- value`
/// list items under the key above them.
fn yaml(text: &str) -> Option<Vec<u32>> {
    let top = |l: &str| !l.starts_with([' ', '\t', '#', '-']) && !l.trim().is_empty();
    let mut in_sops = false;
    let mut meta = std::collections::BTreeMap::new();
    for line in text.lines() {
        if top(line) {
            in_sops = line.trim_end() == "sops:";
        } else if in_sops && let Some((k, v)) = line.trim().split_once(": ") {
            meta.insert(k.to_string(), unquote(v));
        }
    }
    if !text.lines().any(|l| l.trim_end() == "sops:") {
        return None;
    }
    let mode = Mode::from(|k| meta.get(k).cloned());
    let mut leaks = Vec::new();
    // Each open key with the indentation of its line.
    let mut keys: Vec<(usize, String)> = Vec::new();
    let mut block: Option<usize> = None;
    let mut in_sops = false;
    for (n, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed == "---" {
            continue;
        }
        // The lines of a block scalar belong to its key.
        if block.is_some_and(|b| indent > b) {
            continue;
        }
        block = None;
        if top(line) {
            in_sops = line.trim_end() == "sops:";
        }
        if in_sops {
            continue;
        }
        while keys.last().is_some_and(|(i, _)| *i >= indent) {
            keys.pop();
        }
        let item = trimmed
            .strip_prefix("- ")
            .or((trimmed == "-").then_some(""));
        let (entry, at) = match item {
            Some(rest) => (
                rest.trim_start(),
                indent + (trimmed.len() - rest.trim_start().len()),
            ),
            None => (trimmed, indent),
        };
        let pair = entry
            .split_once(": ")
            .or_else(|| entry.strip_suffix(':').map(|k| (k, "")));
        let (key, value) = match pair {
            Some((k, v)) if !k.starts_with(['"', '\'', '{', '[']) || k.ends_with(['"', '\'']) => (
                Some(k.trim_matches(|c| c == '"' || c == '\'').to_string()),
                v.trim(),
            ),
            _ => (None, entry.trim()),
        };
        let mut path: Vec<String> = keys.iter().map(|(_, k)| k.clone()).collect();
        if let Some(key) = &key {
            path.push(key.clone());
        }
        let value = value.split_once(" #").map_or(value, |(v, _)| v).trim();
        if value.is_empty() {
            if let Some(key) = key {
                keys.push((at, key));
            }
            continue;
        }
        if value.starts_with(['|', '>']) {
            block = Some(indent);
        }
        let empty = matches!(value, "null" | "~" | "{}" | "[]");
        if mode.encrypts(&path) && !empty && !is_ciphertext(value) {
            leaks.push(u32::try_from(n + 1).unwrap_or(u32::MAX));
        }
    }
    Some(leaks)
}

fn dotenv(text: &str) -> Option<Vec<u32>> {
    let pairs: Vec<(u32, &str, &str)> = text
        .lines()
        .enumerate()
        .filter_map(|(n, l)| {
            let (k, v) = l.split_once('=')?;
            Some((u32::try_from(n + 1).unwrap_or(u32::MAX), k.trim(), v.trim()))
        })
        .filter(|(_, k, _)| !k.starts_with('#'))
        .collect();
    if !pairs.iter().any(|(_, k, _)| *k == "sops_version") {
        return None;
    }
    let mode = Mode::from(|key| {
        pairs
            .iter()
            .find(|(_, k, _)| k.strip_prefix("sops_") == Some(key))
            .map(|(_, _, v)| (*v).to_string())
    });
    Some(
        pairs
            .iter()
            .filter(|(_, k, v)| {
                !k.starts_with("sops_") && mode.encrypts(&[(*k).to_string()]) && !is_ciphertext(v)
            })
            .map(|(n, _, _)| *n)
            .collect(),
    )
}

fn json(text: &str) -> Option<Vec<u32>> {
    let doc: serde_json::Value = serde_json::from_str(text).ok()?;
    let meta = doc.get("sops").filter(|m| m.is_object())?;
    let mode = Mode::from(|k| {
        meta.get(k)
            .and_then(serde_json::Value::as_str)
            .map(String::from)
    });
    let mut plain = false;
    walk(&doc, &mut Vec::new(), &mode, &mut plain);
    // JSON gives no lines; the file as a whole holds the leak.
    Some(if plain { vec![0] } else { Vec::new() })
}

/// Whether a value under `path` that sops encrypts is plain.
fn walk(v: &serde_json::Value, path: &mut Vec<String>, mode: &Mode, plain: &mut bool) {
    match v {
        serde_json::Value::Object(o) => {
            for (k, v) in o {
                if path.is_empty() && k == "sops" {
                    continue;
                }
                path.push(k.clone());
                walk(v, path, mode, plain);
                path.pop();
            }
        }
        serde_json::Value::Array(a) => a.iter().for_each(|v| walk(v, path, mode, plain)),
        serde_json::Value::Null => {}
        serde_json::Value::String(s) if is_ciphertext(s) => {}
        serde_json::Value::String(_)
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_) => *plain |= mode.encrypts(path),
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
        assert_eq!(state(&path), State::Encrypted);
    }
}
