//! GitHub: the one client Fearlezz talks to it with (REST through curl, and GraphQL for the
//! atomic ref update), what a proof shows there (the status comment and the check page, with
//! one owner for their Markdown), and the one rule for which approval counts. CI writes the
//! comment and answers `/fearlezz approve` and `/fearlezz accept` (`action.rs`); the
//! developer's machine reads an approval back with the same client and the same rule.

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
use crate::proof::Ran;

/// The first line of the status comment, so CI finds its own comment to replace.
const COMMENT_MARKER: &str = "<!-- fearlezz -->";

/// The title under the marker; the headline follows it.
const TITLE: &str = "### Fearlezz";

/// Starts the hidden line of the status comment that says what an approval covers.
const WAITING_MARKER: &str = "<!-- fearlezz-waiting ";

/// The commit status that holds a pull request while anything waits for a person: changed
/// pictures for an approval, a weakened proof for an admin's accept.
pub(crate) const CONTEXT: &str = "fearlezz/approval";

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

    /// Sets the `fearlezz/approval` status of `sha`.
    pub(crate) fn status(
        &self,
        sha: &str,
        state: &str,
        description: &str,
        target: Option<&str>,
    ) -> Result<()> {
        let mut body =
            json!({ "state": state, "context": CONTEXT, "description": clip(description) });
        if let Some(target) = target {
            body["target_url"] = target.into();
        }
        self.send("POST", &format!("statuses/{sha}"), &body)?;
        Ok(())
    }

    /// The latest `fearlezz/approval` status of `sha`, if any.
    pub(crate) fn latest_status(&self, sha: &str) -> Result<Option<Value>> {
        let all = self.get(&format!("commits/{sha}/statuses?per_page=100"))?;
        Ok(all
            .as_array()
            .into_iter()
            .flatten()
            .find(|s| s["context"] == CONTEXT)
            .cloned())
    }

    /// The status comment CI wrote on pull request `pr`: the newest comment with the marker
    /// written by an app (the workflow's token or an app's). A person's comment never is, so
    /// it works without asking GitHub who the token is, which an app's token cannot.
    pub(crate) fn status_comment(&self, pr: u64) -> Result<Option<Found>> {
        Ok(self.comments(pr)?.into_iter().rev().find_map(|c| {
            if c["user"]["type"] != "Bot" {
                return None;
            }
            let comment = StatusComment::parse(&c["body"].as_str()?.replace('\r', ""))?;
            Some(Found {
                id: c["id"].as_u64()?,
                url: c["html_url"].as_str().unwrap_or_default().to_string(),
                created: c["created_at"].as_str().unwrap_or_default().to_string(),
                comment,
            })
        }))
    }

    /// Writes the status comment of pull request `pr`; `old` is the one there. Only the
    /// proving run writes it. What waits (pictures, weakenings) and differs from what `old`
    /// showed comes in a new comment, so GitHub's time of writing it is when it was first shown
    /// and an approval or accept written before then never counts for it; otherwise `old` is
    /// edited in place and keeps that time. Returns its URL.
    pub(crate) fn write_status_comment(
        &self,
        pr: u64,
        old: Option<&Found>,
        comment: &StatusComment,
    ) -> Result<String> {
        let body = comment.render();
        let shown = |c: &StatusComment| {
            c.waiting.as_ref().map(|w| {
                (
                    w.head.clone(),
                    w.checks.clone(),
                    w.pictures.clone(),
                    w.weakened.clone(),
                )
            })
        };
        let written = match old {
            Some(found) if comment.waiting.is_none() || shown(&found.comment) == shown(comment) => {
                self.send(
                    "PATCH",
                    &format!("issues/comments/{}", found.id),
                    &json!({ "body": body }),
                )?
            }
            _ => {
                if let Some(found) = old {
                    // Gone already is fine: the new comment takes its place either way.
                    let (code, answer) =
                        self.call("DELETE", &format!("issues/comments/{}", found.id), None)?;
                    if !(200..300).contains(&code) && code != 404 {
                        let why = answer["message"].as_str().unwrap_or_default();
                        bail!("GitHub answered {code} to deleting the status comment: {why}");
                    }
                }
                self.comment(pr, &body)?
            }
        };
        Ok(written["html_url"].as_str().unwrap_or_default().to_string())
    }
}

