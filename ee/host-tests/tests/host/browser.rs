//! A real browser for what only a browser shows: a work's page drawing itself and updating
//! live over its WebSocket. Headless Chrome (or Chromium), driven over the DevTools protocol on a pipe
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
    /// The session of the page commands go to, once a page is open.
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
        let answer = self.call(method, params, on_page);
        assert!(
            answer.is_ok(),
            "{method} failed: {}",
            answer.as_ref().err().unwrap_or(&Value::Null)
        );
        answer.unwrap_or_default()
    }

    /// Sends one command like `send`, and its answer or Chrome's error.
    fn call(&mut self, method: &str, params: Value, on_page: bool) -> Result<Value, Value> {
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
                return match answer.get("error") {
                    Some(error) => Err(error.clone()),
                    None => Ok(answer["result"].clone()),
                };
            }
        }
    }

    /// Opens `url` in a fresh page and sends commands to it from now on; the page, to come
    /// back to it with `on`.
    pub fn open(&mut self, url: &str) -> String {
        let target = self.send("Target.createTarget", json!({"url": url}), false);
        let attached = self.send(
            "Target.attachToTarget",
            json!({"targetId": target["targetId"], "flatten": true}),
            false,
        );
        let page = attached["sessionId"].as_str().unwrap().to_owned();
        self.page = Some(page.clone());
        // A new page starts as about:blank, whose document is already complete, and `url`
        // replaces it a moment later, destroying whatever script runs in it then. So wait for
        // the document of `url` itself, asking again when the swap cuts a question short.
        let loaded = json!({
            "expression": "location.href !== 'about:blank' && document.readyState === 'complete'",
            "returnByValue": true,
        });
        let started = Instant::now();
        while self
            .call("Runtime.evaluate", loaded.clone(), true)
            .map_or(true, |r| r["result"]["value"] != json!(true))
        {
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "{url} never loaded"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        page
    }

    /// Holds a signed in person's session, as `Cookie: <pair>` names it, for every page of `url`
    /// opened from now on.
    pub fn sign_in(&mut self, url: &str, pair: &str) {
        let (name, value) = pair.split_once('=').unwrap();
        let cookie = json!({"name": name, "value": value, "url": url, "httpOnly": true});
        self.send("Storage.setCookies", json!({"cookies": [cookie]}), false);
    }

    /// Sends commands to `page`, one `open` gave, brought to the front, where a person's
    /// keyboard reaches it.
    pub fn on(&mut self, page: &str) {
        self.page = Some(page.to_owned());
        self.send("Page.bringToFront", json!({}), true);
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

    /// Presses `key` (one character) on the page, as a person on a keyboard does.
    pub fn press(&mut self, key: &str) {
        for kind in ["keyDown", "keyUp"] {
            let mut params = json!({"type": kind, "key": key});
            if kind == "keyDown" {
                params["text"] = json!(key);
            }
            self.send("Input.dispatchKeyEvent", params, true);
        }
    }

    /// Drags the mouse from one point of the page to another, as a person draws with it: a
    /// press, moves on the way, and a release, which the page gets as pointer events.
    pub fn drag(&mut self, from: (f64, f64), to: (f64, f64)) {
        let mouse = |kind: &str, (x, y): (f64, f64), buttons: u32| json!({"type": kind, "x": x, "y": y, "button": "left", "buttons": buttons, "clickCount": 1});
        self.send(
            "Input.dispatchMouseEvent",
            mouse("mouseMoved", from, 0),
            true,
        );
        self.send(
            "Input.dispatchMouseEvent",
            mouse("mousePressed", from, 1),
            true,
        );
        for step in 1..=5 {
            let t = f64::from(step) / 5.0;
            let at = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
            self.send("Input.dispatchMouseEvent", mouse("mouseMoved", at, 1), true);
        }
        self.send(
            "Input.dispatchMouseEvent",
            mouse("mouseReleased", to, 0),
            true,
        );
    }

    /// Types `text` into whatever has focus, as a keyboard would.
    pub fn type_text(&mut self, text: &str) {
        self.send("Input.insertText", json!({"text": text}), true);
    }

    /// Shows the page as a screen `width` wide and `height` high, a phone's when `mobile`, in
    /// the dark scheme when `dark`.
    pub fn view(&mut self, width: u32, height: u32, mobile: bool, dark: bool) {
        let metrics =
            json!({"width": width, "height": height, "deviceScaleFactor": 2, "mobile": mobile});
        self.send("Emulation.setDeviceMetricsOverride", metrics, true);
        let scheme = if dark { "dark" } else { "light" };
        let media = json!({"features": [{"name": "prefers-color-scheme", "value": scheme}]});
        self.send("Emulation.setEmulatedMedia", media, true);
    }

    /// The whole page as a PNG.
    pub fn screenshot(&mut self) -> Vec<u8> {
        use base64::Engine as _;
        let params = json!({"format": "png", "captureBeyondViewport": true});
        let shot = self.send("Page.captureScreenshot", params, true);
        base64::engine::general_purpose::STANDARD
            .decode(shot["data"].as_str().unwrap())
            .unwrap()
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
}

impl Drop for Browser {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
