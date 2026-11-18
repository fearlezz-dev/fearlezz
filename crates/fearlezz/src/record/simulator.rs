//! iOS simulators of Fearlezz's own, kept booted and leased one run at a time: a device
//! named `fearlezz <device type> <n>`, locked by its lease file in the user's cache
//! folder. A run gets a free one (a busy pool grows by one), and the apps a run installed
//! are removed and the keychain reset before and after it, so a look's two sides start
//! alike. A device stays booted for the next run: a cold boot takes over a minute. Stills
//! and films come from `simctl io`, which needs no Screen Recording, so this runs on CI's
//! macOS runners too.

use std::fs::File;
use std::io::{BufRead as _, BufReader};
use std::os::fd::AsRawFd as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use super::{Frame, Parking, Unavailable, WindowInfo};

/// simctl itself, found once through Apple's own xcrun (a Nix shell puts a shim first on
/// PATH that finds no SDK without the shell's DEVELOPER_DIR). It runs directly, not under
/// xcrun, so a signal to stop a recording reaches it rather than leaving it recording.
fn simctl_path() -> Result<&'static Path> {
    static PATH: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        let out = Command::new("/usr/bin/xcrun")
            .args(["-f", "simctl"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()
            .filter(|o| o.status.success())?;
        let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        path.is_file().then_some(path)
    })
    .as_deref()
    .context("xcrun finds no simctl")
}
/// How long `recordVideo` gets to start recording.
const FILM_START: Duration = Duration::from_secs(15);
/// How long the simulator gets to finish writing a film once it stopped.
const FILM_WRITE: Duration = Duration::from_secs(30);
/// Long enough for a first boot of a new device, which took 76 s on a developer's Mac.
const BOOT: Duration = Duration::from_mins(5);

/// None when this Mac has an iOS simulator runtime.
pub(crate) fn unavailable() -> Option<Unavailable> {
    let fix = "install Xcode, then an iOS simulator runtime: xcodebuild -downloadPlatform iOS";
    match list() {
        Ok(list) if list.newest_runtime().is_some() => None,
        Ok(_) => Some(Unavailable {
            why: "the iOS simulator has no iOS runtime here".into(),
            fix: fix.into(),
        }),
        Err(e) => Some(Unavailable {
            why: format!("the iOS simulator is not here ({e:#})"),
            fix: fix.into(),
        }),
    }
}

/// What `simctl list -j` says, the parts read here.
#[derive(Deserialize)]
struct List {
    devicetypes: Vec<DeviceType>,
    runtimes: Vec<Runtime>,
    /// Devices by runtime identifier.
    devices: std::collections::BTreeMap<String, Vec<Device>>,
}

#[derive(Deserialize)]
struct DeviceType {
    name: String,
    identifier: String,
}

#[derive(Deserialize)]
struct Runtime {
    identifier: String,
    name: String,
    version: String,
    platform: Option<String>,
    #[serde(rename = "isAvailable", default)]
    is_available: bool,
    #[serde(rename = "supportedDeviceTypes", default)]
    supported: Vec<DeviceType>,
}

#[derive(Deserialize)]
struct Device {
    name: String,
    udid: String,
    state: String,
    #[serde(rename = "isAvailable", default)]
    is_available: bool,
}

impl List {
    fn newest_runtime(&self) -> Option<&Runtime> {
        self.runtimes
            .iter()
            .filter(|r| r.is_available && r.platform.as_deref().unwrap_or("iOS") == "iOS")
            .filter(|r| r.name.starts_with("iOS"))
            .max_by_key(|r| version(&r.version))
    }