/// A status description as GitHub keeps it: it refuses a longer one.
fn clip(description: &str) -> String {
    description.chars().take(140).collect()
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

/// The two things a person says by comment: approving changed pictures, which needs write
/// access, and accepting a weakened proof, which needs admin rights.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Approve,
    Accept,
}

impl Kind {
    /// The command's word: `/fearlezz <word> <words>`.
    pub(crate) fn word(self) -> &'static str {
        match self {
            Kind::Approve => "approve",
            Kind::Accept => "accept",
        }
    }

    /// What giving it is called in a reply.
    fn noun(self) -> &'static str {
        match self {
            Kind::Approve => "approval",
            Kind::Accept => "accept",
        }
    }
}

/// An approval or accept that counts: who gave it, in their own words, and where.
pub(crate) struct Approval {
    pub by: String,
    pub said: String,
    pub url: String,
}

/// The kind and words of a `/fearlezz approve <words>` or `/fearlezz accept <words>` comment,
/// the words on one line, or None for another comment.
pub(crate) fn command(body: &str) -> Option<(Kind, String)> {
    let body = body.replace('\r', "");
    let first = body.lines().next().unwrap_or_default();
    [Kind::Approve, Kind::Accept].into_iter().find_map(|kind| {
        let start = format!("/fearlezz {}", kind.word());
        if first != start && !first.starts_with(&format!("{start} ")) {
            return None;
        }
        let words = body.strip_prefix(&start).unwrap_or_default();
        Some((kind, words.split_whitespace().collect::<Vec<_>>().join(" ")))
    })
}

/// Whether `comment`, as GitHub's API describes it, approves or accepts: a `/fearlezz approve
/// <words>` by a person with write access, or a `/fearlezz accept <words>` by one with admin
/// rights, in their own words. The outer error is GitHub failing; the inner one says why the
/// comment does not count.
pub(crate) fn judge(api: &Api, comment: &Value) -> Result<Result<Approval, String>> {
    let who = comment["user"]["login"].as_str().unwrap_or_default();
    let Some((kind, said)) = command(comment["body"].as_str().unwrap_or_default()) else {
        return Ok(Err(format!(
            "the comment by @{who} is no `/fearlezz approve <words>` or `/fearlezz accept \
             <words>`."
        )));
    };
    let what = match kind {
        Kind::Approve => "approve pictures",
        Kind::Accept => "accept a weakened proof",
    };
    if comment["user"]["type"] != "User" {
        return Ok(Err(format!("Only a person can {what}, not @{who}.")));
    }
    let permission = match api.call("GET", &format!("collaborators/{who}/permission"), None)? {
        (200, p) => p["permission"].as_str().unwrap_or_default().to_string(),
        _ => String::new(),
    };
    let (allowed, needs) = match kind {
        Kind::Approve => (
            permission == "admin" || permission == "write",
            "write access to",
        ),
        Kind::Accept => (permission == "admin", "admin rights on"),
    };
    if !allowed {
        return Ok(Err(format!(
            "@{who} cannot {what}: that needs {needs} this repository."
        )));
    }
    if said.is_empty() {
        let hint = match kind {
            Kind::Approve => "what you think of the pictures",
            Kind::Accept => "why the weaker proof is right",
        };
        return Ok(Err(format!(
            "@{who}, {}ing needs your own words: `/fearlezz {} <{hint}>`.",
            kind.word().trim_end_matches('e'),
            kind.word()
        )));
    }
    Ok(Ok(Approval {
        by: who.to_string(),
        said,
        url: comment["html_url"].as_str().unwrap_or_default().to_string(),
    }))
}

/// What waits for a person on a pull request: what the status comment shows for its current
/// head (changed pictures, a weakened proof), or, for a fork, whose run writes nothing, the
/// head itself.
pub(crate) struct Pending {
    pub pr: u64,
    pub head: String,
    /// The checks whose changed pictures wait; a fork's approval covers its whole head.
    pub checks: Vec<String>,
    /// What the change weakens about the proof, as the status comment showed it; it waits for
    /// an admin's accept. A fork's run cannot hold it, so its weakening fails its proof.
    pub weakened: Vec<String>,
    pub fork: bool,
    /// The status comment that shows them; None for a fork.
    pub shown: Option<Found>,
}

