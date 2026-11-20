//! A fake GitHub on a local port: the parts the proof host asks (the Actions OIDC keys, the app's
//! installation on a repository and its installation tokens, a repository as a token sees it, a
//! person's permission on it, the OAuth code exchange and the signed in user), kept in memory,
//! and what the app writes (check runs and pull request comments), recorded with the token that
//! wrote them, and the Actions jobs of workflow runs with the re-runs the app asks for. It checks the app's JWT with the app's public key, so a test proves the host signs
//! it as GitHub expects. It also sends webhooks, signed as GitHub signs them.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rsa::RsaPublicKey;
use rsa::pkcs1v15::{Signature, VerifyingKey};
use rsa::signature::Verifier as _;
use rsa::traits::PublicKeyParts as _;
use serde_json::{Value, json};
use sha2::Sha256;

/// The GitHub App's client id, which also names it in its JWTs.
pub const CLIENT_ID: &str = "Iv1.fearlezz";
pub const CLIENT_SECRET: &str = "client-secret";
/// The key id of the Actions OIDC signing key.
pub const KID: &str = "actions-key";

pub struct Repo {
    pub private: bool,
    /// The app's installation on it, if any.
    pub installation: Option<u64>,
    /// Logins with at least read access.
    pub readers: BTreeSet<String>,
    /// Logins with write access.
    pub writers: BTreeSet<String>,
    /// Logins with admin rights.
    pub admins: BTreeSet<String>,
}

/// A check run as the app wrote it, and every write to it in order.
#[derive(Clone, Debug)]
pub struct CheckRun {
    pub id: u64,
    pub repo: String,
    pub head_sha: String,
    /// Each body the app sent for it, the create first.
    pub writes: Vec<Value>,
    /// The token of each write.
    pub tokens: Vec<String>,
}

impl CheckRun {
    /// A field as the latest write that set it left it.
    pub fn field(&self, name: &str) -> Value {
        self.writes
            .iter()
            .rev()
            .find_map(|w| w.get(name).cloned())
            .unwrap_or(Value::Null)
    }

    /// `queued`, `in_progress`, or the conclusion once completed.
    pub fn state(&self) -> String {
        let latest = self.writes.last().unwrap();
        match latest.get("conclusion").and_then(Value::as_str) {
            Some(conclusion) => conclusion.to_owned(),
            None => latest["status"].as_str().unwrap_or_default().to_owned(),
        }
    }
}

/// A comment as the app wrote it on a pull request, and every body it had.
#[derive(Clone, Debug)]
pub struct Comment {
    pub id: u64,
    pub repo: String,
    pub issue: u64,
    pub bodies: Vec<String>,
    pub tokens: Vec<String>,
}

pub struct State {
    pub repos: BTreeMap<String, Repo>,
    /// OAuth codes and the login each signs in.
    pub codes: BTreeMap<String, String>,
    /// Person tokens and their login.
    pub tokens: BTreeMap<String, String>,
    /// The Actions OIDC keys GitHub publishes, by key id; a test can rotate one in.
    pub oidc_keys: Vec<(String, RsaPublicKey)>,
    /// Every request asked, as method and path, so a test can count what the host asks.
    pub asked: Vec<(String, String)>,
    pub check_runs: Vec<CheckRun>,
    pub comments: Vec<Comment>,
    /// The id the next check run or comment gets.
    pub next_id: u64,
    /// Device flow sign ins by device code: the code a person enters, and who entered it.
    pub devices: BTreeMap<String, (String, Option<String>)>,
    /// Refresh tokens of user tokens and their login; each renews once.
    pub refreshes: BTreeMap<String, String>,
    /// The Actions jobs of workflow runs, as GitHub lists them.
    pub jobs: Vec<ActionsJob>,
    /// Every job the app asked to run again: its repository, id and the token that asked.
    pub reruns: Vec<(String, u64, String)>,
    /// Every workflow run whose failed jobs the app asked to run again: its repository, id and
    /// the token that asked.
    pub failed_reruns: Vec<(String, u64, String)>,
    /// The repositories whose check runs GitHub cannot write, and how it fails them.
    pub stalls: BTreeMap<String, Stall>,
}