    /// The device type and runtime: the named type on the newest runtime that has it, or
    /// the newest plain iPhone on the newest runtime that has it.
    fn pick(&self, device: Option<&str>) -> Result<(&DeviceType, &Runtime)> {
        let newest = self
            .newest_runtime()
            .context("the iOS simulator has no iOS runtime here")?;
        // By default the newest plain iPhone ("iPhone 17"), not a Pro, Air or foldable one,
        // on the newest runtime that has it.
        let plain = |t: &&DeviceType| {
            t.name
                .strip_prefix("iPhone ")
                .and_then(|n| n.parse::<u32>().ok())
        };
        let name = match device {
            Some(name) => name,
            None => self
                .runtimes
                .iter()
                .filter(|r| r.is_available && r.name.starts_with("iOS"))
                .flat_map(|r| &r.supported)
                .filter(|t| plain(t).is_some())
                .max_by_key(plain)
                .map(|t| t.name.as_str())
                .with_context(|| {
                    format!(
                        "{} has no iPhone; name a device type with record's device, such as \
                         device = \"iPad Air 11-inch (M4)\"",
                        newest.name
                    )
                })?,
        };
        let kind = self
            .devicetypes
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(name))
            .with_context(|| {
                let iphones: Vec<&str> = newest.supported.iter().map(|t| t.name.as_str()).collect();
                format!(
                    "record: device = \"{name}\" is not a simulator device type here; {} has {}",
                    newest.name,
                    iphones.join(", ")
                )
            })?;
        let runtime = self
            .runtimes
            .iter()
            .filter(|r| r.is_available && r.name.starts_with("iOS"))
            .filter(|r| r.supported.iter().any(|t| t.identifier == kind.identifier))
            .max_by_key(|r| version(&r.version))
            .with_context(|| {
                format!(
                    "no iOS runtime here runs a {}; install one with xcodebuild \
                     -downloadPlatform iOS, or name another device type",
                    kind.name
                )
            })?;
        Ok((kind, runtime))
    }
}

