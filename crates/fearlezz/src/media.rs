//! Comparing two stills or films of a UI, tolerant to timing shifts and codec noise.
//!
//! The method (proven on 29 labelled pairs of real app films): cut each film into stable
//! states, match states across films with dynamic time warping over small thumbnails, then
//! pixel diff matched states after a light blur, ignoring blobs too small to matter. Pixels
//! are compared per color channel, so a change of hue alone is found. A pair of states caught
//! mid transition counts as unchanged when each sits inside the other film's neighbours.
//!
//! Films are compared at 720p at most, which keeps a UI's changes and costs less than half of
//! 1080p; stills keep their own size. Boxes and sizes are always in the file's own pixels.
//!
//! Films are read twice: once for the states' timing and thumbnails, and once more, both
//! films in step along the matched path, for the pixel diffs. Only a few states are in
//! memory at a time, however long or busy a film is. The second read asks ffmpeg for the
//! states' first frames only, so it decodes the film but converts and sends few frames.
#![expect(
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "pixel loops index frame buffers whose size is fixed by the frame's width and \
              height, and convert between pixel coordinates and sample values in range"
)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

/// Gaussian blur sigma that removes codec noise before differencing.
const BLUR_SIGMA: f32 = 1.0;
/// Per pixel difference in any color channel (0 to 255) that counts as different.
const PIX_THRESH: i16 = 28;
/// Films are compared no larger than this, long side by short side.
const FILM_MAX: (usize, usize) = (1280, 720);
/// Changed areas smaller than this many pixels are noise.
const MIN_BLOB: usize = 40;
/// Seconds of timing difference that still count as the same timing.
const TIMING_TOL: f64 = 0.10;
/// A state held at least this long is a moment worth checking for timing.
const STABLE_MIN: f64 = 0.15;
const THUMB_W: usize = 48;
/// Changed areas closer than this many pixels share one box.
const BOX_GROW: u32 = 12;

/// An RGB image, three bytes per pixel.
#[derive(Clone)]
pub(crate) struct Image {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

/// A stable state of a film: consecutive frames with no meaningful change.
pub(crate) struct State {
    pub start: f64,
    pub dur: f64,
    /// Index of the state's first frame in the film.
    pub frame: usize,
    /// The state's first frame, small, for matching states across films.
    pub thumb: Vec<f32>,
}

pub(crate) struct Film {
    /// Where it is read from again for the pixel diffs.
    pub path: PathBuf,
    /// Its own size.
    pub w: usize,
    pub h: usize,
    /// The size its frames are compared at.
    seen: (usize, usize),
    /// The ffmpeg filter that gives that size, when it is not its own.
    scale: Option<String>,
    pub frames: usize,
    pub total: f64,
    pub states: Vec<State>,
    /// Each frame's start and duration, in seconds, so the second read needs no probe.
    times: Vec<(f64, f64)>,
}

impl Film {
    pub(crate) fn is_still(&self) -> bool {
        self.frames == 1
    }

    /// A box at the size the film is compared at, in the film's own pixels.
    fn own_pixels(&self, b: Box4) -> Box4 {
        if self.seen == (self.w, self.h) {
            return b;
        }
        let along = |v: u32, seen: usize, own: usize, up: bool| {
            let at = f64::from(v) * own as f64 / seen as f64;
            (if up { at.ceil() } else { at.floor() } as u32).min(own as u32)
        };
        let (sw, sh) = self.seen;
        [
            along(b[0], sw, self.w, false),
            along(b[1], sh, self.h, false),
            along(b[2], sw, self.w, true),
            along(b[3], sh, self.h, true),
        ]
    }
}

/// A box in full resolution pixels: x0, y0, x1, y1 with x1 and y1 exclusive.
pub(crate) type Box4 = [u32; 4];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Verdict {
    Same,
    /// Same pictures, different timing.
    Timing,
    Changed,
}

/// A stretch of the films where the pictures differ.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Moment {
    pub before: (f64, f64),
    pub after: (f64, f64),
    #[serde(rename = "box")]
    pub area: Box4,
    pub px: usize,
}

/// One matched pair of states.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Pair {
    pub before: usize,
    pub after: usize,
    /// Frame index and start time, in seconds, of each state.
    pub before_frame: usize,
    pub after_frame: usize,
    pub before_at: f64,
    pub after_at: f64,
    pub px: usize,
    pub boxes: Vec<Box4>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Comparison {
    pub verdict: Verdict,
    pub still: bool,
    pub size_before: (usize, usize),
    pub size_after: (usize, usize),
    pub total_before: f64,
    pub total_after: f64,
    /// The largest shift of a held state, in seconds, and where it is in the before film.
    pub max_shift: f64,
    pub max_shift_at: Option<f64>,
    pub states_before: usize,
    pub states_after: usize,
    pub moments: Vec<Moment>,
    pub pairs: Vec<Pair>,
}

