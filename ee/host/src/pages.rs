//! The pages on the app origin: a proof's shell, which frames its bundle from the files origin
//! and passes on what the page asks to approve or accept, and a pull request's latest whole
//! proof. Who sees them mirrors GitHub: anyone for a public
//! repository, people who may read it for a private one, and `404` for everyone else, so a
//! private repository's name is not confirmed.

use serde::Deserialize;
use worker::{Env, Headers, Request, Response};

use crate::session::{self, to_sign_in};
use crate::{Answer, Config, Refusal, cache, crypto, files, github, repo_key};

/// How long the host trusts GitHub's word on who may see a repository.
const SIGHT_SECONDS: i64 = 300;

fn not_found() -> Refusal {
    Refusal::new(404, "not found")
}

/// Whether the request may see the repository's pages: `false` means sign in first, and a
/// person GitHub shows nothing of it gets `404`.
pub async fn open(req: &Request, env: &Env, config: &Config, repo: &str) -> Result<bool, Refusal> {
    if let Some(session) = session::read(req, env).await? {
        let key = format!("reads:{}:{repo}", session.login.to_lowercase());
        let reads = match cache::get(env, &key).await? {
            Some(cached) => Some(cached == "yes"),
            None => {
                let reads = match github::sight(config, repo, &session.token).await? {
                    github::Sight::Reads => Some(true),
                    github::Sight::Nothing => Some(false),
                    // An expired token is no session; a public repository still opens.
                    github::Sight::BadToken => None,
                };
                if let Some(reads) = reads {
                    let value = if reads { "yes" } else { "no" };
                    cache::put(env, &key, value, SIGHT_SECONDS).await?;
                }
                reads
            }
        };
        match reads {
            Some(true) => return Ok(true),
            Some(false) => return Err(not_found()),
            None => {}
        }
    }
    let key = format!("public:{repo}");
    if let Some(public) = cache::get(env, &key).await? {
        return Ok(public == "yes");
    }
    let public = github::public(config, repo).await?;
    cache::put(env, &key, if public { "yes" } else { "no" }, SIGHT_SECONDS).await?;
    Ok(public)
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
    if !open(req, env, config, &repo).await? {
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

/// A proof's page on the app origin: a shell the host writes, framing the bundle's `index.html`
/// from the files origin under a grant for this proof alone. The bundle is HTML CI wrote, so it
/// never runs here, where the session cookie lives.
pub async fn shell(
    req: &Request,
    env: &Env,
    config: &Config,
    owner: &str,
    name: &str,
    id: &str,
    rest: &[&str],
) -> Answer {
    let repo = repo_key(owner, name);
    if !open(req, env, config, &repo).await? {
        return to_sign_in(&req.path());
    }
    match rest {
        // Relative links in the page resolve against the folder, which needs its slash.
        [] => {
            let headers = Headers::new();
            headers.set("Location", &format!("/{owner}/{name}/proofs/{id}/"))?;
            return Ok(Response::empty()?.with_status(301).with_headers(headers));
        }
        [""] => {}
        _ => return Err(not_found()),
    }
    #[derive(Deserialize)]
    struct Row {
        proof: String,
    }
    let row: Option<Row> = env
        .d1("DB")?
        .prepare("SELECT proof FROM proofs WHERE id = ?1 AND repo = ?2 AND done = 1")
        .bind(&[id.into(), repo.as_str().into()])?
        .first(None)
        .await?;
    let row = row.ok_or_else(not_found)?;
    let title = serde_json::from_str::<serde_json::Value>(&row.proof)
        .ok()
        .and_then(|proof| proof["pull"]["title"].as_str().map(str::to_owned));
    let title = match title {
        Some(title) => format!("{owner}/{name}: {title}"),
        None => format!("{owner}/{name}"),
    };
    let frame = format!("{}index.html", files::folder(config, &repo, id).await?);
    let nonce = crypto::random_id()?;

    let headers = Headers::new();
    headers.set("Content-Type", "text/html; charset=utf-8")?;
    // The page holds a grant, which should not outlive the visit in any cache.
    headers.set("Cache-Control", "no-store")?;
    headers.set(
        "Content-Security-Policy",
        &format!(
            "default-src 'none'; style-src 'unsafe-inline'; script-src 'nonce-{nonce}'; connect-src 'self'; frame-src {}; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
            config.files_origin
        ),
    )?;
    headers.set("X-Frame-Options", "DENY")?;
    headers.set("Referrer-Policy", "no-referrer")?;
    let html = shell_html(&title, &frame, &nonce, &config.files_origin);
    Ok(Response::ok(html)?.with_headers(headers))
}

/// The shell: a tiny title above a frame filling the window. The sandbox lets the bundle run its
/// scripts on its own origin, but never navigate the window away from the app. The page cannot
/// reach the API from there, so when a person approves or accepts in it, it asks the shell by
/// message; the shell takes messages only from its frame on the files origin, posts them to its
/// own origin with the session, shows what came of it in a bar above the frame, and tells the
/// page.
fn shell_html(title: &str, frame: &str, nonce: &str, files_origin: &str) -> String {
    let (title, frame) = (escape(title), escape(frame));
    // A JSON string inside a script: `<` escaped, so no value can end the script early.
    let files = serde_json::to_string(files_origin)
        .unwrap_or_default()
        .replace('<', "\\u003c");
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
html, body {{ margin: 0; height: 100%; }}
body {{ display: flex; flex-direction: column; font: 12px/1.5 system-ui, sans-serif; color: #555; background: #fff; }}
header {{ padding: 2px 12px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }}
#said {{ margin: 4px 12px; padding: 6px 12px; border: 1px solid #d0d0d0; border-radius: 8px; font-size: 13px; color: #222; }}
#said.ok {{ border-color: #2f8f5b; }}
#said.no {{ border-color: #c4532c; }}
#said a {{ color: inherit; font-weight: 600; }}
iframe {{ flex: 1; width: 100%; border: 0; }}
@media (prefers-color-scheme: dark) {{ body {{ color: #aaa; background: #111; }} #said {{ color: #eee; border-color: #444; }} }}
</style>
</head>
<body>
<header>{title}</header>
<p id="said" role="status" hidden></p>
<iframe src="{frame}" sandbox="allow-scripts allow-same-origin" title="{title}"></iframe>
<script nonce="{nonce}">
(function () {{
var FILES = {files};
var frame = document.querySelector('iframe'), said = document.getElementById('said');
function show(ok, text, link, label) {{
  said.textContent = text;
  if (link) {{
    var a = document.createElement('a');
    a.href = link; a.textContent = label;
    if (ok) {{ a.target = '_blank'; a.rel = 'noopener'; }}
    said.appendChild(document.createTextNode(' '));
    said.appendChild(a);
  }}
  said.className = ok ? 'ok' : 'no';
  said.hidden = false;
}}
window.addEventListener('message', function (e) {{
  if (e.source !== frame.contentWindow || e.origin !== FILES) return;
  var m = e.data || {{}};
  var kind = m.kind === 'fearlezz:approve' ? 'approve' : m.kind === 'fearlezz:accept' ? 'accept' : null;
  if (!kind || typeof m.words !== 'string') return;
  var check = typeof m.check === 'string' ? m.check : null;
  function tell(ok, text) {{
    frame.contentWindow.postMessage({{ kind: 'fearlezz:posted', of: kind, check: check, ok: ok, text: text }}, FILES);
  }}
  fetch(location.pathname + kind, {{
    method: 'POST', credentials: 'same-origin',
    headers: {{ 'Content-Type': 'application/json' }},
    body: JSON.stringify({{ check: check, words: m.words }})
  }}).then(function (r) {{
    return r.json().catch(function () {{ return {{}}; }}).then(function (b) {{ return {{ status: r.status, body: b }}; }});
  }}).then(function (a) {{
    if (a.status === 201) {{
      show(true, 'Posted on pull request #' + a.body.pull + ' as @' + a.body.login + ': ' + a.body.comment + '.', a.body.url, 'Open the comment');
      tell(true, 'Posted on the pull request as @' + a.body.login + '. CI counts it now.');
    }} else {{
      var why = a.body.error || 'the host answered ' + a.status;
      show(false, 'Not posted: ' + why + '.', a.status === 401 ? '/login?next=' + encodeURIComponent(location.pathname) : null, 'Sign in');
      tell(false, 'Not posted: ' + why + '.');
    }}
  }}, function () {{
    show(false, 'Not posted: the host could not be reached. Try again.');
    tell(false, 'Not posted: the host could not be reached.');
  }});
}});
}})();
</script>
</body>
</html>
"#
    )
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
