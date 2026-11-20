//! A Linux display of Fearlezz's own: Xvfb on a display number it picks itself, the window
//! the check's command opens there read through the X protocol (x11rb), and its frames
//! encoded by ffmpeg, the one Fearlezz compares pictures with. Nothing of the developer's
//! screen is seen, and no window manager runs, so a window is exactly what the app draws.

use std::io::{Read as _, Write as _};
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt as _, ImageFormat, ImageOrder, MapState, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;

use super::{Frame, Parking, Unavailable, WindowInfo};

/// Frames a second of a film: enough to follow a flow, small enough to grab on a runner.
const FPS: u32 = 15;
/// How long Xvfb gets to take a display and accept connections.
const START: Duration = Duration::from_secs(10);

/// None when this machine can start a Linux display and encode what it shows.
pub(crate) fn unavailable() -> Option<Unavailable> {
    if !on_path("Xvfb") {
        return Some(Unavailable {
            why: "a Linux display needs Xvfb, which is not on PATH".into(),
            fix: "install Xvfb, such as `apt install xvfb` (GitHub's Ubuntu runners have it)"
                .into(),
        });
    }
    crate::ffmpeg::missing().map(|why| Unavailable {
        why: format!("a Linux display needs ffmpeg to encode what it shows: {why}"),
        fix: "add ffmpeg to the project's [env] shell, or install it, such as `apt install \
              ffmpeg`"
            .into(),
    })
}

fn on_path(tool: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(tool).is_file()))
}

/// A running Xvfb and a connection to it. Stopped when dropped.
pub(crate) struct Display {
    number: u32,
    xvfb: Child,
    conn: RustConnection,
    root: Window,
    /// Its width and height in pixels.
    size: (u32, u32),
}