impl Comparison {
    /// One line for people and agents.
    pub(crate) fn summary(&self) -> String {
        match self.verdict {
            Verdict::Same => "same".into(),
            Verdict::Timing => format!(
                "only timing: {:.2} s to {:.2} s, largest shift {:+.2} s at {:.2} s",
                self.total_before,
                self.total_after,
                self.max_shift,
                self.max_shift_at.unwrap_or(0.0)
            ),
            Verdict::Changed if self.size_before != self.size_after => format!(
                "changed: size {}x{} to {}x{}",
                self.size_before.0, self.size_before.1, self.size_after.0, self.size_after.1
            ),
            Verdict::Changed if self.still => {
                let b = self.moments[0].area;
                format!("changed: x {} to {}, y {} to {}", b[0], b[2], b[1], b[3])
            }
            Verdict::Changed => {
                let at: Vec<String> = self
                    .moments
                    .iter()
                    .map(|m| format!("{:.2} s", m.after.0))
                    .collect();
                format!("changed at {}", at.join(", "))
            }
        }
    }
}

// ----- decoding -------------------------------------------------------------------------

/// Decodes a still or film with ffmpeg, keeping only its stable states.
/// Reads a film's states: their timing and thumbnails.
pub(crate) fn decode(path: &Path) -> Result<Film> {
    let Probe {
        w,
        h,
        format,
        times,
    } = probe(path)?;
    let seen = if times.len() > 1 {
        fit(w, h, FILM_MAX)
    } else {
        (w, h)
    };
    let scale = (seen != (w, h)).then(|| {
        // Scaling in the film's own YUV format, before the RGB conversion, costs about a third
        // of scaling RGB. Other formats (a GIF's palette) are scaled as RGB.
        let keep = if format.starts_with("yuv") || format.starts_with("nv") {
            format!(",format={format}")
        } else {
            String::new()
        };
        format!("scale={}:{}:flags=area{keep}", seen.0, seen.1)
    });
    let mut frames = Frames::open(path, seen, scale.as_deref(), times, None)?;
    let mut states: Vec<State> = Vec::new();
    // The current state's first frame, which each new frame is compared with.
    let mut first: Option<Image> = None;
    while frames.next()? {
        let (t, d) = frames.time();
        if let (Some(state), Some(f)) = (states.last_mut(), &first)
            && diff(f, &frames.buf, false).px == 0
        {
            state.dur = frames.end - state.start;
            continue;
        }
        states.push(State {
            start: t,
            dur: d,
            frame: frames.read - 1,
            thumb: thumb(&frames.buf),
        });
        first = Some(frames.buf.clone());
    }
    Ok(Film {
        path: path.to_path_buf(),
        w,
        h,
        seen,
        scale,
        frames: frames.read,
        total: frames.end,
        states,
        times: std::mem::take(&mut frames.times),
    })
}

/// Decodes a before and an after side by side.
pub(crate) fn decode_both(before: &Path, after: &Path) -> Result<(Film, Film)> {
    std::thread::scope(|s| {
        let b = s.spawn(|| decode(before));
        let a = decode(after);
        let b = b.join().map_err(|panic| {
            anyhow::anyhow!(
                "decoding {} crashed: {}",
                before.display(),
                crate::panic_text(&*panic)
            )
        })??;
        Ok((b, a?))
    })
}

/// A film's frames as ffmpeg streams them, one at a time.
struct Frames {
    path: PathBuf,
    child: std::process::Child,
    out: std::process::ChildStdout,
    errors: Option<std::thread::JoinHandle<String>>,
    times: Vec<(f64, f64)>,
    /// When only some frames are asked for, their indices in the film, in order.
    pick: Option<Vec<usize>>,
    /// How many frames ffmpeg sent.
    sent: usize,
    /// The last frame read, at the size it is compared at.
    buf: Image,
    /// How many frames of the film were read: one past the last frame's index.
    read: usize,
    /// Where the last frame read ends, in seconds.
    end: f64,
    done: bool,
}

