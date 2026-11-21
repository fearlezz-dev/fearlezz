//! The comparison of `fearlezz-compare` for the proof host's page, which runs it in a worker to
//! compare a picture of one run with the same picture of another: the same code the CLI and CI
//! run, fed the frames the browser decodes.
//!
//! The page hands over a still as the RGBA a canvas gives (its alpha is dropped, as ffmpeg's
//! `rgb24` drops it) and a film as WebCodecs' I420 planes, which equal ffmpeg's `yuv420p`. A
//! film is read once: the frames that start a state are kept as they arrive, and the
//! comparison's second read ([`fearlezz_compare::Reread`]) gives them back, so the page never
//! decodes a film twice. That costs the states' first frames in memory, kept at the size they
//! are compared at; a film busy enough to need more than [`KEPT_MAX`] is refused.
//!
//! The exports are plain functions over numbers, so the page needs no generated glue: it asks
//! [`input`] for room, writes a frame's bytes there, and reads answers from [`output`].

use std::cell::RefCell;

use fearlezz_compare::{Comparison, Cut, Format, Picture, Reread, Unfit};

/// The most bytes a feed keeps for the second read: a film whose states' first frames need more
/// (a long scroll or animation, where every frame starts a state) is refused rather than let run
/// the page's memory out.
pub const KEPT_MAX: usize = 384 << 20;

/// How the page gives a frame's bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Given {
    /// Four bytes per pixel, red, green, blue and alpha: a canvas's image data.
    Rgba,
    /// I420 planes packed with no padding: what `VideoFrame.copyTo` writes.
    Yuv420,
}

impl Given {
    /// The form the comparison reads it in.
    fn format(self) -> Format {
        match self {
            Given::Rgba => Format::Rgb,
            Given::Yuv420 => Format::Yuv420,
        }
    }
}

/// A still or film as the page feeds it: its cut so far and the frames that start its states.
#[derive(Debug)]
pub struct Feed {
    cut: Cut,
    frames: Frames,
}

/// What a feed keeps of its frames for the second read.
#[derive(Debug)]
struct Frames {
    given: Given,
    w: usize,
    h: usize,
    /// The size its frames are compared at, which kept frames are scaled to.
    seen: (usize, usize),
    /// Each frame that starts a state, by its index in the film, in the comparison's format at
    /// `seen`.
    kept: Vec<(usize, Vec<u8>)>,
    /// How many bytes `kept` holds.
    held: usize,
    /// How many frames it was fed.
    fed: usize,
}

impl Feed {
    /// Starts a still or film of `w` by `h` with `frames` frames, given as `given`.
    #[must_use]
    pub fn new(w: usize, h: usize, frames: usize, given: Given) -> Feed {
        let cut = Cut::new(w, h, frames);
        let seen = cut.seen();
        Feed {
            cut,
            frames: Frames {
                given,
                w,
                h,
                seen,
                kept: Vec::new(),
                held: 0,
                fed: 0,
            },
        }
    }

    /// Takes the next frame, which starts at `start` and lasts `dur` seconds.
    ///
    /// # Errors
    ///
    /// When its bytes do not fill the size the feed was started with, or the film needs more
    /// than [`KEPT_MAX`] kept for the second read.
    pub fn push(&mut self, bytes: &[u8], start: f64, dur: f64) -> Result<(), String> {
        let Feed { cut, frames: f } = self;
        let format = f.given.format();
        let owned;
        let data = match f.given {
            Given::Yuv420 => bytes,
            Given::Rgba => {
                if bytes.len() != f.w * f.h * 4 {
                    return Err(Unfit::Bytes {
                        format,
                        w: f.w,
                        h: f.h,
                        len: bytes.len() / 4 * 3,
                    }
                    .to_string());
                }
                owned = bytes
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .flat_map(|&[r, g, b, _]| [r, g, b])
                    .collect::<Vec<u8>>();
                &owned
            }
        };
        let picture = Picture::new(format, f.w, f.h, data).map_err(|e| e.to_string())?;
        let had = cut.states();
        cut.push(&picture, start, dur).map_err(|e| e.to_string())?;
        if cut.states() > had {
            let kept = if f.seen == (f.w, f.h) {
                data.to_vec()
            } else {
                picture.area(f.seen.0, f.seen.1)
            };
            f.held += kept.len();
            if f.held > KEPT_MAX {
                return Err(format!(
                    "the film changes too often to compare here ({} states by frame {})",
                    cut.states(),
                    f.fed + 1
                ));
            }
            f.kept.push((f.fed, kept));
        }
        f.fed += 1;
        Ok(())
    }
}

/// A feed's second read: the frames it kept, in order.
struct Kept<'f> {
    frames: &'f Frames,
    next: usize,
}

impl<'f> Kept<'f> {
    fn new(frames: &'f Frames) -> Self {
        Kept { frames, next: 0 }
    }
}

impl Reread for Kept<'_> {
    type Error = Unfit;

    fn next(&mut self) -> Result<Option<(usize, Picture<'_>)>, Unfit> {
        let f = self.frames;
        let Some((index, bytes)) = f.kept.get(self.next) else {
            return Ok(None);
        };
        self.next += 1;
        let picture = Picture::new(f.given.format(), f.seen.0, f.seen.1, bytes)?;
        Ok(Some((*index, picture)))
    }
}

