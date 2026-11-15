//! A fake proof host on a local port, speaking protocol version 2 of `ee/README.md`, with
//! GitHub Actions' OIDC token endpoint beside it. It keeps every request in order so a test can
//! read back what Fearlezz sent and when, and takes the calls home of checks (`/mark/<name>`)
//! to show when a check's command ran among them.

#![allow(dead_code, reason = "each test file uses only some of the helpers")]

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

/// The OIDC token the fake's token endpoint hands out.
pub(crate) const OIDC: &str = "oidc-token";
/// The token GitHub Actions gives a job to ask for an OIDC token.
pub(crate) const REQUEST_TOKEN: &str = "request-token";
/// The id of every run the fake opens.
pub(crate) const RUN: &str = "r1";
/// The CLI token the fake's device flow hands out.
pub(crate) const HOST_TOKEN: &str = "fzh_test_token";

/// One request the fake took.
#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub method: String,
    pub path: String,
    pub auth: Option<String>,
    pub body: Vec<u8>,
}

#[derive(Default)]
pub(crate) struct State {
    pub requests: Vec<Request>,
    pub installed: bool,
    /// Blobs it already holds, by sha256.
    pub held: BTreeSet<String>,
    /// Blobs sent to it, by sha256.
    pub blobs: BTreeMap<String, Vec<u8>>,
    /// Paths that answer 500.
    pub broken: BTreeSet<String>,
    /// Forgets every blob when done is first asked, as a host that lost them would.
    pub forget_before_done: bool,
    /// How often the CLI asked whether its device sign in went through.
    pub polls: usize,
    /// What `GET /api/v2/work/...` answers a CLI token: what waits for the work; none is 404.
    pub feedback: Option<Value>,
    /// The pictures a CLI token reads at `/api/v2/runs/<run>/files/<path>`, by that path.
    pub pictures: BTreeMap<String, Vec<u8>>,
    /// Takes every protocol request and answers none, as a host that hangs.
    pub hangs: bool,
}

pub(crate) struct FakeHost {
    pub url: String,
    pub state: Arc<Mutex<State>>,
}

impl FakeHost {
    pub(crate) fn start() -> FakeHost {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(State {
            installed: true,
            ..State::default()
        }));
        let shared = Arc::clone(&state);
        let base = url.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let state = Arc::clone(&shared);
                let base = base.clone();
                std::thread::spawn(move || serve(stream, &state, &base));
            }
        });
        FakeHost { url, state }
    }

    /// The variables GitHub Actions sets when a job may request an OIDC token.
    pub(crate) fn token_env(&self) -> Vec<(String, String)> {
        vec![
            (
                "ACTIONS_ID_TOKEN_REQUEST_URL".into(),
                format!("{}/token?api-version=2.0", self.url),
            ),
            (
                "ACTIONS_ID_TOKEN_REQUEST_TOKEN".into(),
                REQUEST_TOKEN.into(),
            ),
        ]
    }

    pub(crate) fn requests(&self) -> Vec<Request> {
        self.state.lock().unwrap().requests.clone()
    }

    /// The requests to the protocol, not to the token endpoint.
    pub(crate) fn api(&self) -> Vec<Request> {
        self.requests()
            .into_iter()
            .filter(|r| r.path.starts_with("/api/"))
            .collect()
    }

    /// The calls it took in order, as `METHOD /path`, with what each check step said
    /// (`POST /api/v2/runs/r1/checks started tests`) and the checks' own calls home
    /// (`GET /mark/tests`), without the token endpoint.
    pub(crate) fn calls(&self) -> Vec<String> {
        self.requests()
            .into_iter()
            .filter(|r| !r.path.starts_with("/token"))
            .map(|r| {
                let line = format!("{} {}", r.method, r.path);
                let sent: Value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
                if let Some(id) = sent["started"]["check"].as_str() {
                    format!("{line} started {id}")
                } else if let Some(id) = sent["finished"]["check"]["id"].as_str() {
                    format!("{line} finished {id}")
                } else {
                    line
                }
            })
            .collect()
    }

    /// The body of the first call to `path`, as JSON.
    pub(crate) fn sent(&self, method: &str, path: &str) -> Value {
        let request = self
            .requests()
            .into_iter()
            .find(|r| r.method == method && r.path == path)
            .unwrap_or_else(|| panic!("no {method} {path}"));
        serde_json::from_slice(&request.body).unwrap()
    }
}