impl Frames {
    /// Streams every frame at size `seen`, which the `scale` filter gives when it is not the
    /// film's own, or with `pick` only the frames at those indices (ascending). Times come
    /// from the probe, never from the filters.
    fn open(
        path: &Path,
        seen: (usize, usize),
        scale: Option<&str>,
        times: Vec<(f64, f64)>,
        pick: Option<Vec<usize>>,
    ) -> Result<Frames> {
        let (w, h) = seen;
        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-v", "error", "-i"]).arg(path);
        let mut filters = Vec::new();
        if let Some(pick) = &pick {
            // The filter still decodes every frame, but converts and sends only these.
            let select: Vec<String> = pick.iter().map(|n| format!("eq(n\\,{n})")).collect();
            filters.push(format!("select='{}'", select.join("+")));
        }
        // After picking, so only the frames sent are scaled.
        filters.extend(scale.map(str::to_owned));
        if !filters.is_empty() {
            cmd.args(["-vf", &filters.join(",")]);
        }
        let mut child = cmd
            .args([
                "-fps_mode",
                "passthrough",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgb24",
                "-",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("running ffmpeg; Fearlezz needs it to read stills and films")?;
        let out = child
            .stdout
            .take()
            .context("ffmpeg gave no output to read")?;
        // Read its error output while the frames stream, so a film that logs a lot cannot fill
        // the pipe and stop ffmpeg.
        let errors = child.stderr.take().map(|mut e| {
            std::thread::spawn(move || {
                let mut text = String::new();
                match e.read_to_string(&mut text) {
                    Ok(_) => text,
                    Err(read) => format!("(its error output could not be read: {read})"),
                }
            })
        });
        Ok(Frames {
            path: path.to_path_buf(),
            child,
            out,
            errors,
            times,
            pick,
            sent: 0,
            buf: Image {
                w,
                h,
                px: vec![0u8; w * h * 3],
            },
            read: 0,
            end: 0.0,
            done: false,
        })
    }

    /// Reads the next frame into `buf`; false after the last.
    fn next(&mut self) -> Result<bool> {
        if self.done {
            return Ok(false);
        }
        match self.out.read_exact(&mut self.buf.px) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                self.done = true;
                let status = self.child.wait()?;
                let err = self
                    .errors
                    .take()
                    .and_then(|t| t.join().ok())
                    .unwrap_or_default();
                if !status.success() || self.sent == 0 {
                    bail!(
                        "ffmpeg could not read {}: {}",
                        self.path.display(),
                        err.trim()
                    );
                }
                return Ok(false);
            }
            Err(e) => {
                // Stopping it is best effort; the read error is what to report.
                drop(self.child.kill());
                drop(self.child.wait());
                self.done = true;
                return Err(e)
                    .with_context(|| format!("reading frames of {}", self.path.display()));
            }
        }
        let index = match &self.pick {
            Some(pick) => pick.get(self.sent).copied().unwrap_or(self.read),
            None => self.read,
        };
        let (t, d) = self.times.get(index).copied().unwrap_or((self.end, 0.05));
        self.read = index + 1;
        self.sent += 1;
        self.end = t + d;
        Ok(true)
    }

    /// The last frame's start and duration, in seconds.
    fn time(&self) -> (f64, f64) {
        let (t, d) = self
            .times
            .get(self.read - 1)
            .copied()
            .unwrap_or((self.end, 0.05));
        (t, d)
    }
}

impl Drop for Frames {
    fn drop(&mut self) {
        // A comparison that stops early leaves ffmpeg mid film; it must not linger.
        if !self.done {
            drop(self.child.kill());
            drop(self.child.wait());
        }
    }
}

/// The full first frames of one film's states around the place the comparison has reached:
/// it reads ahead as asked and forgets states the path has passed. The states are known from
/// the first read, so this read only picks their frames out.
struct Window {
    frames: Frames,
    /// Each state's first frame.
    starts: Vec<usize>,
    /// States from index `first` on, in order.
    images: VecDeque<Image>,
    first: usize,
}

impl Window {
    fn open(film: &Film) -> Result<Window> {
        let starts: Vec<usize> = film.states.iter().map(|s| s.frame).collect();
        // Picking pays off when most frames belong to a state they do not start.
        let pick = (starts.len() * 2 <= film.frames).then(|| starts.clone());

        Ok(Window {
            frames: Frames::open(
                &film.path,
                film.seen,
                film.scale.as_deref(),
                film.times.clone(),
                pick,
            )?,
            starts,
            images: VecDeque::new(),
            first: 0,
        })
    }

    /// Makes states `lo..=hi` available (as far as the film has them), dropping earlier ones.
    fn span(&mut self, lo: usize, hi: usize) -> Result<()> {
        while self.first < lo && !self.images.is_empty() {
            self.images.pop_front();
            self.first += 1;
        }
        if self.images.is_empty() {
            self.first = self.first.max(lo);
        }
        while self.first + self.images.len() <= hi {
            let Some(&frame) = self.starts.get(self.first + self.images.len()) else {
                break;
            };
            while self.frames.read <= frame {
                if !self.frames.next()? {
                    bail!(
                        "{} ended early on the second read",
                        self.frames.path.display()
                    );
                }
            }
            self.images.push_back(self.frames.buf.clone());
        }
        Ok(())
    }

    /// State `c` and its neighbours, of a film with `len` states.
    fn near(&self, c: usize, len: usize) -> Result<Vec<&Image>> {
        (c.saturating_sub(1)..(c + 2).min(len))
            .map(|x| self.image(x))
            .collect()
    }

    fn image(&self, i: usize) -> Result<&Image> {
        i.checked_sub(self.first)
            .and_then(|k| self.images.get(k))
            .with_context(|| format!("state {i} of {} is not read", self.frames.path.display()))
    }
}