impl Display {
    /// Starts Xvfb at `size` on a display number it picks (`-displayfd`), so two runs at
    /// once never take the same one.
    pub(crate) fn start((width, height): (u32, u32)) -> Result<Display> {
        let (read, write) = pipe()?;
        let mut command = Command::new("Xvfb");
        // On Linux Xvfb ends with Fearlezz even when Fearlezz is killed, so no display is
        // left behind by a run an agent's harness stopped.
        #[cfg(target_os = "linux")]
        // SAFETY: prctl is async signal safe and touches only the child being started.
        unsafe {
            std::os::unix::process::CommandExt::pre_exec(&mut command, || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
            });
        }
        let mut xvfb = command
            .arg("-displayfd")
            .arg(write.as_raw_fd().to_string())
            .args(["-screen", "0", &format!("{width}x{height}x24")])
            .args(["-nolisten", "tcp", "-noreset"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("starting Xvfb")?;
        drop(write);
        let number = read_number(read, START).inspect_err(|_| end(&mut xvfb))?;
        let name = format!(":{number}");
        let clock = Instant::now();
        let (conn, screen) = loop {
            match x11rb::connect(Some(&name)) {
                Ok(c) => break c,
                Err(e) if clock.elapsed() >= START => {
                    end(&mut xvfb);
                    bail!("connecting to Xvfb on {name}: {e}");
                }
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        };
        let root = conn.setup().roots[screen].root;
        let display = Display {
            number,
            xvfb,
            conn,
            root,
            size: (width, height),
        };
        display.pixel_format()?;
        Ok(display)
    }

    fn area(&self) -> Area {
        Area {
            root: self.root,
            width: i32::try_from(self.size.0).unwrap_or(i32::MAX),
            height: i32::try_from(self.size.1).unwrap_or(i32::MAX),
        }
    }

    /// The display's name, for `DISPLAY`.
    pub(crate) fn name(&self) -> String {
        format!(":{}", self.number)
    }

    /// Frames are read as 32 bit pixels, blue first: what Xvfb serves at depth 24 on every
    /// machine this runs on. Anything else is refused rather than recorded wrong.
    fn pixel_format(&self) -> Result<()> {
        let setup = self.conn.setup();
        let bits = setup
            .pixmap_formats
            .iter()
            .find(|f| f.depth == 24)
            .map(|f| f.bits_per_pixel);
        if bits != Some(32) || setup.image_byte_order != ImageOrder::LSB_FIRST {
            bail!(
                "Xvfb serves pixels of {bits:?} bits in {:?} order; only 32 bit pixels, blue \
                 first, can be recorded",
                setup.image_byte_order
            );
        }
        Ok(())
    }

    fn text(&self, window: Window, property: AtomEnum) -> Option<String> {
        let reply = self
            .conn
            .get_property(false, window, property, AtomEnum::ANY, 0, 1024)
            .ok()?
            .reply()
            .ok()?;
        Some(String::from_utf8_lossy(&reply.value).into_owned())
    }

    fn pid(&self, window: Window) -> i32 {
        let atom = self
            .conn
            .intern_atom(true, b"_NET_WM_PID")
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.atom);
        atom.filter(|a| *a != 0)
            .and_then(|a| {
                self.conn
                    .get_property(false, window, a, AtomEnum::CARDINAL, 0, 1)
                    .ok()?
                    .reply()
                    .ok()?
                    .value32()?
                    .next()
            })
            .and_then(|pid| i32::try_from(pid).ok())
            .unwrap_or(0)
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        end(&mut self.xvfb);
    }
}

/// Stops Xvfb with SIGTERM, so it removes its lock file and socket; SIGKILL only when it
/// has not ended within a second.
fn end(xvfb: &mut Child) {
    if !matches!(xvfb.try_wait(), Ok(None)) {
        return;
    }
    if let Ok(pid) = i32::try_from(xvfb.id()) {
        // SAFETY: a signal to the Xvfb this display started and has not yet waited for.
        unsafe { libc::kill(pid, libc::SIGTERM) };
    }
    let clock = Instant::now();
    while clock.elapsed() < Duration::from_secs(1) {
        if !matches!(xvfb.try_wait(), Ok(None)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(xvfb.kill());
    drop(xvfb.wait());
}

/// A pipe whose write end the child inherits (Xvfb writes its display number there).
fn pipe() -> Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    // SAFETY: `fds` holds the two descriptors pipe(2) writes; both are owned from here on.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        bail!(
            "making a pipe for Xvfb: {}",
            std::io::Error::last_os_error()
        );
    }
    // SAFETY: pipe(2) succeeded, so both are open descriptors nothing else owns.
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    // SAFETY: setting close-on-exec on our own descriptor; the read end must not reach Xvfb.
    unsafe { libc::fcntl(read.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
    Ok((read, write))
}

/// The display number Xvfb writes once it listens, a line of digits.
fn read_number(read: OwnedFd, within: Duration) -> Result<u32> {
    let (tx, rx) = std::sync::mpsc::channel();
    // Read up to the newline, whether or not Xvfb closes the pipe after it.
    std::thread::spawn(move || {
        let mut file = std::fs::File::from(read);
        let mut text = String::new();
        let mut byte = [0; 1];
        let got = loop {
            match file.read(&mut byte) {
                Ok(0) => break Ok(()),
                Ok(_) if byte[0] == b'\n' => break Ok(()),
                Ok(_) => text.push(char::from(byte[0])),
                Err(e) => break Err(e),
            }
        };
        drop(tx.send(got.map(|()| text)));
    });
    let text = match rx.recv_timeout(within) {
        Ok(Ok(text)) => text,
        Ok(Err(e)) => bail!("reading Xvfb's display number: {e}"),
        Err(_) => bail!("Xvfb took no display in {} s", within.as_secs()),
    };
    text.trim()
        .parse()
        .with_context(|| format!("Xvfb wrote \"{}\" for its display number", text.trim()))
}

impl super::Screen for Display {
    type Film = Film;

    /// The top level windows the app mapped: with no window manager, the root's children.
    fn windows(&mut self) -> Result<Vec<WindowInfo>> {
        let tree = self.conn.query_tree(self.root)?.reply()?;
        let mut windows = Vec::new();
        for id in tree.children {
            let Ok(attrs) = self.conn.get_window_attributes(id)?.reply() else {
                continue;
            };
            if attrs.map_state != MapState::VIEWABLE
                || attrs.override_redirect
                || attrs.class != WindowClass::INPUT_OUTPUT
            {
                continue;
            }
            let Ok(geometry) = self.conn.get_geometry(id)?.reply() else {
                continue;
            };
            let class = self.text(id, AtomEnum::WM_CLASS).unwrap_or_default();
            windows.push(WindowInfo {
                id,
                pid: self.pid(id),
                bundle: String::new(),
                // WM_CLASS is the instance and the class, each ended by a zero byte.
                app: class.split('\0').nth(1).unwrap_or_default().to_string(),
                title: self.text(id, AtomEnum::WM_NAME).unwrap_or_default(),
                x: f64::from(geometry.x),
                y: f64::from(geometry.y),
                width: f64::from(geometry.width),
                height: f64::from(geometry.height),
            });
        }
        Ok(windows)
    }

    fn still(&mut self, window: u32, path: &Path, _frame: &Frame) -> Result<(usize, usize)> {
        let geometry = self.conn.get_geometry(window)?.reply()?;
        let size = (u32::from(geometry.width), u32::from(geometry.height));
        let pixels = grab(&self.conn, self.area(), window, size)?;
        let mut ffmpeg = encoder(size, 1, path, &["-frames:v", "1"])?;
        let mut stdin = ffmpeg.stdin.take().context("ffmpeg's input")?;
        stdin.write_all(&pixels)?;
        drop(stdin);
        finish(ffmpeg, path)?;
        Ok((size.0 as usize, size.1 as usize))
    }

    fn film(&mut self, window: u32, path: &Path, _frame: &Frame) -> Result<Film> {
        let geometry = self.conn.get_geometry(window)?.reply()?;
        let size = (u32::from(geometry.width), u32::from(geometry.height));
        let mut ffmpeg = encoder(
            size,
            FPS,
            path,
            &[
                // H.264 needs even sides.
                "-vf",
                "pad=ceil(iw/2)*2:ceil(ih/2)*2",
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-pix_fmt",
                "yuv420p",
            ],
        )?;
        let stdin = ffmpeg.stdin.take().context("ffmpeg's input")?;
        let (conn, _) = x11rb::connect(Some(&self.name()))
            .with_context(|| format!("connecting to {} to film", self.name()))?;
        let shared = Arc::new(Shared::default());
        let area = self.area();
        let grabber = {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || frames(&conn, area, window, size, stdin, &shared))
        };
        Ok(Film {
            ffmpeg: Some(ffmpeg),
            grabber: Some(grabber),
            shared,
            path: path.to_path_buf(),
        })
    }

    fn park(&mut self, _window: u32) -> Result<Parking> {
        Ok(Parking::Parked("(a display of Fearlezz's own)".into()))
    }

    fn front(&mut self) -> Result<i32> {
        bail!("a display of Fearlezz's own has no app in front")
    }

    fn bring_front(&mut self, _pid: i32) -> Result<()> {
        Ok(())
    }

    fn put_back(&mut self) -> Result<usize> {
        Ok(0)
    }
}

/// The root window and the display's size in pixels, which bound every grab.
#[derive(Clone, Copy)]
struct Area {
    root: Window,
    width: i32,
    height: i32,
}

/// The window's pixels at `size`, 32 bits each, blue first. Only the part of the window on
/// the display can be read (X refuses the rest), so a window larger than the display or
/// placed across its edge keeps black where it is off the display. A window that grew since
/// is cut to `size` and one that shrank is padded with black, so every frame of a film is
/// the size it started at.
fn grab(
    conn: &RustConnection,
    area: Area,
    window: Window,
    (width, height): (u32, u32),
) -> Result<Vec<u8>> {
    let now = conn.get_geometry(window)?.reply()?;
    let at = conn
        .translate_coordinates(window, area.root, 0, 0)?
        .reply()?;
    let (x, y) = (i32::from(at.dst_x), i32::from(at.dst_y));
    let (w, h) = (
        i32::from(now.width).min(i32::try_from(width)?),
        i32::from(now.height).min(i32::try_from(height)?),
    );
    // The visible rectangle, in the window's own coordinates.
    let (left, top) = ((-x).max(0), (-y).max(0));
    let (right, bottom) = (w.min(area.width - x), h.min(area.height - y));
    let full = width as usize * 4;
    let mut out = vec![0; full * height as usize];
    if right <= left || bottom <= top {
        return Ok(out);
    }
    let image = conn
        .get_image(
            ImageFormat::Z_PIXMAP,
            window,
            i16::try_from(left)?,
            i16::try_from(top)?,
            u16::try_from(right - left)?,
            u16::try_from(bottom - top)?,
            !0,
        )?
        .reply()?;
    let row = usize::try_from(right - left)? * 4;
    let (left, top) = (usize::try_from(left)? * 4, usize::try_from(top)?);
    for (i, line) in image.data.chunks_exact(row).enumerate() {
        let start = (top + i) * full + left;
        out[start..start + row].copy_from_slice(line);
    }
    Ok(out)
}

/// ffmpeg reading raw frames of `size` from its input into `path`.
fn encoder((width, height): (u32, u32), fps: u32, path: &Path, output: &[&str]) -> Result<Child> {
    crate::ffmpeg::ffmpeg()?
        .args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "bgr0"])
        .args(["-s", &format!("{width}x{height}")])
        .args(["-framerate", &fps.to_string(), "-i", "-"])
        .args(output)
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("starting ffmpeg")
}

