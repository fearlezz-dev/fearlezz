//! Recording an app's window from outside the app: a still (PNG) or a film (MP4) of one
//! window, found by pid, bundle id, app name or title, or as the window of an app a check's
//! command started. macOS only (ScreenCaptureKit); elsewhere every call says it cannot
//! record here.
//!
//! A check with `record` runs its command while Fearlezz waits for a window that appeared
//! after the command started, takes the still, films until the command ends (or `seconds`
//! pass, then stops the command), and lists what it recorded in `proof.jsonl`, so a check
//! needs no script around the recorder. The command never runs longer than `wait` without a
//! window to record. The decisions run against a [`Screen`], so tests script the windows.

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
    Started(BTreeSet<i32>),
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

/// The largest matching window not in `skip`, so palettes and menus do not win. Tiny windows
/// (status items, helpers) never count.
fn choose<'w>(
    windows: &'w [WindowInfo],
    pick: &Pick<'_>,
    skip: &BTreeSet<u32>,
) -> Option<&'w WindowInfo> {
    windows
        .iter()
        .filter(|w| w.width > 50.0 && w.height > 50.0 && !skip.contains(&w.id) && pick.matches(w))
        .max_by(|a, b| a.area().total_cmp(&b.area()))
}

/// The windows of the machine, as the recorder needs them: ScreenCaptureKit on macOS, a script
/// in tests. Windows are named by their id from the latest [`Screen::windows`].
pub(crate) trait Screen {
    type Film: Filming;
    /// Every normal window of every app.
    fn windows(&mut self) -> Result<Vec<WindowInfo>>;
    /// Writes a still of the window to `path` (PNG) and returns its size in pixels.
    fn still(&mut self, window: u32, path: &Path, frame: &Frame) -> Result<(usize, usize)>;
    /// Starts filming the window into `path` (MP4).
    fn film(&mut self, window: u32, path: &Path, frame: &Frame) -> Result<Self::Film>;
}

/// A film being recorded.
pub(crate) trait Filming {
    /// Whether it ended by itself, such as when the window closed.
    fn ended(&mut self) -> bool;
    /// Stops it and waits until its file is written.
    fn stop(self) -> Result<()>;
}

/// The machine's own screen.
#[derive(Default)]
pub(crate) struct System {
    listed: Vec<sys::Window>,
}

impl System {
    fn find(&self, id: u32) -> Result<&sys::Window> {
        self.listed
            .iter()
            .find(|w| w.info.id == id)
            .with_context(|| format!("window {id} is gone"))
    }
}

impl Screen for System {
    type Film = sys::Film;

    fn windows(&mut self) -> Result<Vec<WindowInfo>> {
        self.listed = sys::windows()?;
        Ok(self.listed.iter().map(|w| w.info.clone()).collect())
    }

    fn still(&mut self, window: u32, path: &Path, frame: &Frame) -> Result<(usize, usize)> {
        sys::still(self.find(window)?, path, frame)
    }

    fn film(&mut self, window: u32, path: &Path, frame: &Frame) -> Result<sys::Film> {
        sys::start_film(self.find(window)?, path, frame)
    }
}

/// `record = { ... }` on a check.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Spec {
    /// A bundle id, app name or part of a window title; by default a window of an app the
    /// command started. Either way only a window that appeared after the command started.
    pub window: Option<String>,
    /// The id of a still taken once the window is up.
    pub still: Option<String>,
    /// The id of a film from the window's appearance until the command ends.
    pub film: Option<String>,
    /// Stop after this long: the film ends and Fearlezz stops the command, which then counts
    /// as passed. For an app that runs until it is quit.
    pub seconds: Option<f64>,
    /// The longest the command may run without a window to record: before its window appears,
    /// and after it closed while the command keeps running. Then Fearlezz stops the command
    /// and the check fails. [`DEFAULT_WAIT`] when not set.
    pub wait: Option<f64>,
    /// Points cut from the top (the title bar); off by default, since some apps have none.
    #[serde(default)]
    pub crop_top: f64,
}