/// What the comments on a pull request give what waits there: the newest approval of its
/// changed pictures (or a fork's head) and the newest accept of its weakened proof, each None
/// when it does not wait, or why none counts.
pub(crate) struct Verdict {
    pub approve: Option<Result<Approval, String>>,
    pub accept: Option<Result<Approval, String>>,
}

impl Verdict {
    /// Everything that waits has what it needs.
    pub(crate) fn done(&self) -> bool {
        [&self.approve, &self.accept]
            .into_iter()
            .all(|p| p.as_ref().is_none_or(Result::is_ok))
    }

    pub(crate) fn of(&self, kind: Kind) -> Option<&Result<Approval, String>> {
        match kind {
            Kind::Approve => self.approve.as_ref(),
            Kind::Accept => self.accept.as_ref(),
        }
    }

    /// Who gave what counts, for the status description.
    fn given(&self) -> Vec<String> {
        let mut given = Vec::new();
        if let Some(Ok(a)) = &self.approve {
            given.push(format!("approved by {}", a.by));
        }
        if let Some(Ok(a)) = &self.accept {
            given.push(format!("accepted by {}", a.by));
        }
        given
    }
}

/// What still waits, as a status description names it: the changed pictures of `checks`
/// unless approved, the `weakened` proof unless accepted.
fn still_waits(
    checks: &[String],
    weakened: &[String],
    approved: bool,
    accepted: bool,
) -> Vec<String> {
    let mut waits = Vec::new();
    if !checks.is_empty() && !approved {
        waits.push(format!(
            "changed pictures of {} wait for /fearlezz approve",
            checks.join(", ")
        ));
    }
    if !weakened.is_empty() && !accepted {
        let mut names: Vec<&str> = weakened.iter().map(|w| named(w)).collect();
        names.dedup();
        waits.push(format!(
            "weakened {} waits for an admin's /fearlezz accept",
            names.join(", ")
        ));
    }
    waits
}

/// The `fearlezz/approval` description while nothing that waits has what it needs.
pub(crate) fn waits_description(checks: &[String], weakened: &[String]) -> String {
    still_waits(checks, weakened, false, false).join("; ")
}

