//! A fake GitHub API on a local port: the parts Fearlezz uses (comments, statuses, pull
//! requests, collaborators, the git data API for the pictures branch, and GraphQL's atomic ref
//! update), kept in memory so a test can set it up and read back what Fearlezz did. Each token
//! acts as an identity, and a read only token's writes are refused, as on GitHub.

#![allow(dead_code, reason = "each test file uses only some of the helpers")]

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

/// The workflow token of a run on this repository.
pub(crate) const WORKFLOW_TOKEN: &str = "token";
/// The read only workflow token of a run on a fork's pull request.
pub(crate) const FORK_TOKEN: &str = "fork-token";
/// A GitHub App's installation token.
pub(crate) const APP_TOKEN: &str = "app-token";
/// A person's token, such as the developer's from `gh auth token`.
pub(crate) const PERSON_TOKEN: &str = "person-token";

/// A comment on a pull request.
#[derive(Clone, Debug)]
pub(crate) struct Comment {
    pub id: u64,
    pub pr: u64,
    pub login: String,
    pub kind: String,
    pub body: String,
    /// When it was written and last edited, by the fake's clock.
    pub created: String,
    pub updated: String,
}

impl Comment {
    pub(crate) fn url(&self) -> String {
        format!(
            "https://github.com/owner/repo/pull/{}#issuecomment-{}",
            self.pr, self.id
        )
    }
}

/// A commit status, with the login and account type that set it.
#[derive(Clone, Debug)]
pub(crate) struct Status {
    pub sha: String,
    pub state: String,
    pub context: String,
    pub description: String,
    pub target: String,
    pub creator: String,
    pub creator_kind: String,
}

/// Who a token acts as, and whether it may write.
#[derive(Clone, Debug)]
pub(crate) struct Token {
    pub login: String,
    pub kind: String,
    pub write: bool,
}

/// An entry of a tree: mode, type, sha.
type Entry = (String, String, String);

pub(crate) struct State {
    pub comments: Vec<Comment>,
    pub statuses: Vec<Status>,
    /// Logins and their permission (admin, write, read).
    pub permissions: BTreeMap<String, String>,
    /// Pull requests: head sha and state.
    pub pulls: BTreeMap<u64, (String, String)>,
    /// Pull requests opened from a fork.
    pub forks: BTreeSet<u64>,
    /// Tokens and who they act as.
    pub tokens: BTreeMap<String, Token>,
    /// Paths below the repository that answer 500.
    pub broken: BTreeSet<String>,
    /// Files another run pushes onto the pictures branch just before the next atomic ref
    /// update, so that update finds the branch moved.
    pub race: Option<Vec<(String, String)>>,
    /// A comment deleted just after the next status is set, as a person deleting an approval
    /// while its run is still going.
    pub delete_on_status: Option<u64>,
    pub refs: BTreeMap<String, String>,
    blobs: BTreeMap<String, Vec<u8>>,
    trees: BTreeMap<String, BTreeMap<String, Entry>>,
    /// Commits: tree and parents.
    pub commits: BTreeMap<String, (String, Vec<String>)>,
    next: u64,
}

impl Default for State {
    fn default() -> State {
        let token = |login: &str, kind: &str, write: bool| Token {
            login: login.into(),
            kind: kind.into(),
            write,
        };
        State {
            comments: Vec::new(),
            statuses: Vec::new(),
            permissions: BTreeMap::new(),
            pulls: BTreeMap::new(),
            forks: BTreeSet::new(),
            tokens: BTreeMap::from([
                (
                    WORKFLOW_TOKEN.into(),
                    token("github-actions[bot]", "Bot", true),
                ),
                (
                    FORK_TOKEN.into(),
                    token("github-actions[bot]", "Bot", false),
                ),
                (APP_TOKEN.into(), token("proof-app[bot]", "Bot", true)),
                (PERSON_TOKEN.into(), token("dev", "User", true)),
            ]),
            broken: BTreeSet::new(),
            race: None,
            delete_on_status: None,
            refs: BTreeMap::new(),
            blobs: BTreeMap::new(),
            trees: BTreeMap::new(),
            commits: BTreeMap::new(),
            next: 0,
        }
    }
}

impl State {
    fn sha(&mut self) -> String {
        self.next += 1;
        format!("{:040x}", self.next)
    }