/// Compares two stills or films the page fed.
///
/// # Errors
///
/// When a feed is not whole, such as a film whose frames stopped before its states' first.
pub fn compare(before: Feed, after: Feed) -> Result<Comparison, Unfit> {
    fearlezz_compare::compare(
        &before.cut.finish(),
        Kept::new(&before.frames),
        &after.cut.finish(),
        Kept::new(&after.frames),
    )
}

/// What the exports hold between calls: the frame being handed over, the feeds by id, and the
/// last answer.
#[derive(Default)]
struct Page {
    input: Vec<u8>,
    feeds: Vec<Option<Feed>>,
    output: Vec<u8>,
}

thread_local! {
    static PAGE: RefCell<Page> = RefCell::default();
}

/// How an export says it went: 0 for done, 1 for an error whose words are in [`output`].
const DONE: u32 = 0;
const FAILED: u32 = 1;

fn answer(page: &mut Page, said: Result<Vec<u8>, String>) -> u32 {
    match said {
        Ok(bytes) => {
            page.output = bytes;
            DONE
        }
        Err(words) => {
            page.output = words.into_bytes();
            FAILED
        }
    }
}

/// Room for `len` bytes, where the page writes the next frame before [`frame`] takes it.
#[expect(
    unsafe_code,
    reason = "an export the page calls by name needs an unmangled symbol"
)]
#[unsafe(no_mangle)]
pub extern "C" fn input(len: usize) -> *mut u8 {
    PAGE.with_borrow_mut(|page| {
        page.input.clear();
        page.input.resize(len, 0);
        page.input.as_mut_ptr()
    })
}

/// Starts a feed of `w` by `h` with `frames` frames, given as RGBA (`yuv` 0) or I420 (1): its id.
#[expect(
    unsafe_code,
    reason = "an export the page calls by name needs an unmangled symbol"
)]
#[unsafe(no_mangle)]
pub extern "C" fn film(w: u32, h: u32, frames: u32, yuv: u32) -> u32 {
    let given = if yuv == 1 { Given::Yuv420 } else { Given::Rgba };
    let feed = Feed::new(w as usize, h as usize, frames as usize, given);
    PAGE.with_borrow_mut(|page| {
        let free = page.feeds.iter().position(Option::is_none);
        let id = free.unwrap_or(page.feeds.len());
        if id == page.feeds.len() {
            page.feeds.push(None);
        }
        if let Some(slot) = page.feeds.get_mut(id) {
            *slot = Some(feed);
        }
        u32::try_from(id).unwrap_or(u32::MAX)
    })
}

/// Feeds feed `id` the frame in [`input`], starting at `start` and lasting `dur` seconds.
#[expect(
    unsafe_code,
    reason = "an export the page calls by name needs an unmangled symbol"
)]
#[unsafe(no_mangle)]
pub extern "C" fn frame(id: u32, start: f64, dur: f64) -> u32 {
    PAGE.with_borrow_mut(|page| {
        let Page { input, feeds, .. } = &mut *page;
        let said = match feeds.get_mut(id as usize).and_then(Option::as_mut) {
            Some(feed) => feed.push(input, start, dur).map(|()| Vec::new()),
            None => Err(format!("no film {id}")),
        };
        answer(page, said)
    })
}

/// Compares feed `before` with feed `after`, ending both: the comparison as JSON in [`output`].
#[expect(
    unsafe_code,
    reason = "an export the page calls by name needs an unmangled symbol"
)]
#[unsafe(no_mangle)]
pub extern "C" fn compared(before: u32, after: u32) -> u32 {
    PAGE.with_borrow_mut(|page| {
        let mut take = |id: u32| page.feeds.get_mut(id as usize).and_then(Option::take);
        let said = match (take(before), take(after)) {
            (Some(b), Some(a)) => compare(b, a)
                .map_err(|e| e.to_string())
                .and_then(|c| serde_json::to_vec(&c).map_err(|e| e.to_string())),
            _ => Err(format!("no films {before} and {after}")),
        };
        answer(page, said)
    })
}

/// Ends feed `id` without comparing it, such as when its film could not be decoded.
#[expect(
    unsafe_code,
    reason = "an export the page calls by name needs an unmangled symbol"
)]
#[unsafe(no_mangle)]
pub extern "C" fn forget(id: u32) {
    PAGE.with_borrow_mut(|page| {
        if let Some(slot) = page.feeds.get_mut(id as usize) {
            *slot = None;
        }
    });
}

/// Where the last answer starts: a comparison's JSON, or an error's words.
#[expect(
    unsafe_code,
    reason = "an export the page calls by name needs an unmangled symbol"
)]
#[unsafe(no_mangle)]
pub extern "C" fn output() -> *const u8 {
    PAGE.with_borrow(|page| page.output.as_ptr())
}

/// How many bytes the last answer has.
#[expect(
    unsafe_code,
    reason = "an export the page calls by name needs an unmangled symbol"
)]
#[unsafe(no_mangle)]
pub extern "C" fn output_len() -> usize {
    PAGE.with_borrow(|page| page.output.len())
}
