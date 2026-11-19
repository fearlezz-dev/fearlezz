//! A fake GitHub API on a local port that answers every request with an empty object and
//! records it, so a test can show that a CI run sends GitHub nothing.

#![allow(dead_code, reason = "each test file uses only some of the helpers")]

use std::sync::{Arc, Mutex};

use super::server::{self, Answer, Request};

/// The fake server, running until the test ends.
pub(crate) struct FakeGitHub {
    pub url: String,
    requests: Arc<Mutex<Vec<Request>>>,
}

impl FakeGitHub {
    pub(crate) fn start() -> FakeGitHub {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::clone(&requests);
        let url = server::start(move |request| {
            shared.lock().unwrap().push(request);
            Answer::json(200, &serde_json::json!({}))
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
