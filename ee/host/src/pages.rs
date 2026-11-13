//! The pages: a proof's bundle as a folder of files, and a pull request's latest whole proof.
//! Who sees them mirrors GitHub: anyone for a public repository, people who may read it for a
//! private one, and `404` for everyone else, so a private repository's name is not confirmed.

use serde::Deserialize;
use worker::{Env, Headers, Range, Request, Response};

use crate::session::{self, to_sign_in};
use crate::{Answer, Config, Refusal, github, repo_key};

fn not_found() -> Refusal {
    Refusal::new(404, "not found")
}

/// Whether the request may see the repository's pages: `false` means sign in first, and a
/// person GitHub shows nothing of it gets `404`.
async fn open(req: &Request, config: &Config, repo: &str) -> Result<bool, Refusal> {
    if let Some(session) = session::read(req, config).await? {
        match github::sight(config, repo, &session.token).await? {
            github::Sight::Reads => return Ok(true),
            github::Sight::Nothing => return Err(not_found()),
            // An expired token is no session; a public repository still opens.
            github::Sight::BadToken => {}
        }
    }
    Ok(github::public(config, repo).await?)
}

pub async fn pull(
    req: &Request,
    env: &Env,
    config: &Config,
    owner: &str,
    name: &str,
    number: &str,
) -> Answer {
    let repo = repo_key(owner, name);
    if !open(req, config, &repo).await? {
        return to_sign_in(&req.path());
    }
    let number: f64 = number.parse().map_err(|_| not_found())?;
    let latest: Option<String> = env
        .d1("DB")?
        .prepare("SELECT proof FROM latest WHERE repo = ?1 AND pull = ?2")
        .bind(&[repo.as_str().into(), number.into()])?
        .first(Some("proof"))
        .await?;
    let id = latest.ok_or_else(not_found)?;
    Ok(session::redirect(
        &format!("/{owner}/{name}/proofs/{id}/"),
        &[],
    )?)
}

pub async fn file(
    req: &Request,
    env: &Env,
    config: &Config,
    owner: &str,
    name: &str,
    id: &str,
    rest: &[&str],
) -> Answer {
    let repo = repo_key(owner, name);
    if !open(req, config, &repo).await? {
        return to_sign_in(&req.path());
    }
    // Relative links in the page resolve against the folder, which needs its slash.
    if rest.is_empty() {
        let headers = Headers::new();
        headers.set("Location", &format!("/{owner}/{name}/proofs/{id}/"))?;
        return Ok(Response::empty()?.with_status(301).with_headers(headers));
    }
    let path = match rest.join("/") {
        path if path.is_empty() => "index.html".to_owned(),
        path => percent_encoding::percent_decode_str(&path)
            .decode_utf8()
            .map_err(|_| not_found())?
            .into_owned(),
    };
    #[derive(Deserialize)]
    struct Row {
        sha256: String,
        size: f64,
    }
    let row: Option<Row> = env
        .d1("DB")?
        .prepare(
            "SELECT f.sha256, f.size FROM files f JOIN proofs p ON p.id = f.proof WHERE p.id = ?1 AND p.repo = ?2 AND p.done = 1 AND f.path = ?3",
        )
        .bind(&[id.into(), repo.as_str().into(), path.as_str().into()])?
        .first(None)
        .await?;
    let row = row.ok_or_else(not_found)?;
    let size = row.size as u64;

    let headers = Headers::new();
    headers.set("Content-Type", content_type(&path))?;
    headers.set("Accept-Ranges", "bytes")?;
    // A proof's files never change: they are named by the proof and stored by content.
    headers.set("Cache-Control", "private, max-age=31536000, immutable")?;
    let range = req
        .headers()
        .get("Range")?
        .and_then(|r| byte_range(&r, size));
    let key = format!("blobs/{repo}/{}", row.sha256);
    let bucket = env.bucket("BLOBS")?;
    let (status, object) = match range {
        Some(Err(())) => {
            headers.set("Content-Range", &format!("bytes */{size}"))?;
            return Ok(Response::empty()?.with_status(416).with_headers(headers));
        }
        Some(Ok((start, end))) => {
            headers.set("Content-Range", &format!("bytes {start}-{end}/{size}"))?;
            let range = Range::OffsetWithLength {
                offset: start,
                length: end - start + 1,
            };
            (206, bucket.get(key).range(range).execute().await?)
        }
        None => (200, bucket.get(key).execute().await?),
    };
    let object = object.ok_or_else(not_found)?;
    let body = object.body().ok_or_else(not_found)?.response_body()?;
    Ok(Response::from_body(body)?
        .with_status(status)
        .with_headers(headers))
}

/// One range of a `Range: bytes=` header as first and last byte, `Err` when it lies past the
/// end, and `None` for anything else, which gets the whole file.
fn byte_range(header: &str, size: u64) -> Option<Result<(u64, u64), ()>> {
    let spec = header.strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (first, last) = spec.trim().split_once('-')?;
    let (start, end) = match (first, last) {
        ("", suffix) => {
            let suffix: u64 = suffix.parse().ok()?;
            if suffix == 0 {
                return Some(Err(()));
            }
            (size.saturating_sub(suffix), size.checked_sub(1)?)
        }
        (first, "") => (first.parse().ok()?, size.checked_sub(1)?),
        (first, last) => {
            let (first, last): (u64, u64) = (first.parse().ok()?, last.parse().ok()?);
            (first, last.min(size.checked_sub(1)?))
        }
    };
    if start >= size || start > end {
        return Some(Err(()));
    }
    Some(Ok((start, end)))
}

fn content_type(path: &str) -> &'static str {
    let extension = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match extension.as_deref() {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("txt" | "log") => "text/plain; charset=utf-8",
        Some("xml") => "application/xml",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mov") => "video/quicktime",
        Some("pdf") => "application/pdf",
        _ => "application/octet-stream",
    }
}
