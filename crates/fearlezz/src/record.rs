//! Recording an app's window from outside the app: a still (PNG) or a film (MP4) of one
//! window, found by pid, bundle id, app name or title, or as the window of an app a check's
//! command started. macOS only (ScreenCaptureKit); elsewhere every call says it cannot
//! record here.
//!
//! A check with `record` runs its command while Fearlezz waits for the window, takes the
//! still, films until the command ends (or `seconds` pass, then stops the command), and lists
//! what it recorded in `proof.jsonl`, so a check needs no script around the recorder.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as sys;
#[cfg(not(target_os = "macos"))]
mod stub;
#[cfg(not(target_os = "macos"))]
use stub as sys;

/// What part of the window a capture keeps.
#[cfg_attr(
    not(target_os = "macos"),
    expect(dead_code, reason = "only the macOS recorder reads it")
)]
pub(crate) struct Frame {
    /// Points cut from the top, such as the title bar, whose buttons change with focus and
    /// where macOS draws a badge while a film records. Zero keeps the whole window.
    pub crop_top: f64,
}

/// A window as ScreenCaptureKit lists it.
#[derive(Clone, Debug)]
#[cfg_attr(
    not(target_os = "macos"),
    expect(dead_code, reason = "only the macOS recorder reads it")
)]
pub(crate) struct WindowInfo {
    pub id: u32,
    pub pid: i32,
    pub bundle: String,
    pub app: String,
    pub title: String,
    pub width: f64,
    pub height: f64,
}

impl WindowInfo {
    fn area(&self) -> f64 {
        self.width * self.height
    }

    /// For example `"Counter" of LookDemo (pid 123)`.
    pub(crate) fn describe(&self) -> String {
        format!("\"{}\" of {} (pid {})", self.title, self.app, self.pid)
    }
}

/// Which window to record.
pub(crate) enum Pick<'a> {
    /// A pid, a bundle id, an app name, or part of a window title.
    Named(&'a str),
    /// A window of one of these processes: the ones a check's command started.
    Started(&'a BTreeSet<i32>),
}

impl Pick<'_> {
    fn matches(&self, w: &WindowInfo) -> bool {
        match self {
            Pick::Named(name) => match name.parse::<i32>() {
                Ok(pid) => w.pid == pid,
                Err(_) => {
                    w.bundle == *name
                        || w.app.eq_ignore_ascii_case(name)
                        || (!w.title.is_empty() && w.title.contains(name))
                }
            },
            Pick::Started(pids) => pids.contains(&w.pid),
        }
    }
}

/// The largest matching window, so palettes and menus do not win. Tiny windows (status items,
/// helpers) never count.
fn choose(windows: Vec<sys::Window>, pick: &Pick<'_>) -> Option<sys::Window> {
    windows
        .into_iter()
        .filter(|w| w.info.width > 50.0 && w.info.height > 50.0 && pick.matches(&w.info))
        .max_by(|a, b| a.info.area().total_cmp(&b.info.area()))
}

/// `record = { ... }` on a check.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Spec {
    /// A bundle id, app name or part of a window title; by default a window of an app the
    /// command started.
    pub window: Option<String>,
    /// The id of a still taken once the window is up.
    pub still: Option<String>,
    /// The id of a film from the window's appearance until the command ends.
    pub film: Option<String>,
    /// Stop after this long: the film ends and Fearlezz stops the command, which then counts
    /// as passed. For an app that runs until it is quit.
    pub seconds: Option<f64>,
    /// Points cut from the top (the title bar); off by default, since some apps have none.
    #[serde(default)]
    pub crop_top: f64,
}

impl Spec {
    pub(crate) fn parse(value: toml::Value) -> Result<Spec> {
        let spec: Spec = value.try_into().context("record")?;
        if spec.still.is_none() && spec.film.is_none() {
            bail!("record: name a still, a film or both, such as record = {{ film = \"app\" }}");
        }
        for id in spec.still.iter().chain(&spec.film) {
            if id.is_empty() || id.contains(['/', '\\']) || id.starts_with('.') {
                bail!("record: \"{id}\" is not a plain name; it becomes the file name");
            }
        }
        let bad = |x: f64| x.is_nan() || x < 0.0;
        if spec.seconds.is_some_and(|s| bad(s) || s == 0.0) || bad(spec.crop_top) {
            bail!("record: seconds must be above zero and crop_top zero or more");
        }
        Ok(spec)
    }