/// How GitHub fails to write a repository's check runs.
#[derive(Clone, Copy, Debug)]
pub enum Stall {
    /// It answers nothing for half a minute, then drops the connection.
    Hang,
    /// It answers `500` at once.
    Fail,
}

/// An Actions job of a workflow run.
#[derive(Clone, Debug)]
pub struct ActionsJob {
    pub repo: String,
    pub run: u64,
    pub id: u64,
    pub name: String,
    /// The runner that took it.
    pub runner: String,
    pub conclusion: String,
}

pub struct FakeGitHub {
    pub url: String,
    state: Arc<Mutex<State>>,
}

struct Keys {
    app: RsaPublicKey,
}

impl FakeGitHub {
    pub fn start(oidc: RsaPublicKey, app: RsaPublicKey) -> FakeGitHub {
        let set = |logins: &[&str]| logins.iter().map(|l| (*l).to_owned()).collect();
        let repo = |private: bool, installation: Option<u64>, readers: &[&str]| Repo {
            private,
            installation,
            readers: set(readers),
            writers: set(&[]),
            admins: set(&[]),
        };
        // Who reviews: a writer gives verdicts, an admin accepts weakenings too.
        let reviewed = |private: bool, installation: u64| Repo {
            private,
            installation: Some(installation),
            readers: set(&["reader", "writer", "admin"]),
            writers: set(&["writer", "admin"]),
            admins: set(&["admin"]),
        };
        let state = State {
            repos: BTreeMap::from([
                ("owner/repo".to_owned(), repo(false, Some(1), &["reader"])),
                ("owner/secret".to_owned(), repo(true, Some(2), &["reader"])),
                ("owner/bare".to_owned(), repo(false, None, &[])),
                ("other/repo".to_owned(), repo(false, Some(3), &[])),
                // Only the caching tests view these, so they can count what the host asks.
                ("owner/cached".to_owned(), repo(true, Some(4), &["reader"])),
                ("owner/open".to_owned(), repo(false, Some(5), &[])),
                // The review tests: verdicts, accepts, the check run and the comment.
                ("owner/review".to_owned(), reviewed(false, 6)),
                // Private, so a share link is the only way in without a sign in.
                ("owner/shared".to_owned(), reviewed(true, 8)),
                // The repository page, private so it needs a reader.
                ("owner/listing".to_owned(), reviewed(true, 9)),
                // A private repository whose live pages end with their access: each person
                // loses it in their own way.
                (
                    "owner/watched".to_owned(),
                    repo(true, Some(10), &["watcher", "leaver"]),
                ),
                // Who may do what, one viewer kind at a time; each `lapsed-*` loses their token.
                (
                    "owner/policy".to_owned(),
                    Repo {
                        private: true,
                        installation: Some(12),
                        readers: set(&[
                            "reader",
                            "writer",
                            "admin",
                            "lapsed-read",
                            "lapsed-say",
                            "lapsed-notes",
                        ]),
                        writers: set(&["writer", "admin"]),
                        admins: set(&["admin"]),
                    },
                ),
                // Its check runs fail or hang when a test says so.
                ("owner/stalled".to_owned(), reviewed(false, 11)),
                // README pictures: public with an admin who accepts weakenings, and private so
                // they are never served.
                ("owner/readme".to_owned(), reviewed(false, 13)),
                ("owner/unseen".to_owned(), repo(true, Some(14), &[])),
                // The local run tests: each CLI test signs in as a login of its own.
                (
                    "owner/local".to_owned(),
                    Repo {
                        private: false,
                        installation: Some(7),
                        readers: set(&["reader", "writer", "dev", "refresher"]),
                        writers: set(&["writer", "dev", "refresher"]),
                        admins: set(&[]),
                    },
                ),
            ]),
            codes: BTreeMap::from([
                ("reader-code".to_owned(), "reader".to_owned()),
                ("stranger-code".to_owned(), "stranger".to_owned()),
                ("writer-code".to_owned(), "writer".to_owned()),
                ("admin-code".to_owned(), "admin".to_owned()),
                ("watcher-code".to_owned(), "watcher".to_owned()),
                ("leaver-code".to_owned(), "leaver".to_owned()),
                ("lapsed-read-code".to_owned(), "lapsed-read".to_owned()),
                ("lapsed-say-code".to_owned(), "lapsed-say".to_owned()),
                ("lapsed-notes-code".to_owned(), "lapsed-notes".to_owned()),
            ]),
            tokens: BTreeMap::new(),
            oidc_keys: vec![(KID.to_owned(), oidc)],
            asked: Vec::new(),
            check_runs: Vec::new(),
            comments: Vec::new(),
            next_id: 1000,
            devices: BTreeMap::new(),
            refreshes: BTreeMap::new(),
            jobs: Vec::new(),
            reruns: Vec::new(),
            failed_reruns: Vec::new(),
            stalls: BTreeMap::new(),
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(state));
        let keys = Arc::new(Keys { app });
        let shared = Arc::clone(&state);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let state = Arc::clone(&shared);
                let keys = Arc::clone(&keys);
                std::thread::spawn(move || serve(stream, &state, &keys));
            }
        });
        FakeGitHub { url, state }
    }

    /// How many times the host asked for this method and path.
    pub fn asked(&self, method: &str, path: &str) -> usize {
        let state = self.state.lock().unwrap();
        state
            .asked
            .iter()
            .filter(|(m, p)| m == method && p == path)
            .count()
    }

    /// The check runs the app wrote on a repository for a commit, oldest first.
    pub fn check_runs(&self, repo: &str, head_sha: &str) -> Vec<CheckRun> {
        let state = self.state.lock().unwrap();
        state
            .check_runs
            .iter()
            .filter(|c| c.repo == repo && c.head_sha == head_sha)
            .cloned()
            .collect()
    }

    /// The comments the app wrote on a pull request, oldest first.
    pub fn comments(&self, repo: &str, issue: u64) -> Vec<Comment> {
        let state = self.state.lock().unwrap();
        state
            .comments
            .iter()
            .filter(|c| c.repo == repo && c.issue == issue)
            .cloned()
            .collect()
    }

    /// A person enters `user_code` on GitHub's device page and approves the app, signed in as
    /// `login`.
    pub fn approve(&self, user_code: &str, login: &str) {
        let mut state = self.state.lock().unwrap();
        let device = state
            .devices
            .values_mut()
            .find(|(code, _)| code == user_code)
            .unwrap_or_else(|| panic!("no device flow shows {user_code}"));
        device.1 = Some(login.to_owned());
    }

    /// Every user token `login` got through the device flow or a refresh expires, as GitHub's
    /// do after eight hours; their refresh tokens still renew them.
    pub fn expire(&self, login: &str) {
        let mut state = self.state.lock().unwrap();
        let prefix = format!("person-{login}-");
        state.tokens.retain(|token, _| !token.starts_with(&prefix));
    }

    /// Job `id` named `name` of the first attempt of workflow run `run`, which `runner` took,
    /// ended with `conclusion`.
    pub fn job_ended(
        &self,
        repo: &str,
        run: u64,
        id: u64,
        name: &str,
        runner: &str,
        conclusion: &str,
    ) {
        let mut state = self.state.lock().unwrap();
        state.jobs.push(ActionsJob {
            repo: repo.to_owned(),
            run,
            id,
            name: name.to_owned(),
            runner: runner.to_owned(),
            conclusion: conclusion.to_owned(),
        });
    }

    /// GitHub no longer takes the person token `token`, as when its person revokes it.
    pub(crate) fn forget(&self, token: &str) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.tokens.remove(token);
    }

    /// `login` may no longer read `repo`.
    pub fn stop_reading(&self, repo: &str, login: &str) {
        let mut state = self.state.lock().unwrap();
        state.repos.get_mut(repo).unwrap().readers.remove(login);
    }

    /// The jobs of workflow run `run` of `repo` the app asked to run again, in order, each with
    /// the token that asked.
    pub fn reruns(&self, repo: &str, run: u64) -> Vec<(u64, String)> {
        let state = self.state.lock().unwrap();
        state
            .reruns
            .iter()
            .filter(|(r, id, _)| {
                r == repo && state.jobs.iter().any(|j| j.id == *id && j.run == run)
            })
            .map(|(_, id, token)| (*id, token.clone()))
            .collect()
    }

    /// The workflow runs of `repo` whose failed jobs the app asked to run again, in order, each
    /// with the token that asked.
    pub(crate) fn failed_reruns(&self, repo: &str) -> Vec<(u64, String)> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .failed_reruns
            .iter()
            .filter(|(r, _, _)| r == repo)
            .map(|(_, run, token)| (*run, token.clone()))
            .collect()
    }

    /// GitHub fails every write to `repo`'s check runs as `stall` says, or writes them again.
    pub fn stall(&self, repo: &str, stall: Option<Stall>) {
        let mut state = self.state.lock().unwrap();
        match stall {
            Some(stall) => state.stalls.insert(repo.to_owned(), stall),
            None => state.stalls.remove(repo),
        };
    }

    /// Publishes another Actions OIDC key, as GitHub does when it rotates them.
    pub fn publish(&self, kid: &str, key: RsaPublicKey) {
        let mut state = self.state.lock().unwrap();
        state.oidc_keys.push((kid.to_owned(), key));
    }
}

