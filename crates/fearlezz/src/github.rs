//! GitHub: the one client Fearlezz talks to it with (REST through curl, and GraphQL for the
//! atomic ref update), what a proof shows there (the status comment and the check page, with
//! one owner for their Markdown), and the one rule for which approval counts. CI writes the
//! comment and answers `/fearlezz approve` (`action.rs`); the developer's machine reads an
//! approval back with the same client and the same rule.

use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::floor;
use crate::look::{Change, Look};
use crate::store::RunResult;

/// The first line of the status comment, so CI finds its own comment to replace.
const COMMENT_MARKER: &str = "<!-- fearlezz -->";

/// The title under the marker; the headline follows it.
const TITLE: &str = "### Fearlezz";

/// Starts the hidden line of the status comment that says what an approval covers.
const WAITING_MARKER: &str = "<!-- fearlezz-pictures ";

/// The commit status that holds a pull request while its changed pictures wait for a person.
pub(crate) const CONTEXT: &str = "fearlezz/pictures";

/// Pictures shown in one comment at most, so it stays readable and under GitHub's size limit.
const SHOWN: usize = 20;

// The client

/// GitHub's API for one repository, through curl, which every runner has. The token goes to
/// curl on stdin, never in its arguments.
#[derive(Clone)]
pub(crate) struct Api {
    base: String,
    graphql: String,
    token: String,
    pub repo: String,
}

impl Api {
    /// The API at `GITHUB_API_URL` (github.com by default) for `repo`, with `token`.
    pub(crate) fn new(token: String, repo: String) -> Api {
        let base = var("GITHUB_API_URL").unwrap_or_else(|| "https://api.github.com".into());
        let graphql = var("GITHUB_GRAPHQL_URL").unwrap_or_else(|| format!("{base}/graphql"));
        Api {
            base,
            graphql,
            token,
            repo,
        }
    }

    /// The API on the developer's machine: their token (`GITHUB_TOKEN`, `GH_TOKEN`, or the
    /// GitHub CLI's), for the repository `origin` points to in `root`.
    pub(crate) fn local(root: &Path) -> Result<Api> {
        let token = match var("GITHUB_TOKEN").or_else(|| var("GH_TOKEN")) {
            Some(token) => token,
            None => {
                let out = Command::new("gh")
                    .args(["auth", "token"])
                    .output()
                    .context(
                        "reading the pull request needs a GitHub token: set GITHUB_TOKEN, or \
                         install the GitHub CLI and sign in with gh auth login",
                    )?;
                let token = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !out.status.success() || token.is_empty() {
                    bail!(
                        "reading the pull request needs a GitHub token: sign in with gh auth \
                         login, or set GITHUB_TOKEN"
                    );
                }
                token
            }
        };
        let out = Command::new("git")
            .args(["remote", "get-url", "origin"])
            .current_dir(root)
            .output()?;
        let remote = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let repo = repository(&remote).with_context(|| {
            format!(
                "the remote origin ({remote}) names no repository on GitHub: point origin at \
                 the repository the pull request is on"
            )
        })?;
        Ok(Api::new(token, repo))
    }

