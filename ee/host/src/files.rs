//! The files origin: a run's pictures and films as a folder, at `/<grant>/<path>`. A grant is
//! sealed with the host's key for one run of one repository and expires within the hour, so
//! this origin needs no session: it reads no cookies and sets none. The page gets one from the
//! app origin (`grant`) for a run it may show. The files are what CI wrote, content of a pull
//! request: only the picture and film types a page shows are served, each with the type its
//! name pins and `nosniff`, so nothing here ever runs as a page, and served from here they
//! could reach nothing of the signed in app anyway.
//!
//! It also serves showcase media at `/showcase/<owner>/<repo>/<check>/<picture>`, with no grant:
//! the newest proven picture of a push to the default branch of a repository with the app,
//! private ones included, for a README, a docs site or a website (`showcase.rs`).

use serde::{Deserialize, Serialize};
use worker::{Env, Headers, Range, Request, Response, ResponseBody};

use crate::{Answer, Config, Refusal, crypto, not_found, now, showcase};

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
    // A grant always holds a dot, so `showcase` is never one.
    if sealed == "showcase" {
        return showcase::serve(req, env, config, rest).await;
    }
    let grant: Grant = crypto::unseal(&config.signing_key, GRANT, sealed)
        .await?
        .filter(|grant: &Grant| grant.exp > now())
        .ok_or_else(not_found)?;
    let stored = stored(env, &grant.repo, &grant.run, &decoded(rest)?).await?;
    // A file at a path of a run can be replaced while the run goes; the grant lasts an hour.
    send(req, env, config, &stored, "private, no-cache").await
}

/// The file `stored` as the files origin answers it, whole or the range a `Range` header asks
/// for, kept by caches as `cache` says. Its `ETag` is its SHA-256: `If-None-Match` with it gets
/// `304`, and a `Range` whose `If-Range` names another tag (or a date, as no file says when it
/// changed) gets the whole file, so a cache never joins the bytes of two versions.
pub(crate) async fn send(
    req: &Request,
    env: &Env,
    config: &Config,
    stored: &Stored,
    cache: &str,
) -> Answer {
    let size = stored.size;
    let tag = format!("\"{}\"", stored.sha256);
    let headers = Headers::new();
    headers.set("ETag", &tag)?;
    headers.set("Cache-Control", cache)?;
    let holds = req.headers().get("If-None-Match")?.is_some_and(|tags| {
        tags.split(',')
            .any(|t| t.trim().trim_start_matches("W/") == tag)
    });
    if holds {
        return Ok(Response::empty()?.with_status(304).with_headers(headers));
    }
    headers.set("Content-Type", stored.kind)?;
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
    // The page reads a text of a look to diff it; the grant in the URL is what opens it.
    headers.set("Access-Control-Allow-Origin", &config.app_origin)?;
    let current = req
        .headers()
        .get("If-Range")?
        .is_none_or(|t| t.trim() == tag);
    let range = req
        .headers()
        .get("Range")?
        .filter(|_| current)
        .and_then(|r| byte_range(&r, size));
    let (status, range) = match range {
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
            (206, Some(range))
        }
        None => (200, None),
    };
    let body = body(env, &stored.key, range).await?;
    Ok(Response::from_body(body)?
        .with_status(status)
        .with_headers(headers))
}

/// A path as a URL holds it, decoded: `404` for one that does not decode.
pub(crate) fn decoded(path: &str) -> Result<String, Refusal> {
    Ok(percent_encoding::percent_decode_str(path)
        .decode_utf8()
        .map_err(|_| not_found())?
        .into_owned())
}

/// Where R2 keeps the bytes `sha256` of the repository `repo` (lowercased): each repository
/// keeps its own, so a blob is only ever served for runs of its repository.
pub(crate) fn blob_key(repo: &str, sha256: &str) -> String {
    format!("blobs/{repo}/{sha256}")
}

/// A run's file, as both origins serve it: the type its name pins, its SHA-256, where its bytes
/// are kept, and their size.
pub(crate) struct Stored {
    pub kind: &'static str,
    pub sha256: String,
    pub key: String,
    pub size: u64,
}

/// The file at `path` of the run `run` of `repo` (lowercased): `404` for a type never served, or
/// a file the run lacks.
pub(crate) async fn stored(
    env: &Env,
    repo: &str,
    run: &str,
    path: &str,
) -> Result<Stored, Refusal> {
    let kind = content_type(path).ok_or_else(not_found)?;
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
        .bind(&[run.into(), repo.into(), path.into()])?
        .first(None)
        .await?;
    let row = row.ok_or_else(not_found)?;
    Ok(Stored {
        kind,
        key: blob_key(repo, &row.sha256),
        sha256: row.sha256,
        size: row.size as u64,
    })
}

/// The bytes kept at `key`, whole or the `range` of them: `404` when R2 holds none.
pub(crate) async fn body(
    env: &Env,
    key: &str,
    range: Option<Range>,
) -> Result<ResponseBody, Refusal> {
    let bucket = env.bucket("BLOBS")?;
    let get = bucket.get(key);
    let object = match range {
        Some(range) => get.range(range).execute().await?,
        None => get.execute().await?,
    };
    let object = object.ok_or_else(not_found)?;
    Ok(object.body().ok_or_else(not_found)?.response_body()?)
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

/// The type a file's name pins, for the pictures, films and texts a page shows (a text of a look
/// is kept as `.txt`, plain text a browser never runs); anything else, which a browser could
/// run or guess at, is never served.
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
        "txt" => Some("text/plain; charset=utf-8"),
        _ => None,
    }
}
