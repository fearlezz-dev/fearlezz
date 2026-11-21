//! A film cut into stable states as its frames arrive, and the second read that gives the
//! states' first frames again for the pixel diffs.

use std::collections::VecDeque;

use crate::diff::{diff, thumb};
use crate::picture::fit;
use crate::{Box4, FILM_MAX, Image, Picture, Unfit};

/// A stable state of a film: consecutive frames with no meaningful change.
#[derive(Debug, Clone)]
pub struct State {
    /// When it starts, in seconds.
    pub start: f64,
    /// How long it holds, in seconds.
    pub dur: f64,
    /// Index of the state's first frame in the film.
    pub frame: usize,
    /// The state's first frame, small and gray, for matching states across films.
    pub(crate) thumb: Vec<f32>,
}

/// A film (or a still, a film of one frame) as its first read leaves it: its states.
#[derive(Debug, Clone)]
pub struct Film {
    /// Its own width.
    pub w: usize,
    /// Its own height.
    pub h: usize,
    /// The size its frames are compared at.
    seen: (usize, usize),
    /// How many frames it has.
    pub frames: usize,
    /// Where its last frame ends, in seconds.
    pub total: f64,
    /// Its stable states, in order.
    pub states: Vec<State>,
}

impl Film {
    /// Whether it has one frame only.
    #[must_use]
    pub fn is_still(&self) -> bool {
        self.frames == 1
    }

    /// The size its frames are compared at: its own, or for a film larger than [`FILM_MAX`]
    /// the largest size within it.
    #[must_use]
    pub fn seen(&self) -> (usize, usize) {
        self.seen
    }

    /// A frame as the comparison reads it, given at its own size or already scaled to the size
    /// it is compared at (by [`Picture::area`], as a caller that keeps frames keeps them).
    pub(crate) fn image(&self, picture: &Picture<'_>) -> Result<Image, Unfit> {
        if picture.size() != (self.w, self.h) && picture.size() != self.seen {
            return Err(Unfit::Size {
                film: (self.w, self.h),
                frame: picture.size(),
            });
        }
        Ok(picture.seen(self.seen))
    }

    /// A box at the size the film is compared at, in the film's own pixels.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a box scaled up to the film's own size stays within it"
    )]
    pub(crate) fn own_pixels(&self, b: Box4) -> Box4 {
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

/// The first read of a film: fed every frame in order, it keeps only the states.
#[derive(Debug)]
pub struct Cut {
    film: Film,
    /// The current state's first frame, which each new frame is compared with.
    first: Option<Image>,
}

impl Cut {
    /// Starts cutting a film of `w` by `h` that has `frames` frames, as its container says:
    /// more than one makes it a film, compared at [`FILM_MAX`] at most; a still keeps its size.
    #[must_use]
    pub fn new(w: usize, h: usize, frames: usize) -> Cut {
        let seen = if frames > 1 {
            fit(w, h, FILM_MAX)
        } else {
            (w, h)
        };
        Cut {
            film: Film {
                w,
                h,
                seen,
                frames: 0,
                total: 0.0,
                states: Vec::new(),
            },
            first: None,
        }
    }

    /// Takes the film's next frame, which starts at `start` and lasts `dur` seconds.
    ///
    /// # Errors
    ///
    /// When the frame is not of the film's size.
    pub fn push(&mut self, picture: &Picture<'_>, start: f64, dur: f64) -> Result<(), Unfit> {
        let image = self.film.image(picture)?;
        let film = &mut self.film;
        film.frames += 1;
        film.total = start + dur;
        if let (Some(state), Some(first)) = (film.states.last_mut(), &self.first)
            && diff(first, &image, false).px == 0
        {
            state.dur = film.total - state.start;
            return Ok(());
        }
        film.states.push(State {
            start,
            dur,
            frame: film.frames - 1,
            thumb: thumb(&image),
        });
        self.first = Some(image);
        Ok(())
    }

    /// How many states it has cut so far: a frame after which it counts one more starts a
    /// state, which a caller that keeps frames for the second read keeps.
    #[must_use]
    pub fn states(&self) -> usize {
        self.film.states.len()
    }

    /// The size its frames are compared at (see [`Film::seen`]).
    #[must_use]
    pub fn seen(&self) -> (usize, usize) {
        self.film.seen
    }

    /// The film, after its last frame.
    #[must_use]
    pub fn finish(self) -> Film {
        self.film
    }
}

/// The second read of a film, in order: it may skip frames that start no state.
pub trait Reread {
    /// What a failed read says; it takes this crate's errors too.
    type Error: From<Unfit>;

    /// The next frame it gives, with its index in the film; `None` after the last.
    ///
    /// # Errors
    ///
    /// When the frame cannot be read.
    fn next(&mut self) -> Result<Option<(usize, Picture<'_>)>, Self::Error>;
}

/// The first frames of one film's states around the place the comparison has reached: it
/// reads ahead as asked and forgets states the path has passed.
pub(crate) struct Window<'f, R> {
    film: &'f Film,
    read: R,
    /// States from index `first` on, in order.
    images: VecDeque<Image>,
    first: usize,
}

impl<'f, R: Reread> Window<'f, R> {
    pub(crate) fn new(film: &'f Film, read: R) -> Self {
        Window {
            film,
            read,
            images: VecDeque::new(),
            first: 0,
        }
    }

    pub(crate) fn film(&self) -> &'f Film {
        self.film
    }

    /// Makes states `lo..=hi` available (as far as the film has them), dropping earlier ones.
    pub(crate) fn span(&mut self, lo: usize, hi: usize) -> Result<(), R::Error> {
        while self.first < lo && !self.images.is_empty() {
            self.images.pop_front();
            self.first += 1;
        }
        if self.images.is_empty() {
            self.first = self.first.max(lo);
        }
        while self.first + self.images.len() <= hi {
            let Some(state) = self.film.states.get(self.first + self.images.len()) else {
                break;
            };
            let frame = state.frame;
            let image = loop {
                let Some((index, picture)) = self.read.next()? else {
                    return Err(Unfit::Ended { frame }.into());
                };
                if index >= frame {
                    break self.film.image(&picture)?;
                }
            };
            self.images.push_back(image);
        }
        Ok(())
    }

    /// State `c` and its neighbours.
    pub(crate) fn near(&self, c: usize) -> Result<Vec<&Image>, Unfit> {
        (c.saturating_sub(1)..(c + 2).min(self.film.states.len()))
            .map(|x| self.image(x))
            .collect()
    }

    /// State `i`, which a span must have made available.
    pub(crate) fn image(&self, i: usize) -> Result<&Image, Unfit> {
        i.checked_sub(self.first)
            .and_then(|k| self.images.get(k))
            .ok_or(Unfit::Unread { state: i })
    }
}
