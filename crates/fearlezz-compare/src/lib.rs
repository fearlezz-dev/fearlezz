//! Comparing two stills or films of a UI, tolerant to timing shifts and codec noise, the same
//! way wherever it runs: the CLI and CI feed it frames ffmpeg decodes, the proof host's page
//! frames the browser decodes. It does no I/O, so it builds for `wasm32-unknown-unknown`.
//!
//! The method (proven on 29 labelled pairs of real app films): cut each film into stable
//! states, match states across films with dynamic time warping over small thumbnails, then
//! pixel diff matched states after a light blur, ignoring blobs too small to matter. Pixels
//! are compared per color channel, so a change of hue alone is found. A pair of states caught
//! mid transition counts as unchanged when each sits inside the other film's neighbours, or
//! when every part of it that differs is the other film's content moved part of the way along
//! a motion that film makes there: a scroll or a slide caught at other moments.
//! Stills are lossless, so they get no codec noise allowance: every pixel more than about 2
//! levels apart counts, however faint, unless it is antialiasing, and a change of fewer than
//! 30 pixels is render jitter, no change a person sees.
//!
//! One algorithm, exact everywhere: a film arrives as its decoder's own YUV 4:2:0 planes, which
//! ffmpeg and the browser's WebCodecs give byte for byte alike, and [`Picture::rgb`] is the one
//! conversion to colour. A still arrives as RGB. Films are compared at 720p at most
//! ([`FILM_MAX`]), scaled by [`Picture::area`] on their planes, which keeps a UI's changes and
//! costs less than half of 1080p; stills keep their own size. Boxes and sizes are always in
//! the file's own pixels. Every step is integer or plain IEEE arithmetic in a fixed order, so
//! a native and a wasm build agree to the bit.
//!
//! A film is read twice: once, frame by frame, into a [`Cut`] that keeps its states' timing
//! and thumbnails, and once more in [`compare`], both films in step along the matched path,
//! through a [`Reread`] that gives the states' first frames. Only a few states are in memory
//! at a time, however long or busy a film is.

mod compare;
mod diff;
mod film;
mod motion;
mod picture;

pub use compare::compare;
pub use fearlezz_proof::media::{Box4, Comparison, Marks, Moment, Pair, Verdict};
pub use film::{Cut, Film, Reread, State};
pub use picture::{FILM_MAX, Format, Image, Picture, fit};

/// What a comparison cannot take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unfit {
    /// A picture's bytes do not fill its format at its size, or the size is empty.
    Bytes {
        /// The format it was given in.
        format: Format,
        /// Its width.
        w: usize,
        /// Its height.
        h: usize,
        /// How many bytes it had.
        len: usize,
    },
    /// A frame of another size than the film it is fed to.
    Size {
        /// The film's size.
        film: (usize, usize),
        /// The frame's size.
        frame: (usize, usize),
    },
    /// The second read of a film ended before this frame, a state's first.
    Ended {
        /// The frame's index in the film.
        frame: usize,
    },
    /// The comparison asked for a state outside the frames it kept: a fault of this crate.
    Unread {
        /// The state's index in the film.
        state: usize,
    },
}

impl std::fmt::Display for Unfit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unfit::Bytes { format, w, h, len } => write!(
                f,
                "a {w}x{h} {format} picture takes {} bytes, not {len}",
                format.len(*w, *h)
            ),
            Unfit::Size { film, frame } => write!(
                f,
                "a frame of {}x{} in a film of {}x{}",
                frame.0, frame.1, film.0, film.1
            ),
            Unfit::Ended { frame } => {
                write!(f, "the film ended before frame {frame} on its second read")
            }
            Unfit::Unread { state } => write!(f, "state {state} of the film was not read"),
        }
    }
}

impl std::error::Error for Unfit {}
