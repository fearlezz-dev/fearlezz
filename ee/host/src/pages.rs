//! The pages on the app origin: a proof's shell, which frames its bundle from the files origin
//! and holds the Approve and Accept controls in its own bar, and a pull request's latest whole
//! proof. Who sees them mirrors GitHub: anyone for a public
//! repository, people who may read it for a private one, and `404` for everyone else, so a
//! private repository's name is not confirmed.

use fearlezz_proof::Proof;
use fearlezz_proof::waits::{Kind, Role, Weakening};
use serde::Deserialize;
use worker::{Env, Headers, Request, Response};

use crate::session::{self, to_sign_in};
use crate::{Answer, Config, Refusal, approve, cache, crypto, files, github, repo_key};

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

/// The signed in person's role on the repository, as GitHub's collaborator permission says,
/// kept as long as who may read; `Less` when their token no longer works. It only decides which
/// controls the shell shows; approving, accepting, making and revoking a link ask GitHub again.
async fn role(
    env: &Env,
    config: &Config,
    person: &session::Session,
    owner: &str,
    name: &str,
) -> Result<Role, Refusal> {
    let key = format!(
        "role:{}:{}",
        person.login.to_lowercase(),
        repo_key(owner, name)
    );
    if let Some(cached) = cache::get(env, &key).await? {
        return Ok(Role::of(&cached));
    }
    let full = format!("{owner}/{name}");
    let Some(role) = github::permission(config, &full, &person.login, &person.token).await? else {
        return Ok(Role::Less);
    };
    let word = match role {
        Role::Admin => "admin",
        Role::Write => "write",
        Role::Less => "read",
    };
    cache::put(env, &key, word, SIGHT_SECONDS).await?;
    Ok(role)
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
    let proof: Proof = serde_json::from_str(&row.proof)
        .map_err(|e| worker::Error::RustError(format!("a stored proof does not read back: {e}")))?;
    let full = format!("{owner}/{name}");
    let path = format!("/{owner}/{name}/proofs/{id}/");
    let person = session::read(req, env).await?;
    let role = match &person {
        Some(person) => role(env, config, person, owner, name).await?,
        None => Role::Less,
    };
    let share = match row.pull {
        Some(pull) if role >= Role::Write => {
            Some(format!("/{owner}/{name}/pull/{}/share", pull as u64))
        }
        _ => None,
    };
    // What waits is read from the host's own copy of the proof, never from the page.
    let waits = match row.pull {
        Some(_) => Waits::of(&proof),
        None => Waits::default(),
    };
    let offer = match &person {
        _ if waits.is_empty() => Offer::Nothing,
        None => Offer::SignIn {
            next: path.clone(),
            kinds: waits.kinds(),
        },
        Some(person) => {
            // The first kind that waits names what they cannot do, when they can do nothing.
            let kind = waits.kinds()[0];
            let can = waits.allowed(role);
            if can.is_empty() {
                Offer::Cannot(format!(
                    "@{} cannot {} here: that needs {} {full} on GitHub.",
                    person.login,
                    kind.act(),
                    kind.needs_on()
                ))
            } else {
                Offer::Controls(can)
            }
        }
    };
    let view = match &person {
        Some(person) => Some(approve::view_token(config, &person.id, &repo, id).await?),
        None => None,
    };
    let view = View {
        title: title(&full, &row.proof),
        frame: format!("{}index.html", files::folder(config, &repo, id).await?),
        act: Act::Post {
            proof: path,
            share,
            view,
            offer,
        },
    };
    render(config, view)
}

/// What waits for a person in a proof of a pull request, as the shell offers it: one Approve per
/// check whose changed pictures wait (a fork's approval names its head; the host words that when
/// it posts), and one Accept below every weakening that waits, since one accept covers them all.
#[derive(Default)]
pub struct Waits {
    approve: Vec<String>,
    accept: Vec<Weakening>,
}

impl Waits {
    fn of(proof: &Proof) -> Waits {
        let obligations = proof.obligations();
        Waits {
            approve: obligations
                .checks()
                .into_iter()
                .map(str::to_owned)
                .collect(),
            accept: obligations.weakenings().into_iter().cloned().collect(),
        }
    }

    fn is_empty(&self) -> bool {
        self.approve.is_empty() && self.accept.is_empty()
    }

    fn kinds(&self) -> Vec<Kind> {
        let mut kinds = Vec::new();
        if !self.approve.is_empty() {
            kinds.push(Kind::Approve);
        }
        if !self.accept.is_empty() {
            kinds.push(Kind::Accept);
        }
        kinds
    }

