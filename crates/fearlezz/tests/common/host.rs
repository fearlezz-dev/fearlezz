//! A fake proof host on a local port, speaking the protocol in `ee/README.md`, with GitHub
//! Actions' OIDC token endpoint beside it. It keeps every request so a test can read back
//! what Fearlezz sent.

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
    let (status, answer) = {
        let mut s = state.lock().unwrap();
        s.requests.push(Request {
            method: method.clone(),
            path: path.clone(),
            auth: auth.clone(),
            body: body.clone(),
        });
        answer(&mut s, &method, &path, auth.as_deref(), &body, base)
    };
    let text = answer.to_string();
    let mut stream = stream;
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
    if auth != Some(&format!("Bearer {OIDC}")) {
        return (401, json!({"error": "no token"}));
    }
    match (method, route) {
        ("GET", "/api/v1/installed") if s.installed => (200, json!({"installed": true})),
        ("GET", "/api/v1/installed") => (
            200,
            json!({"installed": false, "install": "https://github.com/apps/fearlezz"}),
        ),
        ("POST", "/api/v1/proofs") => {
            let Ok(sent) = serde_json::from_slice::<Value>(body) else {
                return (400, json!({"error": "not json"}));
            };
            let mut missing: Vec<String> = sent["files"]
                .as_object()
                .map(|f| {
                    f.values()
                        .filter_map(|v| v["sha256"].as_str())
                        .filter(|sha| !s.held.contains(*sha))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            missing.sort();
            missing.dedup();
            (
                201,
                json!({"id": "p1", "url": format!("{base}/owner/repo/proofs/p1/"), "missing": missing}),
            )
        }
        ("POST", "/api/v1/proofs/p1/done") => {
            (200, json!({"url": format!("{base}/owner/repo/proofs/p1/")}))
        }
        ("PUT", blob) if blob.starts_with("/api/v1/blobs/") => {
            let sha = blob.trim_start_matches("/api/v1/blobs/").to_string();
            s.blobs.insert(sha.clone(), body.to_vec());
            s.held.insert(sha);
            (204, json!({}))
        }
        _ => (404, json!({"error": "unknown"})),
    }
}
