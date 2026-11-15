//! The files origin: a run's pictures and films as a folder, at `/<grant>/<path>`. A grant is
//! sealed with the host's key for one run of one repository and expires within the hour, so
//! this origin needs no session: it reads no cookies and sets none. The page gets one from the
//! app origin (`grant`) for a run it may show. The files are what CI wrote, content of a pull
//! request: only the picture and film types a page shows are served, each with the type its
//! name pins and `nosniff`, so nothing here ever runs as a page, and served from here they
//! could reach nothing of the signed in app anyway.

use serde::{Deserialize, Serialize};
use worker::{Env, Headers, Range, Request, Response};

use crate::{Answer, Config, Refusal, crypto, now};

const GRANT: &str = "grant";

/// How long a grant opens its run's files.
const GRANT_SECONDS: i64 = 3600;

/// What a grant opens: one run of one repository, until `exp`.
#[derive(Serialize, Deserialize)]
struct Grant {
    repo: String,
    run: String,
    exp: i64,
}

fn not_found() -> Refusal {
    Refusal::new(404, "not found")
}

/// What the page reads to show a run's files: the run's folder on the files origin, under a
/// fresh grant, and when that grant ends, so the page asks again before.
#[derive(Serialize)]
pub struct Granted {
    pub files: String,
    pub expires: i64,
}

/// A fresh grant to the files of `run` of `repo` (lowercased). Only for someone who may see
/// the run's work: the caller decides that.
pub async fn grant(config: &Config, repo: &str, run: &str) -> worker::Result<Granted> {
    let expires = now() + GRANT_SECONDS;
    let grant = Grant {
        repo: repo.to_owned(),
        run: run.to_owned(),
        exp: expires,
    };
    let sealed = crypto::seal(&config.signing_key, GRANT, &grant).await?;
    Ok(Granted {
        files: format!("{}/{sealed}/", config.files_origin),
        expires,
    })
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
    let path = percent_encoding::percent_decode_str(rest)
        .decode_utf8()
        .map_err(|_| not_found())?
        .into_owned();
    let kind = content_type(&path).ok_or_else(not_found)?;
    #[derive(Deserialize)]
    struct Row {
        sha256: String,
        size: f64,
    }
    let row: Option<Row> = env
        .d1("DB")?
        .prepare(
            "SELECT f.sha256, f.size FROM files f JOIN runs r ON r.id = f.run JOIN works w ON w.id = r.work WHERE r.id = ?1 AND w.repo = ?2 AND f.path = ?3",
        )
        .bind(&[
            grant.run.as_str().into(),
            grant.repo.as_str().into(),
            path.as_str().into(),
        ])?
        .first(None)
        .await?;
    let row = row.ok_or_else(not_found)?;
    let size = row.size as u64;

    let headers = Headers::new();
    headers.set("Content-Type", kind)?;
    headers.set("Accept-Ranges", "bytes")?;
    headers.set("X-Content-Type-Options", "nosniff")?;
    // The grant is in every URL here; a link out of the page must not carry it along.
    headers.set("Referrer-Policy", "no-referrer")?;
    // Only the app origin's pages may frame the files. Browsers that know frame-ancestors ignore
    // X-Frame-Options, which can name no other origin; older ones refuse the frame outright.
    headers.set(
        "Content-Security-Policy",
        &format!("frame-ancestors {}", config.app_origin),
    )?;
    headers.set("X-Frame-Options", "SAMEORIGIN")?;
    // A file at a path of a run can be replaced while the run goes; the grant lasts an hour.
    headers.set("Cache-Control", "private, no-cache")?;
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

/// The type a file's name pins, for the pictures and films a page shows; anything else, which
/// a browser could run or guess at, is never served.
pub fn content_type(path: &str) -> Option<&'static str> {
    let extension = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match extension.as_deref()? {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        "avif" => Some("image/avif"),
        "mp4" => Some("video/mp4"),
        "webm" => Some("video/webm"),
        _ => None,
    }
}
