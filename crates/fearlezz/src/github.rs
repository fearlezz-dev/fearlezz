//! The pull request on GitHub: changed pictures shown in the status comment, the hidden line
//! an approval by comment checks, and approvals read back from the pull request. CI writes the
//! comment and answers `/fearlezz approve`; the scripts around it live in `action.sh`.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::look::{Change, Look};

/// Starts the hidden line of the status comment that says what an approval covers.
pub(crate) const WAITING_MARKER: &str = "<!-- fearlezz-pictures ";

/// Starts the hidden line of CI's reply to an approval.
pub(crate) const APPROVED_MARKER: &str = "<!-- fearlezz-approved ";

/// Pictures shown in one comment at most, so it stays readable and under GitHub's size limit.
const SHOWN: usize = 20;

/// What waits for a person on the pull request, as the status comment's hidden line holds it.
#[derive(Serialize, Deserialize)]
pub(crate) struct Waiting {
    /// The pull request's head commit the proof is for.
    pub head: String,
    /// A hash of each check's inputs and the base commit: what an approval covers.
    pub fingerprint: String,
    pub checks: Vec<String>,
    /// The headline once a person approves, with `{by}` for their login.
    pub approved: String,
}

/// An approval as CI's reply records it.
#[derive(Serialize, Deserialize)]
pub(crate) struct Approval {
    pub head: String,
    pub fingerprint: String,
    pub checks: Vec<String>,
    pub by: String,
    pub said: String,
    /// Where the reply is, filled in when it is read back.
    #[serde(default)]
    pub url: String,
}

/// A look with pictures to look at, and the inputs key of the run it judged.
pub(crate) struct Waits {
    pub look: Look,
    pub key: String,
}

/// The hash of what an approval covers: each check, its inputs, and the base it compared with.
pub(crate) fn fingerprint(waits: &[Waits]) -> String {
    let mut parts: Vec<String> = waits
        .iter()
        .map(|w| format!("{}\0{}\0{}", w.look.check, w.key, w.look.base_commit))
        .collect();
    parts.sort();
    hex::encode(Sha256::digest(parts.join("\n")))
}

/// The hidden line, safe inside an HTML comment: `>` never appears in it unescaped.
pub(crate) fn hidden(marker: &str, value: &impl Serialize) -> Result<String> {
    let json = serde_json::to_string(value)?.replace('>', "\\u003e");
    Ok(format!("{marker}{json} -->"))
}

/// The folder of a look, whose files the pictures folder mirrors.
fn folder(look: &Look) -> String {
    look.report
        .parent()
        .and_then(Path::file_name)
        .map(|d| d.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Copies the before and after of every picture to look at into `dir`, at the same path below
/// it as below the look folders.
pub(crate) fn copy_pictures(waits: &[Waits], dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    for w in waits {
        let Some(from) = w.look.report.parent() else {
            continue;
        };
        let to = dir.join(folder(&w.look));
        for item in w.look.items.iter().filter(|i| i.change != Change::Same) {
            for rel in [&item.before, &item.after].into_iter().flatten() {
                let target = to.join(rel);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(from.join(rel), &target)
                    .with_context(|| format!("copying {rel} into {}", dir.display()))?;
            }
        }
    }
    Ok(())
}

/// A path segment as a URL holds it.
fn encode(path: &str) -> String {
    let mut out = String::new();
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push('%');
            out.push_str(&hex::encode_upper([b]));
        }
    }
    out
}

/// Text for a table cell or an HTML attribute.
fn cell(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('|', "\\|")
        .replace('\n', " ")
}

/// Where the pictures are shown from.
pub(crate) struct Shown<'a> {
    /// The URL the pictures folder is served at, when CI publishes it.
    pub url: Option<&'a str>,
    /// CI keeps the pictures folder as the `fearlezz-pictures` artifact.
    pub artifact: bool,
}

