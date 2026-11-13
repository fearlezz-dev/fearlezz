//! The pages on the app origin: a proof's shell, which frames its bundle from the files origin
//! and passes on what the page asks to approve or accept, and a pull request's latest whole
//! proof. Who sees them mirrors GitHub: anyone for a public
//! repository, people who may read it for a private one, and `404` for everyone else, so a
//! private repository's name is not confirmed.

use fearlezz_proof::waits::Role;
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

/// Whether the signed in person may share the repository's pull requests: write access, as
/// GitHub's collaborator permission says, kept as long as who may read. It only decides whether
/// the shell shows the control; making and revoking a link ask GitHub again.
async fn shares(
    req: &Request,
    env: &Env,
    config: &Config,
    owner: &str,
    name: &str,
) -> Result<bool, Refusal> {
    let Some(person) = session::read(req, env).await? else {
        return Ok(false);
    };
    let key = format!(
        "writes:{}:{}",
        person.login.to_lowercase(),
        repo_key(owner, name)
    );
    if let Some(cached) = cache::get(env, &key).await? {
        return Ok(cached == "yes");
    }
    let full = format!("{owner}/{name}");
    let writes = match github::permission(config, &full, &person.login, &person.token).await? {
        Some(role) => role >= Role::Write,
        None => return Ok(false),
    };
    cache::put(env, &key, if writes { "yes" } else { "no" }, SIGHT_SECONDS).await?;
    Ok(writes)
}

/// The latest whole proof of a pull request, if it has one.
pub async fn latest(env: &Env, repo: &str, pull: u64) -> worker::Result<Option<String>> {
    env.d1("DB")?
        .prepare("SELECT proof FROM latest WHERE repo = ?1 AND pull = ?2")
        .bind(&[repo.into(), (pull as f64).into()])?
        .first(Some("proof"))
        .await
}

/// A whole proof of a repository, as stored.
#[derive(Deserialize)]
pub struct Whole {
    pub pull: Option<f64>,
    /// Its `proof.json`.
    pub proof: String,
}

pub async fn whole(env: &Env, repo: &str, id: &str) -> worker::Result<Option<Whole>> {
    env.d1("DB")?
        .prepare("SELECT pull, proof FROM proofs WHERE id = ?1 AND repo = ?2 AND done = 1")
        .bind(&[id.into(), repo.into()])?
        .first(None)
        .await
}

/// The shell's title: the repository and the pull request's title, when the proof has one.
pub fn title(full: &str, proof: &str) -> String {
    let title = serde_json::from_str::<serde_json::Value>(proof)
        .ok()
        .and_then(|proof| proof["pull"]["title"].as_str().map(str::to_owned));
    match title {
        Some(title) => format!("{full}: {title}"),
        None => full.to_owned(),
    }
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
    let number: u64 = number.parse().map_err(|_| not_found())?;
    let id = latest(env, &repo, number).await?.ok_or_else(not_found)?;
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
    let row = whole(env, &repo, id).await?.ok_or_else(not_found)?;
    let share = match row.pull {
        Some(pull) if shares(req, env, config, owner, name).await? => {
            Some(format!("/{owner}/{name}/pull/{}/share", pull as u64))
        }
        _ => None,
    };
    let view = View {
        title: title(&format!("{owner}/{name}"), &row.proof),
        frame: format!("{}index.html", files::folder(config, &repo, id).await?),
        act: Act::Post {
            proof: format!("/{owner}/{name}/proofs/{id}/"),
            share,
        },
    };
    render(config, view)
}

/// What a shell shows: its title, the bundle page it frames, and what it does when the page asks
/// to approve or accept.
pub struct View {
    pub title: String,
    pub frame: String,
    pub act: Act,
}

pub enum Act {
    /// Opened at the proof's own address: approvals post to it with the session, and a person
    /// with write access gets the pull request's share links.
    Post {
        proof: String,
        share: Option<String>,
    },
    /// Opened through a share link: the shell answers the page but never posts, and points to
    /// the pull request's own address for signing in.
    ViewOnly { pull: String },
}