fn serve(stream: TcpStream, state: &Mutex<State>, keys: &Keys) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let mut length = 0;
    let mut auth = String::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header).unwrap();
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':').unwrap();
        match name.to_ascii_lowercase().as_str() {
            "content-length" => length = value.trim().parse().unwrap(),
            "authorization" => value.trim().clone_into(&mut auth),
            _ => {}
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let stall = match path
        .trim_matches('/')
        .split('/')
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["repos", owner, name, "check-runs", ..] if method != "GET" => {
            let stalls = &state.lock().unwrap().stalls;
            stalls.get(&format!("{owner}/{name}")).copied()
        }
        _ => None,
    };
    match stall {
        Some(Stall::Hang) => {
            std::thread::sleep(std::time::Duration::from_secs(30));
            return;
        }
        Some(Stall::Fail) => {
            let mut stream = stream;
            let answer = r#"{"message":"Server Error"}"#;
            let _ = write!(
                stream,
                "HTTP/1.1 500 X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                answer.len()
            );
            return;
        }
        None => {}
    }
    let token = auth
        .strip_prefix("Bearer ")
        .or_else(|| auth.strip_prefix("token "))
        .unwrap_or_default();
    let (status, answer) = answer(
        &mut state.lock().unwrap(),
        keys,
        &method,
        &path,
        token,
        &body,
    );
    let answer = answer.to_string();
    let mut stream = stream;
    let _ = write!(
        stream,
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
        answer.len()
    );
}

