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
#[cfg(target_os = "macos")]
mod place;
#[cfg(not(target_os = "macos"))]
use stub as place;

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

/// A window as the window server lists it.
#[derive(Clone, Debug)]
pub(crate) struct WindowInfo {
    pub id: u32,
    pub pid: i32,
    pub bundle: String,
    pub app: String,
    pub title: String,
    /// The top left corner in global points, y down.
    pub x: f64,
    pub y: f64,
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

/// Where a recorded window is kept while it records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Place {
    /// Where it opened: on the developer's screen.
    Stay,
    /// Parked off the developer's screens, focus handed back: `FEARLEZZ_OFF_SCREEN=1`.
    OffScreen,
}

impl Place {
    /// The developer's choice for this machine, from `FEARLEZZ_OFF_SCREEN`.
    pub(crate) fn from_env() -> Place {
        if std::env::var_os("FEARLEZZ_OFF_SCREEN").is_some_and(|v| v == "1") {
            Place::OffScreen
        } else {
            Place::Stay
        }
    }
}

/// The windows of the machine, as the recorder needs them: the window server's list and
/// ScreenCaptureKit on macOS, a script in tests. Windows are named by their id from the latest [`Screen::windows`].
pub(crate) trait Screen {
    type Film: Filming;
    /// Every normal window of every app.
    fn windows(&mut self) -> Result<Vec<WindowInfo>>;
    /// Writes a still of the window to `path` (PNG) and returns its size in pixels.
    fn still(&mut self, window: u32, path: &Path, frame: &Frame) -> Result<(usize, usize)>;
    /// Starts filming the window into `path` (MP4).
    fn film(&mut self, window: u32, path: &Path, frame: &Frame) -> Result<Self::Film>;
    /// Moves the window off the developer's screens, keeping its size, without giving it
    /// focus; says how, such as "by Accessibility", or why it cannot yet. Asked again until
    /// it is parked, so it never waits itself.
    fn park(&mut self, window: u32) -> Result<Parking>;
    /// The pid of the app in front, which gets the developer's keys; fails with why it
    /// cannot be read and the fix.
    fn front(&mut self) -> Result<i32>;
    /// Brings the app of `pid` to the front.
    fn bring_front(&mut self, pid: i32) -> Result<()>;
    /// Puts back the window positions the parked window's app had saved before it was
    /// parked, since macOS saves a frame on every move; returns how many.
    fn put_back(&mut self) -> Result<usize>;
}

/// Where parking a window stands.
pub(crate) enum Parking {
    /// Parked, and how.
    Parked(String),
    /// Not yet, and why: the window manager has not picked the window up.
    NotYet(String),
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
    listed: Vec<WindowInfo>,
    /// Each app's bundle id by pid, read once.
    apps: std::collections::HashMap<i32, String>,
    /// The window positions the parked window's app had saved.
    saved: Option<place::Saved>,
}

impl System {
    fn find(&self, id: u32) -> Result<&WindowInfo> {
        self.listed
            .iter()
            .find(|w| w.id == id)
            .with_context(|| format!("window {id} is gone"))
    }
}

impl Screen for System {
    type Film = sys::Film;

    fn windows(&mut self) -> Result<Vec<WindowInfo>> {
        self.listed = sys::windows(&mut self.apps)?;
        Ok(self.listed.clone())
    }

    fn still(&mut self, window: u32, path: &Path, frame: &Frame) -> Result<(usize, usize)> {
        sys::still(&sys::capturable(window)?, path, frame)
    }

    fn film(&mut self, window: u32, path: &Path, frame: &Frame) -> Result<sys::Film> {
        sys::start_film(&sys::capturable(window)?, path, frame)
    }

    fn park(&mut self, window: u32) -> Result<Parking> {
        let window = self.find(window)?.clone();
        // Read before the first move, which the app saves at once.
        if self.saved.is_none() {
            self.saved = place::saved(&window.bundle);
        }
        place::park(&window)
    }

    fn front(&mut self) -> Result<i32> {
        place::front()
    }

    fn bring_front(&mut self, pid: i32) -> Result<()> {
        place::bring_front(pid)
    }