    /// What a person with `role` may settle of it.
    fn allowed(self, role: Role) -> Waits {
        Waits {
            approve: if Kind::Approve.allows(role) {
                self.approve
            } else {
                Vec::new()
            },
            accept: if Kind::Accept.allows(role) {
                self.accept
            } else {
                Vec::new()
            },
        }
    }
}

/// What the shell's bar offers to settle.
pub enum Offer {
    /// Nothing waits.
    Nothing,
    /// Something waits and nobody is signed in: a way to sign in and come back.
    SignIn { next: String, kinds: Vec<Kind> },
    /// Something waits that the person may not settle: why, and nothing to press.
    Cannot(String),
    /// What the person may settle, each with a field for their own words.
    Controls(Waits),
}

/// What a shell shows: its title, the bundle page it frames, and what its bar offers to settle.
pub struct View {
    pub title: String,
    pub frame: String,
    pub act: Act,
}

pub enum Act {
    /// Opened at the proof's own address: the bar's own controls post to it with the session and
    /// the token of this view, and a person with write access gets the pull request's share links.
    Post {
        proof: String,
        share: Option<String>,
        /// The token of a signed in person's view, which a post must carry.
        view: Option<String>,
        offer: Offer,
    },
    /// Opened through a share link: the shell offers nothing to settle and never posts, and
    /// points to the pull request's own address for signing in.
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
/// scripts on its own origin, but never navigate the window away from the app. Approve and
/// Accept live in the bar, built from the host's own copy of the proof for the person viewing,
/// and only a press of one of them posts. The page, which the pull request's CI wrote, may send
/// the words it holds by message; the shell takes messages only from its frame on the files
/// origin, and a message only fills in the words of a control that is there and focuses it. It
/// never posts, and never adds a control. After a post the shell tells the page what came of it.
/// Opened by a share link it offers nothing to settle. A person with write access also gets a
/// Share control in the bar.
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
    let (bar, note) = match &view.act {
        Act::Post { offer, .. } => bar(offer),
        Act::ViewOnly { .. } => (
            String::new(),
            "this page was opened through a share link, which only views".to_owned(),
        ),
    };
    let script = match &view.act {
        Act::Post {
            proof, share, view, ..
        } => {
            let mut script = POST_SCRIPT
                .replace("__PROOF__", &js(proof))
                .replace("__VIEW__", &js(view.as_deref().unwrap_or_default()));
            if let Some(share) = share {
                script.push_str(&SHARE_SCRIPT.replace("__SHARE__", &js(share)));
            }
            script
        }
        Act::ViewOnly { pull } => VIEW_SCRIPT.replace("__PULL__", &js(pull)),
    };
    let note = js(&note);
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
{bar}
<p id="said" role="status" hidden></p>
<iframe src="{frame}" sandbox="allow-scripts allow-same-origin" title="{title}"></iframe>
<script nonce="{nonce}">
(function () {{
var FILES = {files}, NOTE = {note};
var frame = document.querySelector('iframe'), said = document.getElementById('said');
var calm = window.matchMedia('(prefers-reduced-motion: reduce)');
var acts = Array.prototype.slice.call(document.querySelectorAll('form[data-kind]'));
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
function tell(m) {{ frame.contentWindow.postMessage(m, FILES); }}
// The control for what the page names, if the bar has one: a message can only find one.
function control(kind, check) {{
  return acts.filter(function (f) {{
    return f.dataset.kind === kind && (kind === 'accept' || f.dataset.check === check);
  }})[0] || null;
}}
// A message from the page may fill in words and focus; it never posts.
window.addEventListener('message', function (e) {{
  if (e.source !== frame.contentWindow || e.origin !== FILES) return;
  var m = e.data || {{}};
  if (m.kind !== 'fearlezz:fill' || (m.of !== 'approve' && m.of !== 'accept')) return;
  var check = typeof m.check === 'string' ? m.check : null;
  var form = control(m.of, check);
  if (!form) {{
    if (NOTE) tell({{ kind: 'fearlezz:filled', of: m.of, check: check, ok: false, text: 'Not here: ' + NOTE + '.' }});
    return;
  }}
  var input = form.querySelector('[name=words]');
  if (typeof m.words === 'string' && m.words.trim() && !input.disabled) input.value = m.words.slice(0, 500);
  form.scrollIntoView({{ block: 'nearest', behavior: calm.matches ? 'auto' : 'smooth' }});
  input.focus();
  form.classList.remove('filled'); void form.offsetWidth; form.classList.add('filled');
  tell({{ kind: 'fearlezz:filled', of: m.of, check: check, ok: true, text: 'Your words are in the bar above. Press ' + form.dataset.label + ' there to post them as you.' }});
}});
{script}
}})();
</script>
</body>
</html>
"#
    )
}