/// Long enough for a first build of an app before its window appears.
pub(crate) const DEFAULT_WAIT: Duration = Duration::from_mins(5);

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
        if spec.window.as_ref().is_some_and(|w| w.trim().is_empty()) {
            bail!(
                "record: window names no window; leave it out to record the app the command \
                 starts, or name its bundle id, app name or part of its title"
            );
        }
        for (field, value) in [("seconds", spec.seconds), ("wait", spec.wait)] {
            // A Duration holds every value that passes, so recording never overflows.
            if let Some(s) = value
                && (s <= 0.0 || Duration::try_from_secs_f64(s).is_err())
            {
                bail!("record: {field} = {s} must be a number of seconds above zero");
            }
        }
        if !spec.crop_top.is_finite() || spec.crop_top < 0.0 {
            bail!("record: crop_top must be zero or more");
        }
        Ok(spec)
    }

    fn wait(&self) -> Duration {
        self.wait.map_or(DEFAULT_WAIT, Duration::from_secs_f64)
    }

    fn seconds(&self) -> Option<Duration> {
        self.seconds.map(Duration::from_secs_f64)
    }

    /// What the recorder waits for, for the log.
    fn awaited(&self) -> String {
        self.window
            .as_deref()
            .map_or("an app the command started".into(), |w| format!("\"{w}\""))
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

/// How many stills on request a folder keeps: they are for looking at once, not for proof.
pub(crate) const KEEP_STILLS: usize = 20;

/// Takes a still of the window `name` names into a new file in `dir`, keeping only the
/// [`KEEP_STILLS`] newest stills there.
pub(crate) fn still_of(name: &str, dir: &Path, crop_top: f64) -> Result<Still> {
    if name.trim().is_empty() {
        bail!("name the window: a pid, bundle id, app name or part of a title");
    }
    if let Some(u) = unavailable() {
        bail!("{}; {}", u.why, u.fix);
    }
    let clock = Instant::now();
    let mut screen = System::default();
    let windows = screen.windows()?;
    let window = choose(&windows, &Pick::Named(name), &BTreeSet::new()).with_context(|| {
        format!(
            "no window matches \"{name}\" (a pid, bundle id, app name or part of a title); is \
             the app running with a window open?"
        )
    })?;
    std::fs::create_dir_all(dir)?;
    let stamp = jiff::Timestamp::now().strftime("%Y%m%d-%H%M%S%.3f");
    let slug: String = window
        .app
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let path = dir.join(format!("{slug}-{stamp}.png"));
    let size = screen.still(window.id, &path, &Frame { crop_top })?;
    keep_newest(dir, KEEP_STILLS)?;
    Ok(Still {
        path,
        window: window.clone(),
        size,
        took: clock.elapsed(),
    })
}

/// Removes all but the `keep` most recently written PNGs in `dir`.
fn keep_newest(dir: &Path, keep: usize) -> Result<()> {
    let mut pngs = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "png") {
            pngs.push((std::fs::metadata(&path)?.modified()?, path));
        }
    }
    pngs.sort();
    for (_, old) in pngs.iter().rev().skip(keep) {
        std::fs::remove_file(old).with_context(|| format!("removing {}", old.display()))?;
    }
    Ok(())
}

/// How a recorded run of a check's command ended.
#[derive(Debug)]
pub(crate) enum Ended {
    /// The command exited by itself.
    Exited(ExitStatus),
    /// Fearlezz stopped it after `seconds`, as the spec asks: the app ran until then.
    AfterSeconds,
    /// Fearlezz stopped it after it ran `wait` without a window to record.
    WaitedTooLong,
}

/// How a recorded run of a check's command went.
#[derive(Debug)]
pub(crate) struct Recorded {
    pub ended: Ended,
    /// Whether the recording got what the spec asks for.
    pub recorded: bool,
}

impl Recorded {
    pub(crate) fn passed(&self) -> bool {
        self.recorded
            && match &self.ended {
                Ended::Exited(status) => status.success(),
                Ended::AfterSeconds => true,
                Ended::WaitedTooLong => false,
            }
    }
}

/// How often the command and window are looked at.
const POLL: Duration = Duration::from_millis(100);
/// How often windows are listed while the recorder only waits for the command to end.
const LIST_EVERY: Duration = Duration::from_secs(1);
/// How long a window that just appeared gets to draw its first frame.
const SETTLE: Duration = Duration::from_millis(700);
/// How many listings in a row may fail after the recording before the run fails.
const LISTING_TRIES: usize = 3;

/// Records the window of one run of a check's command.
pub(crate) struct Recorder<'a, S> {
    screen: S,
    spec: &'a Spec,
    /// The windows open before the command started, which are never recorded: an app the
    /// developer has open would otherwise be filmed on both sides of a look.
    before: BTreeSet<u32>,
}

