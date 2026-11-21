//! Comparing films fed frame by frame: their states, what changed, where and when.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

use fearlezz_compare::{Comparison, Cut, Film, Format, Picture, Reread, Unfit, Verdict, compare};

/// A film in memory: each frame's bytes, in one format and size, a tenth of a second each.
struct Frames {
    format: Format,
    w: usize,
    h: usize,
    each: Vec<Vec<u8>>,
}

impl Frames {
    /// A white RGB film of `n` frames with a black box at `x` from frame `from` on.
    fn boxed(w: usize, h: usize, n: usize, x: usize, from: usize) -> Frames {
        let frames = (0..n)
            .map(|i| {
                let mut px = vec![255u8; w * h * 3];
                if i >= from {
                    for y in 10..30 {
                        for xx in x..x + 20 {
                            px[(y * w + xx) * 3..][..3].fill(0);
                        }
                    }
                }
                px
            })
            .collect();
        Frames {
            format: Format::Rgb,
            w,
            h,
            each: frames,
        }
    }

    fn cut(&self) -> Film {
        let mut cut = Cut::new(self.w, self.h, self.each.len());
        for (i, f) in self.each.iter().enumerate() {
            let picture = Picture::new(self.format, self.w, self.h, f).unwrap();
            cut.push(&picture, i as f64 / 10.0, 0.1).unwrap();
        }
        cut.finish()
    }

    /// Its second read, which gives only the frames in `pick` when asked, as ffmpeg does.
    fn again(&self, pick: Option<Vec<usize>>) -> Again<'_> {
        Again {
            frames: self,
            next: 0,
            pick,
        }
    }
}

struct Again<'a> {
    frames: &'a Frames,
    next: usize,
    pick: Option<Vec<usize>>,
}

impl Reread for Again<'_> {
    type Error = Unfit;

    fn next(&mut self) -> Result<Option<(usize, Picture<'_>)>, Unfit> {
        let index = match &self.pick {
            Some(pick) => match pick.get(self.next) {
                Some(&i) => i,
                None => return Ok(None),
            },
            None => self.next,
        };
        self.next += 1;
        let Some(f) = self.frames.each.get(index) else {
            return Ok(None);
        };
        let p = Picture::new(self.frames.format, self.frames.w, self.frames.h, f)?;
        Ok(Some((index, p)))
    }
}

fn compared(before: &Frames, after: &Frames) -> Comparison {
    compare(
        &before.cut(),
        before.again(None),
        &after.cut(),
        after.again(None),
    )
    .unwrap()
}

#[test]
fn a_film_is_cut_into_its_stable_states() {
    let film = Frames::boxed(64, 48, 20, 10, 10).cut();
    assert_eq!(film.frames, 20);
    assert_eq!(film.states.len(), 2);
    assert_eq!(film.states[1].frame, 10);
    assert!((film.states[1].start - 1.0).abs() < 1e-9);
    assert!((film.states[0].dur - 1.0).abs() < 1e-9);
    assert!((film.total - 2.0).abs() < 1e-9);
}

#[test]
fn the_same_film_twice_is_the_same() {
    let film = Frames::boxed(64, 48, 20, 10, 10);
    let c = compared(&film, &film);
    assert_eq!(c.verdict, Verdict::Same, "{c:?}");
}

#[test]
fn a_moved_box_is_a_change_at_its_moment_around_both_places() {
    let c = compared(
        &Frames::boxed(64, 48, 20, 5, 10),
        &Frames::boxed(64, 48, 20, 35, 10),
    );
    assert_eq!(c.verdict, Verdict::Changed, "{c:?}");
    assert_eq!(c.moments.len(), 1, "{c:?}");
    let m = &c.moments[0];
    assert!((m.after.0 - 1.0).abs() < 1e-9, "{m:?}");
    assert!(m.area[0] <= 5 && m.area[2] >= 55, "{m:?}");
}