    /// A stable text of what it records, part of the check's inputs.
    pub(crate) fn fingerprint(&self) -> String {
        format!("{self:?}")
    }

    fn frame(&self) -> Frame {
        Frame {
            crop_top: self.crop_top,
        }
    }
}

/// Why this machine cannot record, and the one time fix.
pub(crate) struct Unavailable {
    pub why: String,
    pub fix: String,
}

/// None when this process may record windows; never prompts.
pub(crate) fn unavailable() -> Option<Unavailable> {
    if std::env::var_os("FEARLEZZ_RECORDER").is_some_and(|v| v == "off") {
        return Some(Unavailable {
            why: "the recorder is off (FEARLEZZ_RECORDER=off)".into(),
            fix: "unset FEARLEZZ_RECORDER on a Mac that may record the screen".into(),
        });
    }
    if cfg!(not(target_os = "macos")) {
        return Some(Unavailable {
            why: "recording a window needs macOS".into(),
            fix: "run it on a Mac; looks of macOS windows are proven on a developer's machine"
                .into(),
        });
    }
    if sys::allowed() {
        return None;
    }
    let app = responsible_app().unwrap_or_else(|| "the app that started the agent".into());
    Some(Unavailable {
        why: format!("{app} may not record the screen"),
        fix: format!(
            "once: System Settings, Privacy & Security, Screen & System Audio Recording, turn on \
             {app}, then quit and reopen it"
        ),
    })
}

/// The app macOS asks Screen Recording of: the nearest ancestor process inside an `.app`.
fn responsible_app() -> Option<String> {
    let mut pid = i32::try_from(std::process::id()).ok()?;
    for _ in 0..64 {
        let path = process_path(pid)?;
        if let Some((app, _)) = path.split_once(".app/Contents/MacOS/") {
            return app.rsplit('/').next().map(str::to_string);
        }
        pid = parent(pid)?;
        if pid <= 1 {
            return None;
        }
    }
    None
}

#[cfg(target_os = "macos")]
#[expect(unsafe_code, reason = "libproc has no safe wrapper in libc")]
fn process_path(pid: i32) -> Option<String> {
    let mut buf = vec![0_u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: the buffer is as large as the size passed.
    let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    (n > 0).then(|| String::from_utf8_lossy(&buf[..n as usize]).into_owned())
}

#[cfg(target_os = "macos")]
#[expect(unsafe_code, reason = "libproc has no safe wrapper in libc")]
fn parent(pid: i32) -> Option<i32> {
    // SAFETY: proc_bsdinfo is plain data, so zeroed is a valid value.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = size_of::<libc::proc_bsdinfo>() as i32;
    // SAFETY: the buffer is a proc_bsdinfo of the size passed.
    let n =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size) };
    (n == size).then_some(info.pbi_ppid as i32)
}

/// Every process below `root`, and `root` itself.
#[cfg(target_os = "macos")]
#[expect(unsafe_code, reason = "libproc has no safe wrapper in libc")]
fn descendants(root: i32) -> BTreeSet<i32> {
    let mut all = BTreeSet::from([root]);
    let mut todo = vec![root];
    while let Some(pid) = todo.pop() {
        let mut buf = vec![0_i32; 256];
        let bytes = (buf.len() * size_of::<i32>()) as i32;
        // SAFETY: the buffer holds as many bytes as passed.
        let n = unsafe { libc::proc_listchildpids(pid, buf.as_mut_ptr().cast(), bytes) };
        for child in buf.iter().take(n.max(0) as usize) {
            if *child > 0 && all.insert(*child) {
                todo.push(*child);
            }
        }
    }
    all
}

#[cfg(not(target_os = "macos"))]
fn process_path(_pid: i32) -> Option<String> {
    None
}