impl<'a, S: Screen> Recorder<'a, S> {
    /// Notes the windows open now; call it before starting the command.
    pub(crate) fn new(mut screen: S, spec: &'a Spec) -> Result<Self> {
        let before = screen.windows()?.iter().map(|w| w.id).collect();
        Ok(Recorder {
            screen,
            spec,
            before,
        })
    }

    /// Runs alongside the check's command (`child`, leader of its own process group): waits
    /// for a new window, records it into `out`, lists the files in `out/proof.jsonl`, and
    /// waits for the command to end. Every step is told in `log`.
    pub(crate) fn during(
        &mut self,
        child: &mut Child,
        out: &Path,
        log: &mut impl Write,
    ) -> Result<Recorded> {
        let spec = self.spec;
        let root = i32::try_from(child.id())?;
        let window = match self.appear(child, root, log)? {
            Ok(window) => window,
            Err(ended) => {
                return Ok(Recorded {
                    ended,
                    recorded: false,
                });
            }
        };
        std::thread::sleep(SETTLE);
        // The window may have taken its final size while it settled; listing again also
        // hands the screen the window as it is now.
        let window = self
            .screen
            .windows()?
            .into_iter()
            .find(|w| w.id == window.id)
            .unwrap_or(window);
        let recording = Instant::now();
        // A deadline past what an Instant holds is no deadline.
        let deadline = spec.seconds().and_then(|s| recording.checked_add(s));
        let frame = spec.frame();
        let mut proofs = Vec::new();
        let mut recorded = true;
        if let Some(name) = &spec.still {
            let file = format!("{name}.png");
            let took = Instant::now();
            match self.screen.still(window.id, &out.join(&file), &frame) {
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
        if let Some(name) = &spec.film {
            let file = format!("{name}.mp4");
            let path = out.join(&file);
            let mut film = self.screen.film(window.id, &path, &frame)?;
            while child.try_wait()?.is_none()
                && !film.ended()
                && deadline.is_none_or(|d| Instant::now() < d)
            {
                std::thread::sleep(POLL);
            }
            let secs = recording.elapsed().as_secs_f64();
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
        }
        append_proofs(out, &proofs)?;
        let ended = self.finish(child, root, window.id, deadline, log)?;
        Ok(Recorded { ended, recorded })
    }

    /// Waits for a window of the pick that was not open before the command started, or says
    /// how the run ended without one.
    fn appear(
        &mut self,
        child: &mut Child,
        root: i32,
        log: &mut impl Write,
    ) -> Result<Result<WindowInfo, Ended>> {
        let spec = self.spec;
        let clock = Instant::now();
        loop {
            if let Some(status) = child.try_wait()? {
                writeln!(
                    log,
                    "record: the command ended before {} showed a window",
                    spec.awaited()
                )?;
                return Ok(Err(Ended::Exited(status)));
            }
            let windows = self.screen.windows()?;
            if let Some(w) = choose(&windows, &self.pick(root), &self.before) {
                writeln!(
                    log,
                    "record: found {} after {:.1} s",
                    w.describe(),
                    clock.elapsed().as_secs_f64()
                )?;
                return Ok(Ok(w.clone()));
            }
            if clock.elapsed() >= spec.wait() {
                writeln!(
                    log,
                    "record: no new window of {} appeared in {} s; stopping the command",
                    spec.awaited(),
                    spec.wait().as_secs_f64()
                )?;
                if let Some(old) = choose(&windows, &self.pick(root), &BTreeSet::new()) {
                    writeln!(
                        log,
                        "record: {} was already open before the command started; only a \
                         window that appears while the command runs is recorded, so quit the \
                         app first",
                        old.describe()
                    )?;
                }
                stop(child)?;
                return Ok(Err(Ended::WaitedTooLong));
            }
            std::thread::sleep(POLL);
        }
    }

    fn pick(&self, root: i32) -> Pick<'a> {
        match &self.spec.window {
            Some(name) => Pick::Named(name),
            None => Pick::Started(descendants(root)),
        }
    }

    /// Waits for the command to end after the recording: stops it at the `seconds` deadline,
    /// or once it ran `wait` with neither the recorded window nor a later one of the pick
    /// open. A later window is told once and never recorded. A listing may fail a few times
    /// in a row (it is a system call with a timeout) before the run fails.
    fn finish(
        &mut self,
        child: &mut Child,
        root: i32,
        recorded: u32,
        deadline: Option<Instant>,
        log: &mut impl Write,
    ) -> Result<Ended> {
        let wait = self.spec.wait();
        let mut told = self.before.clone();
        told.insert(recorded);
        let mut window_at = Instant::now();
        let mut listed_at: Option<Instant> = None;
        let mut failed = 0;
        loop {
            if let Some(status) = child.try_wait()? {
                return Ok(Ended::Exited(status));
            }
            if deadline.is_some_and(|d| Instant::now() >= d) {
                writeln!(log, "record: stopping the command after `seconds`")?;
                stop(child)?;
                return Ok(Ended::AfterSeconds);
            }
            if listed_at.is_none_or(|at| at.elapsed() >= LIST_EVERY) {
                listed_at = Some(Instant::now());
                let windows = match self.screen.windows() {
                    Ok(windows) => {
                        failed = 0;
                        windows
                    }
                    Err(e) if failed + 1 < LISTING_TRIES => {
                        failed += 1;
                        writeln!(
                            log,
                            "record: listing the windows failed, trying again: {e:#}"
                        )?;
                        std::thread::sleep(POLL);
                        continue;
                    }
                    Err(e) => return Err(e),
                };
                let pick = self.pick(root);
                if let Some(new) = choose(&windows, &pick, &told) {
                    writeln!(
                        log,
                        "record: {} appeared after the recorded window; a record keeps only the \
                         first window of a run",
                        new.describe()
                    )?;
                    told.insert(new.id);
                }
                let open = windows.iter().any(|w| w.id == recorded)
                    || choose(&windows, &pick, &self.before).is_some();
                if open {
                    window_at = Instant::now();
                } else if window_at.elapsed() >= wait {
                    writeln!(
                        log,
                        "record: the command ran {} s after its windows closed without ending; \
                         stopping it",
                        wait.as_secs_f64()
                    )?;
                    stop(child)?;
                    return Ok(Ended::WaitedTooLong);
                }
            }
            std::thread::sleep(POLL);
        }
    }
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
        assert!(Pick::Started(BTreeSet::from([1, 42])).matches(&w));
        assert!(!Pick::Started(BTreeSet::from([1])).matches(&w));
    }