/// The shell page, with the headers that keep its grant out of caches, referrers and frames.
pub fn render(config: &Config, view: View) -> Answer {
    let nonce = crypto::random_id()?;
    // Only the signed in shell calls the host; one opened by a link calls nothing.
    let connect = match view.act {
        Act::Post { .. } => "'self'",
        Act::ViewOnly { .. } => "'none'",
    };
    let headers = Headers::new();
    headers.set("Content-Type", "text/html; charset=utf-8")?;
    // The page holds a grant, which should not outlive the visit in any cache.
    headers.set("Cache-Control", "no-store")?;
    headers.set(
        "Content-Security-Policy",
        &format!(
            "default-src 'none'; style-src 'unsafe-inline'; script-src 'nonce-{nonce}'; connect-src {connect}; frame-src {}; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
            config.files_origin
        ),
    )?;
    headers.set("X-Frame-Options", "DENY")?;
    // Keeps the grant, and a share link's token, out of every request the page makes.
    headers.set("Referrer-Policy", "no-referrer")?;
    if matches!(view.act, Act::ViewOnly { .. }) {
        headers.set("X-Robots-Tag", "noindex")?;
    }
    let html = shell_html(&view, &nonce, &config.files_origin);
    Ok(Response::ok(html)?.with_headers(headers))
}

/// The shell: a tiny bar above a frame filling the window. The sandbox lets the bundle run its
/// scripts on its own origin, but never navigate the window away from the app. The page cannot
/// reach the API from there, so when a person approves or accepts in it, it asks the shell by
/// message; the shell takes messages only from its frame on the files origin. At the proof's own
/// address it posts them to its own origin with the session, shows what came of it in a bar
/// above the frame, and tells the page; opened by a share link it tells the page it only views.
/// A person with write access also gets a Share control in the bar.
fn shell_html(view: &View, nonce: &str, files_origin: &str) -> String {
    let (title, frame) = (escape(&view.title), escape(&view.frame));
    let files = js(files_origin);
    let (control, panel) = match &view.act {
        Act::Post { share: Some(_), .. } => (
            r#"<button id="share" type="button" aria-expanded="false" aria-controls="sharing">Share</button>"#,
            r#"<section id="sharing" aria-label="Share links" hidden>
<p>Anyone with a link sees this pull request's latest proof without signing in, until you revoke it. They can only look.</p>
<button id="make" type="button">Make a link</button>
<ul id="links"></ul>
</section>"#,
        ),
        _ => ("", ""),
    };
    let script = match &view.act {
        Act::Post { proof, share } => {
            let mut script = POST_SCRIPT.replace("__PROOF__", &js(proof));
            if let Some(share) = share {
                script.push_str(&SHARE_SCRIPT.replace("__SHARE__", &js(share)));
            }
            script
        }
        Act::ViewOnly { pull } => VIEW_SCRIPT.replace("__PULL__", &js(pull)),
    };
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>{SHELL_STYLE}</style>
</head>
<body>
<header><span class="title">{title}</span>{control}</header>
{panel}
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
  act(kind, check, m.words, tell);
}});
{script}
}})();
</script>
</body>
</html>
"#
    )
}