/// "27.2" as (27, 2, 0), to order runtimes.
fn version(text: &str) -> (u32, u32, u32) {
    let mut parts = text.split('.').map(|p| p.parse().unwrap_or(0));
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

fn simctl(args: &[&str]) -> Result<String> {
    let out = Command::new(simctl_path()?)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .context("running xcrun simctl")?;
    if !out.status.success() {
        bail!(
            "simctl {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn list() -> Result<List> {
    let text = simctl(&["list", "-j", "devicetypes", "runtimes", "devices"])?;
    serde_json::from_str(&text).context("reading simctl's list")
}

/// A simulator leased for one run: booted, cleaned, and locked until dropped.
pub(crate) struct Simulator {
    udid: String,
    name: String,
    /// `--display=<n>` for a device with more than one screen (a foldable): its primary.
    display: Option<String>,
    /// Held locked for the lease; closing it frees the device.
    _lease: File,
}

impl Simulator {
    /// Leases a booted simulator of `device` (by default the newest plain iPhone on the newest
    /// runtime), creating and booting one when every one is busy or there is none yet.
    /// A check that `films` gets a device whose recorder is free.
    pub(crate) fn lease(
        device: Option<&str>,
        films: bool,
        log: &mut impl std::io::Write,
    ) -> Result<Self> {
        // Listing, creating and locking happen under the pool's lock, so two runs never take
        // the same device or make two of one name.
        let pool = lock(&locks()?.join("pool.lock"), true)?.context("locking the pool")?;
        let list = list()?;
        let (kind, runtime) = list.pick(device)?;
        let prefix = format!("fearlezz {} ", kind.name);
        let ours: Vec<&Device> = list
            .devices
            .get(&runtime.identifier)
            .into_iter()
            .flatten()
            .filter(|d| d.is_available && d.name.starts_with(&prefix))
            .collect();
        let mut leased = None;
        for d in &ours {
            if let Some(lock) = lock_device(&d.udid)? {
                leased = Some((d.udid.clone(), d.name.clone(), d.state == "Booted", lock));
                break;
            }
        }
        let (udid, name, booted, lock) = match leased {
            Some(l) => l,
            None => {
                let name = format!("{prefix}{}", ours.len() + 1);
                let udid = simctl(&["create", &name, &kind.identifier, &runtime.identifier])?
                    .trim()
                    .to_string();
                writeln!(log, "record: made the simulator {name} ({})", runtime.name)?;
                let lock = lock_device(&udid)?.context("a new simulator is already leased")?;
                (udid, name, false, lock)
            }
        };
        drop(pool);
        let mut sim = Simulator {
            udid,
            name,
            display: None,
            _lease: lock,
        };
        if !booted {
            let clock = std::time::Instant::now();
            writeln!(log, "record: booting {}", sim.name)?;
            match simctl(&["boot", &sim.udid]) {
                // Another tool booted it meanwhile.
                Err(e) if !format!("{e:#}").contains("current state: Booted") => {
                    return Err(e.context(format!("booting the simulator {}", sim.name)));
                }
                _ => {}
            }
            sim.boot_status()?;
            writeln!(
                log,
                "record: booted {} in {:.0} s",
                sim.name,
                clock.elapsed().as_secs_f64()
            )?;
        }
        sim.clean()?;
        sim.display = sim.primary_display()?;
        if films {
            sim.free_recorder(log)?;
        }
        writeln!(log, "record: on the simulator {} ({})", sim.name, sim.udid)?;
        Ok(sim)
    }

    /// Its id, for `FEARLEZZ_DEVICE` (`flutter test -d`, `simctl launch`).
    pub(crate) fn udid(&self) -> &str {
        &self.udid
    }

    fn boot_status(&self) -> Result<()> {
        let mut child = Command::new(simctl_path()?)
            .args(["bootstatus", &self.udid, "-b"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let clock = std::time::Instant::now();
        loop {
            if child.try_wait()?.is_some() {
                let out = child.wait_with_output()?;
                if out.status.success() {
                    return Ok(());
                }
                bail!(
                    "the simulator {} did not boot: {}",
                    self.name,
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
            if clock.elapsed() >= BOOT {
                drop(child.kill());
                bail!(
                    "the simulator {} did not boot in {} s",
                    self.name,
                    BOOT.as_secs()
                );
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// Starts `recordVideo`, which says "Recording started" once frames are being written,
    /// or why it cannot before it exits.
    fn start_film(&self, path: &Path) -> Result<Film> {
        let mut child = Command::new(simctl_path()?)
            .args(self.io(
                "recordVideo",
                &["--codec=h264", "--force", &path.to_string_lossy()],
            ))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("starting simctl recordVideo")?;
        let stderr = child.stderr.take().context("recordVideo's output")?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            // Every line, read to the end, so recordVideo never writes into a closed pipe;
            // None once it closed its output.
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                drop(tx.send(Some(line)));
            }
            drop(tx.send(None));
        });
        let deadline = std::time::Instant::now() + FILM_START;
        let mut said = Vec::new();
        let mut film = Film {
            child,
            path: path.to_path_buf(),
        };
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match rx.recv_timeout(left) {
                Ok(Some(line)) if line.contains("Recording started") => return Ok(film),
                Ok(Some(line)) => said.push(line),
                Ok(None) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    drop(film.child.wait());
                    bail!(
                        "simctl recordVideo ended before recording: {}",
                        said.join(" ")
                    );
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    // Dropping the film interrupts it, so the device ends the recording too.
                    drop(film);
                    bail!(
                        "simctl recordVideo did not start in {} s: {}",
                        FILM_START.as_secs(),
                        said.join(" ")
                    );
                }
            }
        }
    }

    /// A device keeps the recording of a client that was killed and then refuses every new
    /// one. A trial film finds that while the device is leased, before the check's command
    /// starts, and booting the device again ends it.
    fn free_recorder(&self, log: &mut impl std::io::Write) -> Result<()> {
        let trial = tempfile::Builder::new().suffix(".mp4").tempfile()?;
        match self.start_film(trial.path()) {
            Ok(film) => {
                drop(film);
                Ok(())
            }
            Err(e) if format!("{e:#}").contains("already in progress") => {
                writeln!(
                    log,
                    "record: {} still held a recording a stopped run left; booting it again",
                    self.name
                )?;
                simctl(&["shutdown", &self.udid])?;
                simctl(&["boot", &self.udid])?;
                self.boot_status()
            }
            Err(e) => Err(e),
        }
    }

    /// The primary display of a device with several, which `simctl io` asks to be named:
    /// a trial still says which there are.
    fn primary_display(&self) -> Result<Option<String>> {
        let trial = tempfile::Builder::new().suffix(".png").tempfile()?;
        let path = trial.path().to_string_lossy().into_owned();
        let Err(e) = simctl(&["io", &self.udid, "screenshot", "--type=png", &path]) else {
            return Ok(None);
        };
        Ok(Some(primary_in(&format!("{e:#}")).with_context(|| {
            format!("taking a still of the simulator {}: {e:#}", self.name)
        })?))
    }

    /// `simctl io <udid> <command> <rest>`, on the primary display.
    fn io(&self, command: &str, rest: &[&str]) -> Vec<String> {
        let mut all = vec!["io".to_string(), self.udid.clone(), command.to_string()];
        all.extend(self.display.clone());
        all.extend(rest.iter().map(|a| (*a).to_string()));
        all
    }

    /// Ends every app that runs, removes every app a run installed and resets the keychain.
    /// A recording a killed run left going is ended too: the device takes one at a time.
    fn clean(&self) -> Result<()> {
        drop(
            Command::new("pkill")
                .args([
                    "-INT",
                    "-f",
                    &format!("simctl io {} recordVideo", self.udid),
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status(),
        );
        for app in self.running()? {
            drop(simctl(&["terminate", &self.udid, &app.1]));
        }
        for bundle in self.installed()? {
            simctl(&["uninstall", &self.udid, &bundle])?;
        }
        simctl(&["keychain", &self.udid, "reset"])?;
        Ok(())
    }

    /// The bundle ids of apps a user installed (not the system's own).
    fn installed(&self) -> Result<Vec<String>> {
        let plist = simctl(&["listapps", &self.udid])?;
        let mut convert = Command::new("plutil")
            .args(["-convert", "json", "-o", "-", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("running plutil")?;
        std::io::Write::write_all(
            &mut convert.stdin.take().context("plutil's input")?,
            plist.as_bytes(),
        )?;
        let out = convert.wait_with_output()?;
        let apps: serde_json::Map<String, serde_json::Value> =
            serde_json::from_slice(&out.stdout).context("reading simctl's list of apps")?;
        Ok(apps
            .into_iter()
            .filter(|(_, app)| app["ApplicationType"] == "User")
            .map(|(bundle, _)| bundle)
            .collect())
    }

    /// The apps running, as (pid, bundle id), from the device's launchd.
    fn running(&self) -> Result<Vec<(i32, String)>> {
        let text = simctl(&["spawn", &self.udid, "launchctl", "list"])?;
        Ok(text.lines().filter_map(running_app).collect())
    }
}

/// `--display=1` from simctl's "Cannot choose between 2 displays" list, the line that
/// ends in "(primary)".
fn primary_in(message: &str) -> Option<String> {
    message
        .lines()
        .map(str::trim)
        .find(|l| l.contains("(primary)"))?
        .split_whitespace()
        .find(|w| w.starts_with("--display="))
        .map(str::to_string)
}

/// An app in a line of `launchctl list`: `123  0  UIKitApplication:com.x.app[ab12][rb-legacy]`.
fn running_app(line: &str) -> Option<(i32, String)> {
    let mut cols = line.split_whitespace();
    let pid = cols.next()?.parse().ok()?;
    let label = cols.nth(1)?.strip_prefix("UIKitApplication:")?;
    let bundle = label.split('[').next()?;
    Some((pid, bundle.to_string()))
}

impl Drop for Simulator {
    fn drop(&mut self) {
        // Left booted for the next run; a failed clean is redone when it is next leased.
        drop(self.clean());
    }
}

/// Where the leases are locked: one folder per user, read from HOME alone, since the
/// simulators are the user's whatever TMPDIR or XDG_CACHE_HOME a process has.
fn locks() -> Result<PathBuf> {
    let home =
        std::env::var_os("HOME").context("HOME is not set, so simulators cannot be leased")?;
    let dir = PathBuf::from(home).join("Library/Caches/fearlezz/simulators");
    std::fs::create_dir_all(&dir).with_context(|| format!("making {}", dir.display()))?;
    Ok(dir)
}

/// The lease of a device, locked without waiting; None when another run holds it.
fn lock_device(udid: &str) -> Result<Option<File>> {
    lock(&locks()?.join(format!("{udid}.lock")), false)
}

/// `path` locked, waiting for it when `wait`; None when it is held and not waited for.
fn lock(path: &Path, wait: bool) -> Result<Option<File>> {
    let file = File::create(path).with_context(|| format!("opening {}", path.display()))?;
    let how = if wait {
        libc::LOCK_EX
    } else {
        libc::LOCK_EX | libc::LOCK_NB
    };
    // SAFETY: flock on a descriptor this function owns; it is released when the file closes.
    let locked = unsafe { libc::flock(file.as_raw_fd(), how) } == 0;
    Ok(locked.then_some(file))
}

/// Whether the film at `path` is whole: there, not empty, and no temporary file of the
/// simulator's (`<name>.sb-...`) still beside it.
fn written(path: &Path) -> bool {
    let whole = std::fs::metadata(path).is_ok_and(|m| m.len() > 0);
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return whole;
    };
    let temporary = format!("{}.sb-", name.to_string_lossy());
    whole
        && std::fs::read_dir(dir).is_ok_and(|entries| {
            !entries
                .flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with(&temporary))
        })
}

/// A PNG's width and height, from its header.
fn png_size(path: &Path) -> Result<(usize, usize)> {
    let bytes = std::fs::read(path)?;
    if bytes.len() < 24 || &bytes[1..4] != b"PNG" {
        bail!("{} is not a PNG", path.display());
    }
    let at = |i: usize| u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    Ok((at(16) as usize, at(20) as usize))
}

impl super::Screen for Simulator {
    type Film = Film;

    /// The apps running on the device, each as one window of the whole screen: a phone
    /// shows one app at a time. Its size is unknown until a still, and only ever compared
    /// with the size filter of [`super::choose`].
    fn windows(&mut self) -> Result<Vec<WindowInfo>> {
        Ok(self
            .running()?
            .into_iter()
            .map(|(pid, bundle)| WindowInfo {
                id: pid.unsigned_abs(),
                pid,
                app: bundle.clone(),
                bundle,
                title: String::new(),
                x: 0.0,
                y: 0.0,
                width: 1000.0,
                height: 1000.0,
            })
            .collect())
    }

    fn still(&mut self, _window: u32, path: &Path, _frame: &Frame) -> Result<(usize, usize)> {
        let args = self.io("screenshot", &["--type=png", &path.to_string_lossy()]);
        simctl(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
        png_size(path)
    }

    fn film(&mut self, _window: u32, path: &Path, _frame: &Frame) -> Result<Film> {
        self.start_film(path)
    }

    fn park(&mut self, _window: u32) -> Result<Parking> {
        Ok(Parking::Parked("(a simulator of Fearlezz's own)".into()))
    }

    fn front(&mut self) -> Result<i32> {
        bail!("a simulator of Fearlezz's own has no app in front on this Mac")
    }

    fn bring_front(&mut self, _pid: i32) -> Result<()> {
        Ok(())
    }

    fn put_back(&mut self) -> Result<usize> {
        Ok(0)
    }
}

/// A film `simctl io recordVideo` writes; it records only frames that change, so a still
/// screen makes a short file.
pub(crate) struct Film {
    child: Child,
    path: PathBuf,
}

/// A film dropped without [`Filming::stop`] (the recorder failed) still ends, so no
/// recording runs on into the device's next lease.
impl Drop for Film {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            interrupt(&self.child);
            drop(self.child.wait());
        }
    }
}

/// SIGINT, which ends `recordVideo` and writes its file.
fn interrupt(child: &Child) {
    if let Ok(pid) = i32::try_from(child.id()) {
        // SAFETY: a signal to a child this film started and has not yet waited for.
        unsafe { libc::kill(pid, libc::SIGINT) };
    }
}

impl super::Filming for Film {
    fn ended(&mut self) -> bool {
        self.child.try_wait().ok().flatten().is_some()
    }

    fn stop(mut self) -> Result<()> {
        if self.child.try_wait()?.is_none() {
            interrupt(&self.child);
        }
        let status = self.child.wait()?;
        // The simulator writes the film beside it under a temporary name and moves it in,
        // sometimes after simctl has ended.
        let clock = std::time::Instant::now();
        while !written(&self.path) {
            if clock.elapsed() >= FILM_WRITE {
                bail!(
                    "simctl recordVideo ended with {status} and the film was not written in {} s",
                    FILM_WRITE.as_secs()
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_app_is_read_from_launchctl_and_other_jobs_are_not() {
        assert_eq!(
            running_app("4512\t0\tUIKitApplication:com.apple.Preferences[1d2c][rb-legacy]"),
            Some((4512, "com.apple.Preferences".into()))
        );
        assert_eq!(running_app("-\t0\tUIKitApplication:com.x.app[1][rb]"), None);
        assert_eq!(running_app("120\t0\tcom.apple.backboardd"), None);
    }

    #[test]
    fn the_primary_display_is_read_from_simctls_list() {
        let message = "Cannot choose between 2 displays. Specify one with --display:\n    \
                       --display=3  LCD-1 (primary-1) 2007x2853\n    \
                       --display=1  LCD (primary) 1398x2034\nInvalid argument";
        assert_eq!(primary_in(message).as_deref(), Some("--display=1"));
    }

    #[test]
    fn a_film_is_whole_once_it_has_bytes_and_no_temporary_file_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        let film = dir.path().join("home.mp4");
        assert!(!written(&film), "not there");
        std::fs::write(&film, b"").unwrap();
        assert!(!written(&film), "empty");
        std::fs::write(&film, b"film").unwrap();
        std::fs::write(dir.path().join("home.mp4.sb-c5fd9852-R18j5j"), b"more").unwrap();
        assert!(!written(&film), "still being written");
        std::fs::remove_file(dir.path().join("home.mp4.sb-c5fd9852-R18j5j")).unwrap();
        assert!(written(&film));
    }

    #[test]
    fn runtimes_order_by_version() {
        assert!(version("27.2") > version("26.5"));
        assert!(version("18.10") > version("18.9"));
    }
}