    fn put_back(&mut self) -> Result<usize> {
        self.saved
            .take()
            .map_or(Ok(0), |saved| place::put_back(&saved))
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
/// How often windows are listed while waiting for a new one: a listing takes about 2 ms,
/// and a window to park shows on the developer's screen until it is found.
const WATCH: Duration = Duration::from_millis(10);
/// How often windows are listed while the recorder only waits for the command to end.
const LIST_EVERY: Duration = Duration::from_secs(1);
/// How long a window that just appeared gets to draw its first frame.
const SETTLE: Duration = Duration::from_millis(700);
/// How often focus is looked at while a parked window settles.
const FOCUS_POLL: Duration = Duration::from_millis(20);
/// How often parking is asked again while the window manager has not picked the window up:
/// each ask is a query to AeroSpace, which answers on the same thread that picks windows up.
const PARK_EVERY: Duration = Duration::from_millis(100);
/// How long a window manager gets to pick up a new window before it can be parked:
/// AeroSpace learns of windows on its own schedule, which took over 2 s on a loaded machine.
/// Short in unit tests, whose scripted screen never picks some windows up.
const PICK_UP: Duration = if cfg!(test) {
    Duration::from_secs(1)
} else {
    Duration::from_secs(10)
};
/// How many listings in a row may fail after the recording before the run fails.
const LISTING_TRIES: usize = 3;

/// Records the window of one run of a check's command.
pub(crate) struct Recorder<'a, S> {
    screen: S,
    spec: &'a Spec,
    /// The windows open before the command started, which are never recorded: an app the
    /// developer has open would otherwise be filmed on both sides of a look.
    before: BTreeSet<u32>,
    place: Place,
    /// The app in front before the command started, which gets focus back from a parked app.
    /// None when focus is not watched (the window stays) or no longer can be.
    developer: Option<Result<i32, String>>,
    /// Whether the window was parked, so its app's saved positions are put back.
    parked: bool,
    /// How each film went, for the machine's film log.
    films: Vec<FilmOutcome>,
}

impl<'a, S: Screen> Recorder<'a, S> {
    /// Notes the windows open now; call it before starting the command.
    pub(crate) fn new(mut screen: S, spec: &'a Spec, place: Place) -> Result<Self> {
        let before = screen.windows()?.iter().map(|w| w.id).collect();
        let developer = match place {
            Place::OffScreen => Some(screen.front().map_err(|e| format!("{e:#}"))),
            Place::Stay => None,
        };
        Ok(Recorder {
            screen,
            spec,
            before,
            place,
            developer,
            parked: false,
            films: Vec::new(),
        })
    }

    /// How each film of this recording went, failed ones too.
    pub(crate) fn films(&self) -> &[FilmOutcome] {
        &self.films
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
        let recorded = self.record(child, out, log);
        // Once the command ended, its app saved its last frame: the parked one. A recording
        // that failed left it running, so it is stopped first.
        if self.parked {
            if recorded.is_err() {
                stop(child)?;
            }
            match self.screen.put_back() {
                Ok(0) => {}
                Ok(n) => writeln!(
                    log,
                    "record: put back {n} window position the app had saved"
                )?,
                Err(e) => writeln!(
                    log,
                    "record: could not put back the app's saved window position: {e:#}"
                )?,
            }
        }
        recorded
    }

    fn record(&mut self, child: &mut Child, out: &Path, log: &mut impl Write) -> Result<Recorded> {
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
        self.settle(&window, log)?;
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
            let mut film = match self.screen.film(window.id, &path, &frame) {
                Ok(film) => film,
                Err(e) => {
                    self.films.push(FilmOutcome {
                        name: name.clone(),
                        seconds: 0.0,
                        error: Some(format!("{e:#}")),
                    });
                    return Err(e);
                }
            };
            while child.try_wait()?.is_none()
                && !film.ended()
                && deadline.is_none_or(|d| Instant::now() < d)
            {
                std::thread::sleep(POLL);
            }
            let secs = recording.elapsed().as_secs_f64();
            let error = match film.stop() {
                Ok(()) if path.exists() => {
                    writeln!(log, "record: film {file} of {secs:.1} s")?;
                    proofs.push(("film", file, name));
                    None
                }
                Ok(()) => {
                    writeln!(log, "record: film {file} was not written")?;
                    recorded = false;
                    Some("its file was not written".to_string())
                }
                Err(e) => {
                    writeln!(log, "record: film {file} failed: {e:#}")?;
                    recorded = false;
                    Some(format!("{e:#}"))
                }
            };
            self.films.push(FilmOutcome {
                name: name.clone(),
                seconds: secs,
                error,
            });
        }
        append_proofs(out, &proofs)?;
        let ended = self.finish(child, root, window.id, deadline, log)?;
        Ok(Recorded { ended, recorded })
    }

