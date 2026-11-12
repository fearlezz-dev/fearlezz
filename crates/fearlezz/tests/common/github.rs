//! A fake GitHub REST API on a local port: the parts `fearlezz run --ci` uses on GitHub Actions
//! (comments, statuses, pull requests, collaborators, and the git data API for the pictures
//! branch), kept in memory so a test can set it up and read back what Fearlezz did.

#![allow(dead_code, reason = "each test file uses only some of the helpers")]

use std::collections::BTreeMap;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

/// A comment on a pull request.
#[derive(Clone, Debug)]
pub(crate) struct Comment {
    pub id: u64,
    pub pr: u64,
    pub login: String,
    pub kind: String,
    pub body: String,
}

impl Comment {
    pub(crate) fn url(&self) -> String {
        format!(
            "https://github.com/owner/repo/pull/{}#issuecomment-{}",
            self.pr, self.id
        )
    }
}

/// A commit status.
#[derive(Clone, Debug)]
pub(crate) struct Status {
    pub sha: String,
    pub state: String,
    pub context: String,
    pub description: String,
    pub target: String,
}

/// An entry of a tree: mode, type, sha.
type Entry = (String, String, String);

#[derive(Default)]
pub(crate) struct State {
    pub comments: Vec<Comment>,
    pub statuses: Vec<Status>,
    /// Logins and their permission (admin, write, read).
    pub permissions: BTreeMap<String, String>,
    /// Pull requests: head sha and state.
    pub pulls: BTreeMap<u64, (String, String)>,
    pub refs: BTreeMap<String, String>,
    blobs: BTreeMap<String, Vec<u8>>,
    trees: BTreeMap<String, BTreeMap<String, Entry>>,
    /// Commits: tree and parents.
    pub commits: BTreeMap<String, (String, Vec<String>)>,
    next: u64,
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

    pub(crate) fn add_comment(&mut self, pr: u64, login: &str, kind: &str, body: &str) -> Comment {
        let id = 1000 + self.comments.len() as u64;
        let comment = Comment {
            id,
            pr,
            login: login.into(),
            kind: kind.into(),
            body: body.into(),
        };
        self.comments.push(comment.clone());
        comment
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
                "user": { "login": c.login, "type": c.kind } })
    }

    /// Answers one request: status code and JSON body.
    fn answer(&mut self, method: &str, path: &str, body: &Value) -> (u16, Value) {
        let (path, query) = path.split_once('?').unwrap_or((path, ""));
        let page: usize = query
            .split('&')
            .find_map(|p| p.strip_prefix("page="))
            .and_then(|p| p.parse().ok())
            .unwrap_or(1);
        let Some(rest) = path.strip_prefix("/repos/owner/repo/") else {
            return match path {
                // The workflow token cannot read a user.
                "/user" => (
                    403,
                    json!({ "message": "Resource not accessible by integration" }),
                ),
                _ => (404, json!({ "message": "Not Found" })),
            };
        };
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
                    "github-actions[bot]",
                    "Bot",
                    body["body"].as_str().unwrap(),
                );
                (201, Self::comment_json(&c))
            }
            ("PATCH", ["issues", "comments", id]) => {
                let id: u64 = id.parse().unwrap();
                let Some(c) = self.comments.iter_mut().find(|c| c.id == id) else {
                    return (404, json!({}));
                };
                c.body = body["body"].as_str().unwrap().to_string();
                let c = c.clone();
                (200, Self::comment_json(&c))
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
                });
                (201, json!({}))
            }
            ("GET", ["collaborators", login, "permission"]) => {
                let perm = self
                    .permissions
                    .get(*login)
                    .cloned()
                    .unwrap_or_else(|| "none".into());
                (200, json!({ "permission": perm }))
            }
            ("GET", ["pulls", pr]) => match self.pulls.get(&pr.parse::<u64>().unwrap()) {
                Some((head, state)) => (200, json!({ "head": { "sha": head }, "state": state })),
                None => (404, json!({ "message": "Not Found" })),
            },
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
                let Some(old) = self.refs.get(&name).cloned() else {
                    return (422, json!({ "message": "Reference does not exist" }));
                };
                if !force && !self.commits[&new].1.contains(&old) {
                    return (422, json!({ "message": "Update is not a fast forward" }));
                }
                self.refs.insert(name, new);
                (200, json!({}))
            }
            ("DELETE", ["git", "refs", "heads", branch]) => {
                self.refs.remove(&format!("heads/{branch}"));
                (204, Value::Null)
            }
            _ => (
                404,
                json!({ "message": format!("no fake for {method} {path}") }),
            ),
        }
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
    let mut authorized = false;
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
            authorized = true;
        }
    }
    let mut body = vec![0; length];
    if reader.read_exact(&mut body).is_err() {
        return;
    }
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let (code, answer) = if authorized {
        state.lock().unwrap().answer(&method, &path, &body)
    } else {
        (401, json!({ "message": "Requires authentication" }))
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