fn answer(
    state: &mut State,
    keys: &Keys,
    method: &str,
    path: &str,
    token: &str,
    body: &Value,
) -> (u16, Value) {
    let path = path.split('?').next().unwrap_or_default();
    state.asked.push((method.to_owned(), path.to_owned()));
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    match (method, segments.as_slice()) {
        ("GET", [".well-known", "jwks"]) => {
            let keys: Vec<Value> = state
                .oidc_keys
                .iter()
                .map(|(kid, key)| jwk(kid, key))
                .collect();
            (200, json!({ "keys": keys }))
        }
        ("GET", ["repos", owner, name, "installation"]) => {
            if !app_jwt_valid(token, &keys.app) {
                return (
                    401,
                    json!({"message": "A JSON web token could not be decoded"}),
                );
            }
            match state
                .repos
                .get(&format!("{owner}/{name}"))
                .and_then(|r| r.installation)
            {
                Some(id) => (200, json!({"id": id})),
                None => (404, json!({"message": "Not Found"})),
            }
        }
        ("POST", ["app", "installations", id, "access_tokens"]) => {
            if !app_jwt_valid(token, &keys.app) {
                return (
                    401,
                    json!({"message": "A JSON web token could not be decoded"}),
                );
            }
            (201, json!({"token": format!("installation-{id}")}))
        }
        ("GET", ["repos", owner, name]) => {
            let full = format!("{owner}/{name}");
            let Some(repo) = state.repos.get(&full) else {
                return (404, json!({"message": "Not Found"}));
            };
            let installation = token
                .strip_prefix("installation-")
                .and_then(|id| id.parse::<u64>().ok());
            let person = state.tokens.get(token);
            // A token GitHub does not know, as a revoked or expired one, is bad credentials.
            if installation.is_none() && person.is_none() && !token.is_empty() {
                return (401, json!({"message": "Bad credentials"}));
            }
            let reads = match (installation, person) {
                (Some(id), _) => repo.installation == Some(id),
                (None, Some(login)) => repo.readers.contains(login),
                (None, None) => false,
            };
            if repo.private && !reads {
                return (404, json!({"message": "Not Found"}));
            }
            (
                200,
                json!({
                    "full_name": full,
                    "private": repo.private,
                    "default_branch": "main",
                    "permissions": {"admin": false, "push": false, "pull": reads},
                }),
            )
        }
        ("GET", ["repos", owner, name, "collaborators", login, "permission"]) => {
            let full = format!("{owner}/{name}");
            if !state.tokens.contains_key(token) {
                return (401, json!({"message": "Bad credentials"}));
            }
            let Some(repo) = state.repos.get(&full) else {
                return (404, json!({"message": "Not Found"}));
            };
            let permission = if repo.admins.contains(*login) {
                "admin"
            } else if repo.writers.contains(*login) {
                "write"
            } else if repo.readers.contains(*login) {
                "read"
            } else {
                "none"
            };
            (200, json!({"permission": permission}))
        }
        ("POST", ["repos", owner, name, "check-runs"]) => {
            let repo = format!("{owner}/{name}");
            if !writes_as_app(state, &repo, token) {
                return (
                    403,
                    json!({"message": "Resource not accessible by integration"}),
                );
            }
            state.next_id += 1;
            let id = state.next_id;
            state.check_runs.push(CheckRun {
                id,
                repo,
                head_sha: body["head_sha"].as_str().unwrap_or_default().to_owned(),
                writes: vec![body.clone()],
                tokens: vec![token.to_owned()],
            });
            (201, json!({"id": id}))
        }
        ("PATCH", ["repos", owner, name, "check-runs", id]) => {
            let repo = format!("{owner}/{name}");
            if !writes_as_app(state, &repo, token) {
                return (
                    403,
                    json!({"message": "Resource not accessible by integration"}),
                );
            }
            let id: u64 = id.parse().unwrap_or_default();
            match state
                .check_runs
                .iter_mut()
                .find(|c| c.id == id && c.repo == repo)
            {
                Some(run) => {
                    run.writes.push(body.clone());
                    run.tokens.push(token.to_owned());
                    (200, json!({"id": id}))
                }
                None => (404, json!({"message": "Not Found"})),
            }
        }
        (
            "GET",
            [
                "repos",
                owner,
                name,
                "actions",
                "runs",
                run,
                "attempts",
                "1",
                "jobs",
            ],
        ) => {
            let repo = format!("{owner}/{name}");
            if !writes_as_app(state, &repo, token) {
                return (
                    403,
                    json!({"message": "Resource not accessible by integration"}),
                );
            }
            let run: u64 = run.parse().unwrap_or_default();
            let jobs: Vec<Value> = state
                .jobs
                .iter()
                .filter(|j| j.repo == repo && j.run == run)
                .map(|j| json!({"id": j.id, "run_id": j.run, "run_attempt": 1, "name": j.name, "runner_name": j.runner, "status": "completed", "conclusion": j.conclusion}))
                .collect();
            (200, json!({"total_count": jobs.len(), "jobs": jobs}))
        }
        ("POST", ["repos", owner, name, "actions", "jobs", id, "rerun"]) => {
            let repo = format!("{owner}/{name}");
            if !writes_as_app(state, &repo, token) {
                return (
                    403,
                    json!({"message": "Resource not accessible by integration"}),
                );
            }
            let id: u64 = id.parse().unwrap_or_default();
            if !state.jobs.iter().any(|j| j.repo == repo && j.id == id) {
                return (404, json!({"message": "Not Found"}));
            }
            state.reruns.push((repo, id, token.to_owned()));
            (201, json!({}))
        }
        (
            "POST",
            [
                "repos",
                owner,
                name,
                "actions",
                "runs",
                run,
                "rerun-failed-jobs",
            ],
        ) => {
            let repo = format!("{owner}/{name}");
            if !writes_as_app(state, &repo, token) {
                return (
                    403,
                    json!({"message": "Resource not accessible by integration"}),
                );
            }
            let run: u64 = run.parse().unwrap_or_default();
            state.failed_reruns.push((repo, run, token.to_owned()));
            (201, json!({}))
        }
        ("POST", ["repos", owner, name, "issues", issue, "comments"]) => {
            let repo = format!("{owner}/{name}");
            if !writes_as_app(state, &repo, token) {
                return (
                    403,
                    json!({"message": "Resource not accessible by integration"}),
                );
            }
            state.next_id += 1;
            let id = state.next_id;
            state.comments.push(Comment {
                id,
                repo,
                issue: issue.parse().unwrap_or_default(),
                bodies: vec![body["body"].as_str().unwrap_or_default().to_owned()],
                tokens: vec![token.to_owned()],
            });
            (
                201,
                json!({"id": id, "html_url": format!("https://github.test/c/{id}")}),
            )
        }
        ("PATCH", ["repos", owner, name, "issues", "comments", id]) => {
            let repo = format!("{owner}/{name}");
            if !writes_as_app(state, &repo, token) {
                return (
                    403,
                    json!({"message": "Resource not accessible by integration"}),
                );
            }
            let id: u64 = id.parse().unwrap_or_default();
            match state
                .comments
                .iter_mut()
                .find(|c| c.id == id && c.repo == repo)
            {
                Some(comment) => {
                    comment
                        .bodies
                        .push(body["body"].as_str().unwrap_or_default().to_owned());
                    comment.tokens.push(token.to_owned());
                    (200, json!({"id": id}))
                }
                None => (404, json!({"message": "Not Found"})),
            }
        }
        ("POST", ["login", "device", "code"]) => {
            if body["client_id"] != CLIENT_ID {
                return (404, json!({"error": "Not Found"}));
            }
            state.next_id += 1;
            let device = format!("device-{}", state.next_id);
            let user = format!("FZ{:02}-{:04}", state.next_id % 100, state.next_id);
            state.devices.insert(device.clone(), (user.clone(), None));
            (
                200,
                json!({
                    "device_code": device, "user_code": user,
                    "verification_uri": "https://github.com/login/device",
                    "expires_in": 900, "interval": 5,
                }),
            )
        }
        ("POST", ["login", "oauth", "access_token"])
            if body["grant_type"] == "urn:ietf:params:oauth:grant-type:device_code" =>
        {
            if body["client_id"] != CLIENT_ID {
                return (200, json!({"error": "incorrect_client_credentials"}));
            }
            let code = body["device_code"].as_str().unwrap_or_default();
            match state.devices.get(code).cloned() {
                None => (200, json!({"error": "expired_token"})),
                Some((_, None)) => (200, json!({"error": "authorization_pending"})),
                Some((_, Some(login))) => {
                    state.devices.remove(code);
                    (200, user_token(state, &login))
                }
            }
        }
        ("POST", ["login", "oauth", "access_token"]) if body["grant_type"] == "refresh_token" => {
            let field = |name: &str| body.get(name).and_then(Value::as_str).unwrap_or_default();
            if field("client_id") != CLIENT_ID || field("client_secret") != CLIENT_SECRET {
                return (200, json!({"error": "incorrect_client_credentials"}));
            }
            let refresh = field("refresh_token").to_owned();
            match state.refreshes.remove(&refresh) {
                Some(login) => (200, user_token(state, &login)),
                None => (200, json!({"error": "bad_refresh_token"})),
            }
        }
        ("POST", ["login", "oauth", "access_token"]) => {
            let field = |name: &str| body.get(name).and_then(Value::as_str).unwrap_or_default();
            if field("client_id") != CLIENT_ID || field("client_secret") != CLIENT_SECRET {
                return (200, json!({"error": "incorrect_client_credentials"}));
            }
            let Some(login) = state.codes.get(field("code")).cloned() else {
                return (200, json!({"error": "bad_verification_code"}));
            };
            let token = format!("person-{login}");
            state.tokens.insert(token.clone(), login);
            (200, json!({"access_token": token, "token_type": "bearer"}))
        }
        ("GET", ["user"]) => match state.tokens.get(token) {
            Some(login) => (200, json!({"login": login})),
            None => (401, json!({"message": "Bad credentials"})),
        },
        _ => (404, json!({"message": "Not Found"})),
    }
}