    /// One request: the status code and the answer. A path that starts with `/` is below the
    /// API, one with a scheme is a URL, and any other is below the repository.
    pub(crate) fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<(u16, Value)> {
        let url = if path.contains("://") {
            path.to_string()
        } else if path.starts_with('/') {
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
    pub(crate) fn send(&self, method: &str, path: &str, body: &Value) -> Result<Value> {
        self.must(method, path, Some(body))
    }

    pub(crate) fn get(&self, path: &str) -> Result<Value> {
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

    /// A GraphQL request; its errors are the error.
    pub(crate) fn graphql(&self, query: &str, variables: &Value) -> Result<Value> {
        let answer = self.send(
            "POST",
            &self.graphql,
            &json!({ "query": query, "variables": variables }),
        )?;
        if let Some(errors) = answer["errors"].as_array().filter(|e| !e.is_empty()) {
            let why: Vec<&str> = errors
                .iter()
                .filter_map(|e| e["message"].as_str())
                .collect();
            bail!("GitHub refused: {}", why.join("; "));
        }
        Ok(answer["data"].clone())
    }

    /// Every comment on pull request `pr`, oldest first.
    pub(crate) fn comments(&self, pr: u64) -> Result<Vec<Value>> {
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

    pub(crate) fn comment(&self, pr: u64, body: &str) -> Result<Value> {
        self.send(
            "POST",
            &format!("issues/{pr}/comments"),
            &json!({ "body": body }),
        )
    }

    /// The head commit of pull request `pr` and the repository it comes from.
    pub(crate) fn head(&self, pr: u64) -> Result<(String, String)> {
        let pull = self.get(&format!("pulls/{pr}"))?;
        let text = |p: &str| pull.pointer(p).and_then(Value::as_str).unwrap_or_default();
        Ok((
            text("/head/sha").to_string(),
            text("/head/repo/full_name").to_string(),
        ))
    }

    /// Sets the `fearlezz/pictures` status of `sha`.
    pub(crate) fn status(
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

    /// The latest `fearlezz/pictures` status of `sha`, if any.
    pub(crate) fn latest_status(&self, sha: &str) -> Result<Option<Value>> {
        let all = self.get(&format!("commits/{sha}/statuses?per_page=100"))?;
        Ok(all
            .as_array()
            .into_iter()
            .flatten()
            .find(|s| s["context"] == CONTEXT)
            .cloned())
    }

    /// The status comment CI wrote on pull request `pr`: the first comment with the marker
    /// written by an app (the workflow's token or an app's). A person's comment never is, so
    /// it works without asking GitHub who the token is, which an app's token cannot.
    pub(crate) fn status_comment(&self, pr: u64) -> Result<Option<Found>> {
        Ok(self.comments(pr)?.into_iter().find_map(|c| {
            if c["user"]["type"] != "Bot" {
                return None;
            }
            let comment = StatusComment::parse(&c["body"].as_str()?.replace('\r', ""))?;
            Some(Found {
                id: c["id"].as_u64()?,
                url: c["html_url"].as_str().unwrap_or_default().to_string(),
                updated: c["updated_at"].as_str().unwrap_or_default().to_string(),
                comment,
            })
        }))
    }

    /// Writes the status comment of pull request `pr`, in place of `old`, the one there;
    /// returns its URL.
    pub(crate) fn write_status_comment(
        &self,
        pr: u64,
        old: Option<&Found>,
        comment: &StatusComment,
    ) -> Result<String> {
        let body = comment.render();
        let written = match old {
            Some(found) => self.send(
                "PATCH",
                &format!("issues/comments/{}", found.id),
                &json!({ "body": body }),
            )?,
            None => self.comment(pr, &body)?,
        };
        Ok(written["html_url"].as_str().unwrap_or_default().to_string())
    }

    /// Rewrites the status comment `id`.
    pub(crate) fn rewrite(&self, id: u64, comment: &StatusComment) -> Result<()> {
        self.send(
            "PATCH",
            &format!("issues/comments/{id}"),
            &json!({ "body": comment.render() }),
        )?;
        Ok(())
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// `owner/repo` from a remote URL on GitHub, over HTTPS or SSH.
fn repository(remote: &str) -> Option<String> {
    let rest = remote.split_once("github.com")?.1;
    let rest = rest.trim_start_matches([':', '/']).trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    let (owner, name) = rest.split_once('/')?;
    (!owner.is_empty() && !name.is_empty() && !name.contains('/'))
        .then(|| format!("{owner}/{name}"))
}

// Approvals

/// An approval that counts: who gave it, in their own words, and where.
pub(crate) struct Approval {
    pub by: String,
    pub said: String,
    pub url: String,
}

/// The words of a `/fearlezz approve <words>` comment, on one line, or None for another comment.
pub(crate) fn command(body: &str) -> Option<String> {
    let body = body.replace('\r', "");
    let first = body.lines().next().unwrap_or_default();
    if first != "/fearlezz approve" && !first.starts_with("/fearlezz approve ") {
        return None;
    }
    let words = body.strip_prefix("/fearlezz approve").unwrap_or_default();
    Some(words.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Whether `comment`, as GitHub's API describes it, approves: a `/fearlezz approve <words>`
/// by a person with write access, in their own words. The outer error is GitHub failing; the
/// inner one says why the comment does not approve.
pub(crate) fn judge(api: &Api, comment: &Value) -> Result<Result<Approval, String>> {
    let who = comment["user"]["login"].as_str().unwrap_or_default();
    let Some(said) = command(comment["body"].as_str().unwrap_or_default()) else {
        return Ok(Err(format!(
            "the comment by @{who} is no `/fearlezz approve <words>`."
        )));
    };
    if comment["user"]["type"] != "User" {
        return Ok(Err(format!(
            "Only a person can approve pictures, not @{who}."
        )));
    }
    let permission = match api.call("GET", &format!("collaborators/{who}/permission"), None)? {
        (200, p) => p["permission"].as_str().unwrap_or_default().to_string(),
        _ => String::new(),
    };
    if permission != "admin" && permission != "write" {
        return Ok(Err(format!(
            "@{who} cannot approve pictures: that needs write access to this repository."
        )));
    }
    if said.is_empty() {
        return Ok(Err(format!(
            "@{who}, approving needs your own words: `/fearlezz approve <what you think of the \
             pictures>`."
        )));
    }
    Ok(Ok(Approval {
        by: who.to_string(),
        said,
        url: comment["html_url"].as_str().unwrap_or_default().to_string(),
    }))
}

/// What waits for a person on a pull request: the pictures the status comment shows for its
/// current head, or, for a fork, whose run writes nothing, the head itself.
pub(crate) struct Pending {
    pub pr: u64,
    pub head: String,
    /// The checks whose changed pictures wait; a fork's approval covers its whole head.
    pub checks: Vec<String>,
    pub fork: bool,
    /// The status comment that shows them; None for a fork.
    pub shown: Option<Found>,
}

impl Pending {
    /// What waits on pull request `pr` now, or why nothing does.
    pub(crate) fn read(api: &Api, pr: u64) -> Result<Result<Pending, String>> {
        let (head, from) = api.head(pr)?;
        if from != api.repo {
            return Ok(Ok(Pending {
                pr,
                head,
                checks: Vec::new(),
                fork: true,
                shown: None,
            }));
        }
        let found = api.status_comment(pr)?;
        let Some(waits) = found.as_ref().and_then(|f| f.comment.waiting.clone()) else {
            return Ok(Err(format!(
                "no changed pictures wait for approval on pull request {pr}"
            )));
        };
        if waits.head != head {
            return Ok(Err(format!(
                "nothing to approve for {} yet: wait for its proof, then approve again",
                short(&head)
            )));
        }
        Ok(Ok(Pending {
            pr,
            head,
            checks: waits.checks,
            fork: false,
            shown: found,
        }))
    }

    /// What an approval covers, as a reply names it.
    pub(crate) fn what(&self) -> String {
        if self.fork {
            return "this fork's head".into();
        }
        let checks: Vec<String> = self.checks.iter().map(|c| format!("`{c}`")).collect();
        format!("the changed pictures of {}", checks.join(", "))
    }

    /// The `fearlezz/pictures` description while nobody approved.
    fn waits(&self) -> String {
        if self.fork {
            format!(
                "this fork's head waits for /fearlezz approve {} <words>",
                short(&self.head)
            )
        } else {
            format!(
                "changed pictures of {} wait for /fearlezz approve",
                self.checks.join(", ")
            )
        }
    }

    /// Whether `comment`, as GitHub's API describes it, approves what waits: a `/fearlezz
    /// approve <words>` by a person with write access (see `judge`), written after the status
    /// comment showed these pictures of this head, or for a fork naming its head, so a push in
    /// between never gets the approval of what the person looked at before.
    pub(crate) fn judge(&self, api: &Api, comment: &Value) -> Result<Result<Approval, String>> {
        let who = comment["user"]["login"].as_str().unwrap_or_default();
        let short = short(&self.head);
        if let Some(said) = command(comment["body"].as_str().unwrap_or_default()) {
            if let Some(found) = &self.shown {
                let written = comment["created_at"].as_str().unwrap_or_default();
                if written <= found.since() {
                    return Ok(Err(format!(
                        "@{who}, that approval was written before the proof of {short} showed its \
                         pictures: look at them, then approve again."
                    )));
                }
            }
            let named = said.split_whitespace().any(|w| {
                w.len() >= 7
                    && w.chars().all(|c| c.is_ascii_hexdigit())
                    && self.head.starts_with(&w.to_ascii_lowercase())
            });
            if self.fork && !named {
                return Ok(Err(format!(
                    "@{who}, a fork's pull request is approved by naming the head you looked at: \
                     `/fearlezz approve {short} <words>`, after looking at the pictures in the \
                     `fearlezz-pictures` artifact of its run for {short}."
                )));
            }
        }
        judge(api, comment)
    }

    /// The newest comment on the pull request that approves what waits, or why the newest
    /// `/fearlezz approve` does not.
    pub(crate) fn approval(&self, api: &Api) -> Result<Result<Approval, String>> {
        let mut why = None;
        for comment in api.comments(self.pr)?.iter().rev() {
            if command(comment["body"].as_str().unwrap_or_default()).is_none() {
                continue;
            }
            match self.judge(api, comment)? {
                Ok(approval) => return Ok(Ok(approval)),
                Err(e) => {
                    why.get_or_insert(e);
                }
            }
        }
        Ok(Err(why.unwrap_or_else(|| {
            format!(
                "nobody approved the pictures of {} on pull request {}, or the approval was \
                 deleted",
                short(&self.head),
                self.pr
            )
        })))
    }
}

/// Sets `fearlezz/pictures` on the head of pull request `pr`, and the status comment's
/// headline, to what the approval rule says now: green linked to the approval, or pending. It
/// reads everything again after each write, so a comment deleted or edited while it ran never
/// leaves the status green, in whatever order GitHub runs the events. Returns what waits and
/// the approval, or why nothing waits.
pub(crate) fn settle(
    api: &Api,
    pr: u64,
) -> Result<Result<(Pending, Result<Approval, String>), String>> {
    const ATTEMPTS: usize = 4;
    let mut set: Option<(String, String, String)> = None;
    for attempt in 1..=ATTEMPTS {
        let pending = match Pending::read(api, pr)? {
            Ok(pending) => pending,
            Err(why) => return Ok(Err(why)),
        };
        let verdict = pending.approval(api)?;
        let (state, description, target) = match &verdict {
            Ok(a) => ("success", format!("approved by {}", a.by), a.url.clone()),
            Err(_) => (
                "pending",
                pending.waits(),
                pending
                    .shown
                    .as_ref()
                    .map(|f| f.url.clone())
                    .unwrap_or_default(),
            ),
        };
        if set.is_none() {
            set = api.latest_status(&pending.head)?.and_then(|s| {
                (s["creator"]["type"] == "Bot").then(|| {
                    let text = |k: &str| s[k].as_str().unwrap_or_default().to_string();
                    (pending.head.clone(), text("state"), text("target_url"))
                })
            });
        }
        if let Some(found) = &pending.shown {
            let mut shown = found.comment.clone();
            shown.settle(verdict.as_ref().ok().map(|a| a.by.as_str()), found.since());
            if shown.render() != found.comment.render() {
                api.rewrite(found.id, &shown)?;
            }
        }
        let now = (pending.head.clone(), state.to_string(), target.clone());
        if set.as_ref() == Some(&now) || attempt == ATTEMPTS {
            return Ok(Ok((pending, verdict)));
        }
        let target = (!target.is_empty()).then_some(target.as_str());
        api.status(&pending.head, state, &description, target)?;
        set = Some(now);
    }
    unreachable!("the last attempt returns")
}

/// The approval that counts for check `check` at commit `head`, checked out in `root`, on its
/// open pull request, by the rule CI settles the status with.
pub(crate) fn approval_of_commit(root: &Path, head: &str, check: &str) -> Result<Approval> {
    let api = Api::local(root)?;
    let short = short(head);
    let ask = "ask the developer to approve its pictures there with `/fearlezz approve <their \
               words>`";
    let pulls = api.get(&format!("commits/{head}/pulls"))?;
    let pr = pulls
        .as_array()
        .into_iter()
        .flatten()
        .find(|p| p["state"] == "open" && p["head"]["sha"] == head)
        .and_then(|p| p["number"].as_u64());
    let Some(pr) = pr else {
        bail!(
            "commit {short} is not the head of an open pull request on {}: push it, wait for its \
             proof, and {ask}",
            api.repo
        );
    };
    let pending = Pending::read(&api, pr)?.map_err(|why| anyhow!("{why}; {ask}"))?;
    if pending.head != head {
        bail!("pull request {pr} moved on from {short}: pull its head, then run this again");
    }
    if !pending.fork && !pending.checks.iter().any(|c| c == check) {
        let checks: Vec<String> = pending.checks.iter().map(|c| format!("`{c}`")).collect();
        bail!(
            "the pictures waiting on pull request {pr} are those of {}, not of `{check}`: an \
             approval there covers only what the developer looked at",
            checks.join(", ")
        );
    }
    pending.approval(&api)?.map_err(|why| {
        anyhow!("the pictures of {short} on pull request {pr} are not approved: {why}; {ask}")
    })
}

/// The first seven characters of a commit, as people read it.
pub(crate) fn short(sha: &str) -> String {
    sha.chars().take(7).collect()
}

// The status comment

/// What waits for a person on the pull request, as the status comment's hidden line holds it.
/// The commit status cannot hold it: its description is short, and an approval replaces it.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Waiting {
    /// The pull request's head commit the proof is for; an approval binds to it.
    pub head: String,
    pub checks: Vec<String>,
    /// What the waiting pictures are, so a run of the same head that shows the same pictures
    /// keeps an approval and one that shows others does not.
    #[serde(default)]
    pub pictures: String,
    /// When GitHub first showed these pictures of this head; an approval written before then
    /// does not count. None: when the comment was last written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// The headline while the pictures wait.
    pub waits: String,
    /// The headline once a person approves, before who did.
    pub approved: String,
}

/// The status comment on a pull request: a fixed title, the headline, the rest of the body,
/// and the hidden line of what waits for a person.
#[derive(Clone)]
pub(crate) struct StatusComment {
    pub headline: String,
    pub body: String,
    pub waiting: Option<Waiting>,
}

/// A status comment found on a pull request.
#[derive(Clone)]
pub(crate) struct Found {
    pub id: u64,
    pub url: String,
    /// When it was last written, by GitHub's clock.
    pub updated: String,
    pub comment: StatusComment,
}

impl Found {
    /// When GitHub first showed the waiting pictures.
    pub(crate) fn since(&self) -> &str {
        self.comment
            .waiting
            .as_ref()
            .and_then(|w| w.since.as_deref())
            .unwrap_or(&self.updated)
    }
}

impl StatusComment {
    pub(crate) fn render(&self) -> String {
        let mut text = format!(
            "{COMMENT_MARKER}\n{TITLE}\n\n{}\n\n{}",
            self.headline, self.body
        );
        if let Some(w) = &self.waiting {
            if !text.ends_with('\n') {
                text.push('\n');
            }
            // `>` never appears unescaped, so the line stays inside its HTML comment.
            let json = serde_json::to_string(w)
                .unwrap_or_default()
                .replace('>', "\\u003e");
            text.extend([WAITING_MARKER, &json, " -->\n"]);
        }
        text
    }

    pub(crate) fn parse(text: &str) -> Option<StatusComment> {
        let rest = text.strip_prefix(&format!("{COMMENT_MARKER}\n{TITLE}\n\n"))?;
        let (headline, body) = rest.split_once("\n\n").unwrap_or((rest, ""));
        let mut waiting = None;
        let mut lines: Vec<&str> = Vec::new();
        for line in body.lines() {
            match line
                .strip_prefix(WAITING_MARKER)
                .and_then(|l| l.trim_end().strip_suffix("-->"))
            {
                Some(json) => waiting = serde_json::from_str(json.trim_end()).ok(),
                None => lines.push(line),
            }
        }
        let mut body = lines.join("\n");
        body.push('\n');
        Some(StatusComment {
            headline: headline.to_string(),
            body,
            waiting,
        })
    }

    /// Adds a paragraph at the end.
    pub(crate) fn note(&mut self, text: &str) {
        self.body.extend(["\n", text, "\n"]);
    }

    /// Says who approved the waiting pictures, or that they wait, keeping when they were
    /// first shown, since rewriting the comment moves its time.
    pub(crate) fn settle(&mut self, by: Option<&str>, since: &str) {
        if let Some(w) = &mut self.waiting {
            self.headline = match by {
                Some(by) => format!("{}; @{by} approved the changed pictures", w.approved),
                None => w.waits.clone(),
            };
            w.since.get_or_insert_with(|| since.to_string());
        }
    }
}

// What a proof shows

/// One check in the report.
pub(crate) struct Row {
    pub id: String,
    pub outcome: &'static str,
    /// How many tests ran, for a check that counts them.
    pub tests: Option<u64>,
    pub secs: f64,
    /// The first line of the command, marked when there is more.
    pub run: String,
}

/// What proving one commit shows on GitHub: the status comment and the check page.
pub(crate) struct Proof<'a> {
    /// The commit as people read it, with the merge it proved.
    pub commit: &'a str,
    /// The full head commit an approval binds to.
    pub head: &'a str,
    pub rows: &'a [Row],
    /// What keeps the commit from being proven.
    pub items: &'a [floor::Item],
    pub notes: &'a [String],
    /// Looks whose changed pictures wait for a person.
    pub waits: &'a [Look],
    /// The URL the pictures folder is served at, when CI publishes it.
    pub pictures_url: Option<&'a str>,
    /// CI keeps the pictures folder as the `fearlezz-pictures` artifact.
    pub artifact: bool,
    /// The run's page, with each check's output.
    pub run_url: Option<String>,
}

impl Proof<'_> {
    fn counted(&self, word: &str) -> String {
        let passed = self.rows.iter().filter(|r| r.outcome == "passed").count();
        format!(
            "**{word}**: commit {}, {passed} of {} checks passed",
            self.commit,
            self.rows.len()
        )
    }

    /// Proven or not, with the commit and how many checks passed. It never says proven while
    /// changed pictures wait for a person.
    pub(crate) fn headline(&self) -> String {
        let checks: Vec<String> = self
            .waits
            .iter()
            .map(|w| format!("`{}`", w.check))
            .collect();
        match (self.items.is_empty(), checks.is_empty()) {
            (true, true) => self.counted("Proven"),
            (true, false) => format!(
                "{}; the changed pictures of {} wait for a person",
                self.counted("Pictures wait"),
                checks.join(", ")
            ),
            (false, _) => self.counted("Not proven"),
        }
    }

    /// The checks whose pictures wait.
    pub(crate) fn waiting_checks(&self) -> Vec<String> {
        self.waits.iter().map(|w| w.check.clone()).collect()
    }

    /// The status comment: the headline, what blocks it, the pictures to look at, the checks
    /// folded away, and a link to the check page.
    pub(crate) fn comment(&self) -> Result<StatusComment> {
        let mut text = String::new();
        self.blockers(&mut text)?;
        self.notes(&mut text)?;
        self.pictures(&mut text)?;
        if !self.rows.is_empty() {
            writeln!(
                text,
                "<details><summary>{} checks</summary>\n",
                self.rows.len()
            )?;
            self.table(&mut text)?;
            writeln!(text, "\n</details>\n")?;
        }
        if let Some(url) = &self.run_url {
            writeln!(text, "[The check page]({url}) has each check's output.")?;
        }
        let word = if self.items.is_empty() {
            "Proven"
        } else {
            "Not proven"
        };
        let waiting = (!self.waits.is_empty()).then(|| Waiting {
            head: self.head.to_string(),
            checks: self.waiting_checks(),
            pictures: pictures_id(self.waits),
            since: None,
            waits: self.headline(),
            approved: self.counted(word),
        });
        Ok(StatusComment {
            headline: self.headline(),
            body: text,
            waiting,
        })
    }

    /// The check page: the headline, what blocks it, every check, and the end of each failing
    /// output.
    pub(crate) fn page(&self, failures: &[RunResult]) -> Result<String, std::fmt::Error> {
        let mut text = String::new();
        writeln!(text, "{TITLE}\n\n{}\n", self.headline())?;
        self.blockers(&mut text)?;
        self.notes(&mut text)?;
        self.pictures(&mut text)?;
        self.table(&mut text)?;
        if self.rows.is_empty() {
            writeln!(text, "\nNo check reads what this commit changed.")?;
        }
        for f in failures {
            writeln!(
                text,
                "\n<details><summary>The end of what {} printed</summary>\n\n```",
                cell(&f.check)
            )?;
            for line in crate::run::tail(&f.log, 40) {
                writeln!(text, "{}", line.replace("```", "'''"))?;
            }
            writeln!(text, "```\n\n</details>")?;
        }
        Ok(text)
    }

    /// The table of what keeps the commit from being proven, if anything does.
    fn blockers(&self, text: &mut String) -> std::fmt::Result {
        if self.items.is_empty() {
            return Ok(());
        }
        writeln!(text, "| what | why | what to do |\n| --- | --- | --- |")?;
        for item in self.items {
            writeln!(
                text,
                "| {} | {} | {} |",
                cell(&item.what),
                cell(&item.why),
                cell(&item.fix)
            )?;
        }
        writeln!(text)
    }

    fn notes(&self, text: &mut String) -> std::fmt::Result {
        for note in self.notes {
            writeln!(text, "{note}\n")?;
        }
        Ok(())
    }

    fn table(&self, text: &mut String) -> std::fmt::Result {
        writeln!(text, "| check | result | tests | time | command |")?;
        writeln!(text, "| --- | --- | --- | --- | --- |")?;
        for row in self.rows {
            let tests = row.tests.map(|n| n.to_string()).unwrap_or_default();
            writeln!(
                text,
                "| {} | {} | {tests} | {:.1} s | `{}` |",
                cell(&row.id),
                row.outcome,
                row.secs,
                code(&row.run)
            )?;
        }
        Ok(())
    }

    /// The pictures a person should look at: before and after side by side when the folder is
    /// published, and how to approve them.
    fn pictures(&self, text: &mut String) -> std::fmt::Result {
        if self.waits.is_empty() {
            return Ok(());
        }
        writeln!(text, "#### Pictures to look at\n")?;
        let mut left = SHOWN;
        let mut hidden = 0;
        for look in self.waits {
            let why = look
                .no_before
                .as_ref()
                .map(|w| format!(" (no before: {})", cell(w)))
                .unwrap_or_default();
            writeln!(
                text,
                "**`{}`**: {}{why}\n",
                code(&look.check),
                look.counts()
            )?;
            let items: Vec<_> = look
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
            let dir = folder(look);
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
            match self.pictures_url {
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
        if self.artifact && (self.pictures_url.is_none() || hidden > 0) {
            writeln!(text, "{ARTIFACT}\n")?;
        }
        writeln!(
            text,
            "To approve them, a person with write access comments `/fearlezz approve <what they \
             think of them>`. A new push needs a new approval.\n"
        )
    }
}

/// Where the pictures are when the comment cannot show them.
pub(crate) const ARTIFACT: &str =
    "Every changed picture, before and after, is in the `fearlezz-pictures` artifact of the run.";

/// An item's change, and that it never blocks when the check calls it flaky.
fn label(item: &crate::look::Item) -> String {
    if item.flaky {
        format!("{} (flaky, never blocks)", item.change.label())
    } else {
        item.change.label().to_string()
    }
}

/// Text for a table cell or an HTML attribute.
fn cell(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('|', "\\|")
        .replace('\n', " ")
}

/// Text for a code span in a table cell, where HTML entities would show as typed.
fn code(s: &str) -> String {
    s.replace('`', "'").replace('|', "\\|").replace('\n', " ")
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
pub(crate) fn copy_pictures(waits: &[Look], dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    for look in waits {
        let Some(from) = look.report.parent() else {
            continue;
        };
        let to = dir.join(folder(look));
        for item in look.items.iter().filter(|i| i.change != Change::Same) {
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

/// What the pictures that wait are: each check, and the name, change and bytes of each
/// picture that blocks.
fn pictures_id(waits: &[Look]) -> String {
    use sha2::{Digest as _, Sha256};
    let mut hash = Sha256::new();
    for look in waits {
        hash.update(format!("{}\0", look.check));
        let dir = look.report.parent().unwrap_or(Path::new(""));
        for item in look
            .items
            .iter()
            .filter(|i| i.change != Change::Same && !i.flaky)
        {
            hash.update(format!("{}\0{}\0", item.name, item.change.label()));
            for rel in [&item.before, &item.after] {
                let bytes = rel
                    .as_ref()
                    .and_then(|r| fs::read(dir.join(r)).ok())
                    .unwrap_or_default();
                hash.update(Sha256::digest(&bytes));
            }
        }
    }
    hex::encode(hash.finalize())
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn a_remote_on_github_names_its_repository() {
        for remote in [
            "git@github.com:owner/repo.git",
            "https://github.com/owner/repo",
            "https://github.com/owner/repo.git/",
            "ssh://git@github.com/owner/repo.git",
        ] {
            assert_eq!(
                repository(remote).as_deref(),
                Some("owner/repo"),
                "{remote}"
            );
        }
        assert_eq!(repository("https://gitlab.com/owner/repo"), None);
    }

    #[test]
    fn a_status_comment_reads_back_as_written() {
        let comment = StatusComment {
            headline: "**Pictures wait**: x".into(),
            body: "| a | b |\n\nmore\n".into(),
            waiting: Some(Waiting {
                head: "abc".into(),
                checks: vec!["ui".into()],
                pictures: "p".into(),
                since: None,
                waits: "**Pictures wait**: x".into(),
                approved: "**Proven**: <x>".into(),
            }),
        };
        let mut back = StatusComment::parse(&comment.render()).unwrap();
        assert_eq!(back.render(), comment.render());
        back.settle(Some("lead"), "t1");
        assert_eq!(
            back.headline,
            "**Proven**: <x>; @lead approved the changed pictures"
        );
        back.settle(None, "t2");
        assert_eq!(back.headline, "**Pictures wait**: x");
        assert_eq!(back.waiting.unwrap().since.as_deref(), Some("t1"));
    }
}