/// The pictures a person should look at: before and after side by side when the folder is
/// published, and how to approve them.
pub(crate) fn section(text: &mut String, waits: &[Waits], shown: &Shown<'_>) -> std::fmt::Result {
    if waits.is_empty() {
        return Ok(());
    }
    writeln!(text, "#### Pictures to look at\n")?;
    let mut left = SHOWN;
    let mut hidden = 0;
    for w in waits {
        let why = w
            .look
            .no_before
            .as_ref()
            .map(|w| format!(" (no before: {})", cell(w)))
            .unwrap_or_default();
        writeln!(text, "**`{}`**: {}{why}\n", w.look.check, w.look.counts())?;
        let items: Vec<_> = w
            .look
            .items
            .iter()
            .filter(|i| i.change != Change::Same)
            .collect();
        let take = items.len().min(left);
        hidden += items.len() - take;
        left -= take;
        if take == 0 {
            continue;
        }
        let dir = folder(&w.look);
        let media = |rel: &Option<String>, kind: &str, url: &str| match rel {
            None => String::new(),
            Some(rel) => {
                let src = format!("{url}/{}", encode(&format!("{dir}/{rel}")));
                let moving = [".gif", ".webp", ".apng", ".png"]
                    .iter()
                    .any(|e| rel.to_lowercase().ends_with(e));
                if kind == "still" || moving {
                    format!("<a href=\"{src}\"><img src=\"{src}\" width=\"320\"></a>")
                } else {
                    format!("[film]({src})")
                }
            }
        };
        match shown.url {
            Some(url) => {
                writeln!(text, "| picture | before | after |\n| --- | --- | --- |")?;
                for item in items.iter().take(take) {
                    writeln!(
                        text,
                        "| **{}** {}<br>{} | {} | {} |",
                        cell(&item.name),
                        label(item),
                        cell(&item.summary),
                        media(&item.before, &item.kind, url),
                        media(&item.after, &item.kind, url)
                    )?;
                }
            }
            None => {
                for item in items.iter().take(take) {
                    writeln!(
                        text,
                        "- **{}** {}: {}",
                        cell(&item.name),
                        label(item),
                        cell(&item.summary)
                    )?;
                }
            }
        }
        writeln!(text)?;
    }
    if hidden > 0 {
        writeln!(text, "And {hidden} more.\n")?;
    }
    if shown.artifact && (shown.url.is_none() || hidden > 0) {
        writeln!(
            text,
            "Every changed picture, before and after, is in the `fearlezz-pictures` artifact of \
             the run.\n"
        )?;
    }
    writeln!(
        text,
        "To approve them, a person with write access comments `/fearlezz approve <what they \
         think of them>`. A new push needs a new approval.\n"
    )
}

/// An item's change, and that it never blocks when the check calls it flaky.
fn label(item: &crate::look::Item) -> String {
    if item.flaky {
        format!("{} (flaky, never blocks)", item.change.label())
    } else {
        item.change.label().to_string()
    }
}

/// The approvals CI recorded on the pull request of the branch checked out in `root`, oldest
/// first. Only replies written by a bot count: a person's account, which an agent may share,
/// cannot write one.
pub(crate) fn approvals(root: &Path) -> Result<Vec<Approval>> {
    let number = gh(root, &["pr", "view", "--json", "number", "--jq", ".number"])?;
    let number = number.trim();
    if number.is_empty() {
        bail!("this branch has no pull request on GitHub");
    }
    let comments = gh(
        root,
        &[
            "api",
            &format!("repos/{{owner}}/{{repo}}/issues/{number}/comments"),
            "--paginate",
            "--jq",
            ".[] | select(.user.type == \"Bot\") | {body, url: .html_url}",
        ],
    )?;
    let mut found = Vec::new();
    for line in comments.lines().filter(|l| !l.trim().is_empty()) {
        let comment: Comment = serde_json::from_str(line)
            .with_context(|| format!("reading a comment of pull request {number}"))?;
        for hidden in comment
            .body
            .lines()
            .filter_map(|l| l.trim().strip_prefix(APPROVED_MARKER))
        {
            let json = hidden.trim_end().trim_end_matches("-->").trim_end();
            if let Ok(mut approval) = serde_json::from_str::<Approval>(json) {
                approval.url.clone_from(&comment.url);
                found.push(approval);
            }
        }
    }
    Ok(found)
}

/// A comment on the pull request, as `approvals` asks the GitHub CLI for it.
#[derive(Deserialize)]
struct Comment {
    body: String,
    url: String,
}

/// Runs the GitHub CLI in `root` and returns what it printed.
fn gh(root: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("gh")
        .args(args)
        .current_dir(root)
        .output()
        .context("reading the pull request needs the GitHub CLI: install gh and sign in with gh auth login")?;
    if !out.status.success() {
        bail!(
            "gh {} failed: {}",
            args.first().copied().unwrap_or_default(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