/// The shell's look: the colours of the core's pages (`crates/fearlezz/src/page.rs`), small, with
/// borders all the way round, calm under reduced motion.
const SHELL_STYLE: &str = r#"
:root { --surface:#e7eaee; --mat:#fff; --sunk:#dce0e5; --ink:#121417; --quiet:#58606c; --line:#cdd3da;
  --ok:#1d7a4a; --no:#c4532c; --focus:#1f5fd1; --ease:cubic-bezier(.2,.8,.2,1); color-scheme: light dark; }
@media (prefers-color-scheme: dark) {
  :root { --surface:#101318; --mat:#1a1e25; --sunk:#252a33; --ink:#eceef2; --quiet:#a3aab5; --line:#323943;
    --ok:#4fd08a; --no:#ff7a52; --focus:#7aa7ff; }
}
* { box-sizing: border-box; }
[hidden] { display: none !important; }
html, body { margin: 0; height: 100%; }
body { display: flex; flex-direction: column; font: 12px/1.5 system-ui, sans-serif; color: var(--quiet); background: var(--surface); }
header { display: flex; align-items: center; gap: 8px; padding: 2px 6px 2px 12px; min-height: 26px; }
header .title { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
button { font: inherit; color: var(--ink); background: var(--mat); border: 1px solid var(--line); border-radius: 7px;
  padding: 1px 10px; cursor: pointer; transition: background .2s, border-color .2s; }
button:hover { border-color: var(--quiet); }
button:disabled { opacity: .5; cursor: default; }
button[aria-expanded="true"] { background: var(--sunk); }
:focus-visible { outline: 2px solid var(--focus); outline-offset: 1px; }
#sharing { margin: 2px 12px 4px; padding: 10px 12px; border: 1px solid var(--line); border-radius: 10px;
  background: var(--mat); color: var(--ink); font-size: 13px; animation: rise .3s var(--ease) both; }
#sharing p { margin: 0 0 8px; color: var(--quiet); }
#links { list-style: none; margin: 8px 0 0; padding: 0; display: grid; gap: 6px; }
#links li { display: flex; flex-wrap: wrap; align-items: center; gap: 6px; }
#links li.fresh input { border-color: var(--ok); }
#links input { flex: 1 1 260px; min-width: 0; font: 12px ui-monospace, SFMono-Regular, Menlo, monospace; color: var(--ink);
  background: var(--sunk); border: 1px solid transparent; border-radius: 7px; padding: 3px 8px; }
#links .who { flex-basis: 100%; color: var(--quiet); font-size: 12px; }
#links .none { color: var(--quiet); }
#said { margin: 4px 12px; padding: 6px 12px; border: 1px solid var(--line); border-radius: 8px; font-size: 13px; color: var(--ink); background: var(--mat); }
#said.ok { border-color: var(--ok); }
#said.no { border-color: var(--no); }
#said a { color: inherit; font-weight: 600; }
iframe { flex: 1; width: 100%; border: 0; }
@keyframes rise { from { opacity: 0; transform: translateY(-4px); } to { opacity: 1; transform: none; } }
@media (prefers-reduced-motion: reduce) { *, *::before, *::after { animation-duration: .01ms !important; transition-duration: .01ms !important; } }
"#;

/// At the proof's own address: posts what the page asks to the host with the session.
const POST_SCRIPT: &str = r#"
var PROOF = __PROOF__;
function ask(method, path, body) {
  return fetch(path, {
    method: method, credentials: 'same-origin',
    headers: body ? { 'Content-Type': 'application/json' } : {},
    body: body ? JSON.stringify(body) : undefined
  }).then(function (r) {
    return r.json().catch(function () { return {}; }).then(function (b) { return { status: r.status, body: b }; });
  });
}
function signIn() { return '/login?next=' + encodeURIComponent(location.pathname); }
function act(kind, check, words, tell) {
  ask('POST', PROOF + kind, { check: check, words: words }).then(function (a) {
    if (a.status === 201) {
      show(true, 'Posted on pull request #' + a.body.pull + ' as @' + a.body.login + ': ' + a.body.comment + '.', a.body.url, 'Open the comment');
      tell(true, 'Posted on the pull request as @' + a.body.login + '. CI counts it now.');
    } else {
      var why = a.body.error || 'the host answered ' + a.status;
      show(false, 'Not posted: ' + why + '.', a.status === 401 ? signIn() : null, 'Sign in');
      tell(false, 'Not posted: ' + why + '.');
    }
  }, function () {
    show(false, 'Not posted: the host could not be reached. Try again.');
    tell(false, 'Not posted: the host could not be reached.');
  });
}
"#;

/// The Share control: makes a link, shows it to copy, and lists the live ones with Revoke.
const SHARE_SCRIPT: &str = r#"
var SHARE = __SHARE__;
var toggle = document.getElementById('share'), panel = document.getElementById('sharing');
var list = document.getElementById('links'), make = document.getElementById('make'), loaded = false;
function button(label, run) {
  var b = document.createElement('button');
  b.type = 'button'; b.textContent = label; b.addEventListener('click', run);
  return b;
}
function refused(what, a) {
  var why = a.body.error || 'the host answered ' + a.status;
  show(false, what + ': ' + why + '.', a.status === 401 ? signIn() : null, 'Sign in');
}
function empty() {
  if (list.children.length) return;
  var li = document.createElement('li');
  li.className = 'none'; li.textContent = 'No live links.';
  list.appendChild(li);
}
function row(link, fresh) {
  var none = list.querySelector('.none');
  if (none) none.remove();
  var li = document.createElement('li');
  var input = document.createElement('input');
  input.readOnly = true; input.value = link.url; input.setAttribute('aria-label', 'Share link');
  input.addEventListener('focus', function () { input.select(); });
  var copy = button('Copy', function () {
    function done() { copy.textContent = 'Copied'; setTimeout(function () { copy.textContent = 'Copy'; }, 1500); }
    input.select();
    if (navigator.clipboard) navigator.clipboard.writeText(link.url).then(done, function () { document.execCommand('copy'); done(); });
    else { document.execCommand('copy'); done(); }
  });
  var revoke = button('Revoke', function () {
    revoke.disabled = true;
    ask('POST', SHARE + '/' + encodeURIComponent(link.token) + '/revoke', {}).then(function (a) {
      if (a.status === 200 || a.status === 404) { li.remove(); empty(); }
      else { revoke.disabled = false; refused('Not revoked', a); }
    }, function () { revoke.disabled = false; show(false, 'Not revoked: the host could not be reached. Try again.'); });
  });
  var who = document.createElement('span');
  who.className = 'who';
  who.textContent = 'Made by @' + link.made_by + ', ' + new Date(link.made_at * 1000).toLocaleString();
  li.appendChild(input); li.appendChild(copy); li.appendChild(revoke); li.appendChild(who);
  if (fresh) { li.className = 'fresh'; list.insertBefore(li, list.firstChild); input.focus(); }
  else list.appendChild(li);
}
toggle.addEventListener('click', function () {
  var open = panel.hidden;
  panel.hidden = !open;
  toggle.setAttribute('aria-expanded', open ? 'true' : 'false');
  if (!open || loaded) return;
  loaded = true;
  ask('GET', SHARE).then(function (a) {
    if (a.status !== 200) { loaded = false; refused('Share links not shown', a); return; }
    a.body.links.forEach(function (link) { row(link, false); });
    empty();
  }, function () { loaded = false; show(false, 'Share links not shown: the host could not be reached.'); });
});
make.addEventListener('click', function () {
  make.disabled = true;
  ask('POST', SHARE, {}).then(function (a) {
    make.disabled = false;
    if (a.status === 201) row(a.body, true);
    else refused('No link made', a);
  }, function () { make.disabled = false; show(false, 'No link made: the host could not be reached. Try again.'); });
});
"#;

/// Opened through a share link: tells the page it only views, and where to sign in instead.
const VIEW_SCRIPT: &str = r#"
var PULL = __PULL__;
function act(kind, check, words, tell) {
  show(false, 'Not posted: this page was opened through a share link, which only views. To ' + kind + ', sign in on the pull request.', PULL, 'Open it signed in');
  tell(false, 'Not posted: opened through a share link, which only views.');
}
"#;

/// A JSON string inside a script: `<` escaped, so no value can end the script early.
fn js(text: &str) -> String {
    serde_json::to_string(text)
        .unwrap_or_default()
        .replace('<', "\\u003c")
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
