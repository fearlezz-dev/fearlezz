//! The proof host from CI (protocol version 2 of `ee/README.md`): a GitHub Actions job that
//! may request an OIDC token asks the host whether its repository installed the app, and only
//! then streams its run: it opens the run before any check runs, says when each check starts
//! and ends (its entry with the files it made, each sent by content when the host lacks it),
//! and closes the run with `proof.json`. Talks through curl, which every runner has.

use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use fearlezz_proof::events::{self, Done, Files, Missing, Open, Opened, Step};
use serde::Serialize;
use serde_json::Value;

use crate::look::Look;
use crate::proof::Check;

/// The host a project sends to unless its config names another.
pub(crate) const DEFAULT: &str = "https://fearlezz.seamlezz.com";

/// The host, and how to ask GitHub Actions for a token whose audience it is.
pub(crate) struct Host {
    url: String,
    /// The job's OIDC token endpoint, with the host as audience.
    ask: String,
    /// The job's token for asking.
    request: String,
}

/// What opening a run came to.
pub(crate) enum Opening {
    /// The host took it: send it the rest.
    Opened(Stream),
    /// The repository has not installed the app; where to install it.
    NotInstalled(String),
}

/// A run the host opened, to stream the rest of it to.
pub(crate) struct Stream {
    host: Host,
    run: String,
    /// The host's page of the work, where the run shows live.
    pub url: String,
}

impl Host {
    /// On GitHub Actions, when the job may request an OIDC token (`id-token: write`): the host
    /// `url` (the default without one), asked for a token whose audience is that host on
    /// every call. None elsewhere, so nothing is asked of anyone.
    pub(crate) fn from_actions(url: Option<&str>) -> Option<Host> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let (Some(ask), Some(request)) = (
            var("ACTIONS_ID_TOKEN_REQUEST_URL"),
            var("ACTIONS_ID_TOKEN_REQUEST_TOKEN"),
        ) else {
            return None;
        };
        let url = url.unwrap_or(DEFAULT).trim_end_matches('/').to_string();
        let join = if ask.contains('?') { '&' } else { '?' };
        let ask = format!("{ask}{join}audience={}", encode(&url));
        Some(Host { url, ask, request })
    }

    /// A fresh OIDC token: one lives minutes, and a run with films can take longer.
    fn token(&self) -> Result<String> {
        let (status, answer) = curl("GET", &self.ask, &self.request, &[], Body::None)?;
        Ok(answer
            .get("value")
            .and_then(Value::as_str)
            .filter(|_| status == 200)
            .with_context(|| format!("GitHub gave no OIDC token ({status})"))?
            .to_string())
    }

    /// Opens the run, when the host says the repository installed the app.
    pub(crate) fn open(self, open: &Open) -> Result<Opening> {
        let (status, answer) = self.call("GET", "/api/v2/installed", Body::None)?;
        if status != 200 {
            bail!("the host answered {status} when asked whether the app is installed");
        }
        if answer.get("installed") != Some(&Value::Bool(true)) {
            let install = answer
                .get("install")
                .and_then(Value::as_str)
                .unwrap_or(DEFAULT)
                .to_string();
            return Ok(Opening::NotInstalled(install));
        }
        let (status, answer) = self.call("POST", "/api/v2/runs", json(open)?)?;
        if status != 201 {
            bail!("the host refused the run ({status}): {answer}");
        }
        let opened: Opened = serde_json::from_value(answer)
            .context("the host answered the run without its id and page")?;
        Ok(Opening::Opened(Stream {
            host: self,
            run: opened.run,
            url: opened.url,
        }))
    }

    fn call(&self, method: &str, path: &str, body: Body<'_>) -> Result<(u16, Value)> {
        curl(
            method,
            &format!("{}{path}", self.url),
            &self.token()?,
            &[],
            body,
        )
    }
}

impl Stream {
    /// Says the check `id` starts.
    pub(crate) fn started(&self, id: &str) -> Result<()> {
        let step = Step::Started {
            check: id.to_string(),
        };
        self.step(&step, &[])
    }

    /// Sends a check's entry once it ended, with its output and its look, and the bytes of
    /// each of their files the host lacks.
    pub(crate) fn finished(&self, check: &Check, look: Option<&Look>) -> Result<()> {
        let mut entry = check.clone();
        let mut local = Vec::new();
        if let Some(log) = &check.log {
            let path = crate::proof::output_path(&check.id);
            entry.output = Some(path.clone());
            local.push(Local::of(path, log.clone())?);
        }
        let look = match look {
            Some(look) => {
                let (in_bundle, files) = crate::proof::bundled(look);
                for (path, on_disk) in files {
                    local.push(Local::of(path, on_disk)?);
                }
                Some(Box::new(in_bundle))
            }
            None => None,
        };
        let step = Step::Finished {
            check: Box::new(entry),
            files: manifest(&local),
            look,
        };
        self.step(&step, &local)
    }

    fn step(&self, step: &Step, local: &[Local]) -> Result<()> {
        let path = format!("/api/v2/runs/{}/checks", self.run);
        let (status, answer) = self.host.call("POST", &path, json(step)?)?;
        if status != 200 {
            bail!("the host refused a check ({status}): {answer}");
        }
        self.send(&missing(answer)?, local)
    }

