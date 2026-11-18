//! The one small HTTP server under every fake: it reads each request whole, on a thread of its
//! own, and answers with what the fake makes of it.

#![allow(dead_code, reason = "each test file uses only some of the helpers")]

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// One request a fake took.
#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub method: String,
    pub path: String,
    pub auth: Option<String>,
    pub body: Vec<u8>,
}

/// What a fake answers.
pub(crate) enum Answer {
    /// A status and a body of a content type.
    Body {
        status: u16,
        kind: &'static str,
        body: Vec<u8>,
    },
    /// Nothing: the server reads until the caller hangs up (60 s at most) and says how long
    /// that took.
    Hang(Box<dyn FnOnce(Duration) + Send>),
}

impl Answer {
    pub(crate) fn json(status: u16, value: &serde_json::Value) -> Answer {
        Answer::Body {
            status,
            kind: "application/json",
            body: value.to_string().into_bytes(),
        }
    }
}

/// Starts a server that answers each request through `answer`, and gives its address.
pub(crate) fn start(answer: impl Fn(Request) -> Answer + Send + Sync + 'static) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let answer = Arc::new(answer);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let answer = Arc::clone(&answer);
            std::thread::spawn(move || serve(stream, &*answer));
        }
    });
    url
}

fn serve(stream: TcpStream, answer: &dyn Fn(Request) -> Answer) {
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
    let request = Request {
        method,
        path,
        auth,
        body,
    };
    match answer(request) {
        Answer::Body { status, kind, body } => {
            let mut stream = stream;
            // The client may have gone; the fake took the request either way.
            let sent = write!(
                stream,
                "HTTP/1.1 {status} Fake\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .and_then(|()| stream.write_all(&body));
            if let Err(e) = sent {
                eprintln!("a fake server could not answer: {e}");
            }
        }
        Answer::Hang(waited) => {
            let arrived = Instant::now();
            stream
                .set_read_timeout(Some(Duration::from_secs(60)))
                .unwrap();
            if let Err(e) = std::io::copy(&mut reader, &mut std::io::sink()) {
                eprintln!("a fake server: the caller never hung up: {e}");
            }
            waited(arrived.elapsed());
        }
    }
}