/// A film's size, pixel format and each frame's start and duration, in seconds.
struct Probe {
    w: usize,
    h: usize,
    format: String,
    times: Vec<(f64, f64)>,
}

fn probe(path: &Path) -> Result<Probe> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        // Packets carry the frames' times without decoding them; sorted by time they are the
        // frames in the order ffmpeg sends them.
        .arg("stream=width,height,pix_fmt:packet=pts_time,duration_time")
        .args(["-of", "json"])
        .arg(path)
        .output()
        .context("running ffprobe; Fearlezz needs it to read stills and films")?;
    if !out.status.success() {
        bail!(
            "ffprobe could not read {}: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let info: serde_json::Value = serde_json::from_slice(&out.stdout)?;
    let stream = &info["streams"][0];
    let w = stream["width"].as_u64().context("no width")? as usize;
    let h = stream["height"].as_u64().context("no height")? as usize;
    let format = stream["pix_fmt"].as_str().unwrap_or_default().to_owned();
    if w == 0 || h == 0 {
        bail!("{} is not a still or film ffmpeg can read", path.display());
    }
    let num = |v: &serde_json::Value| {
        let s = v.as_str()?;
        s.parse::<f64>().ok()
    };
    let mut raw: Vec<(Option<f64>, Option<f64>)> = info["packets"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|f| (num(&f["pts_time"]), num(&f["duration_time"])))
                .collect()
        })
        .unwrap_or_default();
    if raw.iter().all(|(t, _)| t.is_some()) {
        raw.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    }
    let mut times = Vec::with_capacity(raw.len());
    for (i, (t, d)) in raw.iter().enumerate() {
        let start = t.unwrap_or(i as f64 * 0.05);
        let next = raw.get(i + 1).and_then(|n| n.0).map(|n| n - start);
        times.push((start, d.or(next).unwrap_or(0.05)));
    }
    Ok(Probe {
        w,
        h,
        format,
        times,
    })
}

/// The largest size within `max` (long side by short side) with the aspect of `w` by `h`;
/// `w` by `h` itself when it fits.
fn fit(w: usize, h: usize, max: (usize, usize)) -> (usize, usize) {
    let (long, short) = (w.max(h), w.min(h));
    let scale = (max.0 as f64 / long as f64)
        .min(max.1 as f64 / short as f64)
        .min(1.0);
    if scale >= 1.0 {
        return (w, h);
    }
    let side = |v: usize| ((v as f64 * scale).round() as usize).max(1);
    (side(w), side(h))
}

/// Luma as Pillow computes it, so results match the prototype.
fn luma(rgb: &[u8]) -> Vec<u8> {
    rgb.as_chunks::<3>()
        .0
        .iter()
        .map(|p| {
            ((u32::from(p[0]) * 19595 + u32::from(p[1]) * 38470 + u32::from(p[2]) * 7471 + 0x8000)
                >> 16) as u8
        })
        .collect()
}

// ----- resampling -----------------------------------------------------------------------

/// Triangle filter weights for resizing `from` samples to `to`, as Pillow's bilinear.
fn weights(from: usize, to: usize) -> Vec<(usize, Vec<f32>)> {
    let scale = from as f64 / to as f64;
    let support = scale.max(1.0);
    (0..to)
        .map(|x| {
            let center = (x as f64 + 0.5) * scale;
            let lo = ((center - support + 0.5).floor().max(0.0)) as usize;
            let hi = ((center + support + 0.5).floor() as usize).min(from);
            let mut ws: Vec<f32> = (lo..hi)
                .map(|i| {
                    let d = ((i as f64 - center + 0.5) / support).abs();
                    (1.0 - d).max(0.0) as f32
                })
                .collect();
            let sum: f32 = ws.iter().sum();
            if sum > 0.0 {
                ws.iter_mut().for_each(|v| *v /= sum);
            }
            (lo, ws)
        })
        .collect()
}