fn serve(stream: TcpStream, state: &Mutex<State>, base: &str) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let mut length = 0;
    let mut auth = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
            break;
        }
        let (name, value) = header.split_once(':').unwrap_or_default();
        let value = value.trim().to_string();
        match name.to_ascii_lowercase().as_str() {
            "content-length" => length = value.parse().unwrap_or(0),
            "authorization" => auth = Some(value),
            _ => {}
        }
    }
    let mut body = vec![0; length];
    if reader.read_exact(&mut body).is_err() {
        return;
    }
    let (picture, hangs) = {
        let mut s = state.lock().unwrap();
        s.requests.push(Request {
            method: method.clone(),
            path: path.clone(),
            auth: auth.clone(),
            body: body.clone(),
        });
        let cli = auth.as_deref() == Some(&format!("Bearer {HOST_TOKEN}"));
        let hangs = s.hangs && path.starts_with("/api/");
        (s.pictures.get(&path).filter(|_| cli).cloned(), hangs)
    };
    if hangs {
        std::thread::sleep(std::time::Duration::from_secs(60));
        return;
    }
    let mut stream = stream;
    if let Some(bytes) = picture {
        let sent = write!(
            stream,
            "HTTP/1.1 200 X\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len()
        )
        .and_then(|()| stream.write_all(&bytes));
        if let Err(e) = sent {
            eprintln!("fake host: {e}");
        }
        return;
    }
    let (status, answer) = {
        let mut s = state.lock().unwrap();
        answer(&mut s, &method, &path, auth.as_deref(), &body, base)
    };
    let text = answer.to_string();
    // The client may have gone; the request is recorded either way.
    if let Err(e) = write!(
        stream,
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    ) {
        eprintln!("fake host: {e}");
    }
}

fn answer(
    s: &mut State,
    method: &str,
    path: &str,
    auth: Option<&str>,
    body: &[u8],
    base: &str,
) -> (u16, Value) {
    let route = path.split('?').next().unwrap_or(path);
    if s.broken.contains(route) {
        return (500, json!({"error": "broken"}));
    }
    if route == "/token" {
        if auth != Some(&format!("Bearer {REQUEST_TOKEN}")) {
            return (401, json!({}));
        }
        return (200, json!({ "value": OIDC }));
    }
    if route.starts_with("/mark/") {
        // A check's command calling home, to show when it ran among the calls.
        return (200, json!({}));
    }
    // The CLI's sign in, and what its token reads.
    let cli = auth == Some(&format!("Bearer {HOST_TOKEN}"));
    match (method, route) {
        ("POST", "/api/v2/login/device") => {
            return (
                200,
                json!({"device_code": "dc", "user_code": "FZ12-3456",
                    "verification_uri": "https://github.com/login/device", "expires_in": 900, "interval": 0}),
            );
        }
        ("POST", "/api/v2/login/token") => {
            s.polls += 1;
            return if s.polls == 1 {
                (200, json!({"pending": true, "interval": 0}))
            } else {
                (200, json!({"token": HOST_TOKEN, "login": "writer"}))
            };
        }
        ("DELETE", "/api/v2/login") => return (204, json!({})),
        ("GET", work) if work.starts_with("/api/v2/work/") => {
            return match (&s.feedback, cli) {
                (_, false) => (
                    401,
                    json!({"error": "sign in to the proof host: fearlezz setup login"}),
                ),
                (Some(feedback), true) => (200, feedback.clone()),
                (None, true) => (404, json!({"error": "not found"})),
            };
        }
        ("GET", file) if file.contains("/files/") => return (404, json!({"error": "not found"})),
        _ => {}
    }
    // The protocol takes CI's OIDC token, and the CLI's own token for a local run.
    if auth != Some(&format!("Bearer {OIDC}")) && !cli {
        return (401, json!({"error": "no token"}));
    }
    let sent: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let page = format!("{base}/owner/repo/work/feat/x");
    match (method, route) {
        ("GET", "/api/v2/installed") if s.installed => (200, json!({"installed": true})),
        ("GET", "/api/v2/installed") => (
            200,
            json!({"installed": false, "install": "https://github.com/apps/fearlezz"}),
        ),
        ("POST", "/api/v2/runs") => (201, json!({"run": RUN, "url": page})),
        ("POST", "/api/v2/runs/r1/checks") => {
            let missing = s.lacks(&sent["finished"]["files"]);
            (200, json!({ "missing": missing }))
        }
        ("POST", "/api/v2/runs/r1/done") => {
            if std::mem::take(&mut s.forget_before_done) {
                s.held.clear();
            }
            let missing = s.lacks(&sent["files"]);
            if missing.is_empty() {
                (200, json!({ "url": page }))
            } else {
                (
                    409,
                    json!({"error": "files are missing", "missing": missing}),
                )
            }
        }
        ("PUT", blob) if blob.starts_with("/api/v2/blobs/") => {
            let sha = blob.trim_start_matches("/api/v2/blobs/").to_string();
            s.blobs.insert(sha.clone(), body.to_vec());
            s.held.insert(sha);
            (204, json!({}))
        }
        _ => (404, json!({"error": "unknown"})),
    }
}

impl State {
    /// The SHA-256s of `files` (`{path: {sha256, size}}`) it does not hold, sorted, once each.
    fn lacks(&self, files: &Value) -> Vec<String> {
        let mut missing: Vec<String> = files
            .as_object()
            .map(|f| {
                f.values()
                    .filter_map(|v| v["sha256"].as_str())
                    .filter(|sha| !self.held.contains(*sha))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        missing.sort();
        missing.dedup();
        missing
    }
}
