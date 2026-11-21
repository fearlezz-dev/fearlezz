//! Showcase media: a stable address on the files origin for each picture a check takes,
//! `/showcase/<owner>/<repo>/<check>/<picture>`, answering with the newest proven one of a push
//! to the repository's default branch, so a README, a docs site or a website can embed what the
//! proof shows and follows each push without anyone copying files.
//!
//! A README's picture loads through GitHub's image proxy, which signs in to nothing, even for a
//! private repository. So the pictures of any repository with the app are served, private ones
//! included, and the address holds no grant: anyone who knows it can load the picture. A
//! repository without the app is `404`, as anything else on this origin. A website may embed
//! them too, also from a script or a canvas, so these answers, unlike the rest of this origin,
//! let any origin read them (`Access-Control-Allow-Origin: *`) and load them
//! (`Cross-Origin-Resource-Policy: cross-origin`).
//!
//! Only a CI run that a push started counts: its OIDC token's event is `push`, its ref
//! `refs/heads/<default branch>`, and the commit it proves the one the token says was pushed.
//! The ref alone would let in `pull_request_target` and `workflow_run` runs, which carry the
//! default branch's ref while they often check out a fork's code. A pull request, another branch
//! or a local run never counts, and only a run whose proof did not fail, as the host's review
//! judges it (`fearlezz_proof::review::failed`): a showcase shows what the default branch
//! proved, so a newer push that failed leaves the older picture.
//!
//! Newest means the newest push, not the newest run: pushes are ordered as the host first saw
//! each commit (its first run's place in `runs`, as `opened` holds whole seconds only), so a run
//! of an older push started again (after a flake, or by an admin's accept) does not bring its
//! picture back over a newer push's.
//!
//! The picture is the after of the look's item of that name: the commit's own picture. A check id
//! and a picture name may both hold slashes, so the path names the longest check of the run that
//! its leading segments spell, and the picture is the rest. Only stills and films are served.
//!
//! GitHub plays no film in a README, only an animated image in `<img>`, so a push run's CLI
//! sends each film's animated WebP copy too (the look item's `webp`, a file of the run beside
//! the film). A picture name that no item has names the copy of the film whose name with
//! `.webp` in place of its extension (`fearlezz_proof::look::webp_name`, the rule `fearlezz setup
//! showcase` prints by) is that name: `home.webp` is the copy of `home.webm`. A still the check
//! took under that name wins.

use fearlezz_proof::Proof;
use fearlezz_proof::look::{Item, Look};
use serde::{Deserialize, Serialize};
use worker::{Cache, Env, Headers, Request, Response, wasm_bindgen::JsValue};

use crate::work::read;
use crate::{Answer, Config, Refusal, access, files, github, not_found, repo_key, review};

/// How long a cache keeps a showcase picture, and the host the file it names: a push shows within
/// minutes, and neither GitHub's image proxy nor the host looks up the run on every view.
const SECONDS: u32 = 300;

/// How many runs of pushes one query reads. The lookup reads page after page, newest push first,
/// until a run whose proof did not fail, so a picture outlasts any number of newer pushes that
/// failed or did not run its check.
const PAGE: usize = 20;

pub(crate) async fn serve(req: &Request, env: &Env, config: &Config, path: &str) -> Answer {
    let segments = path
        .split('/')
        .map(files::decoded)
        .collect::<Result<Vec<_>, _>>()?;
    let [owner, name, rest @ ..] = segments.as_slice() else {
        return Err(not_found());
    };
    if rest.len() < 2
        || segments.iter().any(String::is_empty)
        || !github::named(owner)
        || !github::named(name)
    {
        return Err(not_found());
    }
    let repo = repo_key(owner, name);
    let cache_control = format!("public, max-age={SECONDS}");
    // Kept by the address alone: a query changes nothing it answers.
    let mut key = req.url()?;
    key.set_query(None);
    key.set_fragment(None);
    let key = key.to_string();
    let cache = Cache::default();
    let found = match cache.get(&key, false).await? {
        Some(mut kept) => kept.json::<Found>().await?,
        None => {
            let found = find(env, config, &repo, rest)
                .await?
                .ok_or_else(not_found)?;
            let headers = Headers::new();
            headers.set("Cache-Control", &cache_control)?;
            let kept = Response::from_json(&found)?.with_headers(headers);
            cache.put(&key, kept).await?;
            found
        }
    };
    let stored = files::Stored {
        kind: shown(&found.path).ok_or_else(not_found)?,
        key: files::blob_key(&repo, &found.sha256),
        sha256: found.sha256,
        size: found.size,
    };
    let mut answer = files::send(req, env, config, &stored, &cache_control).await?;
    // Any website may embed and read a showcase picture, unlike a run's files.
    let headers = answer.headers_mut();
    headers.set("Access-Control-Allow-Origin", "*")?;
    headers.set("Cross-Origin-Resource-Policy", "cross-origin")?;
    Ok(answer)
}

/// The file a showcase address names, as the cache keeps it.
#[derive(Serialize, Deserialize)]
struct Found {
    path: String,
    sha256: String,
    size: u64,
}

