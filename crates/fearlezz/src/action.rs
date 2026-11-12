//! GitHub Actions: what `fearlezz run --ci` does for the event a workflow runs on. A pull
//! request is proven, its changed pictures pushed to the pictures branch, its status comment
//! written and its `fearlezz/pictures` status set; a `/fearlezz approve <words>` comment is
//! answered; a closed pull request's pictures are removed; the schedule removes the pictures of
//! every closed pull request. Everything goes through GitHub's REST API.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::github::{self, APPROVED_MARKER, COMMENT_MARKER, WAITING_MARKER};

/// The branch the changed pictures are pushed to, one folder per pull request.
pub(crate) const BRANCH: &str = "fearlezz-pictures";

/// The commit status that holds a pull request while its changed pictures wait for a person.
const CONTEXT: &str = "fearlezz/pictures";

/// Who the workflow token comments as; that token cannot ask GitHub who it is.
const WORKFLOW_LOGIN: &str = "github-actions[bot]";

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
            api: Api {
                base: var("GITHUB_API_URL").unwrap_or_else(|| "https://api.github.com".into()),
                token,
                repo: repo.clone(),
            },
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
                if self.event.pointer("/issue/pull_request").is_some()
                    && command(&self.text("/comment/body")).is_some()
                {
                    Task::Approve
                } else {
                    Task::Nothing
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
            Task::Clean => {
                let Some(pr) = self.number("/pull_request/number") else {
                    bail!("the event file names no pull request to clean up after");
                };
                let dir = format!("pr-{pr}");
                self.api.squash(|entry| entry == dir)?;
                println!("removed the pictures of pull request {pr} from {BRANCH}");
            }
            Task::Squash => {
                let open = |entry: &str| {
                    let Some(n) = entry.strip_prefix("pr-") else {
                        return true;
                    };
                    // A pull request GitHub no longer knows is closed too.
                    self.api
                        .get(&format!("pulls/{n}"))
                        .is_ok_and(|p| p["state"] == "open")
                };
                self.api.squash(|entry| !open(entry))?;
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
        let from = self.text("/pull_request/head/repo/full_name");
        let run = format!(
            "{}-{}",
            std::env::var("GITHUB_RUN_ID").unwrap_or_else(|_| "0".into()),
            std::env::var("GITHUB_RUN_ATTEMPT").unwrap_or_else(|_| "1".into())
        );
        // A fork's token cannot push, so its pictures stay in the run's artifact.
        let url = (from == self.repo).then(|| {
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
            url,
        })
    }

    /// Answers a `/fearlezz approve <words>` comment: a person with write access approves the
    /// changed pictures of the head the status comment waits for.
    fn approve(&self) -> Result<()> {
        let pr = self
            .number("/issue/number")
            .context("the event file names no pull request")?;
        let who = self.text("/comment/user/login");
        let body = self.text("/comment/body").replace('\r', "");
        let Some(said) = command(&body) else {
            return Ok(());
        };
        let refuse = |why: String| -> Result<()> {
            self.api.comment(pr, &why)?;
            println!("refused the approval of @{who}");
            Ok(())
        };
        if self.text("/comment/user/type") != "User" {
            return refuse(format!("Only a person can approve pictures, not @{who}."));
        }
        let permission = self
            .api
            .get(&format!("collaborators/{who}/permission"))
            .map(|p| p["permission"].as_str().unwrap_or_default().to_string())
            .unwrap_or_default();
        if permission != "admin" && permission != "write" {
            return refuse(format!(
                "@{who} cannot approve pictures: that needs write access to this repository."
            ));
        }
        if said.is_empty() {
            return refuse(format!(
                "@{who}, approving needs your own words: `/fearlezz approve <what you think of \
                 the pictures>`."
            ));
        }
        let head = self.api.get(&format!("pulls/{pr}"))?["head"]["sha"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let short: String = head.chars().take(7).collect();
        let found = self.api.status_comment(pr)?;
        let Some((id, comment, waiting)) = found.and_then(|(id, body)| {
            let waiting = waiting(&body)?;
            Some((id, body, waiting))
        }) else {
            return refuse(format!(
                "@{who}, no changed pictures wait for approval on this pull request."
            ));
        };
        if waiting.head != head {
            return refuse(format!(
                "@{who}, nothing to approve for {short} yet: wait for its proof, then approve \
                 again."
            ));
        }
        let url = self.text("/comment/html_url");
        self.api
            .status(&head, "success", &format!("approved by {who}"), Some(&url))?;
        // The status comment says what was approved, in place of the line that said it waits.
        let headline = waiting.approved.replace("{by}", &who);
        let mut replaced = false;
        let lines: Vec<&str> = comment
            .lines()
            .map(|line| {
                if !replaced && line.starts_with("**") {
                    replaced = true;
                    headline.as_str()
                } else {
                    line
                }
            })
            .collect();
        self.api.send(
            "PATCH",
            &format!("issues/comments/{id}"),
            &json!({ "body": lines.join("\n") }),
        )?;
        let checks: Vec<String> = waiting.checks.iter().map(|c| format!("`{c}`")).collect();
        let approval = github::hidden(
            APPROVED_MARKER,
            &github::Approval {
                head: head.clone(),
                fingerprint: waiting.fingerprint,
                checks: waiting.checks,
                by: who.clone(),
                said: said.clone(),
                url: String::new(),
            },
        )?;
        self.api.comment(
            pr,
            &format!(
                "@{who} approved the changed pictures of {} at {short}:\n\n> {said}\n\nA new push \
                 needs a new approval.\n{approval}",
                checks.join(", ")
            ),
        )?;
        println!("@{who} approved the changed pictures of pull request {pr} at {short}");
        Ok(())
    }
}

/// The words of a `/fearlezz approve <words>` comment, on one line, or None for another comment.
fn command(body: &str) -> Option<String> {
    let body = body.replace('\r', "");
    let first = body.lines().next().unwrap_or_default();
    if first != "/fearlezz approve" && !first.starts_with("/fearlezz approve ") {
        return None;
    }
    let words = body.strip_prefix("/fearlezz approve").unwrap_or_default();
    Some(words.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// The hidden line of a status comment that says what waits for approval.
fn waiting(body: &str) -> Option<github::Waiting> {
    body.lines().find_map(|line| {
        let json = line.strip_prefix(WAITING_MARKER)?.trim_end();
        serde_json::from_str(json.strip_suffix("-->")?.trim_end()).ok()
    })
}

/// The pull request a proof runs for.
pub(crate) struct Pull {
    api: Api,
    number: u64,
    head: String,
    /// This run's folder on the pictures branch: run id and attempt.
    run: String,
    /// Where this run's pictures are served once pushed; None for a fork.
    pub url: Option<String>,
}

impl Pull {
    /// Pushes the changed pictures in `dir`, writes the status comment and sets the status.
    /// GitHub refusing any of it is a warning: the proof stands on its own.
    pub(crate) fn report(&self, comment: &str, checks: &[String], dir: Option<&Path>) {
        if let Err(e) = self.try_report(comment, checks, dir) {
            println!(
                "::warning::could not report to pull request {}: {e:#}",
                self.number
            );
        }
    }

    fn try_report(&self, comment: &str, checks: &[String], dir: Option<&Path>) -> Result<()> {
        let mut text = comment.to_string();
        if let (Some(dir), Some(_), false) = (dir, &self.url, checks.is_empty()) {
            match self.publish(dir) {
                Ok(()) => println!("pictures on {BRANCH} at pr-{}/{}", self.number, self.run),
                Err(e) => {
                    println!(
                        "::warning::could not push the pictures to {BRANCH}; the workflow needs \
                         contents: write ({e:#})"
                    );
                    write!(
                        text,
                        "\nThe pictures could not be put on the `{BRANCH}` branch, so they do not \
                         show above; they are in the `fearlezz-pictures` artifact of the run.\n"
                    )?;
                }
            }
        }
        let url = match self.api.status_comment(self.number)? {
            Some((id, _)) => self.api.send(
                "PATCH",
                &format!("issues/comments/{id}"),
                &json!({ "body": text }),
            )?,
            None => self.api.comment(self.number, &text)?,
        }["html_url"]
            .as_str()
            .map(str::to_string);
        if checks.is_empty() {
            self.api
                .status(&self.head, "success", "no changed pictures", None)
        } else {
            let what = format!(
                "changed pictures of {} wait for /fearlezz approve",
                checks.join(", ")
            );
            self.api
                .status(&self.head, "pending", &what, url.as_deref())
        }
    }

    /// Puts the pictures in `dir` on the pictures branch as this run's folder of the pull
    /// request, in place of its earlier runs.
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
            let old = self.api.tip()?;
            let mut root = match &old {
                Some(commit) => self.api.root(commit)?,
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
            // Another pull request's push in between makes this one no fast forward: start over.
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

/// GitHub's REST API for one repository, through curl, which every runner has. The token goes
/// to curl on stdin, never in its arguments.
#[derive(Clone)]
struct Api {
    base: String,
    token: String,
    repo: String,
}

impl Api {
    /// One request: the status code and the answer. A path without a leading slash is below the
    /// repository.
    fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Result<(u16, Value)> {
        let url = if path.starts_with('/') {
            format!("{}{path}", self.base)
        } else {
            format!("{}/repos/{}/{path}", self.base, self.repo)
        };
        let mut cmd = Command::new("curl");
        cmd.args(["-sS", "-K", "-", "-X", method, "-w", "\n%{http_code}"])
            .args(["-H", "Accept: application/vnd.github+json"])
            .args(["-H", "X-GitHub-Api-Version: 2022-11-28"])
            .args(["-H", "User-Agent: fearlezz"])
            // A large body goes at once, not after a 100 Continue.
            .args(["-H", "Expect:"]);
        let file = match body {
            Some(body) => {
                let mut file = tempfile::NamedTempFile::new()?;
                serde_json::to_writer(&mut file, body)?;
                file.flush()?;
                cmd.args(["-H", "Content-Type: application/json", "--data-binary"])
                    .arg(format!("@{}", file.path().display()));
                Some(file)
            }
            None => None,
        };
        let mut child = cmd
            .arg(&url)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("talking to GitHub needs curl")?;
        if let Some(mut stdin) = child.stdin.take() {
            writeln!(stdin, "header = \"Authorization: Bearer {}\"", self.token)?;
        }
        let out = child.wait_with_output()?;
        drop(file);
        if !out.status.success() {
            bail!(
                "{method} {url} failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let (answer, code) = text.rsplit_once('\n').unwrap_or(("", &text));
        let code: u16 = code
            .trim()
            .parse()
            .with_context(|| format!("{method} {url} gave no status code"))?;
        let answer = if answer.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(answer).unwrap_or_else(|_| Value::String(answer.to_string()))
        };
        Ok((code, answer))
    }

    /// A request that must succeed.
    fn send(&self, method: &str, path: &str, body: &Value) -> Result<Value> {
        self.must(method, path, Some(body))
    }

    fn get(&self, path: &str) -> Result<Value> {
        self.must("GET", path, None)
    }

    fn must(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
        let (code, answer) = self.call(method, path, body)?;
        if !(200..300).contains(&code) {
            let why = answer["message"].as_str().unwrap_or_default();
            bail!("GitHub answered {code} to {method} {path}: {why}");
        }
        Ok(answer)
    }

    /// Every comment on pull request `pr`, oldest first.
    fn comments(&self, pr: u64) -> Result<Vec<Value>> {
        let mut all = Vec::new();
        for page in 1.. {
            let got = self.get(&format!("issues/{pr}/comments?per_page=100&page={page}"))?;
            let got = got.as_array().cloned().unwrap_or_default();
            let last = got.len() < 100;
            all.extend(got);
            if last {
                break;
            }
        }
        Ok(all)
    }

    fn comment(&self, pr: u64, body: &str) -> Result<Value> {
        self.send(
            "POST",
            &format!("issues/{pr}/comments"),
            &json!({ "body": body }),
        )
    }

    /// The id and body of the status comment this token wrote on pull request `pr`.
    fn status_comment(&self, pr: u64) -> Result<Option<(u64, String)>> {
        let login = self
            .get("/user")
            .ok()
            .and_then(|u| u["login"].as_str().map(str::to_string))
            .unwrap_or_else(|| WORKFLOW_LOGIN.into());
        Ok(self.comments(pr)?.into_iter().find_map(|c| {
            let body = c["body"].as_str()?;
            (c["user"]["login"] == login.as_str() && body.starts_with(COMMENT_MARKER))
                .then(|| Some((c["id"].as_u64()?, body.to_string())))?
        }))
    }

    fn status(
        &self,
        sha: &str,
        state: &str,
        description: &str,
        target: Option<&str>,
    ) -> Result<()> {
        // GitHub refuses a longer description.
        let description: String = description.chars().take(140).collect();
        let mut body = json!({ "state": state, "context": CONTEXT, "description": description });
        if let Some(target) = target {
            body["target_url"] = target.into();
        }
        self.send("POST", &format!("statuses/{sha}"), &body)?;
        Ok(())
    }

    /// The pictures branch's tip, or None when it does not exist.
    fn tip(&self) -> Result<Option<String>> {
        match self.call("GET", &format!("git/ref/heads/{BRANCH}"), None)? {
            (200, answer) => Ok(answer["object"]["sha"].as_str().map(str::to_string)),
            (404, _) => Ok(None),
            (code, _) => bail!("GitHub answered {code} to reading {BRANCH}"),
        }
    }

    /// The top folder entries of `commit`, as a new tree takes them.
    fn root(&self, commit: &str) -> Result<Vec<Value>> {
        let tree = self.get(&format!("git/commits/{commit}"))?["tree"]["sha"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let tree = self.get(&format!("git/trees/{tree}"))?;
        Ok(tree["tree"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| json!({ "path": e["path"], "mode": e["mode"], "type": e["type"], "sha": e["sha"] }))
            .collect())
    }

    /// Removes the top folders `gone` picks from the pictures branch and makes it a single
    /// commit again, so it never grows with the history of every run. A push in between starts
    /// it over; the branch goes when nothing stays.
    fn squash(&self, gone: impl Fn(&str) -> bool) -> Result<()> {
        let mut decided: BTreeSet<String> = BTreeSet::new();
        let mut kept: BTreeSet<String> = BTreeSet::new();
        for _ in 0..ATTEMPTS {
            let Some(old) = self.tip()? else {
                return Ok(());
            };
            let mut root = self.root(&old)?;
            root.retain(|e| {
                let path = e["path"].as_str().unwrap_or_default().to_string();
                if !decided.contains(&path) && !kept.contains(&path) {
                    if gone(&path) {
                        decided.insert(path.clone());
                    } else {
                        kept.insert(path.clone());
                    }
                }
                kept.contains(&path)
            });
            let new = if root.is_empty() {
                None
            } else {
                let tree = self.send("POST", "git/trees", &json!({ "tree": root }))?["sha"].clone();
                let commit = self.send(
                    "POST",
                    "git/commits",
                    &json!({ "message": "Pictures of open pull requests", "tree": tree, "parents": [] }),
                )?;
                Some(commit["sha"].clone())
            };
            // The lease: only while the branch is still where it was read.
            if self.tip()?.as_deref() != Some(old.as_str()) {
                println!("{BRANCH} moved while squashing; trying again");
                continue;
            }
            match new {
                None => {
                    self.must("DELETE", &format!("git/refs/heads/{BRANCH}"), None)?;
                }
                Some(commit) => {
                    self.send(
                        "PATCH",
                        &format!("git/refs/heads/{BRANCH}"),
                        &json!({ "sha": commit, "force": true }),
                    )?;
                }
            }
            return Ok(());
        }
        println!("::warning::could not squash {BRANCH}: it kept moving");
        Ok(())
    }
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
    println!("installed ffmpeg 7.0.2 into {}", dir.display());
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

    #[test]
    fn only_the_approve_command_counts() {
        assert_eq!(command("/fearlezz approve"), Some(String::new()));
        assert_eq!(
            command("/fearlezz approve  looks\r\nright "),
            Some("looks right".into())
        );
        assert_eq!(command("/fearlezz approved? not yet"), None);
        assert_eq!(command("lgtm\n/fearlezz approve fine"), None);
    }
}
