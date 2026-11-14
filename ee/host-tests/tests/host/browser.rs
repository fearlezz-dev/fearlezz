//! A real browser for what only a browser shows: the shell and the page it frames talking by
//! message. Headless Chrome (or Chromium), driven over the DevTools protocol on a pipe
//! (`--remote-debugging-pipe`: JSON messages ending in a NUL byte, Chrome reading fd 3 and
//! writing fd 4), so the tests need no WebSocket client and no driver binary.

use std::fs::File;
use std::io::{BufRead as _, BufReader, Write as _};
use std::os::fd::{AsRawFd as _, OwnedFd};
use std::os::unix::process::CommandExt as _;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

pub struct Browser {
    child: Child,
    to: File,
    from: BufReader<File>,
    next: u64,
    /// The page's session, once a page is open.
    page: Option<String>,
    _profile: tempfile::TempDir,
}

/// Where Chrome is: the macOS apps, or a binary on `PATH` as Linux and CI runners have it.
fn chrome() -> PathBuf {
    let apps = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
    ];
    if let Some(app) = apps.iter().map(PathBuf::from).find(|p| p.exists()) {
        return app;
    }
    let names = [
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
    ];
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .flat_map(|dir| names.iter().map(move |n| dir.join(n)))
        .find(|p| p.exists())
        .unwrap_or_else(|| {
            panic!(
                "the browser tests need Google Chrome or Chromium: install it (on macOS in /Applications, elsewhere on PATH)"
            )
        })
}

impl Browser {
    pub fn start() -> Browser {
        let profile = tempfile::tempdir().unwrap();
        let (chrome_reads, to) = std::io::pipe().unwrap();
        let (from, chrome_writes) = std::io::pipe().unwrap();
        let (reads, writes): (OwnedFd, OwnedFd) = (chrome_reads.into(), chrome_writes.into());
        let (reads_fd, writes_fd) = (reads.as_raw_fd(), writes.as_raw_fd());
        let mut cmd = Command::new(chrome());
        cmd.args([
            "--headless=new",
            "--remote-debugging-pipe",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-gpu",
            "--disable-extensions",
            "--hide-scrollbars",
            "--window-size=1100,720",
        ])
        .arg(format!("--user-data-dir={}", profile.path().display()))
        .arg("about:blank")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
        // Ubuntu's AppArmor keeps Chrome's sandbox from starting on CI runners.
        #[cfg(target_os = "linux")]
        cmd.arg("--no-sandbox");
        // SAFETY: only dup2, which is async signal safe, runs between fork and exec.
        unsafe {
            cmd.pre_exec(move || {
                if libc::dup2(reads_fd, 3) < 0 || libc::dup2(writes_fd, 4) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = cmd.spawn().unwrap();
        // Chrome holds its ends now; with ours closed, a Chrome that dies ends our reads.
        drop((reads, writes));
        Browser {
            child,
            to: File::from(OwnedFd::from(to)),
            from: BufReader::new(File::from(OwnedFd::from(from))),
            next: 0,
            page: None,
            _profile: profile,
        }
    }

    /// Sends one command, to the page when `on_page`, and waits for its answer, passing over
    /// the events Chrome sends meanwhile.
    fn send(&mut self, method: &str, params: Value, on_page: bool) -> Value {
        self.next += 1;
        let id = self.next;
        let mut message = json!({"id": id, "method": method, "params": params});
        if on_page {
            message["sessionId"] = json!(self.page.clone().expect("a page is open"));
        }
        let mut bytes = message.to_string().into_bytes();
        bytes.push(0);
        self.to.write_all(&bytes).unwrap();
        loop {
            let mut raw = Vec::new();
            let read = self.from.read_until(0, &mut raw).unwrap();
            assert!(read > 0, "Chrome closed its pipe");
            raw.pop();
            let answer: Value = serde_json::from_slice(&raw).unwrap();
            if answer["id"] == id {
                assert!(
                    answer.get("error").is_none(),
                    "{method} failed: {}",
                    answer["error"]
                );
                return answer["result"].clone();
            }
        }
    }

    /// Gives the browser a cookie for `url`, as signing in would have.
    pub fn cookie(&mut self, url: &str, name: &str, value: &str) {
        let cookie = json!({"name": name, "value": value, "url": url, "httpOnly": true});
        self.send("Storage.setCookies", json!({"cookies": [cookie]}), false);
    }

    /// Opens `url` in a fresh page and attaches to it.
    pub fn open(&mut self, url: &str) {
        let target = self.send("Target.createTarget", json!({"url": url}), false);
        let attached = self.send(
            "Target.attachToTarget",
            json!({"targetId": target["targetId"], "flatten": true}),
            false,
        );
        self.page = Some(attached["sessionId"].as_str().unwrap().to_owned());
    }

    /// The value of a script run in the page, awaited when it is a promise.
    pub fn eval(&mut self, script: &str) -> Value {
        let params = json!({"expression": script, "returnByValue": true, "awaitPromise": true});
        let result = self.send("Runtime.evaluate", params, true);
        assert!(
            result.get("exceptionDetails").is_none(),
            "{script} threw: {}",
            result["exceptionDetails"]
        );
        result["result"]["value"].clone()
    }

    /// Waits until `script` gives `true` in the page, for at most 20 seconds.
    pub fn wait_for(&mut self, script: &str) {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(20) {
            if self.eval(script) == json!(true) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("{script} never held");
    }

    /// Asks for the light or the dark scheme, as the system would.
    pub fn scheme(&mut self, scheme: &str) {
        let features = [json!({"name": "prefers-color-scheme", "value": scheme})];
        let params = json!({"features": features});
        self.send("Emulation.setEmulatedMedia", params, true);
    }

    /// A PNG of the page as it shows.
    pub fn picture(&mut self) -> Vec<u8> {
        use base64::Engine as _;
        let shot = self.send("Page.captureScreenshot", json!({"format": "png"}), true);
        base64::engine::general_purpose::STANDARD
            .decode(shot["data"].as_str().unwrap())
            .unwrap()
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