#[cfg(not(target_os = "macos"))]
fn parent(_pid: i32) -> Option<i32> {
    None
}

#[cfg(not(target_os = "macos"))]
fn descendants(root: i32) -> BTreeSet<i32> {
    BTreeSet::from([root])
}

/// A still taken on request, for an agent looking at its running app.
pub(crate) struct Still {
    pub path: PathBuf,
    pub window: WindowInfo,
    pub size: (usize, usize),
    pub took: Duration,
}

/// Takes a still of the window `name` names into a new file in `dir`.
pub(crate) fn still_of(name: &str, dir: &Path, crop_top: f64) -> Result<Still> {
    if let Some(u) = unavailable() {
        bail!("{}; {}", u.why, u.fix);
    }
    let clock = Instant::now();
    let window = choose(sys::windows()?, &Pick::Named(name)).with_context(|| {
        format!(
            "no window matches \"{name}\" (a pid, bundle id, app name or part of a title); is \
             the app running with a window open?"
        )
    })?;
    std::fs::create_dir_all(dir)?;
    let stamp = jiff::Timestamp::now().strftime("%Y%m%d-%H%M%S%.3f");
    let slug: String = window
        .info
        .app
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let path = dir.join(format!("{slug}-{stamp}.png"));
    let size = sys::still(&window, &path, &Frame { crop_top })?;
    Ok(Still {
        path,
        window: window.info,
        size,
        took: clock.elapsed(),
    })
}

/// How a recorded run of a check's command ended.
pub(crate) struct Recorded {
    /// The command's own exit, or None when Fearlezz stopped it after `seconds`.
    pub status: Option<ExitStatus>,
    /// Whether the recording got what the spec asks for.
    pub recorded: bool,
}

/// How often the command and window are looked at.
const POLL: Duration = Duration::from_millis(100);
/// How long a window that just appeared gets to draw its first frame.
const SETTLE: Duration = Duration::from_millis(700);

/// Runs alongside a check's command (`child`, leader of its own process group): waits for the
/// window, records it into `out`, and lists the files in `out/proof.jsonl`. Every step is
/// told in `log`.
pub(crate) fn during(
    child: &mut Child,
    spec: &Spec,
    out: &Path,
    log: &mut impl Write,
) -> Result<Recorded> {
    let clock = Instant::now();
    let root = i32::try_from(child.id())?;
    // Wait for the window while the command runs.
    let window = loop {
        if let Some(status) = child.try_wait()? {
            writeln!(
                log,
                "record: the command ended before {} showed a window",
                spec.window
                    .as_deref()
                    .map_or("an app it started".into(), |w| format!("\"{w}\""))
            )?;
            return Ok(Recorded {
                status: Some(status),
                recorded: false,
            });
        }
        let started = descendants(root);
        let pick = match &spec.window {
            Some(name) => Pick::Named(name),
            None => Pick::Started(&started),
        };
        if let Some(w) = choose(sys::windows()?, &pick) {
            break w;
        }
        std::thread::sleep(POLL);
    };
    writeln!(
        log,
        "record: found {} after {:.1} s",
        window.info.describe(),
        clock.elapsed().as_secs_f64()
    )?;
    std::thread::sleep(SETTLE);
    // The window may have taken its final size while it settled.
    let id = window.info.id;
    let window = sys::windows()?
        .into_iter()
        .find(|w| w.info.id == id)
        .unwrap_or(window);
    let frame = spec.frame();
    let mut proofs = Vec::new();
    let mut recorded = true;
    if let Some(name) = &spec.still {
        let file = format!("{name}.png");
        let took = Instant::now();
        match sys::still(&window, &out.join(&file), &frame) {
            Ok((w, h)) => {
                writeln!(
                    log,
                    "record: still {file} {w}x{h} in {:.2} s",
                    took.elapsed().as_secs_f64()
                )?;
                proofs.push(("still", file, name));
            }
            Err(e) => {
                writeln!(log, "record: still {file} failed: {e:#}")?;
                recorded = false;
            }
        }
    }
    let mut stopped = false;
    if let Some(name) = &spec.film {
        let file = format!("{name}.mp4");
        let path = out.join(&file);
        let mut film = sys::start_film(&window, &path, &frame)?;
        let filming = Instant::now();
        let deadline = spec.seconds.map(Duration::from_secs_f64);
        loop {
            if child.try_wait()?.is_some() || film.ended() {
                break;
            }
            if deadline.is_some_and(|d| filming.elapsed() >= d) {
                stopped = true;
                break;
            }
            std::thread::sleep(POLL);
        }
        let secs = filming.elapsed().as_secs_f64();
        match film.stop() {
            Ok(()) if path.exists() => {
                writeln!(log, "record: film {file} of {secs:.1} s")?;
                proofs.push(("film", file, name));
            }
            Ok(()) => {
                writeln!(log, "record: film {file} was not written")?;
                recorded = false;
            }
            Err(e) => {
                writeln!(log, "record: film {file} failed: {e:#}")?;
                recorded = false;
            }
        }
    } else if let Some(seconds) = spec.seconds {
        let until = Instant::now() + Duration::from_secs_f64(seconds);
        while Instant::now() < until && child.try_wait()?.is_none() {
            std::thread::sleep(POLL);
        }
        stopped = child.try_wait()?.is_none();
    }
    append_proofs(out, &proofs)?;
    let status = if stopped && child.try_wait()?.is_none() {
        writeln!(
            log,
            "record: stopping the command after {:.1} s",
            clock.elapsed().as_secs_f64()
        )?;
        stop(child)?;
        None
    } else {
        Some(child.wait()?)
    };
    Ok(Recorded { status, recorded })
}