    #[test]
    fn a_record_names_what_it_records_as_plain_names() {
        let parse = |s: &str| Spec::parse(toml::from_str::<toml::Value>(s).unwrap());
        parse("film = \"app\"").unwrap();
        parse("window = \"x\"").unwrap_err();
        parse("still = \"../x\"").unwrap_err();
        parse("film = \"a\"\nseconds = 0").unwrap_err();
        parse("film = \"a\"\nwait = 0").unwrap_err();
        parse("film = \"a\"\nwindow = \"\"").unwrap_err();
        // Longer than a Duration holds: a config error, not a panic while recording.
        let huge = parse("film = \"a\"\nwait = 1e20").unwrap_err();
        assert!(format!("{huge:#}").contains("wait"), "{huge:#}");
        parse("film = \"a\"\nseconds = 1e20").unwrap_err();
    }

    #[test]
    fn only_the_newest_stills_on_request_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..5 {
            std::fs::write(dir.path().join(format!("{i}.png")), "x").unwrap();
            // Distinct modification times, oldest first.
            std::thread::sleep(Duration::from_millis(20));
        }
        std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
        keep_newest(dir.path(), 2).unwrap();
        let mut left: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["3.png", "4.png", "notes.txt"]);
    }

    /// The pid that stands for the command's own process in a script.
    const CHILD: i32 = -1;

    /// A window from `from` until `until` seconds after the screen was made.
    struct Shown {
        from: f64,
        until: f64,
        window: WindowInfo,
    }

    /// A scripted screen: windows come and go on a clock; stills and films write small files.
    struct Script {
        start: Instant,
        shown: Vec<Shown>,
        child: i32,
        /// Listings from this many seconds on fail, `failures` times in a row.
        fail_from: f64,
        failures: usize,
    }

    struct ScriptFilm {
        start: Instant,
        until: f64,
        path: PathBuf,
    }

    impl Filming for ScriptFilm {
        fn ended(&mut self) -> bool {
            self.start.elapsed().as_secs_f64() >= self.until
        }

        fn stop(self) -> Result<()> {
            std::fs::write(&self.path, "film")?;
            Ok(())
        }
    }

    impl Screen for Script {
        type Film = ScriptFilm;

        fn windows(&mut self) -> Result<Vec<WindowInfo>> {
            let now = self.start.elapsed().as_secs_f64();
            if now >= self.fail_from && self.failures > 0 {
                self.failures -= 1;
                bail!("the listing timed out");
            }
            Ok(self
                .shown
                .iter()
                .filter(|s| s.from <= now && now < s.until)
                .map(|s| {
                    let mut w = s.window.clone();
                    if w.pid == CHILD {
                        w.pid = self.child;
                    }
                    w
                })
                .collect())
        }

        fn still(&mut self, _window: u32, path: &Path, _frame: &Frame) -> Result<(usize, usize)> {
            std::fs::write(path, "still")?;
            Ok((800, 600))
        }

        fn film(&mut self, window: u32, path: &Path, _frame: &Frame) -> Result<ScriptFilm> {
            let until = self
                .shown
                .iter()
                .find(|s| s.window.id == window)
                .map_or(0.0, |s| s.until);
            Ok(ScriptFilm {
                start: self.start,
                until,
                path: path.to_path_buf(),
            })
        }
    }

    fn shown(id: u32, pid: i32, from: f64, until: f64) -> Shown {
        let mut window = info(pid, "Demo", "Demo window");
        window.id = id;
        Shown {
            from,
            until,
            window,
        }
    }

    struct Run {
        recorded: Recorded,
        log: String,
        out: tempfile::TempDir,
        took: Duration,
        /// Whether the command still ran afterwards.
        left_running: bool,
    }

    impl Run {
        fn proofs(&self) -> String {
            std::fs::read_to_string(self.out.path().join("proof.jsonl")).unwrap_or_default()
        }
    }

    /// Records `command` with `spec` against a screen that shows `script`, as a check would.
    fn record(spec: &str, script: Vec<Shown>, command: &str) -> Run {
        failing(spec, script, command, (f64::INFINITY, 0)).unwrap()
    }

    /// As [`record`], with `failures` listings in a row failing from `fail_from` seconds on.
    fn failing(
        spec: &str,
        script: Vec<Shown>,
        command: &str,
        (fail_from, failures): (f64, usize),
    ) -> Result<Run> {
        let spec = Spec::parse(toml::from_str::<toml::Value>(spec)?)?;
        let out = tempfile::tempdir()?;
        let screen = Script {
            start: Instant::now(),
            shown: script,
            child: 0,
            fail_from,
            failures,
        };
        let mut recorder = Recorder::new(screen, &spec)?;
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c").arg(command);
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        let mut child = cmd.spawn()?;
        recorder.screen.child = i32::try_from(child.id())?;
        let clock = Instant::now();
        let mut log = Vec::new();
        let recorded = recorder.during(&mut child, out.path(), &mut log);
        let took = clock.elapsed();
        let left_running = child.try_wait()?.is_none();
        if left_running {
            stop(&mut child)?;
        }
        Ok(Run {
            recorded: recorded?,
            log: String::from_utf8(log)?,
            out,
            took,
            left_running,
        })
    }

    #[test]
    fn a_new_window_of_the_command_is_stilled_and_filmed_until_the_command_ends() {
        let run = record(
            "still = \"home\"\nfilm = \"home\"",
            vec![shown(7, CHILD, 0.2, 60.0)],
            "sleep 1.5",
        );
        assert!(run.recorded.passed(), "{:?}\n{}", run.recorded, run.log);
        assert!(run.out.path().join("home.png").exists());
        assert!(run.out.path().join("home.mp4").exists());
        let proofs: Vec<serde_json::Value> = run
            .proofs()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(
            proofs,
            [
                serde_json::json!({ "kind": "still", "file": "home.png", "id": "home" }),
                serde_json::json!({ "kind": "film", "file": "home.mp4", "id": "home" }),
            ]
        );
        assert!(
            run.log.contains("record: found \"Demo window\""),
            "{}",
            run.log
        );
    }

    #[test]
    fn a_window_already_open_is_never_recorded_and_the_wait_ends() {
        // The developer's own Demo is open before the command starts; the command opens none.
        let run = record(
            "film = \"home\"\nwindow = \"Demo\"\nwait = 0.5",
            vec![shown(7, 4242, 0.0, 60.0)],
            "sleep 30",
        );
        assert!(
            matches!(run.recorded.ended, Ended::WaitedTooLong),
            "{}",
            run.log
        );
        assert!(!run.recorded.passed());
        assert!(!run.left_running, "the command was stopped");
        assert!(run.took < Duration::from_secs(10), "{:?}", run.took);
        assert_eq!(run.proofs(), "");
        assert!(
            run.log
                .contains("no new window of \"Demo\" appeared in 0.5 s"),
            "{}",
            run.log
        );
        assert!(
            run.log
                .contains("was already open before the command started"),
            "{}",
            run.log
        );
    }

    #[test]
    fn a_window_that_opens_after_the_start_is_recorded_even_if_another_matches() {
        let run = record(
            "film = \"home\"\nwindow = \"Demo\"",
            vec![shown(7, 4242, 0.0, 60.0), shown(8, 4243, 0.3, 60.0)],
            "sleep 1.5",
        );
        assert!(run.recorded.passed(), "{}", run.log);
        assert!(run.log.contains("(pid 4243)"), "{}", run.log);
    }

    #[test]
    fn seconds_stop_a_command_that_runs_until_it_is_quit_and_it_passes() {
        let run = record(
            "film = \"home\"\nseconds = 0.5",
            vec![shown(7, CHILD, 0.1, 60.0)],
            "sleep 30",
        );
        assert!(
            matches!(run.recorded.ended, Ended::AfterSeconds),
            "{}",
            run.log
        );
        assert!(run.recorded.passed());
        assert!(!run.left_running);
        assert!(run.out.path().join("home.mp4").exists());
    }

    #[test]
    fn a_command_that_keeps_running_after_its_window_closed_is_stopped_after_wait() {
        let run = record(
            "film = \"home\"\nwait = 0.5",
            vec![shown(7, CHILD, 0.1, 1.2)],
            "sleep 30",
        );
        assert!(
            matches!(run.recorded.ended, Ended::WaitedTooLong),
            "{}",
            run.log
        );
        assert!(!run.recorded.passed());
        assert!(!run.left_running);
        assert!(run.took < Duration::from_secs(10), "{:?}", run.took);
        assert!(run.out.path().join("home.mp4").exists(), "the film is kept");
        assert!(run.log.contains("after its windows closed"), "{}", run.log);
    }

    #[test]
    fn a_second_window_after_the_first_closed_is_told_and_not_filmed() {
        let run = record(
            "film = \"app\"",
            vec![shown(7, CHILD, 0.1, 1.2), shown(8, CHILD, 1.6, 60.0)],
            "sleep 3",
        );
        assert!(run.recorded.passed(), "{}", run.log);
        assert!(
            run.log.contains("keeps only the first window of a run"),
            "{}",
            run.log
        );
        assert_eq!(run.proofs().lines().count(), 1);
    }

    #[test]
    fn a_second_window_that_stays_open_keeps_the_command_running() {
        let run = record(
            "film = \"app\"\nwait = 0.5",
            vec![shown(7, CHILD, 0.1, 1.2), shown(8, CHILD, 1.6, 60.0)],
            "sleep 4",
        );
        assert!(
            matches!(run.recorded.ended, Ended::Exited(_)),
            "{:?}\n{}",
            run.recorded,
            run.log
        );
        assert!(run.recorded.passed(), "{}", run.log);
        assert_eq!(
            run.log
                .matches("keeps only the first window of a run")
                .count(),
            1,
            "{}",
            run.log
        );
    }

    #[test]
    fn a_listing_that_fails_once_after_the_recording_is_told_and_tolerated() {
        let run = failing(
            "film = \"app\"",
            vec![shown(7, CHILD, 0.1, 1.2)],
            "sleep 3",
            (1.5, 1),
        )
        .unwrap();
        assert!(run.recorded.passed(), "{}", run.log);
        assert!(run.log.contains("the listing timed out"), "{}", run.log);
    }

    #[test]
    fn listings_that_keep_failing_after_the_recording_fail_the_run() {
        let err = failing(
            "film = \"app\"",
            vec![shown(7, CHILD, 0.1, 1.2)],
            "sleep 30",
            (1.5, 100),
        )
        .err()
        .expect("the run fails");
        assert!(
            format!("{err:#}").contains("the listing timed out"),
            "{err:#}"
        );
    }

    #[test]
    fn a_command_that_ends_before_a_window_fails_the_recording() {
        let run = record("film = \"home\"", vec![], "exit 0");
        assert!(matches!(run.recorded.ended, Ended::Exited(_)));
        assert!(!run.recorded.passed());
        assert!(
            run.log
                .contains("ended before an app the command started showed a window"),
            "{}",
            run.log
        );
    }
}
