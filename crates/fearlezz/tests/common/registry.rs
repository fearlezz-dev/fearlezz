//! A fake package registry on a local port: it answers a GET with the file at that path under
//! its folder, or 404. A test's publish command writes the file, as a real publish makes the
//! version appear.

#![allow(dead_code, reason = "each test file uses only some of the helpers")]

use std::path::PathBuf;

use super::server::{self, Answer};

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
        let served = dir.clone();
        let url = server::start(move |request| {
            let file = served.join(request.path.trim_start_matches('/'));
            // A `<path>.status` file beside it makes the path answer with that status instead,
            // as a registry that stumbles does.
            let stumble = std::fs::read_to_string(format!("{}.status", file.display()))
                .ok()
                .and_then(|s| s.trim().parse().ok());
            // A `<path>.auth` file makes the path answer only a request that carries that
            // Authorization header, as a private repository does; others get 404.
            let locked = std::fs::read_to_string(format!("{}.auth", file.display()))
                .is_ok_and(|auth| request.auth.as_deref() != Some(auth.trim()));
            let (status, body) = match (stumble, std::fs::read(&file)) {
                (Some(status), _) => (status, b"{}".to_vec()),
                _ if locked => (404, b"{}".to_vec()),
                (None, Ok(body)) if file.is_file() => (200, body),
                _ => (404, b"{}".to_vec()),
            };
            Answer::Body {
                status,
                kind: "application/json",
                body,
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