/// The bar's Approve and Accept for what `offer` lets the person settle, and what the shell
/// tells the page when it names something the bar has no control for.
fn bar(offer: &Offer) -> (String, String) {
    match offer {
        Offer::Nothing => (String::new(), String::new()),
        Offer::SignIn { next, kinds } => {
            let what = kinds
                .iter()
                .map(|k| k.word())
                .collect::<Vec<_>>()
                .join(" or ");
            let link = format!("/login?next={}", next.replace('/', "%2F"));
            (
                format!(
                    r#"<section id="waits" aria-label="Waits for you"><p class="hint">Something here waits for a person. <a href="{}">Sign in to {what}</a></p></section>"#,
                    escape(&link)
                ),
                format!("sign in above to {what}"),
            )
        }
        Offer::Cannot(why) => (
            format!(
                r#"<section id="waits" aria-label="Waits for you"><p class="hint">{}</p></section>"#,
                prose(why)
            ),
            why.trim_end_matches('.').to_owned(),
        ),
        Offer::Controls(waits) => {
            let mut html = String::from(r#"<section id="waits" aria-label="Waits for you">"#);
            for check in &waits.approve {
                let check = escape(check);
                html.push_str(&format!(
                    r#"<form class="act" data-kind="approve" data-check="{check}" data-label="Approve"><span class="what">Pictures of <code>{check}</code></span><input name="words" required maxlength="500" autocomplete="off" placeholder="{hint}" aria-label="Your words on the pictures of {check}"><button type="submit">Approve</button></form>"#,
                    hint = Kind::Approve.hint(),
                ));
            }
            if !waits.accept.is_empty() {
                let label = match waits.accept.len() {
                    1 => "Accept".to_owned(),
                    2 => "Accept both".to_owned(),
                    n => format!("Accept all {n}"),
                };
                html.push_str(&format!(
                    r#"<form class="act" data-kind="accept" data-label="{label}"><ul>"#
                ));
                for weakening in &waits.accept {
                    html.push_str(&format!(
                        "<li>{}</li>",
                        prose(&sentence(&weakening.to_string()))
                    ));
                }
                html.push_str(&format!(
                    r#"</ul><input name="words" required maxlength="500" autocomplete="off" placeholder="{hint}" aria-label="Why the weaker proof is right"><button type="submit">{label}</button></form>"#,
                    hint = Kind::Accept.hint(),
                ));
            }
            html.push_str("</section>");
            (html, String::new())
        }
    }
}

/// Text with a capital first letter and a full stop, as a sentence.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    let mut out: String = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    };
    if !out.ends_with('.') {
        out.push('.');
    }
    out
}

/// Escaped text whose `backticked` names read as code.
fn prose(text: &str) -> String {
    escape(text)
        .split('`')
        .enumerate()
        .map(|(i, part)| {
            if i % 2 == 1 {
                format!("<code>{part}</code>")
            } else {
                part.to_owned()
            }
        })
        .collect()
}

