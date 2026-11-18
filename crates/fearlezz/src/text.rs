//! Comparing two texts line by line: the one diff generated code and text looks share.

use fearlezz_proof::look::DiffLine;
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

/// The lines of `after` against `before` as the proof host's page shows them: each line kept,
/// only in `before` or only in `after`, and each run of kept lines longer than twice `context`
/// folded to a count, `context` kept lines staying on each side of a change (none before the
/// first change or after the last). A last line with or without a line break is the same line.
pub(crate) fn shown(before: &str, after: &str, context: usize) -> Vec<DiffLine> {
    fn split(text: &str) -> Vec<&str> {
        if text.is_empty() {
            return Vec::new();
        }
        text.strip_suffix('\n')
            .unwrap_or(text)
            .split('\n')
            .collect()
    }
    let (was, now) = (split(before), split(after));
    let lines: Vec<DiffLine> = TextDiff::from_slices(&was, &now)
        .iter_all_changes()
        .map(|change| {
            let text = change.value().to_string();
            match change.tag() {
                ChangeTag::Delete => DiffLine::Was { text },
                ChangeTag::Insert => DiffLine::Now { text },
                ChangeTag::Equal => DiffLine::Same { text },
            }
        })
        .collect();
    let mut out = Vec::new();
    let mut run = Vec::new();
    let mut changed = false;
    for line in lines {
        if matches!(line, DiffLine::Same { .. }) {
            run.push(line);
        } else {
            fold(
                &mut out,
                &mut run,
                if changed { context } else { 0 },
                context,
            );
            changed = true;
            out.push(line);
        }
    }
    fold(&mut out, &mut run, if changed { context } else { 0 }, 0);
    out
}

/// Folds a run of kept lines, keeping `before` lines at its start and `after` at its end.
fn fold(out: &mut Vec<DiffLine>, run: &mut Vec<DiffLine>, before: usize, after: usize) {
    let n = run.len();
    if n > before + after {
        out.extend(run.iter().take(before).cloned());
        out.push(DiffLine::Fold {
            count: u32::try_from(n - before - after).unwrap_or(u32::MAX),
        });
        out.extend(run.iter().skip(n - after).cloned());
    } else {
        out.append(run);
    }
    run.clear();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn read(lines: &[DiffLine]) -> Vec<String> {
        lines
            .iter()
            .map(|l| match l {
                DiffLine::Same { text } => format!(" {text}"),
                DiffLine::Was { text } => format!("-{text}"),
                DiffLine::Now { text } => format!("+{text}"),
                DiffLine::Fold { count } => format!("({count})"),
            })
            .collect()
    }

    #[test]
    fn kept_lines_fold_past_the_context_around_each_change() {
        let before = (1..=12)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        let after = before.replace("5\n", "five\n");
        assert_eq!(
            read(&shown(&before, &after, 2)),
            ["(2)", " 3", " 4", "-5", "+five", " 6", " 7", "(5)"]
        );
        // A last line without its line break is the same line.
        assert_eq!(read(&shown("a\nb", "a\nb\n", 3)), ["(2)"]);
        assert_eq!(read(&shown("", "a\n", 3)), ["+a"]);
    }
}
