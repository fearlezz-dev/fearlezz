//! Reading stills and films with ffmpeg for the comparison in `fearlezz-compare`, and the
//! other pictures Fearlezz makes with it: a note's frame and a film's animated WebP copy.
//!
//! A film whose frames are `yuv420p` (what VP9 and H.264 recordings hold) is read as those
//! planes at its own size, so ffmpeg neither converts nor scales it: the comparison does both,
//! the same way here, on CI and in the proof host's page. Other pictures (a PNG still, a GIF's
//! palette) are read as RGB.
//!
//! Films are read twice: once for the states' timing and thumbnails, and once more, both
//! films in step along the matched path, for the pixel diffs. Only a few states are in
//! memory at a time, however long or busy a film is. The second read asks ffmpeg for the
//! states' first frames only, so it decodes the film but sends few frames.
#![expect(
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a changed still has a moment, and ffprobe's JSON and ffmpeg's stats are indexed \
              and their sizes and sample values converted where they are read"
)]

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context as _, Result, bail};
use fearlezz_compare::{Cut, Format, Picture, Reread};

pub(crate) use fearlezz_proof::media::{Comparison, Verdict};

/// A comparison in one line, for people and agents.
pub(crate) fn summary(c: &Comparison) -> String {
    match c.verdict {
        Verdict::Same => "same".into(),
        Verdict::Timing => format!(
            "only timing: {:.2} s to {:.2} s, largest shift {:+.2} s at {:.2} s",
            c.total_before,
            c.total_after,
            c.max_shift,
            c.max_shift_at.unwrap_or(0.0)
        ),
        Verdict::Changed if c.size_before != c.size_after => format!(
            "changed: size {}x{} to {}x{}",
            c.size_before.0, c.size_before.1, c.size_after.0, c.size_after.1
        ),
        Verdict::Changed if c.still => {
            let b = c.moments[0].area;
            format!("changed: x {} to {}, y {} to {}", b[0], b[2], b[1], b[3])
        }
        Verdict::Changed => {
            let at: Vec<String> = c
                .moments
                .iter()
                .map(|m| format!("{:.2} s", m.after.0))
                .collect();
            match (at.is_empty(), c.marks_changed) {
                (true, _) => "changed: its marks differ".into(),
                (false, true) => format!("changed at {}, and its marks differ", at.join(", ")),
                (false, false) => format!("changed at {}", at.join(", ")),
            }
        }
    }
}

// ----- decoding -------------------------------------------------------------------------

/// A still or film as its first read leaves it, and how to read it again.
pub(crate) struct Film {
    /// Where it is read from again for the pixel diffs.
    path: PathBuf,
    /// The form ffmpeg gives its frames in.
    format: Format,
    /// Its states, from the first read, and the marks its check said, which a caller sets.
    pub(crate) cut: fearlezz_compare::Film,
    /// Each frame's start and duration, in seconds, so the second read needs no probe.
    times: Vec<(f64, f64)>,
}

impl Film {
    /// Its second read: only its states' first frames when most frames start no state.
    fn reread(&self) -> Result<Frames> {
        let starts: Vec<usize> = self.cut.states.iter().map(|s| s.frame).collect();
        let pick = (starts.len() * 2 <= self.cut.frames).then_some(starts);
        Frames::open(
            &self.path,
            self.format,
            (self.cut.w, self.cut.h),
            self.times.clone(),
            pick,
        )
    }
}