/// Whether `token` is the app's installation token on `repo`: only the app writes check runs
/// and its own comments.
/// A new expiring user token for `login` with its refresh token, as GitHub answers it.
fn user_token(state: &mut State, login: &str) -> Value {
    state.next_id += 1;
    let token = format!("person-{login}-{}", state.next_id);
    let refresh = format!("refresh-{}", state.next_id);
    state.tokens.insert(token.clone(), login.to_owned());
    state.refreshes.insert(refresh.clone(), login.to_owned());
    json!({
        "access_token": token, "token_type": "bearer", "expires_in": 28800,
        "refresh_token": refresh, "refresh_token_expires_in": 15_897_600,
    })
}

fn writes_as_app(state: &State, repo: &str, token: &str) -> bool {
    let installation = token
        .strip_prefix("installation-")
        .and_then(|id| id.parse::<u64>().ok());
    installation.is_some() && state.repos.get(repo).and_then(|r| r.installation) == installation
}

/// The `X-Hub-Signature-256` GitHub sends with a webhook: HMAC-SHA256 of the body with the
/// webhook's secret.
pub fn signature(secret: &str, body: &[u8]) -> String {
    use hmac::Mac as _;
    let mut mac = hmac::Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    let bytes = mac.finalize().into_bytes();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256={hex}")
}