    /// Creates a tree from entries whose paths may have folders, as GitHub does.
    fn tree(&mut self, entries: &[(String, Entry)]) -> String {
        let mut here: BTreeMap<String, Entry> = BTreeMap::new();
        let mut below: BTreeMap<String, Vec<(String, Entry)>> = BTreeMap::new();
        for (path, entry) in entries {
            match path.split_once('/') {
                Some((dir, rest)) => below
                    .entry(dir.to_string())
                    .or_default()
                    .push((rest.to_string(), entry.clone())),
                None => {
                    here.insert(path.clone(), entry.clone());
                }
            }
        }
        for (dir, entries) in below {
            let sha = self.tree(&entries);
            here.insert(dir, ("040000".into(), "tree".into(), sha));
        }
        let sha = self.sha();
        self.trees.insert(sha.clone(), here);
        sha
    }

    /// Every file path in a tree.
    fn files(&self, tree: &str, prefix: &str, out: &mut Vec<String>) {
        for (name, (_, kind, sha)) in self.trees.get(tree).into_iter().flatten() {
            let path = format!("{prefix}{name}");
            if kind == "tree" {
                self.files(sha, &format!("{path}/"), out);
            } else {
                out.push(path);
            }
        }
    }

    /// Every file on a branch, or none when it does not exist.
    pub(crate) fn branch_files(&self, branch: &str) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(commit) = self.refs.get(&format!("heads/{branch}")) {
            self.files(&self.commits[commit].0, "", &mut out);
        }
        out
    }

    /// The parents of a branch's tip.
    pub(crate) fn branch_parents(&self, branch: &str) -> Vec<String> {
        self.refs
            .get(&format!("heads/{branch}"))
            .map(|c| self.commits[c].1.clone())
            .unwrap_or_default()
    }

    /// Puts files (path and content) on a branch as one commit on top of it.
    pub(crate) fn put(&mut self, branch: &str, files: &[(&str, &str)]) {
        let mut entries: Vec<(String, Entry)> = Vec::new();
        let old = self.refs.get(&format!("heads/{branch}")).cloned();
        let mut paths = Vec::new();
        if let Some(c) = &old {
            let tree = self.commits[c].0.clone();
            self.files(&tree, "", &mut paths);
        }
        for path in paths {
            let sha = self.sha();
            self.blobs.insert(sha.clone(), b"old".to_vec());
            entries.push((path, ("100644".into(), "blob".into(), sha)));
        }
        for (path, content) in files {
            let sha = self.sha();
            self.blobs.insert(sha.clone(), content.as_bytes().to_vec());
            entries.push(((*path).to_string(), ("100644".into(), "blob".into(), sha)));
        }
        let tree = self.tree(&entries);
        let commit = self.sha();
        self.commits
            .insert(commit.clone(), (tree, old.into_iter().collect()));
        self.refs.insert(format!("heads/{branch}"), commit);
    }

    /// The time now, one second later than the last time asked.
    pub(crate) fn tick(&mut self) -> String {
        self.next += 1;
        let s = self.next;
        format!(
            "2026-01-01T{:02}:{:02}:{:02}Z",
            s / 3600,
            s / 60 % 60,
            s % 60
        )
    }

    pub(crate) fn add_comment(&mut self, pr: u64, login: &str, kind: &str, body: &str) -> Comment {
        let id = 1000 + self.next;
        let now = self.tick();
        let comment = Comment {
            id,
            pr,
            login: login.into(),
            kind: kind.into(),
            body: body.into(),
            created: now.clone(),
            updated: now,
        };
        self.comments.push(comment.clone());
        comment
    }

    /// Edits a comment's body, as its author does.
    pub(crate) fn edit_comment(&mut self, id: u64, body: &str) {
        let now = self.tick();
        let c = self.comments.iter_mut().find(|c| c.id == id).unwrap();
        c.body = body.into();
        c.updated = now;
    }

    /// The status comment CI writes, by `login` of `kind`, saying the pictures of `checks` wait
    /// at `head`.
    pub(crate) fn add_status_comment(
        &mut self,
        pr: u64,
        (login, kind): (&str, &str),
        head: &str,
        checks: &[&str],
    ) -> Comment {
        let waiting = json!({ "head": head, "checks": checks, "pictures": "p" });
        let body = format!(
            "<!-- fearlezz -->\n### Fearlezz\n\n**Pictures need approval**\n\n<!-- fearlezz-pictures {waiting} -->\n"
        );
        self.add_comment(pr, login, kind, &body)
    }

    /// The latest status of a commit for a context.
    pub(crate) fn status(&self, sha: &str, context: &str) -> Option<Status> {
        self.statuses
            .iter()
            .rev()
            .find(|s| s.sha == sha && s.context == context)
            .cloned()
    }

    fn comment_json(c: &Comment) -> Value {
        json!({ "id": c.id, "body": c.body, "html_url": c.url(),
                "created_at": c.created, "updated_at": c.updated,
                "issue_url": format!("https://api.github.com/repos/owner/repo/issues/{}", c.pr),
                "user": { "login": c.login, "type": c.kind } })
    }

    /// Answers one request made with `token`: status code and JSON body.
    fn answer(&mut self, token: &str, method: &str, path: &str, body: &Value) -> (u16, Value) {
        let Some(who) = self.tokens.get(token).cloned() else {
            return (401, json!({ "message": "Bad credentials" }));
        };
        if method != "GET" && !who.write {
            return (
                403,
                json!({ "message": "Resource not accessible by integration" }),
            );
        }
        let (path, query) = path.split_once('?').unwrap_or((path, ""));
        let page: usize = query
            .split('&')
            .find_map(|p| p.strip_prefix("page="))
            .and_then(|p| p.parse().ok())
            .unwrap_or(1);
        match path {
            "/graphql" => return self.graphql(body),
            "/repos/owner/repo" => {
                return (
                    200,
                    json!({ "node_id": "R_repo", "full_name": "owner/repo" }),
                );
            }
            // An installation token cannot read a user.
            "/user" if who.kind == "Bot" => {
                return (
                    403,
                    json!({ "message": "Resource not accessible by integration" }),
                );
            }
            "/user" => return (200, json!({ "login": who.login, "type": who.kind })),
            _ => {}
        }
        let Some(rest) = path.strip_prefix("/repos/owner/repo/") else {
            return (404, json!({ "message": "Not Found" }));
        };
        if self.broken.contains(rest) {
            return (500, json!({ "message": "Server Error" }));
        }
        let parts: Vec<&str> = rest.split('/').collect();
        match (method, parts.as_slice()) {
            ("GET", ["issues", pr, "comments"]) => {
                let pr: u64 = pr.parse().unwrap();
                let all: Vec<Value> = self
                    .comments
                    .iter()
                    .filter(|c| c.pr == pr)
                    .map(Self::comment_json)
                    .collect();
                let page: Vec<Value> = all.into_iter().skip((page - 1) * 100).take(100).collect();
                (200, Value::Array(page))
            }
            ("POST", ["issues", pr, "comments"]) => {
                let c = self.add_comment(
                    pr.parse().unwrap(),
                    &who.login,
                    &who.kind,
                    body["body"].as_str().unwrap(),
                );
                (201, Self::comment_json(&c))
            }
            ("GET", ["issues", "comments", id]) => {
                let id: u64 = id.parse().unwrap();
                match self.comments.iter().find(|c| c.id == id) {
                    Some(c) => (200, Self::comment_json(c)),
                    None => (404, json!({ "message": "Not Found" })),
                }
            }
            ("PATCH", ["issues", "comments", id]) => {
                let id: u64 = id.parse().unwrap();
                if !self.comments.iter().any(|c| c.id == id) {
                    return (404, json!({}));
                }
                self.edit_comment(id, body["body"].as_str().unwrap());
                let c = self.comments.iter().find(|c| c.id == id).unwrap().clone();
                (200, Self::comment_json(&c))
            }
            ("DELETE", ["issues", "comments", id]) => {
                let id: u64 = id.parse().unwrap();
                if !self.comments.iter().any(|c| c.id == id) {
                    return (404, json!({ "message": "Not Found" }));
                }
                self.comments.retain(|c| c.id != id);
                (204, Value::Null)
            }
            ("POST", ["statuses", sha]) => {
                let text = |k: &str| body[k].as_str().unwrap_or_default().to_string();
                assert!(
                    text("description").chars().count() <= 140,
                    "GitHub refuses a status description over 140 characters"
                );
                self.statuses.push(Status {
                    sha: (*sha).to_string(),
                    state: text("state"),
                    context: text("context"),
                    description: text("description"),
                    target: text("target_url"),
                    creator: who.login.clone(),
                    creator_kind: who.kind.clone(),
                });
                if let Some(id) = self.delete_on_status.take() {
                    self.comments.retain(|c| c.id != id);
                }
                (201, json!({}))
            }
            ("GET", ["commits", sha, "statuses"]) => {
                let all: Vec<Value> = self
                    .statuses
                    .iter()
                    .rev()
                    .filter(|s| s.sha == *sha)
                    .map(|s| {
                        let target = if s.target.is_empty() {
                            Value::Null
                        } else {
                            s.target.clone().into()
                        };
                        json!({ "state": s.state, "context": s.context,
                                "description": s.description, "target_url": target,
                                "creator": { "login": s.creator, "type": s.creator_kind } })
                    })
                    .collect();
                let page: Vec<Value> = all.into_iter().skip((page - 1) * 100).take(100).collect();
                (200, Value::Array(page))
            }
            ("GET", ["commits", sha, "pulls"]) => {
                let found: Vec<Value> = self
                    .pulls
                    .iter()
                    .filter(|(_, (head, _))| head == sha)
                    .map(|(n, (head, state))| {
                        json!({ "number": n, "state": state, "head": { "sha": head } })
                    })
                    .collect();
                (200, Value::Array(found))
            }
            ("GET", ["collaborators", login, "permission"]) => {
                let perm = self
                    .permissions
                    .get(*login)
                    .cloned()
                    .unwrap_or_else(|| "none".into());
                (200, json!({ "permission": perm }))
            }
            ("GET", ["pulls", pr]) => {
                let n = pr.parse::<u64>().unwrap();
                let from = if self.forks.contains(&n) {
                    "someone/repo"
                } else {
                    "owner/repo"
                };
                match self.pulls.get(&n) {
                    Some((head, state)) => (
                        200,
                        json!({ "head": { "sha": head, "repo": { "full_name": from } },
                                "state": state }),
                    ),
                    None => (404, json!({ "message": "Not Found" })),
                }
            }
            ("GET", ["git", "ref", "heads", branch]) => {
                match self.refs.get(&format!("heads/{branch}")) {
                    Some(sha) => (200, json!({ "object": { "sha": sha } })),
                    None => (404, json!({ "message": "Not Found" })),
                }
            }
            ("GET", ["git", "commits", sha]) => match self.commits.get(*sha) {
                Some((tree, _)) => (200, json!({ "sha": sha, "tree": { "sha": tree } })),
                None => (404, json!({})),
            },
            ("GET", ["git", "trees", sha]) => match self.trees.get(*sha) {
                Some(entries) => {
                    let tree: Vec<Value> = entries
                        .iter()
                        .map(|(path, (mode, kind, sha))| {
                            json!({ "path": path, "mode": mode, "type": kind, "sha": sha })
                        })
                        .collect();
                    (200, json!({ "sha": sha, "tree": tree }))
                }
                None => (404, json!({})),
            },
            ("POST", ["git", "blobs"]) => {
                assert_eq!(body["encoding"], "base64");
                let sha = self.sha();
                let content = body["content"].as_str().unwrap().as_bytes().to_vec();
                self.blobs.insert(sha.clone(), content);
                (201, json!({ "sha": sha }))
            }
            ("POST", ["git", "trees"]) => {
                assert!(body.get("base_tree").is_none(), "trees are built whole");
                let entries: Vec<(String, Entry)> = body["tree"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|e| {
                        let s = |k: &str| e[k].as_str().unwrap().to_string();
                        (s("path"), (s("mode"), s("type"), s("sha")))
                    })
                    .collect();
                if entries.is_empty() {
                    return (422, json!({ "message": "tree.tree is empty" }));
                }
                (201, json!({ "sha": self.tree(&entries) }))
            }
            ("POST", ["git", "commits"]) => {
                let tree = body["tree"].as_str().unwrap().to_string();
                let parents: Vec<String> = body["parents"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| p.as_str().unwrap().to_string())
                    .collect();
                let sha = self.sha();
                self.commits.insert(sha.clone(), (tree, parents));
                (201, json!({ "sha": sha }))
            }
            ("POST", ["git", "refs"]) => {
                let name = body["ref"]
                    .as_str()
                    .unwrap()
                    .trim_start_matches("refs/")
                    .to_string();
                if self.refs.contains_key(&name) {
                    return (422, json!({ "message": "Reference already exists" }));
                }
                self.refs
                    .insert(name, body["sha"].as_str().unwrap().to_string());
                (201, json!({}))
            }
            ("PATCH", ["git", "refs", "heads", branch]) => {
                let name = format!("heads/{branch}");
                let new = body["sha"].as_str().unwrap().to_string();
                let force = body["force"].as_bool().unwrap_or(false);
                assert!(!force, "a forced ref update can lose another run's push");
                let Some(old) = self.refs.get(&name).cloned() else {
                    return (422, json!({ "message": "Reference does not exist" }));
                };
                if !self.commits[&new].1.contains(&old) {
                    return (422, json!({ "message": "Update is not a fast forward" }));
                }
                self.refs.insert(name, new);
                (200, json!({}))
            }
            _ => (
                404,
                json!({ "message": format!("no fake for {method} {path}") }),
            ),
        }
    }

    /// GraphQL's `updateRefs`: every update applies only while its ref is where `beforeOid`
    /// says, all of them or none.
    fn graphql(&mut self, body: &Value) -> (u16, Value) {
        let query = body["query"].as_str().unwrap_or_default();
        assert!(
            query.contains("updateRefs"),
            "no fake for the query {query}"
        );
        let input = &body["variables"]["input"];
        assert_eq!(input["repositoryId"], "R_repo");
        if let Some(files) = self.race.take() {
            let files: Vec<(&str, &str)> = files
                .iter()
                .map(|(p, c)| (p.as_str(), c.as_str()))
                .collect();
            self.put("fearlezz-pictures", &files);
        }
        let zero = "0".repeat(40);
        let name = |u: &Value| {
            u["name"]
                .as_str()
                .unwrap()
                .trim_start_matches("refs/")
                .to_string()
        };
        let updates = input["refUpdates"].as_array().unwrap();
        for u in updates {
            let before = u["beforeOid"].as_str().unwrap_or_default();
            let now = self
                .refs
                .get(&name(u))
                .cloned()
                .unwrap_or_else(|| zero.clone());
            if before != now {
                return (
                    200,
                    json!({ "data": { "updateRefs": null },
                            "errors": [{ "message": format!("{} is at {now}, not {before}", name(u)) }] }),
                );
            }
        }
        for u in updates {
            let after = u["afterOid"].as_str().unwrap().to_string();
            if after == zero {
                self.refs.remove(&name(u));
            } else {
                self.refs.insert(name(u), after);
            }
        }
        (
            200,
            json!({ "data": { "updateRefs": { "clientMutationId": null } } }),
        )
    }
}