/// The shell's look: the colours of the core's pages (`crates/fearlezz/src/page.rs`), small, with
/// borders all the way round, calm under reduced motion.
const SHELL_STYLE: &str = r#"
:root { --surface:#e7eaee; --mat:#fff; --sunk:#dce0e5; --ink:#121417; --quiet:#58606c; --line:#cdd3da;
  --ok:#1d7a4a; --on-ok:#fff; --no:#c4532c; --focus:#1f5fd1; --ease:cubic-bezier(.2,.8,.2,1); color-scheme: light dark; }
@media (prefers-color-scheme: dark) {
  :root { --surface:#101318; --mat:#1a1e25; --sunk:#252a33; --ink:#eceef2; --quiet:#a3aab5; --line:#323943;
    --ok:#4fd08a; --on-ok:#0c1a12; --no:#ff7a52; --focus:#7aa7ff; }
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
#waits { margin: 2px 12px 6px; display: grid; gap: 6px; color: var(--ink); font-size: 13px; animation: rise .3s var(--ease) both; }
#waits .hint { margin: 0; padding: 6px 12px; border: 1px solid var(--line); border-radius: 10px; background: var(--mat); color: var(--quiet); }
#waits .hint a { color: var(--ink); font-weight: 600; }
#waits code { font: 12px ui-monospace, SFMono-Regular, Menlo, monospace; background: var(--sunk); border-radius: 5px; padding: 0 4px; }
#waits .act { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; padding: 8px 10px 8px 12px;
  border: 1px solid var(--line); border-radius: 10px; background: var(--mat); transition: border-color .3s, box-shadow .3s; }
#waits .what { font-weight: 600; white-space: nowrap; }
#waits ul { flex-basis: 100%; margin: 0; padding: 0 0 0 18px; color: var(--quiet); }
#waits input { flex: 1 1 240px; min-width: 0; font: inherit; color: var(--ink); background: var(--surface);
  border: 1px solid var(--line); border-radius: 7px; padding: 4px 10px; transition: border-color .2s; }
#waits input:focus { border-color: var(--focus); outline: none; }
#waits button { background: var(--ok); border-color: var(--ok); color: var(--on-ok); font-weight: 600; padding: 4px 16px; }
#waits button:hover { border-color: var(--ink); }
#waits .act.filled { animation: filled 1.2s var(--ease); }
#waits .act.sent { border-color: var(--ok); }
#waits .act.sent input { opacity: .6; }
@keyframes filled { 0% { box-shadow: 0 0 0 0 color-mix(in srgb, var(--focus) 45%, transparent); }
  100% { box-shadow: 0 0 0 10px transparent; } }
iframe { flex: 1; width: 100%; border: 0; }
@keyframes rise { from { opacity: 0; transform: translateY(-4px); } to { opacity: 1; transform: none; } }
@media (prefers-reduced-motion: reduce) { *, *::before, *::after { animation-duration: .01ms !important; transition-duration: .01ms !important; } }
"#;

/// At the proof's own address: a press of the bar's own Approve or Accept posts its words to the
/// host with the session and the token of this view, shows what came of it, and tells the page.
const POST_SCRIPT: &str = r#"
var PROOF = __PROOF__;
var VIEW = __VIEW__;
function ask(method, path, body) {
  var headers = { 'Fearlezz-View': VIEW };
  if (body) headers['Content-Type'] = 'application/json';
  return fetch(path, {
    method: method, credentials: 'same-origin', headers: headers,
    body: body ? JSON.stringify(body) : undefined
  }).then(function (r) {
    return r.json().catch(function () { return {}; }).then(function (b) { return { status: r.status, body: b }; });
  });
}
function signIn() { return '/login?next=' + encodeURIComponent(location.pathname); }
acts.forEach(function (form) {
  var kind = form.dataset.kind, check = form.dataset.check || null;
  var input = form.querySelector('[name=words]'), button = form.querySelector('button');
  function told(ok, text) { tell({ kind: 'fearlezz:posted', of: kind, check: check, ok: ok, text: text }); }
  function again(text) {
    show(false, 'Not posted: ' + text + '.');
    told(false, 'Not posted: ' + text + '.');
    button.disabled = false; button.textContent = form.dataset.label;
  }
  form.addEventListener('submit', function (e) {
    e.preventDefault();
    var words = input.value.trim();
    if (!words || button.disabled) { input.focus(); return; }
    button.disabled = true; button.textContent = 'Posting';
    ask('POST', PROOF + kind, { check: check, words: words }).then(function (a) {
      if (a.status === 201) {
        show(true, 'Posted on pull request #' + a.body.pull + ' as @' + a.body.login + ': ' + a.body.comment + '.', a.body.url, 'Open the comment');
        told(true, 'Posted on the pull request as @' + a.body.login + '. CI counts it now.');
        form.classList.add('sent'); input.disabled = true; button.textContent = 'Posted';
        return;
      }
      var why = a.body.error || 'the host answered ' + a.status;
      again(why);
      if (a.status === 401) show(false, 'Not posted: ' + why + '.', signIn(), 'Sign in');
    }, function () { again('the host could not be reached. Try again'); });
  });
});
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

/// Opened through a share link: when the page offers words, says the link only views, and where to
/// sign in instead.
const VIEW_SCRIPT: &str = r#"
var PULL = __PULL__;
window.addEventListener('message', function (e) {
  if (e.source !== frame.contentWindow || e.origin !== FILES) return;
  if ((e.data || {}).kind !== 'fearlezz:fill') return;
  show(false, 'This page was opened through a share link, which only views. To approve or accept, sign in on the pull request.', PULL, 'Open it signed in');
});
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