/// A public key as a JWKS entry, the way GitHub publishes its Actions keys.
fn jwk(kid: &str, key: &RsaPublicKey) -> Value {
    json!({
        "kty": "RSA",
        "alg": "RS256",
        "use": "sig",
        "kid": kid,
        "n": URL_SAFE_NO_PAD.encode(key.n().to_bytes_be()),
        "e": URL_SAFE_NO_PAD.encode(key.e().to_bytes_be()),
    })
}

/// Whether a token is a JWT the app signed, naming it and not expired, as GitHub requires.
fn app_jwt_valid(token: &str, key: &RsaPublicKey) -> bool {
    let mut parts = token.split('.');
    let (Some(header), Some(claims), Some(signature)) = (parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let Ok(signature) = URL_SAFE_NO_PAD.decode(signature) else {
        return false;
    };
    let Ok(signature) = Signature::try_from(signature.as_slice()) else {
        return false;
    };
    let signed = format!("{header}.{claims}");
    if VerifyingKey::<Sha256>::new(key.clone())
        .verify(signed.as_bytes(), &signature)
        .is_err()
    {
        return false;
    }
    let Ok(claims) = URL_SAFE_NO_PAD.decode(claims) else {
        return false;
    };
    let claims: Value = serde_json::from_slice(&claims).unwrap_or(Value::Null);
    let now = crate::now();
    claims["iss"] == CLIENT_ID
        && claims["exp"]
            .as_i64()
            .is_some_and(|exp| exp > now && exp <= now + 600)
        && claims["iat"].as_i64().is_some_and(|iat| iat <= now)
}