    /// Gives a window that just appeared [`SETTLE`] to draw its first frame. Off screen, it is
    /// parked first, and while it settles the developer's app gets focus back whenever the
    /// window's app takes it; focus the developer moved elsewhere stays there.
    fn settle(&mut self, window: &WindowInfo, log: &mut impl Write) -> Result<()> {
        if self.place == Place::Stay {
            writeln!(
                log,
                "record: the window stays on your screen; FEARLEZZ_OFF_SCREEN=1 in your \
                 environment parks recorded windows off screen"
            )?;
            std::thread::sleep(SETTLE);
            return Ok(());
        }
        if let Some(Err(why)) = &self.developer {
            writeln!(log, "record: focus is not given back: {why}")?;
            self.developer = None;
        }
        let mut told = false;
        let since = Instant::now();
        // Settling starts once parking ended; until then the window manager may still have
        // to pick the window up, and focus goes back all the while.
        let mut settled_at = None;
        let mut tried: Option<Instant> = None;
        while settled_at.is_none_or(|at| Instant::now() < at) {
            self.give_back(window, &mut told, log)?;
            if settled_at.is_none() && tried.is_none_or(|t| t.elapsed() >= PARK_EVERY) {
                tried = Some(Instant::now());
                let failed = match self.screen.park(window.id) {
                    Ok(Parking::Parked(how)) => {
                        self.parked = true;
                        writeln!(log, "record: parked the window off screen {how}")?;
                        None
                    }
                    Ok(Parking::NotYet(why)) if since.elapsed() >= PICK_UP => {
                        Some(format!("{why} after {PICK_UP:?}"))
                    }
                    Ok(Parking::NotYet(_)) => {
                        std::thread::sleep(FOCUS_POLL);
                        continue;
                    }

                    Err(e) => Some(format!("{e:#}")),
                };
                if let Some(e) = failed {
                    writeln!(log, "record: could not park the window off screen: {e}")?;
                }
                settled_at = Some(Instant::now() + SETTLE);
            }
            std::thread::sleep(FOCUS_POLL);
        }
        Ok(())
    }