fn resize(px: &[u8], w: usize, h: usize, nw: usize, nh: usize) -> Vec<u8> {
    let ch = 1;
    let wx = weights(w, nw);
    let wy = weights(h, nh);
    let mut tmp = vec![0f32; nw * h * ch];
    for y in 0..h {
        for (x, (lo, ws)) in wx.iter().enumerate() {
            for c in 0..ch {
                let mut acc = 0.0;
                for (k, wt) in ws.iter().enumerate() {
                    acc += f32::from(px[(y * w + lo + k) * ch + c]) * wt;
                }
                tmp[(y * nw + x) * ch + c] = acc;
            }
        }
    }
    let mut out = vec![0u8; nw * nh * ch];
    for (y, (lo, ws)) in wy.iter().enumerate() {
        for x in 0..nw {
            for c in 0..ch {
                let mut acc = 0.0;
                for (k, wt) in ws.iter().enumerate() {
                    acc += tmp[((lo + k) * nw + x) * ch + c] * wt;
                }
                out[(y * nw + x) * ch + c] = acc.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// A small gray copy, for matching states across films.
fn thumb(g: &Image) -> Vec<f32> {
    let th = ((THUMB_W * g.h) as f64 / g.w as f64).round().max(1.0) as usize;
    resize(&luma(&g.px), g.w, g.h, THUMB_W, th)
        .into_iter()
        .map(f32::from)
        .collect()
}

// ----- masks ----------------------------------------------------------------------------

struct Mask {
    w: usize,
    h: usize,
    on: Vec<bool>,
}

impl Mask {
    fn count(&self) -> usize {
        self.on.iter().filter(|v| **v).count()
    }

    fn get(&self, x: isize, y: isize) -> bool {
        x >= 0
            && y >= 0
            && (x as usize) < self.w
            && (y as usize) < self.h
            && self.on[y as usize * self.w + x as usize]
    }

    /// Erosion then dilation with a cross; outside the mask counts as off.
    fn open(&self) -> Mask {
        let (w, h) = (self.w, self.h);
        let mut eroded = vec![false; w * h];
        for y in 1..h.saturating_sub(1) {
            for x in 1..w.saturating_sub(1) {
                let i = y * w + x;
                let on = &self.on;
                eroded[i] = on[i] && on[i - 1] && on[i + 1] && on[i - w] && on[i + w];
            }
        }
        let mut on = vec![false; w * h];
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                on[i] = eroded[i]
                    || (x > 0 && eroded[i - 1])
                    || (x + 1 < w && eroded[i + 1])
                    || (y > 0 && eroded[i - w])
                    || (y + 1 < h && eroded[i + w]);
            }
        }
        Mask { w, h, on }
    }

    /// Connected parts (4 neighbours): for each, its pixels' indices.
    fn parts(&self) -> Vec<Vec<usize>> {
        let mut seen = vec![false; self.on.len()];
        let mut parts = Vec::new();
        for start in 0..self.on.len() {
            if !self.on[start] || seen[start] {
                continue;
            }
            seen[start] = true;
            let mut stack = vec![start];
            let mut part = Vec::new();
            while let Some(i) = stack.pop() {
                part.push(i);
                let (x, y) = ((i % self.w) as isize, (i / self.w) as isize);
                for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                    let (nx, ny) = (x + dx, y + dy);
                    if self.get(nx, ny) {
                        let j = ny as usize * self.w + nx as usize;
                        if !seen[j] {
                            seen[j] = true;
                            stack.push(j);
                        }
                    }
                }
            }
            parts.push(part);
        }
        parts
    }

    /// Keeps only parts of at least `MIN_BLOB` pixels.
    fn drop_specks(&mut self) {
        for part in self.parts() {
            if part.len() < MIN_BLOB {
                for i in part {
                    self.on[i] = false;
                }
            }
        }
    }

    /// Bounding boxes of the areas left after growing the mask by `grow` pixels (city block).
    fn boxes(&self, grow: u32) -> Vec<Box4> {
        let far = u32::MAX / 2;
        let mut dist: Vec<u32> = self.on.iter().map(|v| if *v { 0 } else { far }).collect();
        let (w, h) = (self.w, self.h);
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if x > 0 {
                    dist[i] = dist[i].min(dist[i - 1] + 1);
                }
                if y > 0 {
                    dist[i] = dist[i].min(dist[i - w] + 1);
                }
            }
        }
        for y in (0..h).rev() {
            for x in (0..w).rev() {
                let i = y * w + x;
                if x + 1 < w {
                    dist[i] = dist[i].min(dist[i + 1] + 1);
                }
                if y + 1 < h {
                    dist[i] = dist[i].min(dist[i + w] + 1);
                }
            }
        }
        let grown = Mask {
            w,
            h,
            on: dist.iter().map(|d| *d <= grow).collect(),
        };
        grown
            .parts()
            .into_iter()
            .map(|part| {
                let mut b = [u32::MAX, u32::MAX, 0, 0];
                for i in part {
                    let (x, y) = ((i % w) as u32, (i / w) as u32);
                    b = [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)];
                }
                b
            })
            .collect()
    }
}

