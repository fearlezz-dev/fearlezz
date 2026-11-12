//! GitHub Actions: what `fearlezz run --ci` does for the event a workflow runs on. A pull
//! request is proven, its changed pictures pushed to the pictures branch, its status comment
//! written and its `fearlezz/pictures` status set; a `/fearlezz approve <words>` comment is
//! answered, and deleting it takes the approval back; a closed pull request's pictures are
//! removed; the schedule removes the pictures of every closed pull request. A fork's run has a
//! read only token, so it writes nothing to GitHub.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::github::{self, Api, StatusComment};

/// The branch the changed pictures are pushed to, one folder per pull request.
pub(crate) const BRANCH: &str = "fearlezz-pictures";

/// Pushes that lose a race with another pull request's push start over this often.
const ATTEMPTS: usize = 3;

/// The workflow run Fearlezz runs in, as GitHub describes it to every step.
pub(crate) struct Actions {
    name: String,
    event: Value,
    api: Api,
    server: String,
    repo: String,
}

/// What the event asks for.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Task {
    Prove,
    Approve,
    /// An approval comment was deleted.
    Withdraw,
    Clean,
    Squash,
    /// A comment that is no approval.
    Nothing,
}

impl Actions {
    /// The workflow run, when Fearlezz runs on GitHub Actions with a token; None elsewhere.
    pub(crate) fn detect() -> Result<Option<Actions>> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        if var("GITHUB_ACTIONS").as_deref() != Some("true") {
            return Ok(None);
        }
        let (Some(name), Some(path), Some(token), Some(repo)) = (
            var("GITHUB_EVENT_NAME"),
            var("GITHUB_EVENT_PATH"),
            var("GITHUB_TOKEN").or_else(|| var("GH_TOKEN")),
            var("GITHUB_REPOSITORY"),
        ) else {
            return Ok(None);
        };
        let event = serde_json::from_slice(
            &fs::read(&path).with_context(|| format!("reading the event file {path}"))?,
        )
        .with_context(|| format!("reading the event file {path}"))?;
        Ok(Some(Actions {
            name,
            event,
            api: Api::new(token, repo.clone()),
            server: var("GITHUB_SERVER_URL").unwrap_or_else(|| "https://github.com".into()),
            repo,
        }))
    }

    fn text(&self, pointer: &str) -> String {
        self.event
            .pointer(pointer)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }

    fn number(&self, pointer: &str) -> Option<u64> {
        self.event.pointer(pointer).and_then(Value::as_u64)
    }

    pub(crate) fn task(&self) -> Task {
        match self.name.as_str() {
            "issue_comment" => {
                let approval = self.event.pointer("/issue/pull_request").is_some()
                    && github::command(&self.text("/comment/body")).is_some();
                match self.text("/action").as_str() {
                    "deleted" if approval => Task::Withdraw,
                    "created" if approval => Task::Approve,
                    _ => Task::Nothing,
                }
            }
            "pull_request" | "pull_request_target" if self.text("/action") == "closed" => {
                Task::Clean
            }
            "schedule" => Task::Squash,
            _ => Task::Prove,
        }
    }

    /// Does what the event asks, except proving, which needs the checkout.
    pub(crate) fn act(&self, task: &Task) -> Result<bool> {
        match task {
            Task::Approve => self.approve()?,
            Task::Withdraw => self.withdraw()?,
            Task::Clean => {
                let Some(pr) = self.number("/pull_request/number") else {
                    bail!("the event file names no pull request to clean up after");
                };
                let dir = format!("pr-{pr}");
                if self.squash(|entry| Ok(entry == dir), false)? {
                    println!("removed the pictures of pull request {pr} from {BRANCH}");
                } else {
                    println!("pull request {pr} has no pictures on {BRANCH}");
                }
            }
            Task::Squash => {
                // Only a pull request GitHub says is closed, or no longer knows, loses its
                // pictures; any other answer stops the squash.
                let gone = |entry: &str| -> Result<bool> {
                    let Some(n) = entry.strip_prefix("pr-") else {
                        return Ok(false);
                    };
                    match self.api.call("GET", &format!("pulls/{n}"), None)? {
                        (200, pull) => Ok(pull["state"] != "open"),
                        (404, _) => Ok(true),
                        (code, _) => bail!(
                            "GitHub answered {code} to reading pull request {n}, so no pictures \
                             were removed; the next scheduled run tries again"
                        ),
                    }
                };
                self.squash(gone, true)?;
                println!("removed the pictures of closed pull requests from {BRANCH}");
            }
            Task::Nothing => println!("nothing to do: the comment is no /fearlezz approve"),
            Task::Prove => bail!("proving needs the checkout"),
        }
        println!("next: nothing; the pull request shows what happened");
        Ok(true)
    }

    /// The pull request a proof is for, when the event is one.
    pub(crate) fn pull(&self) -> Option<Pull> {
        let number = self.number("/pull_request/number")?;
        let head = self.text("/pull_request/head/sha");
        let fork = self.text("/pull_request/head/repo/full_name") != self.repo;
        let run = format!(
            "{}-{}",
            std::env::var("GITHUB_RUN_ID").unwrap_or_else(|_| "0".into()),
            std::env::var("GITHUB_RUN_ATTEMPT").unwrap_or_else(|_| "1".into())
        );
        let url = (!fork).then(|| {
            format!(
                "{}/{}/raw/{BRANCH}/pr-{number}/{run}",
                self.server, self.repo
            )
        });
        Some(Pull {
            api: self.api.clone(),
            number,
            head,
            run,
            fork,
            url,
        })
    }

    /// Answers a `/fearlezz approve <words>` comment: a person with write access approves the
    /// changed pictures of the head the status comment waits for, or the head of a fork's pull
    /// request, whose run could not say what waits.
    fn approve(&self) -> Result<()> {
        let pr = self
            .number("/issue/number")
            .context("the event file names no pull request")?;
        let comment = &self.event["comment"];
        let who = self.text("/comment/user/login");
        let refuse = |why: String| -> Result<()> {
            self.api.comment(pr, &why)?;
            println!("refused the approval of @{who}");
            Ok(())
        };
        let approval = match github::judge(&self.api, comment)? {
            Ok(approval) => approval,
            Err(why) => return refuse(why),
        };
        let (head, from) = self.api.head(pr)?;
        let short: String = head.chars().take(7).collect();
        let found = self.api.status_comment(pr)?.and_then(|f| {
            let waits = f.comment.waiting.clone()?;
            Some((f, waits))
        });
        let what = match (found, from != self.repo) {
            (Some((mut found, waits)), _) => {
                if waits.head != head {
                    return refuse(format!(
                        "@{who}, nothing to approve for {short} yet: wait for its proof, then \
                         approve again."
                    ));
                }
                self.api.status(
                    &head,
                    "success",
                    &format!("approved by {who}"),
                    Some(&approval.url),
                )?;
                found.comment.approve(&who);
                self.api.rewrite(&found)?;
                let checks: Vec<String> = waits.checks.iter().map(|c| format!("`{c}`")).collect();
                format!("the changed pictures of {}", checks.join(", "))
            }
            (None, true) => {
                self.api.status(
                    &head,
                    "success",
                    &format!("approved by {who}"),
                    Some(&approval.url),
                )?;
                "this fork's pull request".to_string()
            }
            (None, false) => {
                return refuse(format!(
                    "@{who}, no changed pictures wait for approval on this pull request."
                ));
            }
        };
        self.api.comment(
            pr,
            &format!(
                "@{who} approved {what} at {short}:\n\n> {}\n\nA new push needs a new approval; \
                 deleting the approval takes it back.",
                approval.said
            ),
        )?;
        println!("@{who} approved pull request {pr} at {short}");
        Ok(())
    }

    /// Takes an approval back when its comment is deleted: the status waits again, and so
    /// does the status comment.
    fn withdraw(&self) -> Result<()> {
        let pr = self
            .number("/issue/number")
            .context("the event file names no pull request")?;
        let who = self.text("/comment/user/login");
        let id = self.number("/comment/id").unwrap_or_default();
        let (head, _) = self.api.head(pr)?;
        let short: String = head.chars().take(7).collect();
        let in_force = self.api.latest_status(&head)?.is_some_and(|s| {
            s["state"] == "success"
                && s["target_url"]
                    .as_str()
                    .is_some_and(|t| t.ends_with(&format!("#issuecomment-{id}")))
        });
        if !in_force {
            println!("the deleted comment was no approval in force on {short}");
            return Ok(());
        }
        let mut target = None;
        if let Some(mut found) = self.api.status_comment(pr)? {
            found.comment.withdraw();
            self.api.rewrite(&found)?;
            target = Some(found.url);
        }
        self.api.status(
            &head,
            "pending",
            "the approval was deleted; changed pictures wait for /fearlezz approve",
            target.as_deref(),
        )?;
        self.api.comment(
            pr,
            &format!(
                "The approval by @{who} was deleted, so the pictures of {short} wait for \
                 `/fearlezz approve <words>` again."
            ),
        )?;
        println!("took back the approval of pull request {pr} at {short}");
        Ok(())
    }

    /// Removes the top folders `gone` picks from the pictures branch and makes it a single
    /// commit again, so it never grows with the history of every run; the branch goes when
    /// nothing stays. With `always` false it changes nothing when nothing is gone. The update
    /// is atomic: a push in between makes it start over, so no push is lost. Returns whether
    /// anything was removed.
    fn squash(&self, gone: impl Fn(&str) -> Result<bool>, always: bool) -> Result<bool> {
        let mut decided: BTreeSet<String> = BTreeSet::new();
        for _ in 0..ATTEMPTS {
            let Some(old) = tip(&self.api)? else {
                return Ok(false);
            };
            let mut root = Vec::new();
            let mut removed = false;
            for entry in root_of(&self.api, &old)? {
                let path = entry["path"].as_str().unwrap_or_default().to_string();
                if decided.contains(&path) || gone(&path)? {
                    decided.insert(path);
                    removed = true;
                } else {
                    root.push(entry);
                }
            }
            if !removed && !always {
                return Ok(false);
            }
            let new = if root.is_empty() {
                None
            } else {
                let tree = self
                    .api
                    .send("POST", "git/trees", &json!({ "tree": root }))?["sha"]
                    .clone();
                let commit = self.api.send(
                    "POST",
                    "git/commits",
                    &json!({ "message": "Pictures of open pull requests", "tree": tree, "parents": [] }),
                )?;
                commit["sha"].as_str().map(str::to_string)
            };
            if self.update_ref(&old, new.as_deref())? {
                return Ok(removed);
            }
            println!("{BRANCH} moved while squashing; trying again");
        }
        println!("::warning::could not squash {BRANCH}: it kept moving");
        Ok(false)
    }

    /// Points the pictures branch at `new` (or deletes it), only while it is still at `old`,
    /// in one step on GitHub's side. False when it moved.
    fn update_ref(&self, old: &str, new: Option<&str>) -> Result<bool> {
        let id = self.api.get(&format!("/repos/{}", self.repo))?["node_id"].clone();
        let zero = "0".repeat(40);
        let answer = self.api.graphql(
            "mutation($input: UpdateRefsInput!) { updateRefs(input: $input) { clientMutationId } }",
            &json!({ "input": {
                "repositoryId": id,
                "refUpdates": [{
                    "name": format!("refs/heads/{BRANCH}"),
                    "beforeOid": old,
                    "afterOid": new.unwrap_or(&zero),
                    "force": true,
                }],
            }}),
        );
        match answer {
            Ok(_) => Ok(true),
            // Whether the branch moved is read again rather than guessed from the message.
            Err(e) if tip(&self.api)?.as_deref() != Some(old) => {
                println!("{BRANCH} moved: {e:#}");
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }
}

/// The pull request a proof runs for.
pub(crate) struct Pull {
    api: Api,
    number: u64,
    head: String,
    /// This run's folder on the pictures branch: run id and attempt.
    run: String,
    /// A fork's run has a read only token.
    fork: bool,
    /// Where this run's pictures are served once pushed; None for a fork.
    pub url: Option<String>,
}

impl Pull {
    /// Pushes the changed pictures in `dir`, writes the status comment and sets the status.
    /// GitHub refusing any of it is a warning: the proof stands on its own.
    pub(crate) fn report(&self, comment: &StatusComment, checks: &[String], dir: Option<&Path>) {
        if self.fork {
            println!(
                "::notice::this run of a fork's pull request has a read only token, so it writes \
                 nothing to the pull request: the check page has its status and the \
                 fearlezz-pictures artifact its changed pictures; a person with write access \
                 approves its head with /fearlezz approve <words>"
            );
            return;
        }
        if let Err(e) = self.try_report(comment, checks, dir) {
            println!(
                "::warning::could not report to pull request {}: {e:#}",
                self.number
            );
        }
    }

    fn try_report(
        &self,
        comment: &StatusComment,
        checks: &[String],
        dir: Option<&Path>,
    ) -> Result<()> {
        let (state, what) = if checks.is_empty() {
            ("success", "no changed pictures".to_string())
        } else {
            (
                "pending",
                format!(
                    "changed pictures of {} wait for /fearlezz approve",
                    checks.join(", ")
                ),
            )
        };
        // A run that finishes after a newer push's run leaves that run's comment and pictures.
        let (now, _) = self.api.head(self.number)?;
        if now != self.head {
            println!(
                "pull request {} moved on to {}; this run leaves its comment and pictures alone",
                self.number,
                now.chars().take(7).collect::<String>()
            );
            return self.api.status(&self.head, state, &what, None);
        }
        let mut comment = comment.clone();
        if let (Some(dir), false) = (dir, checks.is_empty()) {
            match self.publish(dir) {
                Ok(()) => println!("pictures on {BRANCH} at pr-{}/{}", self.number, self.run),
                Err(e) => {
                    println!(
                        "::warning::could not push the pictures to {BRANCH}; the workflow needs \
                         contents: write ({e:#})"
                    );
                    comment.note(&format!(
                        "The pictures could not be put on the `{BRANCH}` branch, so they do not \
                         show above. {}",
                        github::ARTIFACT
                    ));
                }
            }
        }
        let url = self.api.write_status_comment(self.number, &comment)?;
        let target = (!checks.is_empty()).then_some(url.as_str());
        self.api.status(&self.head, state, &what, target)
    }

    /// Puts the pictures in `dir` on the pictures branch as this run's folder of the pull
    /// request, in place of its earlier runs, as a fast forward: a push in between makes it
    /// start over.
    fn publish(&self, dir: &Path) -> Result<()> {
        let mut entries = Vec::new();
        for (rel, path) in files(dir)? {
            let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
            let blob = self.api.send(
                "POST",
                "git/blobs",
                &json!({ "content": base64(&bytes), "encoding": "base64" }),
            )?;
            entries.push(json!({
                "path": format!("{}/{rel}", self.run),
                "mode": "100644",
                "type": "blob",
                "sha": blob["sha"],
            }));
        }
        if entries.is_empty() {
            return Ok(());
        }
        let folder = self
            .api
            .send("POST", "git/trees", &json!({ "tree": entries }))?["sha"]
            .clone();
        let name = format!("pr-{}", self.number);
        let message = format!("Pictures of pull request {}, run {}", self.number, self.run);
        for _ in 0..ATTEMPTS {
            let old = tip(&self.api)?;
            let mut root = match &old {
                Some(commit) => root_of(&self.api, commit)?,
                None => Vec::new(),
            };
            root.retain(|e| e["path"] != name.as_str());
            root.push(json!({ "path": name, "mode": "040000", "type": "tree", "sha": folder }));
            let tree = self
                .api
                .send("POST", "git/trees", &json!({ "tree": root }))?["sha"]
                .clone();
            let parents: Vec<&String> = old.iter().collect();
            let commit = self.api.send(
                "POST",
                "git/commits",
                &json!({ "message": message, "tree": tree, "parents": parents }),
            )?["sha"]
                .clone();
            let (code, _) = match old {
                Some(_) => self.api.call(
                    "PATCH",
                    &format!("git/refs/heads/{BRANCH}"),
                    Some(&json!({ "sha": commit, "force": false })),
                )?,
                None => self.api.call(
                    "POST",
                    "git/refs",
                    Some(&json!({ "ref": format!("refs/heads/{BRANCH}"), "sha": commit })),
                )?,
            };
            match code {
                200..=299 => return Ok(()),
                422 => println!("pushing the pictures lost a race; trying again"),
                _ => bail!("GitHub answered {code} to updating {BRANCH}"),
            }
        }
        bail!("every push lost a race with another")
    }
}

/// The pictures branch's tip, or None when it does not exist.
fn tip(api: &Api) -> Result<Option<String>> {
    match api.call("GET", &format!("git/ref/heads/{BRANCH}"), None)? {
        (200, answer) => Ok(answer["object"]["sha"].as_str().map(str::to_string)),
        (404, _) => Ok(None),
        (code, _) => bail!("GitHub answered {code} to reading {BRANCH}"),
    }
}

/// The top folder entries of `commit`, as a new tree takes them.
fn root_of(api: &Api, commit: &str) -> Result<Vec<Value>> {
    let tree = api.get(&format!("git/commits/{commit}"))?["tree"]["sha"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let tree = api.get(&format!("git/trees/{tree}"))?;
    Ok(tree["tree"]
        .as_array()
        .into_iter()
        .flatten()
        .map(
            |e| json!({ "path": e["path"], "mode": e["mode"], "type": e["type"], "sha": e["sha"] }),
        )
        .collect())
}

/// Every file below `dir`, with its path relative to it, in a stable order.
fn files(dir: &Path) -> Result<Vec<(String, PathBuf)>> {
    let mut found = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(here) = todo.pop() {
        let Ok(read) = fs::read_dir(&here) else {
            continue;
        };
        for entry in read {
            let path = entry?.path();
            if path.is_dir() {
                todo.push(path);
            } else if let Ok(rel) = path.strip_prefix(dir) {
                found.push((rel.to_string_lossy().replace('\\', "/"), path.clone()));
            }
        }
    }
    found.sort();
    Ok(found)
}

/// Bytes as base64, as GitHub takes a blob.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0usize, |n, (i, b)| n | (usize::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[(n >> (18 - 6 * i)) & 63]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The ffmpeg Fearlezz installs on a runner when a check needs it: John Van Sickle's static
/// Linux builds of ffmpeg 7.0.2, as the ffmpeg-static project republishes them under its
/// release tag b6.1.1, with the sha256 of each gzipped file.
const FFMPEG_RELEASE: &str = "https://github.com/eugeneware/ffmpeg-static/releases/download/b6.1.1";
const FFMPEG_SUMS: [(&str, &str); 4] = [
    (
        "ffmpeg-linux-x64",
        "bfe8a8fc511530457b528c48d77b5737527b504a3797a9bc4866aeca69c2dffa",
    ),
    (
        "ffprobe-linux-x64",
        "25d9b6ccb05e3d9de9e04e31e2506d8dd7f9f0418981965ac6df12e8d3afd067",
    ),
    (
        "ffmpeg-linux-arm64",
        "754a678672298bc68156adff58aa7385a592c2b30b1d0ae8750c45c915c4bac0",
    ),
    (
        "ffprobe-linux-arm64",
        "2ab6aba60ee84412dff9188720703376cb4e7aaf7e0b5e43aa8249f2acae5bf8",
    ),
];

/// Installs the pinned ffmpeg into the runner's tool cache, unless ffmpeg is on PATH already,
/// and returns the folder to put in front of PATH.
pub(crate) fn install_ffmpeg() -> Result<Option<PathBuf>> {
    let found = Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if found {
        println!("ffmpeg for the pictures: the one on PATH");
        return Ok(None);
    }
    let arch = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x64",
        ("linux", "aarch64") => "arm64",
        (os, arch) => {
            println!(
                "::warning::no static ffmpeg for {os} on {arch}; install ffmpeg before Fearlezz"
            );
            return Ok(None);
        }
    };
    let cache = std::env::var_os("RUNNER_TOOL_CACHE")
        .or_else(|| std::env::var_os("RUNNER_TEMP"))
        .map_or_else(std::env::temp_dir, PathBuf::from);
    let dir = cache.join("fearlezz-ffmpeg").join("b6.1.1");
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    for tool in ["ffmpeg", "ffprobe"] {
        let target = dir.join(tool);
        if !target.exists() {
            let name = format!("{tool}-linux-{arch}");
            let sum = FFMPEG_SUMS
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, s)| *s)
                .unwrap_or_default();
            fetch(&format!("{FFMPEG_RELEASE}/{name}.gz"), sum, &target)?;
        }
    }
    println!(
        "ffmpeg for the pictures: 7.0.2, installed into {}",
        dir.display()
    );
    Ok(Some(dir))
}

/// Downloads a gzipped program, checks its sha256 and unpacks it to `target`.
fn fetch(url: &str, sum: &str, target: &Path) -> Result<()> {
    use std::io::Read as _;
    let out = Command::new("curl")
        .args(["-fsSL", "--retry", "3", url])
        .output()
        .context("downloading ffmpeg needs curl")?;
    if !out.status.success() {
        bail!(
            "downloading {url} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let got = hex::encode(Sha256::digest(&out.stdout));
    if got != sum {
        bail!("{url} has sha256 {got}, not the pinned {sum}");
    }
    let mut program = Vec::new();
    flate2::read::GzDecoder::new(out.stdout.as_slice())
        .read_to_end(&mut program)
        .with_context(|| format!("unpacking {url}"))?;
    let partial = target.with_extension("partial");
    fs::write(&partial, program)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&partial, fs::Permissions::from_mode(0o755))?;
    }
    fs::rename(&partial, target)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_pads_like_the_standard() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0x00]), "//4A");
    }
}