    /// Brings the developer's app back to the front when the window's app has taken it.
    fn give_back(
        &mut self,
        window: &WindowInfo,
        told: &mut bool,
        log: &mut impl Write,
    ) -> Result<()> {
        let Some(Ok(developer)) = self.developer else {
            return Ok(());
        };
        if developer == window.pid || self.screen.front().ok() != Some(window.pid) {
            return Ok(());
        }
        match self.screen.bring_front(developer) {
            Ok(()) if !*told => {
                *told = true;
                writeln!(log, "record: gave focus back to the app in front before")?;
            }
            Ok(()) => {}
            Err(e) => {
                writeln!(log, "record: could not give focus back: {e:#}")?;
                self.developer = None;
            }
        }
        Ok(())
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
            std::thread::sleep(WATCH);
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

/// How one film went.
pub(crate) struct FilmOutcome {
    pub(crate) name: String,
    pub(crate) seconds: f64,
    /// Why it failed, when it did.
    pub(crate) error: Option<String>,
}

/// What the machine was doing when films ended: films fail machine wide when the capture
/// service is stuck, which shows as failures in a row under one service pid.
pub(crate) struct Machine {
    pub(crate) capture_service: Option<u32>,
    pub(crate) load: Option<f64>,
}

impl Machine {
    pub(crate) fn now() -> Self {
        Machine {
            capture_service: sys::capture_service(),
            load: sys::load(),
        }
    }
}

/// Appends one line per film to the machine's film log at `file` (every repository and
/// worktree writes to the same one), so how often films fail can be counted.
pub(crate) fn note_films(
    file: &Path,
    repo: &Path,
    check: &str,
    films: &[FilmOutcome],
    machine: &Machine,
) -> Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let at = jiff::Timestamp::now().to_string();
    let mut lines = String::new();
    for film in films {
        let line = serde_json::json!({
            "at": at,
            "repo": repo.display().to_string(),
            "check": check,
            "film": film.name,
            "ok": film.error.is_none(),
            "seconds": (film.seconds * 10.0).round() / 10.0,
            "error": film.error,
            "replayd": machine.capture_service,
            "load": machine.load,
        });
        lines.push_str(&format!("{line}\n"));
    }
    // One write, so lines from runs in other worktrees never interleave.
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)?
        .write_all(lines.as_bytes())?;
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
            x: 100.0,
            y: 100.0,
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
        /// Why parking fails, when it does.
        park_fails: Option<&'static str>,
        /// Parking waits until this many seconds after the screen was made, as AeroSpace
        /// does until it picks up a new window.
        park_from: f64,
        /// Why the app in front cannot be read, when it cannot.
        front_fails: Option<&'static str>,
        /// Why saved window positions cannot be put back, when they cannot.
        put_back_fails: Option<&'static str>,
        /// Why films cannot be finished, when they cannot.
        film_fails: Option<&'static str>,
        /// The app in front from each time on, the latest that started; CHILD for the
        /// command's app.
        fronts: Vec<(f64, i32)>,
        /// The app brought to the front last, which wins over `fronts` until it changes.
        brought: Option<(f64, i32)>,
        /// What the recorder did to the screen, in order.
        events: Vec<String>,
    }

    struct ScriptFilm {
        start: Instant,
        until: f64,
        path: PathBuf,
        fails: Option<&'static str>,
    }

    impl Filming for ScriptFilm {
        fn ended(&mut self) -> bool {
            self.start.elapsed().as_secs_f64() >= self.until
        }

        fn stop(self) -> Result<()> {
            if let Some(why) = self.fails {
                bail!(why);
            }
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

        fn still(&mut self, window: u32, path: &Path, _frame: &Frame) -> Result<(usize, usize)> {
            self.events.push(format!("still {window}"));
            std::fs::write(path, "still")?;
            Ok((800, 600))
        }

        fn park(&mut self, window: u32) -> Result<Parking> {
            if let Some(why) = self.park_fails {
                bail!(why);
            }
            if self.start.elapsed().as_secs_f64() < self.park_from {
                return Ok(Parking::NotYet("the script has not picked it up".into()));
            }
            self.events.push(format!("park {window}"));
            Ok(Parking::Parked("by script".into()))
        }

        fn front(&mut self) -> Result<i32> {
            if let Some(why) = self.front_fails {
                bail!(why);
            }
            let now = self.start.elapsed().as_secs_f64();
            let scripted = self
                .fronts
                .iter()
                .rev()
                .find(|(from, _)| *from <= now)
                .copied();
            let latest = match (scripted, self.brought) {
                (Some(s), Some(b)) if b.0 >= s.0 => Some(b),
                (s, b) => s.or(b),
            };
            latest
                .map(|(_, pid)| if pid == CHILD { self.child } else { pid })
                .context("no app is in front")
        }

        fn bring_front(&mut self, pid: i32) -> Result<()> {
            self.events.push(format!("front {pid}"));
            self.brought = Some((self.start.elapsed().as_secs_f64(), pid));
            Ok(())
        }

        fn put_back(&mut self) -> Result<usize> {
            if let Some(why) = self.put_back_fails {
                bail!(why);
            }
            let running = std::process::Command::new("kill")
                .args(["-0", &self.child.to_string()])
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            self.events.push(
                if running {
                    "put back while the command ran"
                } else {
                    "put back"
                }
                .into(),
            );
            Ok(1)
        }

        fn film(&mut self, window: u32, path: &Path, _frame: &Frame) -> Result<ScriptFilm> {
            self.events.push(format!("film {window}"));
            let until = self
                .shown
                .iter()
                .find(|s| s.window.id == window)
                .map_or(0.0, |s| s.until);
            Ok(ScriptFilm {
                start: self.start,
                until,
                path: path.to_path_buf(),
                fails: self.film_fails,
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
        /// What the recorder did to the screen, in order.
        events: Vec<String>,
        /// How each film went, for the machine's film log.
        films: Vec<FilmOutcome>,
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

    /// The developer's side of a run: where windows go and who is in front when.
    struct Desk {
        place: Place,
        park_fails: Option<&'static str>,
        park_from: f64,
        front_fails: Option<&'static str>,
        put_back_fails: Option<&'static str>,
        film_fails: Option<&'static str>,
        fronts: Vec<(f64, i32)>,
    }

    impl Default for Desk {
        fn default() -> Self {
            Desk {
                place: Place::Stay,
                park_fails: None,
                park_from: 0.0,
                front_fails: None,
                put_back_fails: None,
                film_fails: None,
                fronts: Vec::new(),
            }
        }
    }

    /// As [`record`], with the developer's desk set up as `desk`.
    fn at_desk(spec: &str, script: Vec<Shown>, command: &str, desk: Desk) -> Run {
        run(spec, script, command, (f64::INFINITY, 0), desk).unwrap()
    }

    /// As [`record`], with `failures` listings in a row failing from `fail_from` seconds on.
    fn failing(spec: &str, script: Vec<Shown>, command: &str, fails: (f64, usize)) -> Result<Run> {
        run(spec, script, command, fails, Desk::default())
    }

    fn run(
        spec: &str,
        script: Vec<Shown>,
        command: &str,
        (fail_from, failures): (f64, usize),
        desk: Desk,
    ) -> Result<Run> {
        let spec = Spec::parse(toml::from_str::<toml::Value>(spec)?)?;
        let out = tempfile::tempdir()?;
        let screen = Script {
            start: Instant::now(),
            shown: script,
            child: 0,
            fail_from,
            failures,
            park_fails: desk.park_fails,
            park_from: desk.park_from,
            front_fails: desk.front_fails,
            put_back_fails: desk.put_back_fails,
            film_fails: desk.film_fails,
            fronts: desk.fronts,
            brought: None,
            events: Vec::new(),
        };
        let mut recorder = Recorder::new(screen, &spec, desk.place)?;
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
        let films = std::mem::take(&mut recorder.films);
        let events = recorder.screen.events;
        Ok(Run {
            recorded: recorded.with_context(|| format!("screen events {events:?}"))?,
            log: String::from_utf8(log)?,
            out,
            took,
            left_running,
            events,
            films,
        })
    }

    /// The developer's editor, in front before the command starts.
    const EDITOR: i32 = 500;

    #[test]
    fn off_screen_parks_the_window_as_soon_as_it_appears_before_recording_it() {
        let run = at_desk(
            "still = \"home\"\nfilm = \"home\"",
            vec![shown(7, CHILD, 0.2, 60.0)],
            "sleep 2",
            Desk {
                place: Place::OffScreen,
                ..Desk::default()
            },
        );
        assert!(run.recorded.passed(), "{}", run.log);
        assert_eq!(
            run.events,
            ["park 7", "still 7", "film 7", "put back"],
            "{}",
            run.log
        );
        assert!(
            run.log
                .contains("record: parked the window off screen by script"),
            "{}",
            run.log
        );
    }

    #[test]
    fn a_window_that_cannot_be_parked_is_recorded_where_it_opened_and_says_why() {
        let run = at_desk(
            "film = \"home\"",
            vec![shown(7, CHILD, 0.2, 60.0)],
            "sleep 1.5",
            Desk {
                place: Place::OffScreen,
                park_fails: Some("Accessibility is off for the terminal"),
                ..Desk::default()
            },
        );
        assert!(run.recorded.passed(), "{}", run.log);
        assert_eq!(run.events, ["film 7"]);
        assert!(
            run.log.contains(
                "record: could not park the window off screen: Accessibility is off for the terminal"
            ),
            "{}",
            run.log
        );
    }

    #[test]
    fn without_off_screen_nothing_moves_and_the_log_says_how_to_turn_it_on() {
        let run = at_desk(
            "film = \"home\"",
            vec![shown(7, CHILD, 0.2, 60.0)],
            "sleep 1.5",
            Desk {
                fronts: vec![(0.0, EDITOR), (0.3, CHILD)],
                ..Desk::default()
            },
        );
        assert!(run.recorded.passed(), "{}", run.log);
        assert_eq!(
            run.events,
            ["film 7"],
            "neither parked nor focus taken back"
        );
        assert!(run.log.contains("FEARLEZZ_OFF_SCREEN=1"), "{}", run.log);
    }

    #[test]
    fn off_screen_gives_focus_back_when_the_app_takes_it_while_it_settles() {
        // The app takes focus after its window shows, as a real app does.
        let run = at_desk(
            "film = \"home\"",
            vec![shown(7, CHILD, 0.1, 60.0)],
            "sleep 2",
            Desk {
                place: Place::OffScreen,
                fronts: vec![(0.0, EDITOR), (0.4, CHILD)],
                ..Desk::default()
            },
        );
        assert!(run.recorded.passed(), "{}", run.log);
        assert_eq!(
            run.events,
            ["park 7", &format!("front {EDITOR}"), "film 7", "put back"],
            "{}",
            run.log
        );
        assert!(run.log.contains("record: gave focus back"), "{}", run.log);
    }

    #[test]
    fn off_screen_gives_focus_back_before_parking_when_the_app_took_it_first() {
        let run = at_desk(
            "film = \"home\"",
            vec![shown(7, CHILD, 0.1, 60.0)],
            "sleep 2",
            Desk {
                place: Place::OffScreen,
                fronts: vec![(0.0, EDITOR), (0.05, CHILD)],
                ..Desk::default()
            },
        );
        assert!(run.recorded.passed(), "{}", run.log);
        assert_eq!(
            run.events,
            [&format!("front {EDITOR}"), "park 7", "film 7", "put back"],
            "{}",
            run.log
        );
    }

    #[test]
    fn off_screen_gives_focus_back_while_parking_waits_for_the_window_manager() {
        let run = at_desk(
            "film = \"home\"",
            vec![shown(7, CHILD, 0.1, 60.0)],
            "sleep 2.5",
            Desk {
                place: Place::OffScreen,
                park_from: 1.0,
                fronts: vec![(0.0, EDITOR), (0.5, CHILD)],
                ..Desk::default()
            },
        );
        assert!(run.recorded.passed(), "{}", run.log);
        assert_eq!(
            run.events,
            [&format!("front {EDITOR}"), "park 7", "film 7", "put back"],
            "{}",
            run.log
        );
    }

    #[test]
    fn a_window_the_window_manager_never_picks_up_is_recorded_where_it_opened() {
        let run = at_desk(
            "film = \"home\"",
            vec![shown(7, CHILD, 0.1, 60.0)],
            "sleep 4",
            Desk {
                place: Place::OffScreen,
                park_from: f64::INFINITY,
                ..Desk::default()
            },
        );
        assert!(run.recorded.passed(), "{}", run.log);
        assert_eq!(run.events, ["film 7"], "{}", run.log);
        assert!(
            run.log.contains(&format!(
                "record: could not park the window off screen: the script has not picked it \
                 up after {PICK_UP:?}"
            )),
            "{}",
            run.log
        );
    }

    #[test]
    fn off_screen_says_why_focus_is_not_given_back_when_it_cannot_read_focus() {
        let run = at_desk(
            "film = \"home\"",
            vec![shown(7, CHILD, 0.1, 60.0)],
            "sleep 1.5",
            Desk {
                place: Place::OffScreen,
                front_fails: Some("the terminal may not read which app is in front"),
                ..Desk::default()
            },
        );
        assert!(run.recorded.passed(), "{}", run.log);
        assert_eq!(run.events, ["park 7", "film 7", "put back"], "{}", run.log);
        assert!(
            run.log.contains(
                "record: focus is not given back: the terminal may not read which app is in front"
            ),
            "{}",
            run.log
        );
    }

    #[test]
    fn off_screen_puts_back_the_saved_window_position_once_the_command_ended() {
        let run = at_desk(
            "film = \"home\"\nseconds = 0.5",
            vec![shown(7, CHILD, 0.1, 60.0)],
            "sleep 30",
            Desk {
                place: Place::OffScreen,
                ..Desk::default()
            },
        );
        assert!(run.recorded.passed(), "{}", run.log);
        assert_eq!(run.events.last().map(String::as_str), Some("put back"));
        assert!(!run.left_running, "the command ended before");
        assert!(
            run.log
                .contains("record: put back 1 window position the app had saved"),
            "{}",
            run.log
        );
    }

    #[test]
    fn a_recording_that_fails_stops_the_command_before_putting_back_saved_positions() {
        let err = run(
            "film = \"app\"",
            vec![shown(7, CHILD, 0.1, 1.2)],
            "sleep 30",
            (1.5, 100),
            Desk {
                place: Place::OffScreen,
                ..Desk::default()
            },
        )
        .err()
        .expect("the run fails");
        let err = format!("{err:#}");
        assert!(err.contains("\"put back\""), "{err}");
        assert!(!err.contains("while the command ran"), "{err}");
    }

    #[test]
    fn every_film_is_noted_with_how_it_went() {
        let good = record(
            "film = \"home\"",
            vec![shown(7, CHILD, 0.1, 60.0)],
            "sleep 1.5",
        );
        assert_eq!(good.films.len(), 1, "{}", good.log);
        assert_eq!(good.films[0].name, "home");
        assert_eq!(good.films[0].error, None);
        let bad = at_desk(
            "film = \"home\"",
            vec![shown(7, CHILD, 0.1, 60.0)],
            "sleep 1.5",
            Desk {
                film_fails: Some("failure to process first sample buffer"),
                ..Desk::default()
            },
        );
        assert_eq!(bad.films.len(), 1, "{}", bad.log);
        assert_eq!(
            bad.films[0].error.as_deref(),
            Some("failure to process first sample buffer")
        );
    }

    #[test]
    fn the_film_log_gets_one_line_per_film_with_the_machine_state() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("deep").join("films.jsonl");
        let films = [
            FilmOutcome {
                name: "home".into(),
                seconds: 4.2,
                error: None,
            },
            FilmOutcome {
                name: "menu".into(),
                seconds: 0.0,
                error: Some("failure to process first sample buffer".into()),
            },
        ];
        let machine = Machine {
            capture_service: Some(812),
            load: Some(33.4),
        };
        note_films(&file, Path::new("/work/app"), "look", &films, &machine).unwrap();
        note_films(&file, Path::new("/work/app"), "look", &films[..1], &machine).unwrap();
        let lines: Vec<serde_json::Value> = std::fs::read_to_string(&file)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1]["repo"], "/work/app");
        assert_eq!(lines[1]["check"], "look");
        assert_eq!(lines[1]["film"], "menu");
        assert_eq!(lines[1]["ok"], false);
        assert_eq!(lines[1]["error"], "failure to process first sample buffer");
        assert_eq!(lines[1]["replayd"], 812);
        assert_eq!(lines[1]["load"], 33.4);
        assert_eq!(lines[0]["ok"], true);
        assert!(lines[0]["at"].as_str().is_some_and(|at| at.contains('T')));
    }

    #[test]
    fn a_window_that_was_not_parked_has_no_position_to_put_back() {
        let run = at_desk(
            "film = \"home\"",
            vec![shown(7, CHILD, 0.1, 60.0)],
            "sleep 1.5",
            Desk {
                place: Place::OffScreen,
                park_fails: Some("Accessibility is off for the terminal"),
                ..Desk::default()
            },
        );
        assert_eq!(run.events, ["film 7"], "{}", run.log);
    }

    #[test]
    fn saved_positions_that_cannot_be_put_back_are_told_and_the_run_still_passes() {
        let run = at_desk(
            "film = \"home\"",
            vec![shown(7, CHILD, 0.1, 60.0)],
            "sleep 1.5",
            Desk {
                place: Place::OffScreen,
                put_back_fails: Some("the preferences cannot be written"),
                ..Desk::default()
            },
        );
        assert!(run.recorded.passed(), "{}", run.log);
        assert!(
            run.log.contains(
                "record: could not put back the app's saved window position: the preferences \
                 cannot be written"
            ),
            "{}",
            run.log
        );
    }

    #[test]
    fn off_screen_leaves_focus_the_developer_moved_to_another_app() {
        let run = at_desk(
            "film = \"home\"",
            vec![shown(7, CHILD, 0.1, 60.0)],
            "sleep 2",
            Desk {
                place: Place::OffScreen,
                fronts: vec![(0.0, EDITOR), (0.3, 600)],
                ..Desk::default()
            },
        );
        assert!(run.recorded.passed(), "{}", run.log);
        assert_eq!(run.events, ["park 7", "film 7", "put back"], "{}", run.log);
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