/// Separable Gaussian blur with mirrored edges, as scipy's default.
fn blur(px: &[f32], w: usize, h: usize) -> Vec<f32> {
    let r = (4.0 * BLUR_SIGMA + 0.5) as usize;
    let mut k: Vec<f32> = (0..=2 * r)
        .map(|x| (-0.5 * ((x as f32 - r as f32) / BLUR_SIGMA).powi(2)).exp())
        .collect();
    let sum: f32 = k.iter().sum();
    k.iter_mut().for_each(|v| *v /= sum);
    let reflect = |i: isize, n: usize| -> usize {
        let n = n as isize;
        let mut i = i;
        loop {
            if i < 0 {
                i = -i - 1;
            } else if i >= n {
                i = 2 * n - i - 1;
            } else {
                return i as usize;
            }
        }
    };
    // One pass along a line of `n` samples spaced `step` apart, starting at `base`.
    let pass = |src: &[f32], dst: &mut [f32], base: usize, n: usize, step: usize| {
        for c in 0..n {
            let mut acc = 0.0;
            if c >= r && c + r < n {
                let first = base + (c - r) * step;
                for (j, kv) in k.iter().enumerate() {
                    acc += src[first + j * step] * kv;
                }
            } else {
                for (j, kv) in k.iter().enumerate() {
                    let at = reflect(c as isize + j as isize - r as isize, n);
                    acc += src[base + at * step] * kv;
                }
            }
            dst[base + c * step] = acc;
        }
    };
    let mut tmp = vec![0f32; w * h];
    for y in 0..h {
        pass(px, &mut tmp, y * w, w, 1);
    }
    let mut out = vec![0f32; w * h];
    for x in 0..w {
        pass(&tmp, &mut out, x, h, w);
    }
    out
}

// ----- comparing ------------------------------------------------------------------------

struct Diff {
    /// How many pixels differ; without boxes asked for, only whether any do.
    px: usize,
    boxes: Vec<Box4>,
}

/// Pixels that really differ between two images in any color channel, and with `want_boxes`
/// boxes around them. Cutting a film into states only needs to know whether frames differ.
fn diff(a: &Image, b: &Image, want_boxes: bool) -> Diff {
    if a.w != b.w || a.h != b.h {
        return Diff {
            px: a.w * a.h,
            boxes: vec![[0, 0, a.w as u32, a.h as u32]],
        };
    }
    let (w, h) = (a.w, a.h);
    let far =
        |p: &[u8; 3], q: &[u8; 3]| (0..3).any(|c| i16::from(p[c].abs_diff(q[c])) > PIX_THRESH);
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..h {
        let row = y * w * 3..(y + 1) * w * 3;
        let (ra, rb) = (&a.px[row.clone()], &b.px[row]);
        // Most rows of two frames of one state differ by codec noise only; this check runs
        // over whole rows at once, so only rows with a real difference are searched.
        let most = ra.iter().zip(rb).map(|(p, q)| p.abs_diff(*q)).max();
        if most.is_none_or(|m| i16::from(m) <= PIX_THRESH) {
            continue;
        }
        let (pa, pb) = (ra.as_chunks::<3>().0, rb.as_chunks::<3>().0);
        if let (Some(first), Some(last)) = (
            pa.iter().zip(pb).position(|(p, q)| far(p, q)),
            pa.iter().zip(pb).rposition(|(p, q)| far(p, q)),
        ) {
            x0 = x0.min(first);
            x1 = x1.max(last);
            y0 = y0.min(y);
            y1 = y;
        }
    }
    if x0 == usize::MAX {
        return Diff {
            px: 0,
            boxes: Vec::new(),
        };
    }
    let (x0, y0) = (x0.saturating_sub(8), y0.saturating_sub(8));
    let (x1, y1) = ((x1 + 9).min(w), (y1 + 9).min(h));
    let (cw, ch) = (x1 - x0, y1 - y0);
    let mut raw = Mask {
        w: cw,
        h: ch,
        on: vec![false; cw * ch],
    };
    let mut mask = Mask {
        w: cw,
        h: ch,
        on: Vec::new(),
    };
    // Green first: it carries most of the brightness, so it alone finds most changes.
    let channels = [1, 0, 2];
    for (k, c) in channels.into_iter().enumerate() {
        // The blur is linear, so blurring the difference equals differencing the blurs.
        let delta: Vec<f32> = (y0..y1)
            .flat_map(|y| {
                (x0..x1).map(move |x| {
                    let i = (y * w + x) * 3 + c;
                    f32::from(a.px[i]) - f32::from(b.px[i])
                })
            })
            .collect();
        // A blur never raises the largest difference, so a channel within the threshold
        // everywhere needs none.
        if !delta.iter().all(|d| d.abs() <= f32::from(PIX_THRESH)) {
            for (o, d) in raw.on.iter_mut().zip(blur(&delta, cw, ch)) {
                *o |= d.abs() > f32::from(PIX_THRESH);
            }
        }
        // Without boxes only whether anything differs matters, and the channels seen so far
        // can only grow the areas, so a real area in them is enough.
        if (!want_boxes || k + 1 == channels.len()) && raw.on.contains(&true) {
            mask = raw.open();
            mask.drop_specks();
            if !want_boxes && mask.on.contains(&true) {
                break;
            }
        }
    }
    let px = mask.count();
    let boxes = if px == 0 || !want_boxes {
        Vec::new()
    } else {
        mask.boxes(BOX_GROW)
            .into_iter()
            .map(|b| {
                [
                    b[0] + x0 as u32,
                    b[1] + y0 as u32,
                    b[2] + x0 as u32,
                    b[3] + y0 as u32,
                ]
            })
            .collect()
    };
    Diff { px, boxes }
}