    /// Closes the run with the bundle written in `dir`: its `proof.json`, and every other
    /// file it holds, sent first when the host says it lacks them.
    pub(crate) fn done(&self, dir: &Path) -> Result<()> {
        let proof = serde_json::from_slice(&fs::read(dir.join("proof.json"))?)
            .context("reading the bundle's proof.json")?;
        let local = bundle(dir)?;
        let done = Done {
            proof: Box::new(proof),
            files: manifest(&local),
        };
        let path = format!("/api/v2/runs/{}/done", self.run);
        let (status, answer) = self.host.call("POST", &path, json(&done)?)?;
        let status = if status == 409 {
            // The host refuses done while it lacks a file, and names what it lacks.
            self.send(&missing(answer)?, &local)?;
            let (status, answer) = self.host.call("POST", &path, json(&done)?)?;
            if status != 200 {
                bail!("the host did not close the run ({status}): {answer}");
            }
            status
        } else {
            status
        };
        if status != 200 {
            bail!("the host did not close the run ({status})");
        }
        Ok(())
    }

    /// Sends each file the host named as missing.
    fn send(&self, missing: &[String], local: &[Local]) -> Result<()> {
        for sha in missing {
            let Some(file) = local.iter().find(|f| &f.file.sha256 == sha) else {
                bail!("the host asked for a file this run does not have ({sha})");
            };
            let (status, _) = self.host.call(
                "PUT",
                &format!("/api/v2/blobs/{sha}"),
                Body::File(&file.on_disk),
            )?;
            if !(200..300).contains(&status) {
                bail!("the host refused {} ({status})", file.path);
            }
        }
        Ok(())
    }
}

fn json<T: Serialize>(value: &T) -> Result<Body<'static>> {
    Ok(Body::Json(serde_json::to_value(value)?))
}

fn missing(answer: Value) -> Result<Vec<String>> {
    let missing: Missing =
        serde_json::from_value(answer).context("the host did not say which files it lacks")?;
    Ok(missing.missing)
}

/// One file of a run: its path in the bundle, where it is on this machine, and its content.
struct Local {
    path: String,
    on_disk: PathBuf,
    file: events::File,
}

impl Local {
    fn of(path: String, on_disk: PathBuf) -> Result<Local> {
        use sha2::{Digest as _, Sha256};
        let mut hash = Sha256::new();
        let mut file =
            fs::File::open(&on_disk).with_context(|| format!("reading {}", on_disk.display()))?;
        let mut buf = vec![0; 64 * 1024];
        let mut size = 0;
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hash.update(buf.get(..n).unwrap_or_default());
            size += n as u64;
        }
        Ok(Local {
            path,
            on_disk,
            file: events::File {
                sha256: hex::encode(hash.finalize()),
                size,
            },
        })
    }
}

fn manifest(local: &[Local]) -> Files {
    local
        .iter()
        .map(|f| (f.path.clone(), f.file.clone()))
        .collect()
}

/// Every file of the bundle in `dir` but `proof.json`, which goes as JSON.
fn bundle(dir: &Path) -> Result<Vec<Local>> {
    let mut out = Vec::new();
    let mut todo: Vec<PathBuf> = vec![dir.to_path_buf()];
    while let Some(at) = todo.pop() {
        for entry in fs::read_dir(&at).with_context(|| format!("reading {}", at.display()))? {
            let path = entry?.path();
            if path.is_dir() {
                todo.push(path);
                continue;
            }
            let rel = path
                .strip_prefix(dir)?
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            if rel == "proof.json" {
                continue;
            }
            out.push(Local::of(rel, path)?);
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// `text` for a URL query.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// What a request sends.
pub(crate) enum Body<'a> {
    None,
    Json(Value),
    /// A file's bytes, as they are.
    File(&'a Path),
}

/// One request through curl with a bearer token, for the proof host and GitHub's OIDC token
/// endpoint: the status code and the answer, `null` when empty and a string when it is not JSON. The token
/// goes to curl on stdin, never in its arguments, and a large body goes at once, not after a
/// 100 Continue.
pub(crate) fn curl(
    method: &str,
    url: &str,
    token: &str,
    headers: &[&str],
    body: Body<'_>,
) -> Result<(u16, Value)> {
    let mut cmd = Command::new("curl");
    // A host that cannot be reached must not hold the checks up: they run between the calls.
    cmd.args(["-sS", "-K", "-", "-X", method, "-w", "\n%{http_code}"])
        .args(["--connect-timeout", "10"])
        .args(["-H", "User-Agent: fearlezz", "-H", "Expect:"]);
    for header in headers {
        cmd.args(["-H", header]);
    }
    let json = match body {
        Body::None => None,
        Body::Json(value) => {
            let mut file = tempfile::NamedTempFile::new()?;
            serde_json::to_writer(&mut file, &value)?;
            file.flush()?;
            cmd.args(["-H", "Content-Type: application/json", "--data-binary"])
                .arg(format!("@{}", file.path().display()));
            Some(file)
        }
        Body::File(path) => {
            cmd.args([
                "-H",
                "Content-Type: application/octet-stream",
                "--data-binary",
            ])
            .arg(format!("@{}", path.display()));
            None
        }
    };
    let mut child = cmd
        .arg(url)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("talking to the proof host needs curl")?;
    if let Some(mut stdin) = child.stdin.take() {
        writeln!(stdin, "header = \"Authorization: Bearer {token}\"")?;
    }
    let out = child.wait_with_output()?;
    drop(json);
    if !out.status.success() {
        bail!(
            "{method} {url} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let (answer, code) = text.rsplit_once('\n').unwrap_or(("", &text));
    let code: u16 = code
        .trim()
        .parse()
        .with_context(|| format!("{method} {url} gave no status code"))?;
    let answer = if answer.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(answer).unwrap_or_else(|_| Value::String(answer.to_string()))
    };
    Ok((code, answer))
}
