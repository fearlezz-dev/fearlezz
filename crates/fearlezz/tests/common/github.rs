//! A fake GitHub API on a local port that answers every request with an empty object and
//! records it, so a test can show that a CI run sends GitHub nothing.

#![allow(dead_code, reason = "each test file uses only some of the helpers")]

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

/// One request the fake took: its method and path.
#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub(crate) method: String,
    pub(crate) path: String,
}

/// The fake server, running until the test ends.
pub(crate) struct FakeGitHub {
    pub url: String,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl FakeGitHub {
    pub(crate) fn start() -> FakeGitHub {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::clone(&requests);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                serve(stream, &shared);
            }
        });
        FakeGitHub { url, requests }
    }

    /// Every request that would change something on GitHub.
    pub(crate) fn writes(&self) -> Vec<Request> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.method != "GET" && r.method != "HEAD")
            .cloned()
            .collect()
    }
}

/// Reads one HTTP request, records it and answers with an empty object.
fn serve(stream: TcpStream, requests: &Mutex<Vec<Request>>) {
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let mut words = line.split_whitespace();
    let method = words.next().unwrap_or_default().to_string();
    let path = words.next().unwrap_or_default().to_string();
    let mut length = 0;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
            break;
        }
        if let Some(n) = header.to_ascii_lowercase().strip_prefix("content-length:") {
            length = n.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; length];
    if reader.read_exact(&mut body).is_err() {
        return;
    }
    requests.lock().unwrap().push(Request { method, path });
    let mut stream = &stream;
    if let Err(e) = write!(
        stream,
        "HTTP/1.1 200 Fake\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
    ) {
        eprintln!("the fake GitHub could not answer: {e}");
    }
}