fn finish(ffmpeg: Child, path: &Path) -> Result<()> {
    let out = ffmpeg.wait_with_output()?;
    if !out.status.success() {
        bail!(
            "ffmpeg could not write {}: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

/// What the grabber and its film share.
#[derive(Default)]
struct Shared {
    /// Set by the film to end the grabbing.
    stop: AtomicBool,
    /// Set by the grabber when it ended by itself: the window went, or ffmpeg did.
    ended: AtomicBool,
    /// Frames written to ffmpeg.
    frames: AtomicU32,
    /// Why grabbing ended by itself.
    why: Mutex<Option<String>>,
}

impl Shared {
    fn end(&self, why: String) {
        if let Ok(mut slot) = self.why.lock() {
            *slot = Some(why);
        }
        self.ended.store(true, Ordering::Relaxed);
    }
}

/// Grabs the window [`FPS`] times a second into ffmpeg until told to stop or the window
/// goes. A grab that runs late repeats the last frame, so the film keeps the run's time.
fn frames(
    conn: &RustConnection,
    area: Area,
    window: Window,
    size: (u32, u32),
    mut stdin: ChildStdin,
    shared: &Shared,
) {
    let tick = Duration::from_secs(1) / FPS;
    let start = Instant::now();
    let mut written: u32 = 0;
    while !shared.stop.load(Ordering::Relaxed) {
        let frame = match grab(conn, area, window, size) {
            Ok(frame) => frame,
            Err(e) => return shared.end(format!("reading the window: {e:#}")),
        };
        let due = (start.elapsed().as_secs_f64() * f64::from(FPS)) as u32 + 1;
        while written < due {
            if let Err(e) = stdin.write_all(&frame) {
                return shared.end(format!("writing to ffmpeg: {e}"));
            }
            written += 1;
            shared.frames.store(written, Ordering::Relaxed);
        }
        let next = start + tick * written;
        std::thread::sleep(next.saturating_duration_since(Instant::now()));
    }
}

/// A film of a window on the display.
pub(crate) struct Film {
    /// Taken by [`Film::stop`].
    ffmpeg: Option<Child>,
    grabber: Option<JoinHandle<()>>,
    shared: Arc<Shared>,
    path: std::path::PathBuf,
}

impl Film {
    /// Ends the grabbing; ffmpeg's input closes with the grabber.
    fn end_grabbing(&mut self) -> Result<()> {
        self.shared.stop.store(true, Ordering::Relaxed);
        match self.grabber.take().map(JoinHandle::join) {
            Some(Err(_)) => bail!("the frame grabber failed"),
            _ => Ok(()),
        }
    }
}

/// A film dropped without [`super::Filming::stop`] (the recorder failed) still ends its
/// grabber and ffmpeg.
impl Drop for Film {
    fn drop(&mut self) {
        drop(self.end_grabbing());
        if let Some(mut ffmpeg) = self.ffmpeg.take() {
            drop(ffmpeg.kill());
            drop(ffmpeg.wait());
        }
    }
}

impl super::Filming for Film {
    fn ended(&mut self) -> bool {
        self.shared.ended.load(Ordering::Relaxed)
    }

    fn stop(mut self) -> Result<()> {
        self.end_grabbing()?;
        let ffmpeg = self.ffmpeg.take().context("the film was stopped already")?;
        // A window gone before its first frame is no film; one that closed later is.
        if self.shared.frames.load(Ordering::Relaxed) == 0
            && let Some(why) = self.shared.why.lock().ok().and_then(|w| w.clone())
        {
            drop(ffmpeg.wait_with_output());
            bail!("no frame was filmed: {why}");
        }
        finish(ffmpeg, &self.path)
    }
}
