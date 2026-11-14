//! The proof host from CI (`ee/README.md`): a GitHub Actions job that may request an OIDC
//! token asks the host whether its repository installed the app, and only then sends its
//! proof bundle: the proof as JSON, then each file the host does not hold yet, by content.
//! Talks through the one curl client in `github.rs`.

use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

use crate::github::{Body, curl};

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

/// What sending a bundle came to.
pub(crate) enum Sent {
    /// The page that shows it.
    Page(String),
    /// The repository has not installed the app; where to install it.
    NotInstalled(String),
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

    /// A fresh OIDC token: one lives minutes, and sending films can take longer.
    fn token(&self) -> Result<String> {
        let (status, answer) = curl("GET", &self.ask, &self.request, &[], Body::None)?;
        Ok(answer["value"]
            .as_str()
            .filter(|_| status == 200)
            .with_context(|| format!("GitHub gave no OIDC token ({status})"))?
            .to_string())
    }

    /// Sends the bundle in `dir`, when the host says the repository installed the app.
    pub(crate) fn send(&self, dir: &Path) -> Result<Sent> {
        let (status, answer) = self.call("GET", "/api/v1/installed", Body::None)?;
        if status != 200 {
            bail!("the host answered {status} when asked whether the app is installed");
        }
        if answer["installed"] != true {
            let install = answer["install"].as_str().unwrap_or(DEFAULT).to_string();
            return Ok(Sent::NotInstalled(install));
        }
        let proof: Value = serde_json::from_slice(&fs::read(dir.join("proof.json"))?)?;
        let files = files(dir)?;
        let manifest: serde_json::Map<String, Value> = files
            .iter()
            .map(|f| (f.path.clone(), json!({"sha256": f.sha256, "size": f.size})))
            .collect();
        let (status, answer) = self.call(
            "POST",
            "/api/v1/proofs",
            Body::Json(&json!({"proof": proof, "files": manifest})),
        )?;
        if status != 201 {
            bail!("the host refused the proof ({status}): {answer}");
        }
        let id = answer["id"]
            .as_str()
            .context("the host gave the proof no id")?
            .to_string();
        let missing: Vec<&str> = answer["missing"]
            .as_array()
            .map(|m| m.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        for sha in missing {
            let Some(file) = files.iter().find(|f| f.sha256 == sha) else {
                bail!("the host asked for a file the bundle does not have ({sha})");
            };
            let (status, _) = self.call(
                "PUT",
                &format!("/api/v1/blobs/{sha}"),
                Body::File(&dir.join(&file.path)),
            )?;
            if !(200..300).contains(&status) {
                bail!("the host refused {} ({status})", file.path);
            }
        }
        let (status, answer) =
            self.call("POST", &format!("/api/v1/proofs/{id}/done"), Body::None)?;
        match answer["url"].as_str() {
            Some(url) if status == 200 => Ok(Sent::Page(url.to_string())),
            _ => bail!("the host did not take the proof as whole ({status}): {answer}"),
        }
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

/// One file of a bundle, by its path in it.
struct File {
    path: String,
    sha256: String,
    size: u64,
}

/// Every file of the bundle but `proof.json`, which goes as JSON.
fn files(dir: &Path) -> Result<Vec<File>> {
    use sha2::{Digest as _, Sha256};
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
            let mut hash = Sha256::new();
            let mut file = fs::File::open(&path)?;
            let mut buf = [0; 64 * 1024];
            let mut size = 0;
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hash.update(buf.get(..n).unwrap_or_default());
                size += n as u64;
            }
            out.push(File {
                path: rel,
                sha256: hex::encode(hash.finalize()),
                size,
            });
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
