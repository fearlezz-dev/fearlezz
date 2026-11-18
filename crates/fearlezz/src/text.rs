//! Comparing two texts line by line: the one diff generated code and text looks share.

use similar::{ChangeTag, TextDiff};

/// How many lines `after` adds and removes compared with `before`.
pub(crate) fn counts(before: &str, after: &str) -> (usize, usize) {
    let diff = TextDiff::from_lines(before, after);
    let mut added = 0;
    let mut removed = 0;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => added += 1,
            ChangeTag::Delete => removed += 1,
            ChangeTag::Equal => {}
        }
    }
    (added, removed)
}

/// The unified diff from `before` to `after` of the file called `name`, with three lines of
/// context; empty when they are the same.
pub(crate) fn unified(name: &str, before: &str, after: &str) -> String {
    TextDiff::from_lines(before, after)
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{name}"), &format!("b/{name}"))
        .to_string()
}

/// Every line of `was` and `now` in order, as a diff without headers or hunks: a line only in
/// `was` gets `-`, only in `now` `+`, in both a space.
pub(crate) fn lines(was: &[&str], now: &[&str]) -> Vec<String> {
    TextDiff::from_slices(was, now)
        .iter_all_changes()
        .map(|change| {
            let sign = match change.tag() {
                ChangeTag::Delete => '-',
                ChangeTag::Insert => '+',
                ChangeTag::Equal => ' ',
            };
            format!("{sign}{}", change.value())
        })
        .collect()
}

/// Bytes as text for a diff: a file that is not UTF-8 is named as binary instead, by its size
/// and digest, so two that differ never read the same.
pub(crate) fn readable(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_string();
    }
    let digest: String = crate::files::sha256_hex(bytes).chars().take(16).collect();
    format!("(binary, {} bytes, sha256 {digest}...)\n", bytes.len())
}

/// How many characters of one line are shown, so a minified file stays readable.
const WIDTH: usize = 300;

/// The first `lines` lines of `text`, each cut at a readable width, and how many lines were
/// left out.
pub(crate) fn head(text: &str, lines: usize) -> (Vec<String>, usize) {
    let cut: Vec<String> = text
        .lines()
        .take(lines)
        .map(|line| {
            let shown: String = line.chars().take(WIDTH).collect();
            let more = line.chars().count().saturating_sub(WIDTH);
            if more == 0 {
                shown
            } else {
                format!("{shown}... ({more} more characters)")
            }
        })
        .collect();
    let left = text.lines().count() - cut.len();
    (cut, left)
}
