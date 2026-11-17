//! A fake package registry on a local port: it answers a GET with the file at that path under
//! its folder, or 404. A test's publish command writes the file, as a real publish makes the
//! version appear.

#![allow(dead_code, reason = "each test file uses only some of the helpers")]

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};

pub(crate) struct FakeRegistry {
    pub url: String,
    /// What it serves: the file at `<dir>/<path>` answers `GET /<path>`.
    pub dir: PathBuf,
    _dir: tempfile::TempDir,
}

impl FakeRegistry {
    pub(crate) fn start() -> FakeRegistry {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().canonicalize().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let served = dir.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                serve(&stream, &served);
            }
        });
        FakeRegistry {
            url,
            dir,
            _dir: tmp,
        }
    }

    /// Makes `path` answer with `body`, as if it had been published.
    pub(crate) fn put(&self, path: &str, body: &str) {
        let full = self.dir.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, body).unwrap();
    }
}

fn serve(stream: &TcpStream, dir: &Path) {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let path = line
        .split_whitespace()
        .nth(1)
        .unwrap_or_default()
        .to_string();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
            break;
        }
    }
    let file = dir.join(path.trim_start_matches('/'));
    let (status, body) = match std::fs::read(&file) {
        Ok(body) if file.is_file() => ("200 OK", body),
        _ => ("404 Not Found", b"{}".to_vec()),
    };
    let mut stream = stream;
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    if stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(&body))
        .is_err()
    {
        eprintln!("the fake registry could not answer");
    }
}