#[test]
fn a_later_box_is_only_timing() {
    let c = compared(
        &Frames::boxed(64, 48, 20, 10, 10),
        &Frames::boxed(64, 48, 25, 10, 15),
    );
    assert_eq!(c.verdict, Verdict::Timing, "{c:?}");
    assert!((c.max_shift - 0.5).abs() < 1e-9, "{c:?}");
}

/// A second read that gives only the states' first frames gives the same comparison.
#[test]
fn a_second_read_of_only_the_first_frames_of_states_is_enough() {
    let (before, after) = (
        Frames::boxed(64, 48, 20, 5, 10),
        Frames::boxed(64, 48, 20, 35, 10),
    );
    let (b, a) = (before.cut(), after.cut());
    let starts = |f: &Film| Some(f.states.iter().map(|s| s.frame).collect());
    let picked = compare(&b, before.again(starts(&b)), &a, after.again(starts(&a))).unwrap();
    assert_eq!(
        format!("{picked:?}"),
        format!("{:?}", compared(&before, &after))
    );
}

#[test]
fn films_of_different_sizes_are_changed_whole_without_a_second_read() {
    let (before, after) = (
        Frames::boxed(64, 48, 3, 5, 1),
        Frames::boxed(48, 64, 3, 5, 1),
    );
    let empty = Frames {
        each: Vec::new(),
        ..Frames::boxed(64, 48, 0, 0, 0)
    };
    let c = compare(
        &before.cut(),
        empty.again(None),
        &after.cut(),
        empty.again(None),
    )
    .unwrap();
    assert_eq!(c.verdict, Verdict::Changed);
    assert_eq!(c.moments[0].area, [0, 0, 48, 64]);
}

#[test]
fn a_second_read_that_ends_early_says_which_frame_it_missed() {
    let film = Frames::boxed(64, 48, 20, 5, 10);
    let other = Frames::boxed(64, 48, 20, 35, 10);
    let short = Frames {
        each: film.each[..5].to_vec(),
        ..Frames::boxed(64, 48, 0, 0, 0)
    };
    let err = compare(
        &film.cut(),
        short.again(None),
        &other.cut(),
        other.again(None),
    );
    assert_eq!(err.unwrap_err(), Unfit::Ended { frame: 10 });
}

#[test]
fn a_frame_of_another_size_is_refused() {
    let mut cut = Cut::new(64, 48, 2);
    let px = vec![0u8; 32 * 48 * 3];
    let picture = Picture::new(Format::Rgb, 32, 48, &px).unwrap();
    assert_eq!(
        cut.push(&picture, 0.0, 0.1),
        Err(Unfit::Size {
            film: (64, 48),
            frame: (32, 48)
        })
    );
}

/// A film larger than 720p is compared smaller; a still keeps its size; boxes come back in the
/// film's own pixels.
#[test]
fn a_large_film_is_compared_at_720p_and_answers_in_its_own_pixels() {
    assert_eq!(Cut::new(1920, 1080, 2).finish().seen(), (1280, 720));
    assert_eq!(Cut::new(1920, 1080, 1).finish().seen(), (1920, 1080));
    let yuv = |x: usize| {
        let (w, h) = (1600, 900);
        let frames = (0..2)
            .map(|i| {
                let mut y = vec![235u8; w * h];
                if i == 1 {
                    for row in 200..400 {
                        y[row * w + x..][..300].fill(16);
                    }
                }
                [y, vec![128u8; Format::Yuv420.len(w, h) - w * h]].concat()
            })
            .collect();
        Frames {
            format: Format::Yuv420,
            w,
            h,
            each: frames,
        }
    };
    let c = compared(&yuv(400), &yuv(1000));
    assert_eq!(c.verdict, Verdict::Changed, "{c:?}");
    let area = c.moments[0].area;
    assert!(
        area[0] <= 400 && (1300..=1600).contains(&area[2]),
        "{area:?}"
    );
    assert!(area[1] <= 200 && (400..=900).contains(&area[3]), "{area:?}");
}