/// The name a weakening is about: the first part in backticks of what the status comment
/// says weakened, such as `tests` in "check `tests` is weaker: ...".
fn named(weakening: &str) -> &str {
    weakening.split('`').nth(1).unwrap_or(weakening)
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
                weakened: Vec::new(),
                fork: true,
                shown: None,
            }));
        }
        let found = api.status_comment(pr)?;
        let Some(waits) = found.as_ref().and_then(|f| f.comment.waiting.clone()) else {
            return Ok(Err(format!(
                "nothing waits for approval on pull request {pr}"
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
            weakened: waits.weakened,
            fork: false,
            shown: found,
        }))
    }

    /// What an approval or accept covers, as a reply names it.
    pub(crate) fn what(&self, kind: Kind) -> String {
        match kind {
            Kind::Approve if self.fork => "this fork's head".into(),
            Kind::Approve => {
                let checks: Vec<String> = self.checks.iter().map(|c| format!("`{c}`")).collect();
                format!("the changed pictures of {}", checks.join(", "))
            }
            Kind::Accept => "the weakened proof".into(),
        }
    }

    /// The `fearlezz/approval` description for `verdict`.
    pub(crate) fn description(&self, verdict: &Verdict) -> String {
        let given = verdict.given();
        if verdict.done() {
            return given.join(", ");
        }
        let waits = if self.fork {
            vec![format!(
                "this fork's head waits for /fearlezz approve {} <words>",
                short(&self.head)
            )]
        } else {
            still_waits(
                &self.checks,
                &self.weakened,
                matches!(verdict.approve, Some(Ok(_))),
                matches!(verdict.accept, Some(Ok(_))),
            )
        };
        given
            .into_iter()
            .chain(waits)
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// What still waits after `verdict`, in a sentence for a reply.
    pub(crate) fn still(&self, verdict: &Verdict) -> String {
        let mut waits = Vec::new();
        if let Some(Err(_)) = verdict.approve {
            waits.push(format!(
                "{} for `/fearlezz approve <words>`",
                self.what(Kind::Approve)
            ));
        }
        if let Some(Err(_)) = verdict.accept {
            waits.push(format!(
                "{} for an admin's `/fearlezz accept <words>`",
                self.what(Kind::Accept)
            ));
        }
        if waits.is_empty() {
            format!("Nothing else waits: `{CONTEXT}` is green.")
        } else {
            format!("Still waiting: {}.", waits.join("; "))
        }
    }

    /// Whether `comment`, as GitHub's API describes it, counts for what waits: a `/fearlezz
    /// approve <words>` by a person with write access while changed pictures wait, or a
    /// `/fearlezz accept <words>` by one with admin rights while a weakened proof waits (see
    /// `judge`), written after the status comment showed them for this head, or for a fork's
    /// approval naming its head, so a push in between never gets the approval of what the
    /// person looked at before.
    pub(crate) fn judge(&self, api: &Api, comment: &Value) -> Result<Result<Approval, String>> {
        let who = comment["user"]["login"].as_str().unwrap_or_default();
        let short = short(&self.head);
        if let Some((kind, said)) = command(comment["body"].as_str().unwrap_or_default()) {
            match kind {
                Kind::Approve if !self.fork && self.checks.is_empty() => {
                    return Ok(Err(format!(
                        "@{who}, no changed pictures wait for approval at {short}."
                    )));
                }
                Kind::Accept if self.weakened.is_empty() => {
                    return Ok(Err(format!(
                        "@{who}, no weakened proof waits for an accept at {short}."
                    )));
                }
                _ => {}
            }
            if let Some(found) = &self.shown {
                let written = comment["created_at"].as_str().unwrap_or_default();
                if written <= found.created.as_str() {
                    let shown = match kind {
                        Kind::Approve => "its pictures",
                        Kind::Accept => "what it weakens",
                    };
                    return Ok(Err(format!(
                        "@{who}, that {} was written before the proof of {short} showed \
                         {shown}: look at them, then {} again.",
                        kind.noun(),
                        kind.word()
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

    /// The newest of `comments` that counts as `kind` for what waits, or why the newest
    /// `/fearlezz <kind>` does not.
    fn newest(
        &self,
        api: &Api,
        comments: &[Value],
        kind: Kind,
    ) -> Result<Result<Approval, String>> {
        let mut why = None;
        for comment in comments.iter().rev() {
            if command(comment["body"].as_str().unwrap_or_default()).is_none_or(|(k, _)| k != kind)
            {
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
                "nobody {}ed {} of {} on pull request {}, or the {} was deleted",
                kind.word().trim_end_matches('e'),
                self.what(kind),
                short(&self.head),
                self.pr,
                kind.noun()
            )
        })))
    }

    /// The newest approval of the changed pictures that counts, or why none does.
    pub(crate) fn approval(&self, api: &Api) -> Result<Result<Approval, String>> {
        self.newest(api, &api.comments(self.pr)?, Kind::Approve)
    }

    /// What the comments on the pull request give what waits.
    pub(crate) fn verdict(&self, api: &Api) -> Result<Verdict> {
        let comments = api.comments(self.pr)?;
        let approve = (self.fork || !self.checks.is_empty())
            .then(|| self.newest(api, &comments, Kind::Approve))
            .transpose()?;
        let accept = (!self.weakened.is_empty())
            .then(|| self.newest(api, &comments, Kind::Accept))
            .transpose()?;
        Ok(Verdict { approve, accept })
    }
}

/// Sets `fearlezz/approval` on the head of pull request `pr` to what the approval rule says
/// now: green, linked to the accept (or else the approval), when everything that waits has
/// what it needs, or pending, saying what still waits. It never writes the status comment,
/// whose only writer is the proving run. It reads everything again after each write, so a
/// comment deleted or edited while it ran never leaves the status green, in whatever order
/// GitHub runs the events; its last attempt writes its verdict too. When nothing that waits
/// can be read (the status comment gone or broken), a green status an approval set goes back
/// to pending. Returns what waits and the verdict, or why nothing waits.
pub(crate) fn settle(api: &Api, pr: u64) -> Result<Result<(Pending, Verdict), String>> {
    const ATTEMPTS: usize = 4;
    // What `fearlezz/approval` of a head says: head, state, description, target.
    type Set = (String, String, String, String);
    let mut set: Option<Set> = None;
    let mut last = None;
    for _ in 0..ATTEMPTS {
        let pending = match Pending::read(api, pr)? {
            Ok(pending) => pending,
            Err(why) => {
                withdraw(api, pr, &why)?;
                return Ok(Err(why));
            }
        };
        let verdict = pending.verdict(api)?;
        let description = pending.description(&verdict);
        let (state, target) = if verdict.done() {
            let by = [&verdict.accept, &verdict.approve]
                .into_iter()
                .find_map(|p| p.as_ref().and_then(|r| r.as_ref().ok()));
            ("success", by.map(|a| a.url.clone()).unwrap_or_default())
        } else {
            (
                "pending",
                pending
                    .shown
                    .as_ref()
                    .map(|f| f.url.clone())
                    .unwrap_or_default(),
            )
        };
        if set.is_none() {
            set = bot_status(api, &pending.head)?;
        }
        let now = (
            pending.head.clone(),
            state.to_string(),
            clip(&description),
            target.clone(),
        );
        if set.as_ref() == Some(&now) {
            return Ok(Ok((pending, verdict)));
        }
        let target = (!target.is_empty()).then_some(target.as_str());
        api.status(&pending.head, state, &description, target)?;
        set = Some(now);
        last = Some((pending, verdict));
    }
    last.map(Ok).context("settling made no attempt")
}

/// The latest `fearlezz/approval` status of `head` if an app set it: head, state,
/// description, target.
fn bot_status(api: &Api, head: &str) -> Result<Option<(String, String, String, String)>> {
    Ok(api.latest_status(head)?.and_then(|s| {
        (s["creator"]["type"] == "Bot").then(|| {
            let text = |k: &str| s[k].as_str().unwrap_or_default().to_string();
            (
                head.to_string(),
                text("state"),
                text("description"),
                text("target_url"),
            )
        })
    }))
}

/// Takes back a green `fearlezz/approval` an approval set on the head of pull request `pr`
/// when nothing that waits can be read, `why`: what it approved can no longer be told.
fn withdraw(api: &Api, pr: u64, why: &str) -> Result<()> {
    let (head, _) = api.head(pr)?;
    if let Some((_, state, _, target)) = bot_status(api, &head)?
        && state == "success"
        && !target.is_empty()
    {
        api.status(
            &head,
            "pending",
            &format!("the approval no longer counts: {why}"),
            None,
        )?;
    }
    Ok(())
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
        if pending.checks.is_empty() {
            bail!("no changed pictures wait on pull request {pr}, so there is nothing to approve");
        }
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
    /// The checks whose changed pictures wait for an approval.
    #[serde(default)]
    pub checks: Vec<String>,
    /// What the waiting pictures are, so a run of the same head that shows the same pictures
    /// keeps the comment, and with it an approval, and one that shows others writes a new one.
    #[serde(default)]
    pub pictures: String,
    /// What the change weakens about the proof, each as `what: why`; it waits for an admin's
    /// accept, and a run that weakens something else writes a new comment.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub weakened: Vec<String>,
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
    /// When it was written, by GitHub's clock: when its waiting pictures were first shown.
    pub created: String,
    pub comment: StatusComment,
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
}

// What a proof shows

/// What proving one commit shows on GitHub, drawn from its proof: the status comment and the
/// check page.
pub(crate) struct Report<'a> {
    proof: &'a crate::proof::Proof,
    /// What keeps the commit from being proven.
    items: Vec<floor::Item>,
    /// What the change weakens about the proof, waiting for an admin's accept.
    weakened: Vec<floor::Item>,
    notes: Vec<String>,
    /// Looks whose changed pictures wait for a person.
    waits: Vec<Look>,
    /// The URL the pictures folder is served at, when CI publishes it.
    pictures_url: Option<&'a str>,
    /// CI keeps the pictures folder as the `fearlezz-pictures` artifact.
    artifact: bool,
    /// The proof page on the proof host, when it took the bundle.
    page: Option<String>,
}

impl<'a> Report<'a> {
    pub(crate) fn new(
        proof: &'a crate::proof::Proof,
        pictures_url: Option<&'a str>,
        artifact: bool,
        page: Option<String>,
    ) -> Report<'a> {
        Report {
            proof,
            items: proof.blockers(),
            weakened: proof.waiting_weakened().to_vec(),
            notes: proof.notes(),
            waits: proof.waits(),
            pictures_url,
            artifact,
            page,
        }
    }

    /// The link to the proof page, when there is one.
    fn page_link(&self, text: &mut String) -> std::fmt::Result {
        match &self.page {
            Some(url) => writeln!(
                text,
                "[The proof page]({url}) shows everything this run showed: every check, its \
                 tests, findings and output, and every picture.\n"
            ),
            None => Ok(()),
        }
    }

    fn counted(&self, word: &str) -> String {
        let ran = self.proof.ran_here().count();
        let passed = self
            .proof
            .ran_here()
            .filter(|c| c.outcome == Ran::Passed)
            .count();
        format!(
            "**{word}**: commit {}, {passed} of {ran} checks passed",
            self.proof.commit
        )
    }

    /// Proven or not, with the commit and how many checks passed. It never says proven while
    /// changed pictures need a person's approval or a weakened proof an admin's accept, and
    /// never whether they have it: an approval sets the `fearlezz/approval` status, never the
    /// comment, so the comment says only what it can keep true.
    pub(crate) fn headline(&self) -> String {
        let checks: Vec<String> = self
            .waits
            .iter()
            .map(|w| format!("`{}`", w.check))
            .collect();
        if !self.items.is_empty() {
            return self.counted("Not proven");
        }
        let mut waits = Vec::new();
        if !checks.is_empty() {
            waits.push(format!(
                "the changed pictures of {} need a person's approval",
                checks.join(", ")
            ));
        }
        if !self.weakened.is_empty() {
            waits.push("the weakened proof needs an admin's accept".to_string());
        }
        let word = match (checks.is_empty(), self.weakened.is_empty()) {
            (true, true) => return self.counted("Proven"),
            (false, true) => "Pictures need approval",
            (true, false) => "Weakened proof needs an admin",
            (false, false) => "Needs approval",
        };
        format!("{}; {}", self.counted(word), waits.join(" and "))
    }

    /// The checks whose pictures wait.
    pub(crate) fn waiting_checks(&self) -> Vec<String> {
        self.waits.iter().map(|w| w.check.clone()).collect()
    }

    /// The looks whose pictures wait.
    pub(crate) fn waits(&self) -> &[Look] {
        &self.waits
    }

    /// The status comment: the headline, what blocks it, the pictures to look at, the checks
    /// folded away, and a link to the check page.
    pub(crate) fn comment(&self) -> Result<StatusComment> {
        let mut text = String::new();
        self.page_link(&mut text)?;
        self.blockers(&mut text)?;
        self.weakened(&mut text)?;
        self.notes(&mut text)?;
        self.pictures(&mut text)?;
        let ran = self.proof.ran_here().count();
        if ran > 0 {
            writeln!(text, "<details><summary>{ran} checks</summary>\n")?;
            self.table(&mut text)?;
            writeln!(text, "\n</details>\n")?;
        }
        if let Some(url) = &self.proof.run_url {
            writeln!(text, "[The check page]({url}) has each check's output.")?;
        }
        let waiting = (!self.waits.is_empty() || !self.weakened.is_empty()).then(|| Waiting {
            head: self.proof.head.clone(),
            checks: self.waiting_checks(),
            pictures: pictures_id(&self.waits),
            weakened: self.weakened_lines(),
        });
        Ok(StatusComment {
            headline: self.headline(),
            body: text,
            waiting,
        })
    }

    /// The check page: the headline, what blocks it, every check, and the end of each failing
    /// output.
    pub(crate) fn page(&self) -> Result<String, std::fmt::Error> {
        let mut text = String::new();
        writeln!(text, "{TITLE}\n\n{}\n", self.headline())?;
        self.page_link(&mut text)?;
        self.blockers(&mut text)?;
        self.weakened(&mut text)?;
        self.notes(&mut text)?;
        self.pictures(&mut text)?;
        self.table(&mut text)?;
        if self.proof.ran_here().next().is_none() {
            writeln!(text, "\nNo check reads what this commit changed.")?;
        }
        for check in self
            .proof
            .checks
            .iter()
            .filter(|c| c.outcome == Ran::Failed)
        {
            let Some(log) = &check.log else { continue };
            writeln!(
                text,
                "\n<details><summary>The end of what {} printed</summary>\n\n```",
                cell(&check.id)
            )?;
            for line in crate::run::tail(log, 40) {
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
        for item in &self.items {
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

    /// What the change weakens about the proof, each as `what: why`.
    pub(crate) fn weakened_lines(&self) -> Vec<String> {
        self.weakened
            .iter()
            .map(|i| format!("{}: {}", i.what, i.why))
            .collect()
    }

    /// The weakened proof an admin should read, and how they accept it.
    fn weakened(&self, text: &mut String) -> std::fmt::Result {
        if self.weakened.is_empty() {
            return Ok(());
        }
        writeln!(
            text,
            "#### Weakened proof
"
        )?;
        writeln!(text, "| what | why | what to do |\n| --- | --- | --- |")?;
        for item in &self.weakened {
            writeln!(
                text,
                "| {} | {} | {} |",
                cell(&item.what),
                cell(&item.why),
                cell(&item.fix)
            )?;
        }
        writeln!(
            text,
            "\nTo accept it, a person with admin rights comments `/fearlezz accept <why the weaker \
             proof is right>`. A new push, or a run that weakens something else, needs a new \
             accept. The `{CONTEXT}` status says whether everything that waits has what it \
             needs.\n"
        )
    }

    fn notes(&self, text: &mut String) -> std::fmt::Result {
        for note in &self.notes {
            writeln!(text, "{note}\n")?;
        }
        Ok(())
    }

    fn table(&self, text: &mut String) -> std::fmt::Result {
        writeln!(text, "| check | result | tests | time | command |")?;
        writeln!(text, "| --- | --- | --- | --- | --- |")?;
        for check in self.proof.ran_here() {
            let tests = check.tests.map(|n| n.to_string()).unwrap_or_default();
            let run = match check.run.split_once('\n') {
                Some((first, _)) => format!("{first} ..."),
                None => check.run.clone(),
            };
            writeln!(
                text,
                "| {} | {} | {tests} | {:.1} s | `{}` |",
                cell(&check.id),
                check.outcome.word(),
                check.seconds,
                code(&run)
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
        for look in &self.waits {
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
             think of them>`. A new push needs a new approval. The `{CONTEXT}` status says \
             whether everything that waits has what it needs.\n"
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

/// What the pictures that wait are: each check and the base commit it was compared with, and
/// the name and change of each picture that blocks, with the bytes of a still. A film's bytes are not: two recordings of the same film
/// differ by frame timing, so a rerun of the same head that films the same change keeps an
/// approval, while one whose film the look no longer calls changed, or a changed still, does
/// not.
fn pictures_id(waits: &[Look]) -> String {
    use sha2::{Digest as _, Sha256};
    let mut hash = Sha256::new();
    for look in waits {
        // The same film against another base commit is another comparison.
        hash.update(format!("{}\0{}\0", look.check, look.base_commit));
        let dir = look.report.parent().unwrap_or(Path::new(""));
        for item in look
            .items
            .iter()
            .filter(|i| i.change != Change::Same && !i.flaky)
        {
            hash.update(format!("{}\0{}\0", item.name, item.change.label()));
            if item.kind == "film" {
                continue;
            }
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
    fn only_the_approve_and_accept_commands_count() {
        assert_eq!(
            command("/fearlezz approve"),
            Some((Kind::Approve, String::new()))
        );
        assert_eq!(
            command("/fearlezz approve  looks\r\nright "),
            Some((Kind::Approve, "looks right".into()))
        );
        assert_eq!(
            command("/fearlezz accept the lint was noise"),
            Some((Kind::Accept, "the lint was noise".into()))
        );
        assert_eq!(command("/fearlezz approved? not yet"), None);
        assert_eq!(command("/fearlezz acceptable"), None);
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
            headline: "**Pictures need approval**: x".into(),
            body: "| a | b |\n\nmore\n".into(),
            waiting: Some(Waiting {
                head: "abc".into(),
                checks: vec!["ui".into()],
                pictures: "p>".into(),
                weakened: vec!["check `tests` is weaker: it runs another command".into()],
            }),
        };
        let back = StatusComment::parse(&comment.render()).unwrap();
        assert_eq!(back.render(), comment.render());
    }
}
