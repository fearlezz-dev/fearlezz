//! The files origin: a proof's bundle as a folder of files, at `/<grant>/<path>`. The grant is
//! sealed by the app origin for one proof of one repository and expires within the hour, so
//! this origin needs no session: it reads no cookies and sets none. A bundle's page is HTML CI
//! wrote; served from here it can reach nothing of the signed in app.

use serde::{Deserialize, Serialize};
use worker::{Env, Headers, Range, Request, Response};

use crate::{Answer, Config, Refusal, crypto, now};

const GRANT: &str = "grant";
/// Long enough to read a proof; a reload of the shell hands out a fresh grant.
const GRANT_SECONDS: i64 = 3600;

#[derive(Serialize, Deserialize)]
struct Grant {
    repo: String,
    proof: String,
    exp: i64,
}

fn not_found() -> Refusal {
    Refusal::new(404, "not found")
}

/// The folder of a proof on the files origin, under a fresh grant; ends with a slash, so the
/// page's relative links resolve inside it.
pub async fn folder(config: &Config, repo: &str, proof: &str) -> worker::Result<String> {
    let grant = Grant {
        repo: repo.to_owned(),
        proof: proof.to_owned(),
        exp: now() + GRANT_SECONDS,
    };
    let sealed = crypto::seal(&config.signing_key, GRANT, &grant).await?;
    Ok(format!("{}/{sealed}/", config.files_origin))
}

pub async fn serve(req: &Request, env: &Env, config: &Config, path: &str) -> Answer {
    if req.method() != worker::Method::Get {
        return Err(not_found());
    }
    let (sealed, rest) = path
        .trim_start_matches('/')
        .split_once('/')
        .ok_or_else(not_found)?;
    let grant: Grant = crypto::unseal(&config.signing_key, GRANT, sealed)
        .await?
        .filter(|grant: &Grant| grant.exp > now())
        .ok_or_else(not_found)?;
    let path = match rest {
        "" => "index.html".to_owned(),
        rest => percent_encoding::percent_decode_str(rest)
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
        .bind(&[
            grant.proof.as_str().into(),
            grant.repo.as_str().into(),
            path.as_str().into(),
        ])?
        .first(None)
        .await?;
    let row = row.ok_or_else(not_found)?;
    let size = row.size as u64;

    let headers = Headers::new();
    headers.set("Content-Type", content_type(&path))?;
    headers.set("Accept-Ranges", "bytes")?;
    headers.set("X-Content-Type-Options", "nosniff")?;
    // The grant is in every URL here; a link out of the page must not carry it along.
    headers.set("Referrer-Policy", "no-referrer")?;
    // Only the app's shell may frame the files. Browsers that know frame-ancestors ignore
    // X-Frame-Options, which can name no other origin; older ones refuse the frame outright.
    headers.set(
        "Content-Security-Policy",
        &format!("frame-ancestors {}", config.app_origin),
    )?;
    headers.set("X-Frame-Options", "SAMEORIGIN")?;
    // A proof's files never change, but the URL holds a grant that lasts an hour.
    headers.set("Cache-Control", "private, max-age=3600, immutable")?;
    let range = req
        .headers()
        .get("Range")?
        .and_then(|r| byte_range(&r, size));
    let key = format!("blobs/{}/{}", grant.repo, row.sha256);
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