fn append_proofs(out: &Path, proofs: &[(&str, String, &String)]) -> Result<()> {
    if proofs.is_empty() {
        return Ok(());
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(out.join("proof.jsonl"))?;
    for (kind, path, id) in proofs {
        let line = serde_json::json!({ "kind": kind, "file": path, "id": id });
        writeln!(file, "{line}")?;
    }
    Ok(())
}

/// Ends the command's process group: politely, then for sure.
#[cfg(unix)]
#[expect(
    unsafe_code,
    reason = "signals to a process group have no safe wrapper"
)]
pub(crate) fn stop(child: &mut Child) -> Result<()> {
    let group = -i32::try_from(child.id())?;
    // SAFETY: signalling the group the child leads, which Fearlezz created for it.
    unsafe { libc::kill(group, libc::SIGTERM) };
    let until = Instant::now() + Duration::from_secs(3);
    while Instant::now() < until {
        if child.try_wait()?.is_some() {
            return Ok(());
        }
        std::thread::sleep(POLL);
    }
    // SAFETY: as above.
    unsafe { libc::kill(group, libc::SIGKILL) };
    child.wait()?;
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn stop(child: &mut Child) -> Result<()> {
    child.kill()?;
    child.wait()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(pid: i32, app: &str, title: &str) -> WindowInfo {
        WindowInfo {
            id: 1,
            pid,
            bundle: format!("com.example.{app}"),
            app: app.into(),
            title: title.into(),
            width: 800.0,
            height: 600.0,
        }
    }

    #[test]
    fn a_name_matches_a_pid_a_bundle_an_app_or_part_of_a_title() {
        let w = info(42, "Demo", "Counter window");
        assert!(Pick::Named("42").matches(&w));
        assert!(Pick::Named("com.example.Demo").matches(&w));
        assert!(Pick::Named("demo").matches(&w));
        assert!(Pick::Named("Counter").matches(&w));
        assert!(!Pick::Named("Other").matches(&w));
        assert!(Pick::Started(&BTreeSet::from([1, 42])).matches(&w));
        assert!(!Pick::Started(&BTreeSet::from([1])).matches(&w));
    }

    #[test]
    fn a_record_names_what_it_records_as_plain_names() {
        let parse = |s: &str| Spec::parse(toml::from_str::<toml::Value>(s).unwrap());
        parse("film = \"app\"").unwrap();
        parse("window = \"x\"").unwrap_err();
        parse("still = \"../x\"").unwrap_err();
        parse("film = \"a\"\nseconds = 0").unwrap_err();
    }
}