/// The file a showcase shows for `segments` (the check, then the picture) of `repo`
/// (lowercased): None for a repository without the app, or no such proven picture.
async fn find(
    env: &Env,
    config: &Config,
    repo: &str,
    segments: &[String],
) -> Result<Option<Found>, Refusal> {
    let Some(branch) = access::showcase_branch(env, config, repo).await? else {
        return Ok(None);
    };
    let Some((run, path)) = newest(env, repo, &branch, segments).await? else {
        return Ok(None);
    };
    if shown(&path).is_none() {
        return Ok(None);
    }
    let stored = files::stored(env, repo, &run, &path).await?;
    Ok(Some(Found {
        path,
        sha256: stored.sha256,
        size: stored.size,
    }))
}

/// The type a showcase picture at `path` is served as: only a still or a film, never a text.
fn shown(path: &str) -> Option<&'static str> {
    files::content_type(path)
        .filter(|kind| kind.starts_with("image/") || kind.starts_with("video/"))
}

/// The run and the path of its file that a showcase shows for `segments` (the check, then the
/// picture) of `repo` (lowercased): of the newest push to `branch` whose done CI run ran the
/// check, took its pictures and did not fail its proof. None when there is none, or when that
/// run took no such picture.
async fn newest(
    env: &Env,
    repo: &str,
    branch: &str,
    segments: &[String],
) -> Result<Option<(String, String)>, Refusal> {
    // Every way the segments split into a check and a picture.
    let checks: Vec<String> = (1..segments.len())
        .map(|k| segments.get(..k).unwrap_or_default().join("/"))
        .collect();
    let places = |from: usize| {
        (0..checks.len())
            .map(|i| format!("?{}", i + from))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let db = env.d1("DB")?;

    // The runs of pushes that ran one of the checks, by the `runs_by_work` index, each with its
    // own place in `runs` and its commit's first (`runs_by_head`): a push counts from then. The
    // newest push first and, of one push, its newest run; a page goes on after the last
    // candidate of the page before.
    #[derive(Deserialize)]
    struct Candidate {
        run: String,
        work: String,
        head: String,
        proof: String,
        place: f64,
        pushed: f64,
    }
    let sql = format!(
        "SELECT * FROM (SELECT r.id AS run, r.work, r.head, r.proof, r.rowid AS place, \
         (SELECT MIN(p.rowid) FROM runs p WHERE p.work = r.work AND p.head = r.head AND p.event = 'push') AS pushed \
         FROM runs r \
         WHERE r.work = (SELECT id FROM works WHERE repo = ?1 AND branch = ?2) \
         AND r.git_ref = ?3 AND r.event = 'push' AND r.sha = r.head \
         AND r.state = 'done' AND r.proof IS NOT NULL \
         AND EXISTS (SELECT 1 FROM checks c WHERE c.run = r.id AND c.look IS NOT NULL AND c.id IN ({}))) \
         WHERE pushed < ?4 OR (pushed = ?4 AND place < ?5) \
         ORDER BY pushed DESC, place DESC LIMIT {PAGE}",
        places(6)
    );
    let mut past = (f64::MAX, f64::MAX);
    loop {
        let mut binds: Vec<JsValue> = vec![
            repo.into(),
            branch.into(),
            format!("refs/heads/{branch}").into(),
            past.0.into(),
            past.1.into(),
        ];
        binds.extend(checks.iter().map(|c| JsValue::from(c.as_str())));
        let candidates = db
            .prepare(&sql)
            .bind(&binds)?
            .all()
            .await?
            .results::<Candidate>()?;
        let Some(last) = candidates.last() else {
            return Ok(None);
        };
        past = (last.pushed, last.place);
        for candidate in candidates {
            let proof: Proof = read(&candidate.proof)?;
            let accept = review::accept(&db, &candidate.work, &candidate.head).await?;
            if fearlezz_proof::review::failed(&proof, &candidate.head, accept.as_ref()) {
                continue;
            }
            #[derive(Deserialize)]
            struct Taken {
                id: String,
                look: String,
            }
            let sql = format!(
                "SELECT id, look FROM checks WHERE run = ?1 AND look IS NOT NULL AND id IN ({})",
                places(2)
            );
            let mut binds: Vec<JsValue> = vec![candidate.run.as_str().into()];
            binds.extend(checks.iter().map(|c| JsValue::from(c.as_str())));
            let taken = db
                .prepare(sql)
                .bind(&binds)?
                .all()
                .await?
                .results::<Taken>()?;
            // The longest check the segments spell names it; the rest is the picture.
            let Some((at, taken)) = taken
                .into_iter()
                .filter_map(|t| Some((checks.iter().position(|c| *c == t.id)?, t)))
                .max_by_key(|(at, _)| *at)
            else {
                continue;
            };
            let look: Look = read(&taken.look)?;
            let picture = segments.get(at + 1..).unwrap_or_default().join("/");
            return Ok(named(look.items, &picture).map(|path| (candidate.run, path)));
        }
    }
}

/// The file of a look's `items` that `picture` names: the after of the item of that name; else
/// the animated WebP copy of the film whose copy is served at that name
/// (`fearlezz_proof::look::webp_name`: `home.webp` is the copy of `home.webm`, `home.mp4` or
/// `home`).
fn named(items: Vec<Item>, picture: &str) -> Option<String> {
    if let Some(item) = items.iter().find(|item| item.name == picture) {
        return item.after.clone();
    }
    items
        .into_iter()
        .find(|item| item.webp.is_some() && fearlezz_proof::look::webp_name(&item.name) == picture)
        .and_then(|item| item.webp)
}
