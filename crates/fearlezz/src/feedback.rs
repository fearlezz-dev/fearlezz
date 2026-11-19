//! What a person says on the proof host, as the agent reads it: `fearlezz setup login` signs the
//! CLI in through GitHub's device flow run by the host and keeps the host's token in the user's
//! config folder (never in the repository), readable by the user alone. With it, `fearlezz`
//! shows what waits on the host for the branch, `fearlezz --notes` saves each open note's
//! pictures with its region drawn into the cache folder, and the stop hook holds the agent while
//! notes are open. With it each `fearlezz run` also streams to the host as a local run
//! (`host.rs`), unproven. The token reads, and opens and streams the person's local runs; the
//! host refuses it any verdict, note, resolve or accept.
//!
//! Without a sign in nothing is asked of the host, so free users and repositories without the
//! host never wait on it.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use fearlezz_proof::notes::{Feedback, Noted, Side};
use fearlezz_proof::plural;
use fearlezz_proof::review::Standing;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::git::Git;
use crate::host::{self, Body, Limit};
use crate::media;

/// How long `fearlezz` waits for the host before it goes on without it.
const QUICK_SECONDS: u32 = 2;

/// One sign in to one host.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Login {
    pub login: String,
    pub token: String,
}

/// What the host says when a sign in starts.
#[derive(Deserialize)]
struct Codes {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

/// What the host says while the CLI waits for the person to enter the code.
#[derive(Deserialize)]
#[serde(untagged)]
enum Polled {
    /// `{"pending": true, "interval"}`: not yet; ask again after `interval` seconds.
    Pending { interval: u64 },
    /// `{"token", "login"}`: signed in.
    Token(Login),
}

/// The sign ins, by host URL, in the user's config folder.
fn logins_file() -> Result<PathBuf> {
    Ok(crate::files::UserDir::Config.path()?.join("logins.json"))
}

fn logins() -> BTreeMap<String, Login> {
    logins_file()
        .ok()
        .and_then(|f| fs::read(f).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Writes the sign ins readable and writable by the user alone.
fn keep(all: &BTreeMap<String, Login>) -> Result<()> {
    let file = logins_file()?;
    let dir = file.parent().context("no config folder")?;
    fs::create_dir_all(dir)?;
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
        options.mode(0o600);
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    let mut out = options
        .open(&file)
        .with_context(|| format!("writing {}", file.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        // A file made before with wider permissions keeps them through open.
        out.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    out.write_all(serde_json::to_string_pretty(all)?.as_bytes())?;
    Ok(())
}

/// The sign in to `url`, if there is one.
pub(crate) fn login_of(url: &str) -> Option<Login> {
    logins().remove(url)
}

/// `fearlezz setup login`: GitHub's device flow through the host at `url`. Prints the code to
/// enter and where, waits until the person did, and keeps the token.
pub(crate) fn login(url: &str) -> Result<()> {
    let (status, codes) = host::curl_within(
        "POST",
        &format!("{url}/api/v2/login/device"),
        "",
        &[],
        Body::Json(json!({})),
        Some(Limit::within(10)),
    )?;
    if status != 200 {
        bail!("the proof host at {url} did not start a sign in ({status}): {codes}");
    }
    let codes: Codes = serde_json::from_value(codes).context("the host's sign in did not read")?;
    println!(
        "open {} and enter {} to sign in to the proof host",
        codes.verification_uri, codes.user_code
    );
    std::io::stdout().flush()?;
    let mut interval = codes.interval;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(codes.expires_in);
    let body = json!({"device_code": codes.device_code});
    let signed = loop {
        if std::time::Instant::now() > deadline {
            bail!("the code expired before it was entered; run fearlezz setup login again");
        }
        std::thread::sleep(std::time::Duration::from_secs(interval));
        let (status, answer) = host::curl_within(
            "POST",
            &format!("{url}/api/v2/login/token"),
            "",
            &[],
            Body::Json(body.clone()),
            Some(Limit::within(10)),
        )?;
        if status != 200 {
            let why = answer
                .get("error")
                .and_then(Value::as_str)
                .map_or_else(|| answer.to_string(), str::to_owned);
            bail!("{why}");
        }
        match serde_json::from_value(answer).context("the host's answer did not read")? {
            Polled::Pending { interval: next } => interval = next,
            Polled::Token(login) => break login,
        }
    };
    let mut all = logins();
    all.insert(url.to_owned(), signed.clone());
    keep(&all)?;
    println!(
        "signed in as {} to {url}; each fearlezz run now shows there, unproven, so people can \
         leave notes before a pull request; the token never gives a verdict",
        signed.login
    );
    println!(
        "next: fearlezz run shows the run on the host; fearlezz setup logout stops sending runs"
    );
    Ok(())
}

/// `fearlezz setup logout`: ends the sign in to `url` on the host and forgets its token.
pub(crate) fn logout(url: &str) -> Result<()> {
    let mut all = logins();
    let Some(gone) = all.remove(url) else {
        println!("not signed in to {url}");
        println!("next: fearlezz setup login signs in to see notes from the proof host");
        return Ok(());
    };
    if let Err(e) = host::curl_within(
        "DELETE",
        &format!("{url}/api/v2/login"),
        &gone.token,
        &[],
        Body::None,
        Some(Limit::within(QUICK_SECONDS)),
    ) {
        eprintln!("the proof host did not hear the logout ({e:#}); its token expires on its own");
    }
    keep(&all)?;
    println!("signed out of {url}");
    println!("next: fearlezz setup login signs in again");
    Ok(())
}

/// What asking the host for the branch's work came to.
pub(crate) enum Read {
    /// Not signed in, not a GitHub repository, or the host has no such work: nothing to say.
    Nothing,
    /// The host answered.
    Feedback(Login, Box<Feedback>),
    /// Something to say in one line instead.
    Said(String),
}

/// What waits on the host at `url` for the branch checked out in `git`.
pub(crate) fn read(git: &Git, url: &str, seconds: Option<u32>) -> Read {
    let Some(login) = login_of(url) else {
        return Read::Nothing;
    };
    let (Some(repository), Ok(branch)) = (git.github_repository(), git.branch()) else {
        return Read::Nothing;
    };
    let branch: Vec<String> = branch.split('/').map(host::segment).collect();
    let path = format!("{url}/api/v2/work/{repository}/{}", branch.join("/"));
    match host::curl_within(
        "GET",
        &path,
        &login.token,
        &[],
        Body::None,
        seconds.map(Limit::within),
    ) {
        Err(_) => Read::Said(format!(
            "proof host: {url} could not be reached; going on without it"
        )),
        Ok((404, _)) => Read::Nothing,
        Ok((401, _)) => Read::Said(
            "proof host: the sign in expired; sign in again with fearlezz setup login".into(),
        ),
        Ok((200, answer)) => match serde_json::from_value::<Feedback>(answer) {
            Ok(feedback) => Read::Feedback(login, Box::new(feedback)),
            Err(e) => Read::Said(format!("proof host: its answer did not read ({e})")),
        },
        Ok((status, _)) => Read::Said(format!(
            "proof host: answered {status}; going on without it"
        )),
    }
}

/// What `fearlezz` says of the host, quickly: its lines, and how many notes are open.
pub(crate) fn status(git: &Git, url: &str) -> (Vec<String>, usize) {
    match read(git, url, Some(QUICK_SECONDS)) {
        Read::Nothing => (Vec::new(), 0),
        Read::Said(line) => (vec![line], 0),
        Read::Feedback(_, feedback) => (lines(&feedback), feedback.notes.len()),
    }
}

/// How many notes are open on the branch's work, for the stop hook; none when it cannot tell.
pub(crate) fn open_notes(git: &Git, url: &str) -> usize {
    match read(git, url, Some(QUICK_SECONDS)) {
        Read::Feedback(_, feedback) => feedback.notes.len(),
        Read::Nothing | Read::Said(_) => 0,
    }
}

/// The work, then each picture to look at with its verdict and open notes, then notes on other
/// pictures.
fn lines(feedback: &Feedback) -> Vec<String> {
    let work = &feedback.work;
    let mut head = format!("proof host: {}", work.branch);
    if let Some(pull) = &work.pull {
        head = format!("{head}, pull request #{}", pull.number);
    }
    let head = format!("{head}, {}", feedback.url);
    let mut out = vec![head];
    let open = |check: &str, picture: &str| {
        feedback
            .notes
            .iter()
            .filter(|n| n.note.check == check && n.note.picture == picture)
            .count()
    };
    let mut shown: Vec<(String, String)> = Vec::new();
    let width = feedback
        .review
        .iter()
        .flat_map(|r| &r.pictures)
        .map(|p| p.check.len() + p.name.len() + 1)
        .chain(
            feedback
                .notes
                .iter()
                .map(|n| n.note.check.len() + n.note.picture.len() + 1),
        )
        .max()
        .unwrap_or(0);
    if let Some(review) = &feedback.review {
        for p in &review.pictures {
            let said = match (p.standing, &p.verdict) {
                (Standing::NeedsChange, _) => "needs change".into(),
                (Standing::LooksRight, Some(v)) => format!("looks right, said by {}", v.by),
                (Standing::LooksRight, None) => "looks right".into(),
                (Standing::Waits, _) => "not looked at yet".into(),
            };
            let notes = open(&p.check, &p.name);
            let notes = if notes > 0 {
                format!(", {}", plural(notes, "open note"))
            } else {
                String::new()
            };
            let name = format!("{} {}", p.check, p.name);
            out.push(format!("  {name:<width$}  {said}{notes}"));
            shown.push((p.check.clone(), p.name.clone()));
        }
    }
    for n in &feedback.notes {
        let key = (n.note.check.clone(), n.note.picture.clone());
        if shown.contains(&key) {
            continue;
        }
        let name = format!("{} {}", key.0, key.1);
        out.push(format!(
            "  {name:<width$}  {}",
            plural(open(&key.0, &key.1), "open note")
        ));
        shown.push(key);
    }
    out
}

/// `fearlezz --notes`: every open note with its words, and its pictures saved with the region
/// drawn below `cache`, outside the repository. False when there is nothing to read.
pub(crate) fn notes(git: &Git, url: &str) -> Result<bool> {
    let (login, feedback) = match read(git, url, Some(10)) {
        Read::Feedback(login, feedback) => (login, feedback),
        Read::Said(line) => bail!("{line}"),
        Read::Nothing if login_of(url).is_none() => {
            println!("not signed in to the proof host at {url}, so its notes cannot be read");
            println!("next: fearlezz setup login");
            return Ok(false);
        }
        Read::Nothing => {
            println!("the proof host has nothing of this branch yet");
            println!(
                "next: fearlezz run shows the run there, and a person leaves notes on its page"
            );
            return Ok(true);
        }
    };
    let repository = feedback.work.repository.replace('/', "-");
    let branch: String = feedback
        .work
        .branch
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let dir = crate::files::UserDir::Cache
        .path()?
        .join("notes")
        .join(repository)
        .join(branch);
    // Each reading starts clean: a resolved note leaves nothing behind.
    match fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            return Err(e).with_context(|| format!("clearing {}", dir.display()));
        }
        _ => {}
    }
    fs::create_dir_all(dir.join("sources"))?;
    if feedback.notes.is_empty() {
        println!(
            "no open notes on {}: {}",
            feedback.work.branch, feedback.url
        );
        println!("next: fearlezz shows what else waits on the host");
        return Ok(true);
    }
    for (k, noted) in feedback.notes.iter().enumerate() {
        show(url, &login, &dir, k + 1, noted);
    }
    println!(
        "next: change the code so each note is settled, then fearlezz run; a person resolves the \
         notes on the page: {}",
        feedback.url
    );
    Ok(true)
}

/// Prints one open note, numbered `n`, and saves its pictures (or texts) in `dir`.
fn show(url: &str, login: &Login, dir: &Path, n: usize, noted: &Noted) {
    let note = &noted.note;
    let at = note
        .time
        .map(|t| format!(", at {t:.2} s"))
        .unwrap_or_default();
    println!(
        "note {n}  {}  {}  {}{at}",
        note.check,
        note.picture,
        note.side.label()
    );
    println!("  \"{}\"", note.words);
    println!(
        "  by {}, on the run of {}",
        note.by,
        fearlezz_proof::short(&note.head)
    );
    // A note on a text is about the whole text, saved as it is.
    let text = noted.kind == crate::look::TEXT;
    for (side, path) in [("after", &noted.after), ("before", &noted.before)] {
        let Some(path) = path else { continue };
        let drawn = !text
            && matches!(
                (side, note.side),
                (_, Side::Both) | ("after", Side::After) | ("before", Side::Before)
            );
        let out = dir.join(format!("{n}-{side}.{}", if text { "txt" } else { "png" }));
        match picture(
            url, login, &note.run, path, dir, n, side, noted, drawn, &out,
        ) {
            Ok(()) => {
                let what = if text {
                    format!("the {side} text")
                } else if drawn {
                    format!("the {side} picture, region {n} drawn")
                } else {
                    format!("the {side} picture, to compare")
                };
                println!("  {}  {what}", out.display());
            }
            Err(e) => {
                let what = if text { "text" } else { "picture" };
                println!("  the {side} {what} could not be saved: {e:#}");
            }
        }
    }
}

/// Fetches one picture of a note's run and saves it as a PNG at `out`, the region drawn when
/// `drawn`, a film's frame at the note's moment; a text is saved as it is.
#[expect(
    clippy::too_many_arguments,
    reason = "one picture of one note, said in full"
)]
fn picture(
    url: &str,
    login: &Login,
    run: &str,
    path: &str,
    dir: &Path,
    n: usize,
    side: &str,
    noted: &Noted,
    drawn: bool,
    out: &Path,
) -> Result<()> {
    let extension = path.rsplit_once('.').map_or("png", |(_, e)| e);
    let source = dir.join("sources").join(format!("{n}-{side}.{extension}"));
    let segments: Vec<String> = path.split('/').map(host::segment).collect();
    let file = format!(
        "{url}/api/v2/runs/{}/files/{}",
        host::segment(run),
        segments.join("/")
    );
    let status = host::download(&file, &login.token, &source)?;
    if status != 200 {
        bail!("the proof host answered {status}");
    }
    if noted.kind == crate::look::TEXT {
        fs::copy(&source, out)?;
        return Ok(());
    }
    let r = noted.note.region;
    let time = (noted.kind == "film").then(|| noted.note.time.unwrap_or(0.0));
    media::noted(&source, time, drawn.then_some([r.x, r.y, r.w, r.h]), out)
}