/// Pixels of `x` outside the per pixel and channel range of its neighbours, after noise
/// filtering. A frame caught inside a transition sits within its neighbours' range.
fn outside_range(x: &Image, near: &[&Image]) -> usize {
    if near.iter().any(|n| n.w != x.w || n.h != x.h) {
        return x.w * x.h;
    }
    let on = (0..x.w * x.h)
        .map(|p| {
            (p * 3..p * 3 + 3).any(|i| {
                let (lo, hi) = near.iter().fold((255i16, 0i16), |(lo, hi), n| {
                    (lo.min(i16::from(n.px[i])), hi.max(i16::from(n.px[i])))
                });
                let v = i16::from(x.px[i]);
                v < lo - PIX_THRESH || v > hi + PIX_THRESH
            })
        })
        .collect::<Vec<_>>();
    if !on.iter().any(|v| *v) {
        return 0;
    }
    let mut mask = Mask { w: x.w, h: x.h, on }.open();
    mask.drop_specks();
    mask.count()
}

/// Matches states of two films; returns the path of (before, after) pairs and the costs.
fn warp(a: &[Vec<f32>], b: &[Vec<f32>]) -> (Vec<(usize, usize)>, Vec<Vec<f32>>) {
    let (n, m) = (a.len(), b.len());
    let cost: Vec<Vec<f32>> = a
        .iter()
        .map(|x| {
            b.iter()
                .map(|y| x.iter().zip(y).map(|(p, q)| (p - q).abs()).sum::<f32>() / x.len() as f32)
                .collect()
        })
        .collect();
    let mut d = vec![vec![f32::INFINITY; m + 1]; n + 1];
    d[0][0] = 0.0;
    for i in 1..=n {
        for j in 1..=m {
            d[i][j] = cost[i - 1][j - 1] + d[i - 1][j].min(d[i][j - 1]).min(d[i - 1][j - 1]);
        }
    }
    let (mut i, mut j, mut path) = (n, m, Vec::new());
    while i > 0 && j > 0 {
        path.push((i - 1, j - 1));
        let options = [d[i - 1][j - 1], d[i - 1][j], d[i][j - 1]];
        let mut k = 0;
        for (idx, v) in options.iter().enumerate() {
            if *v < options[k] {
                k = idx;
            }
        }
        match k {
            0 => {
                i -= 1;
                j -= 1;
            }
            1 => i -= 1,
            _ => j -= 1,
        }
    }
    path.reverse();
    (path, cost)
}