/// The fake server, running until the test ends.
pub(crate) struct FakeGitHub {
    pub url: String,
    pub state: Arc<Mutex<State>>,
}

impl FakeGitHub {
    pub(crate) fn start() -> FakeGitHub {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(State::default()));
        let shared = Arc::clone(&state);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                serve(stream, &shared);
            }
        });
        FakeGitHub { url, state }
    }

    pub(crate) fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }
}

/// Reads one HTTP request and writes the answer.
fn serve(stream: TcpStream, state: &Mutex<State>) {
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let mut words = line.split_whitespace();
    let method = words.next().unwrap_or_default().to_string();
    let path = words.next().unwrap_or_default().to_string();
    let mut length = 0;
    let mut token = String::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
            break;
        }
        let lower = header.to_ascii_lowercase();
        if let Some(n) = lower.strip_prefix("content-length:") {
            length = n.trim().parse().unwrap_or(0);
        }
        if lower.starts_with("authorization: bearer ") {
            token = header["authorization: bearer ".len()..].trim().to_string();
        }
    }
    let mut body = vec![0; length];
    if reader.read_exact(&mut body).is_err() {
        return;
    }
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let (code, answer) = if token.is_empty() {
        (401, json!({ "message": "Requires authentication" }))
    } else {
        state.lock().unwrap().answer(&token, &method, &path, &body)
    };
    let text = if answer.is_null() {
        String::new()
    } else {
        answer.to_string()
    };
    let mut stream = &stream;
    if let Err(e) = write!(
        stream,
        "HTTP/1.1 {code} Fake\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    ) {
        eprintln!("the fake GitHub could not answer: {e}");
    }
}