/// Reads a still or film's states: their timing and thumbnails.
pub(crate) fn decode(path: &Path) -> Result<Film> {
    let Probe {
        w,
        h,
        format,
        times,
    } = probe(path)?;
    // The planes the comparison converts itself, the same in every build; ffmpeg turns any
    // other format into RGB, which a still already is.
    let format = if format == "yuv420p" {
        Format::Yuv420
    } else {
        Format::Rgb
    };
    let mut cut = Cut::new(w, h, times.len());
    let mut frames = Frames::open(path, format, (w, h), times, None)?;
    while let Some((_, start, dur)) = frames.advance()? {
        cut.push(&frames.picture()?, start, dur)
            .with_context(|| format!("reading frames of {}", path.display()))?;
    }
    Ok(Film {
        path: path.to_path_buf(),
        format,
        cut: cut.finish(),
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

/// Compares two stills or films; each is read again from disk for the pixel diffs, unless
/// their sizes differ.
pub(crate) fn compare(before: &Film, after: &Film) -> Result<Comparison> {
    fearlezz_compare::compare(
        &before.cut,
        Again::new(before),
        &after.cut,
        Again::new(after),
    )
    .with_context(|| {
        format!(
            "comparing {} with {}",
            before.path.display(),
            after.path.display()
        )
    })
}

/// A film's second read, started when the comparison first asks it for a frame.
struct Again<'f> {
    film: &'f Film,
    frames: Option<Frames>,
}

impl<'f> Again<'f> {
    fn new(film: &'f Film) -> Self {
        Again { film, frames: None }
    }
}

impl Reread for Again<'_> {
    type Error = anyhow::Error;

    fn next(&mut self) -> Result<Option<(usize, Picture<'_>)>> {
        if self.frames.is_none() {
            self.frames = Some(self.film.reread()?);
        }
        let Some(frames) = self.frames.as_mut() else {
            return Ok(None);
        };
        let Some((index, _, _)) = frames.advance()? else {
            return Ok(None);
        };
        Ok(Some((index, frames.picture()?)))
    }
}

/// A film's frames as ffmpeg streams them, one at a time, in its own size.
struct Frames {
    path: PathBuf,
    child: std::process::Child,
    out: std::process::ChildStdout,
    errors: Option<std::thread::JoinHandle<String>>,
    times: Vec<(f64, f64)>,
    /// When only some frames are asked for, their indices in the film, in order.
    pick: Option<Vec<usize>>,
    format: Format,
    size: (usize, usize),
    /// How many frames ffmpeg sent.
    sent: usize,
    /// The last frame read.
    buf: Vec<u8>,
    /// How many frames of the film were read: one past the last frame's index.
    read: usize,
    /// Where the last frame read ends, in seconds.
    end: f64,
    done: bool,
}

impl Frames {
    /// Streams every frame in `format` at the film's own `size`, or with `pick` only the
    /// frames at those indices (ascending). Times come from the probe, never from ffmpeg's
    /// output.
    fn open(
        path: &Path,
        format: Format,
        size: (usize, usize),
        times: Vec<(f64, f64)>,
        pick: Option<Vec<usize>>,
    ) -> Result<Frames> {
        let mut cmd = crate::ffmpeg::ffmpeg()?;
        cmd.args(["-v", "error", "-i"]).arg(path);
        if let Some(pick) = &pick {
            // The filter still decodes every frame, but sends only these.
            let select: Vec<String> = pick.iter().map(|n| format!("eq(n\\,{n})")).collect();
            cmd.args(["-vf", &format!("select='{}'", select.join("+"))]);
        }
        let pix_fmt = match format {
            Format::Yuv420 => "yuv420p",
            Format::Rgb => "rgb24",
        };
        let mut child = cmd
            .args(["-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt"])
            .args([pix_fmt, "-"])
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
            format,
            size,
            sent: 0,
            buf: vec![0u8; format.len(size.0, size.1)],
            read: 0,
            end: 0.0,
            done: false,
        })
    }

    /// Reads the next frame into `buf`: its index in the film, start and duration in
    /// seconds; `None` after the last.
    fn advance(&mut self) -> Result<Option<(usize, f64, f64)>> {
        if self.done {
            return Ok(None);
        }
        match self.out.read_exact(&mut self.buf) {
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
                return Ok(None);
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
        let (start, dur) = self.times.get(index).copied().unwrap_or((self.end, 0.05));
        self.read = index + 1;
        self.sent += 1;
        self.end = start + dur;
        Ok(Some((index, start, dur)))
    }

    /// The last frame read.
    fn picture(&self) -> Result<Picture<'_>> {
        Picture::new(self.format, self.size.0, self.size.1, &self.buf)
            .with_context(|| format!("reading frames of {}", self.path.display()))
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

/// A film's size, pixel format and each frame's start and duration, in seconds.
struct Probe {
    w: usize,
    h: usize,
    format: String,
    times: Vec<(f64, f64)>,
}

fn probe(path: &Path) -> Result<Probe> {
    let out = crate::process::read(
        crate::ffmpeg::ffprobe()?
            .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
            // Packets carry the frames' times without decoding them; sorted by time they are
            // the frames in the order ffmpeg sends them.
            .arg("stream=width,height,pix_fmt:packet=pts_time,duration_time")
            .args(["-of", "json"])
            .arg(path),
        None,
    )
    .with_context(|| {
        format!(
            "ffprobe could not read {}; Fearlezz needs it to read stills and films",
            path.display()
        )
    })?;
    let info: serde_json::Value = serde_json::from_slice(&out)?;
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

/// Writes one frame of a still or film as a PNG at `out`: a film's frame at `time` seconds (its
/// last frame when it is shorter), and with `region` (fractions of the picture: x, y, w, h)
/// outlined in red when given. The pictures of a person's note, for the agent to look at.
pub(crate) fn noted(
    source: &Path,
    time: Option<f64>,
    region: Option<[f64; 4]>,
    out: &Path,
) -> Result<()> {
    let filter = match region {
        Some([x, y, w, h]) => {
            format!("drawbox=x=iw*{x}:y=ih*{y}:w=iw*{w}:h=ih*{h}:color=red:t=max(3\\,ih/100)")
        }
        None => "null".to_owned(),
    };
    let run = |seek: &[String]| -> Result<bool> {
        crate::process::read(
            crate::ffmpeg::ffmpeg()?
                .args(["-v", "error", "-nostdin", "-y"])
                .args(seek)
                .arg("-i")
                .arg(source)
                .args(["-frames:v", "1", "-update", "1", "-vf", &filter])
                .arg(out),
            None,
        )
        .with_context(|| format!("ffmpeg could not draw a note on {}", source.display()))?;
        Ok(std::fs::metadata(out).is_ok_and(|m| m.len() > 0))
    };
    let seek = time
        .map(|t| vec!["-ss".to_owned(), format!("{t:.3}")])
        .unwrap_or_default();
    if run(&seek)? {
        return Ok(());
    }
    // A moment past the film's end: its last frame.
    if run(&["-sseof".to_owned(), "-0.1".to_owned()])? {
        return Ok(());
    }
    bail!("ffmpeg took no frame from {}", source.display())
}

/// How far apart, in 8 bit levels, the darkest and lightest sample of each channel of a frame
/// may be for it to count as one flat colour: compression leaves a little noise on a blank
/// frame, and a frame with anything drawn on it spans far more.
const FLAT: u32 = 12;

/// How the animated WebP copy of a film came out.
pub(crate) enum WebpCopy {
    /// Written.
    Made,
    /// Not made: Fearlezz found no ffmpeg to make it with, for this reason.
    NoFfmpeg(crate::ffmpeg::Missing),
}

/// Writes `to`, the animated WebP copy of the film `film`, which it leaves as it is: it loops
/// forever, 720 pixels wide at most (never larger than the film), at 12 frames a second,
/// holding its last frame 1.5 s so each loop shows how it ends before it starts over.
/// It starts at the film's first frame with content: a recorder that starts as the page opens
/// leaves blank frames first, which would flash on every loop. A film all blank is kept whole.
///
/// Quality 75 at compression level 2: a 13.8 s app film of 1280 by 800 came out near 300 KB
/// even at quality 80, and a 15 s test pattern that changes every pixel of every frame (the
/// worst case) at 2 MB, in seconds where the default level 4 took ten times as long for 10%
/// fewer bytes.
///
/// It runs the ffmpeg Fearlezz runs for every picture (`crate::ffmpeg`): the project's shell
/// first, then PATH, then on GitHub Actions the pinned build.
pub(crate) fn webp_copy(film: &Path, to: &Path) -> Result<WebpCopy> {
    if let Some(missing) = crate::ffmpeg::missing() {
        return Ok(WebpCopy::NoFfmpeg(missing));
    }
    let scan = scan(film).with_context(|| format!("ffmpeg could not read {}", film.display()))?;
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    // A little before the frame, so rounding in its printed time never drops it.
    let trim = if scan.content > scan.start {
        format!(
            "trim=start={:.4},setpts=PTS-STARTPTS,",
            scan.content - 0.001
        )
    } else {
        String::new()
    };
    crate::process::read(
        crate::ffmpeg::ffmpeg()?
            .args(["-v", "error", "-nostdin", "-y", "-i"])
            .arg(film)
            .arg("-vf")
            .arg(format!(
                "{trim}fps=12,scale=w='min(720,iw)':h=-1:flags=lanczos,\
                 tpad=stop_mode=clone:stop_duration=1.5"
            ))
            .args(["-an", "-c:v", "libwebp_anim", "-quality", "75"])
            .args(["-compression_level", "2", "-loop", "0", "-f", "webp"])
            .arg(to),
        None,
    )
    .with_context(|| format!("ffmpeg could not make a WebP of {}", film.display()))?;
    Ok(WebpCopy::Made)
}

/// When a film's frames start, and when its first frame with content starts, in seconds on
/// its own clock.
struct Scan {
    start: f64,
    content: f64,
}

/// Reads every frame of the film at `path` once, small, for its time and the range of each
/// channel (`signalstats`), and finds where it starts and its first frame that is not one flat
/// colour (the start when every frame is).
fn scan(path: &Path) -> Result<Scan> {
    let out = crate::process::read(
        crate::ffmpeg::ffmpeg()?
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(path)
            .args([
                "-an",
                "-vf",
                "scale='min(160,iw)':-2,signalstats,metadata=mode=print:file=-",
                "-f",
                "null",
                "-",
            ]),
        None,
    )?;
    // Each frame is a line `frame:N pts:P pts_time:T`, then one `key=value` line per stat.
    let mut frames: Vec<(f64, bool)> = Vec::new();
    let mut range = [(u32::MAX, 0u32); 3];
    let flat =
        |range: &[(u32, u32); 3]| range.iter().all(|(lo, hi)| hi.saturating_sub(*lo) <= FLAT);
    for line in String::from_utf8_lossy(&out).lines() {
        if let Some(time) = line
            .split_whitespace()
            .find_map(|w| w.strip_prefix("pts_time:"))
        {
            if let Some(last) = frames.last_mut() {
                last.1 = flat(&range);
            }
            range = [(u32::MAX, 0); 3];
            frames.push((time.parse().unwrap_or_default(), true));
            continue;
        }
        let Some((key, value)) = line
            .strip_prefix("lavfi.signalstats.")
            .and_then(|l| l.split_once('='))
        else {
            continue;
        };
        let Ok(value) = value.trim().parse::<f64>() else {
            continue;
        };
        let at = match key.get(..1) {
            Some("Y") => 0,
            Some("U") => 1,
            Some("V") => 2,
            _ => continue,
        };
        let Some(slot) = range.get_mut(at) else {
            continue;
        };
        let value = value as u32;
        match key.get(1..) {
            Some("MIN") => slot.0 = value,
            Some("MAX") => slot.1 = value,
            _ => {}
        }
    }
    if let Some(last) = frames.last_mut() {
        last.1 = flat(&range);
    }
    let start = frames.first().map_or(0.0, |f| f.0);
    let content = frames.iter().find(|f| !f.1).map_or(start, |f| f.0);
    Ok(Scan { start, content })
}