/// Compares two films. Both are read again from disk for the pixel diffs.
pub(crate) fn compare(before: &Film, after: &Film) -> Result<Comparison> {
    let still = before.is_still() && after.is_still();
    let mut out = Comparison {
        verdict: Verdict::Same,
        still,
        size_before: (before.w, before.h),
        size_after: (after.w, after.h),
        total_before: round3(before.total),
        total_after: round3(after.total),
        max_shift: 0.0,
        max_shift_at: None,
        states_before: before.states.len(),
        states_after: after.states.len(),
        moments: Vec::new(),
        pairs: Vec::new(),
    };
    if before.w != after.w || before.h != after.h {
        out.verdict = Verdict::Changed;
        out.moments.push(Moment {
            before: (0.0, before.total),
            after: (0.0, after.total),
            area: [0, 0, after.w as u32, after.h as u32],
            px: after.w * after.h,
        });
        return Ok(out);
    }
    let (ko, kn) = (&before.states, &after.states);
    let thumbs = |s: &[State]| s.iter().map(|k| k.thumb.clone()).collect::<Vec<_>>();
    let (path, cost) = warp(&thumbs(ko), &thumbs(kn));

    // The path only moves forward, so each film needs the states one before to one after.
    let (mut wo, mut wn) = (Window::open(before)?, Window::open(after)?);
    let visited: BTreeSet<(usize, usize)> = path.iter().copied().collect();
    let mut cache: BTreeMap<(usize, usize), (usize, Vec<Box4>)> = BTreeMap::new();
    for &(i, j) in &path {
        wo.span(i.saturating_sub(1), i + 1)?;
        wn.span(j.saturating_sub(1), j + 1)?;
        cache.retain(|&(ci, cj), _| ci + 1 >= i && cj + 1 >= j);
        let mut pd = |i: usize, j: usize| -> Result<(usize, Vec<Box4>)> {
            if let Some(d) = cache.get(&(i, j)) {
                return Ok(d.clone());
            }
            let d = diff(wo.image(i)?, wn.image(j)?, true);
            let d = (d.px, d.boxes);
            cache.insert((i, j), d.clone());
            Ok(d)
        };
        let (mut px, mut boxes) = pd(i, j)?;
        if px > 0 {
            // When the films caught a transition at different moments, each side of the pair
            // has its true counterpart next to the other: a neighbour the path also visits
            // that matches it fully, or the other film's neighbours, whose range it sits in.
            // Both sides must have one; a state only one film shows is a change.
            let mut before_found = false;
            for jj in [j.wrapping_sub(1), j + 1] {
                if jj < kn.len() && visited.contains(&(i, jj)) && pd(i, jj)?.0 == 0 {
                    before_found = true;
                }
            }
            let mut after_found = false;
            for ii in [i.wrapping_sub(1), i + 1] {
                if ii < ko.len() && visited.contains(&(ii, j)) && pd(ii, j)?.0 == 0 {
                    after_found = true;
                }
            }
            before_found = before_found || outside_range(wo.image(i)?, &wn.near(j, kn.len())?) == 0;
            after_found = after_found || outside_range(wn.image(j)?, &wo.near(i, ko.len())?) == 0;
            if before_found && after_found {
                px = 0;
                boxes.clear();
            }
        }
        let boxes = boxes.into_iter().map(|b| after.own_pixels(b)).collect();
        out.pairs.push(Pair {
            before: i,
            after: j,
            before_frame: ko[i].frame,
            after_frame: kn[j].frame,
            before_at: round3(ko[i].start),
            after_at: round3(kn[j].start),
            px,
            boxes,
        });
    }

    // Timing: how much later each held state of the before film shows in the after film.
    let mut best: BTreeMap<usize, usize> = BTreeMap::new();
    for &(i, j) in &path {
        let e = best.entry(i).or_insert(j);
        if cost[i][j] < cost[i][*e] {
            *e = j;
        }
    }
    let mut max_shift = (0.0f64, None);
    for (i, k) in ko.iter().enumerate() {
        if k.dur >= STABLE_MIN
            && let Some(j) = best.get(&i)
        {
            let shift = kn[*j].start - k.start;
            if max_shift.1.is_none() || shift.abs() > max_shift.0.abs() {
                max_shift = (shift, Some(k.start));
            }
        }
    }
    out.max_shift = round3(max_shift.0);
    out.max_shift_at = max_shift.1.map(round3);

    let mut current: Option<(Moment, Vec<Box4>)> = None;
    let mut groups = Vec::new();
    for p in &out.pairs {
        let (o, n) = (&ko[p.before], &kn[p.after]);
        if p.px == 0 {
            groups.extend(current.take());
            continue;
        }
        let (m, boxes) = current.get_or_insert_with(|| {
            (
                Moment {
                    before: (o.start, 0.0),
                    after: (n.start, 0.0),
                    area: [0; 4],
                    px: 0,
                },
                Vec::new(),
            )
        });
        m.before.1 = o.start + o.dur;
        m.after.1 = n.start + n.dur;
        m.px = m.px.max(p.px);
        boxes.extend(&p.boxes);
    }
    groups.extend(current.take());
    out.moments = groups
        .into_iter()
        .map(|(mut m, boxes)| {
            m.area = boxes.iter().fold([u32::MAX, u32::MAX, 0, 0], |a, b| {
                [
                    a[0].min(b[0]),
                    a[1].min(b[1]),
                    a[2].max(b[2]),
                    a[3].max(b[3]),
                ]
            });
            m.before = (round3(m.before.0), round3(m.before.1));
            m.after = (round3(m.after.0), round3(m.after.1));
            m
        })
        .collect();

    out.verdict = if !out.moments.is_empty() {
        Verdict::Changed
    } else if !still
        && ((after.total - before.total).abs() > TIMING_TOL || max_shift.0.abs() > TIMING_TOL)
    {
        Verdict::Timing
    } else {
        Verdict::Same
    };
    Ok(out)
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// Writes the given frames of a film as PNG files no wider than `max_w`, in one ffmpeg
/// pass. Returns each requested frame with its file, in the order asked.
pub(crate) fn extract(
    film: &Path,
    frames: &[usize],
    max_w: usize,
    dir: &Path,
    prefix: &str,
) -> Result<Vec<(usize, PathBuf)>> {
    let mut wanted: Vec<usize> = frames.to_vec();
    wanted.sort_unstable();
    wanted.dedup();
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    std::fs::create_dir_all(dir)?;
    let select: Vec<String> = wanted.iter().map(|n| format!("eq(n\\,{n})")).collect();
    let filter = format!("select='{}',scale='min({max_w},iw)':-2", select.join("+"));
    let pattern = dir.join(format!("{prefix}%04d.png"));
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-y", "-i"])
        .arg(film)
        .args(["-vf", &filter, "-fps_mode", "passthrough"])
        .arg(&pattern)
        .output()
        .context("running ffmpeg")?;
    if !out.status.success() {
        bail!(
            "ffmpeg could not take frames from {}: {}",
            film.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(wanted
        .iter()
        .enumerate()
        .map(|(k, n)| (*n, dir.join(format!("{prefix}{:04}.png", k + 1))))
        .collect())
}
