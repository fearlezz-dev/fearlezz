//! Comparing films fed frame by frame: their states, what changed, where and when.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

use fearlezz_compare::{Comparison, Cut, Film, Format, Picture, Reread, Unfit, Verdict, compare};
use fearlezz_proof::media::{Mark, Marks, Span, Timing};

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

/// A caller that keeps frames for the second read, as the page does, keeps those that start a
/// state: the frames after which the cut counts one more.
#[test]
fn the_cut_counts_its_states_as_frames_arrive() {
    let frames = Frames::boxed(64, 48, 20, 10, 10);
    let mut cut = Cut::new(64, 48, 20);
    let mut starts = Vec::new();
    for (i, f) in frames.each.iter().enumerate() {
        let had = cut.states();
        cut.push(
            &Picture::new(Format::Rgb, 64, 48, f).unwrap(),
            i as f64 / 10.0,
            0.1,
        )
        .unwrap();
        if cut.states() > had {
            starts.push(i);
        }
    }
    assert_eq!(starts, [0, 10]);
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

/// A still: one frame of `w` by `h` RGB pixels of colour `bg`, with `f` drawing on it.
fn still(w: usize, h: usize, bg: [u8; 3], f: impl Fn(&mut [u8], usize)) -> Frames {
    let mut px = bg.repeat(w * h);
    f(&mut px, w);
    Frames {
        format: Format::Rgb,
        w,
        h,
        each: vec![px],
    }
}

/// Fills the `x0..x1` by `y0..y1` rectangle of `px`, `w` wide, with colour `c`.
fn fill(px: &mut [u8], w: usize, (x0, x1): (usize, usize), (y0, y1): (usize, usize), c: [u8; 3]) {
    for y in y0..y1 {
        for x in x0..x1 {
            px[(y * w + x) * 3..][..3].copy_from_slice(&c);
        }
    }
}

#[test]
fn a_faint_block_moved_in_a_still_is_a_change() {
    let at = |y: usize| {
        still(200, 120, [13, 14, 17], move |px, w| {
            fill(px, w, (20, 180), (y, y + 40), [30, 32, 39]);
        })
    };
    let c = compared(&at(20), &at(32));
    assert_eq!(c.verdict, Verdict::Changed, "{c:?}");
}

#[test]
fn an_antialiased_edge_drawn_differently_in_a_still_is_the_same() {
    // A black block on white whose right edge is one column of grey, a shade apart.
    let edge = |grey: u8| {
        still(120, 80, [255; 3], move |px, w| {
            fill(px, w, (20, 60), (20, 60), [0; 3]);
            fill(px, w, (60, 61), (20, 60), [grey; 3]);
        })
    };
    let c = compared(&edge(120), &edge(170));
    assert_eq!(c.verdict, Verdict::Same, "{c:?}");
}

#[test]
fn a_change_of_fewer_than_30_pixels_in_a_still_is_the_same() {
    let dot = |side: usize| {
        still(100, 100, [255; 3], move |px, w| {
            fill(px, w, (40, 40 + side), (40, 40 + side), [200, 0, 0]);
        })
    };
    let blank = still(100, 100, [255; 3], |_, _| {});
    assert_eq!(
        compared(&blank, &dot(5)).verdict,
        Verdict::Same,
        "25 pixels"
    );
    let c = compared(&blank, &dot(6));
    assert_eq!(c.verdict, Verdict::Changed, "36 pixels: {c:?}");
}

/// Marks at `at` seconds, named in order.
fn marks(at: &[(&str, f64)]) -> Marks {
    Marks {
        marks: at
            .iter()
            .map(|(name, at)| Mark {
                name: (*name).into(),
                at: *at,
                hidden: false,
            })
            .collect(),
        spans: Vec::new(),
    }
}

/// Compares `before` and `after`, each film with the marks its check said.
fn marked(before: (&Frames, Marks), after: (&Frames, Marks)) -> Comparison {
    let (mut b, mut a) = (before.0.cut(), after.0.cut());
    b.marks = before.1;
    a.marks = after.1;
    compare(&b, before.0.again(None), &a, after.0.again(None)).unwrap()
}

#[test]
fn a_box_as_long_after_its_mark_is_the_same_however_late_the_mark() {
    // The page took half a second longer to answer: the box came as long after the press.
    let c = marked(
        (
            &Frames::boxed(64, 48, 20, 10, 10),
            marks(&[("pressed", 0.9)]),
        ),
        (
            &Frames::boxed(64, 48, 25, 10, 15),
            marks(&[("pressed", 1.4)]),
        ),
    );
    assert_eq!(c.verdict, Verdict::Same, "{c:?}");
}

#[test]
fn a_box_later_after_the_same_mark_is_timing() {
    let c = marked(
        (
            &Frames::boxed(64, 48, 20, 10, 10),
            marks(&[("pressed", 0.9)]),
        ),
        (
            &Frames::boxed(64, 48, 25, 10, 15),
            marks(&[("pressed", 0.9)]),
        ),
    );
    assert_eq!(c.verdict, Verdict::Timing, "{c:?}");
    assert!((c.max_shift - 0.5).abs() < 1e-9, "{c:?}");
}

#[test]
fn a_box_later_inside_a_free_span_is_the_same() {
    let loading = |end: f64| Marks {
        marks: Vec::new(),
        spans: vec![Span {
            name: "loading".into(),
            start: 0.0,
            end,
            timing: Timing::Free,
        }],
    };
    // The box ends the loading; whatever came after it comes as long after.
    let c = marked(
        (&Frames::boxed(64, 48, 20, 10, 10), loading(1.05)),
        (&Frames::boxed(64, 48, 25, 10, 15), loading(1.55)),
    );
    assert_eq!(c.verdict, Verdict::Same, "{c:?}");
}

/// A film taken before its check marked anything is compared from its start, as it always was,
/// so marking a check's films is not itself a change.
#[test]
fn a_film_without_marks_is_timed_from_its_start() {
    let c = marked(
        (&Frames::boxed(64, 48, 20, 10, 10), Marks::default()),
        (
            &Frames::boxed(64, 48, 20, 10, 10),
            marks(&[("pressed", 0.9)]),
        ),
    );
    assert_eq!(c.verdict, Verdict::Same, "{c:?}");
    assert!(!c.marks_changed, "{c:?}");
}

#[test]
fn a_mark_one_film_has_and_the_other_not_is_a_change() {
    let c = marked(
        (
            &Frames::boxed(64, 48, 20, 10, 10),
            marks(&[("pressed", 0.9)]),
        ),
        (
            &Frames::boxed(64, 48, 20, 10, 10),
            marks(&[("pressed", 0.9), ("saved", 1.5)]),
        ),
    );
    assert_eq!(c.verdict, Verdict::Changed, "{c:?}");
    assert!(c.marks_changed, "{c:?}");
}

impl Frames {
    /// A white RGB film of runs of frames, each run blank or with a black box at its `x`.
    fn runs(w: usize, h: usize, runs: &[(usize, Option<usize>)]) -> Frames {
        let mut each = Vec::new();
        for &(n, x) in runs {
            for _ in 0..n {
                let mut px = vec![255u8; w * h * 3];
                if let Some(x) = x {
                    for y in 10..30 {
                        for xx in x..x + 20 {
                            px[(y * w + xx) * 3..][..3].fill(0);
                        }
                    }
                }
                each.push(px);
            }
        }
        Frames {
            format: Format::Rgb,
            w,
            h,
            each,
        }
    }
}

/// The marks cut both films into sections, and a state is only matched with one of the other
/// film's in the same section: the box shown before the press never answers for the box shown
/// after it, however alike they look.
#[test]
fn a_state_is_matched_only_within_its_marked_section() {
    let c = marked(
        (
            &Frames::runs(64, 48, &[(5, None), (5, Some(10)), (10, None)]),
            marks(&[("pressed", 1.0)]),
        ),
        (
            &Frames::runs(64, 48, &[(15, None), (5, Some(10))]),
            marks(&[("pressed", 1.5)]),
        ),
    );
    assert!(
        !c.pairs.iter().any(|p| p.before == 1 && p.after == 1),
        "the box before the press is not matched with the box after it: {:?}",
        c.pairs
    );
    assert_eq!(c.verdict, Verdict::Changed, "{c:?}");
}

/// The films play on one timeline aligned at their marks: before the first mark the shorter
/// film starts later, so both reach the mark together, and after it each plays its own stretch,
/// the shorter holding its last frame for the rest.
#[test]
fn the_timeline_aligns_the_films_at_their_marks() {
    let c = marked(
        (
            &Frames::boxed(64, 48, 20, 10, 10),
            marks(&[("pressed", 0.9)]),
        ),
        (
            &Frames::boxed(64, 48, 26, 10, 15),
            marks(&[("pressed", 1.4)]),
        ),
    );
    let t = &c.timeline;
    assert_eq!(t.len(), 2, "{t:?}");
    let (lead, rest) = (&t[0], &t[1]);
    assert_eq!((lead.at, lead.length), (0.0, 1.4), "{t:?}");
    assert_eq!(
        (lead.before.from, lead.before.to, lead.before.delay),
        (0.0, 0.9, 0.5),
        "{t:?}"
    );
    assert_eq!(
        (lead.after.from, lead.after.to, lead.after.delay),
        (0.0, 1.4, 0.0),
        "{t:?}"
    );
    assert_eq!((rest.at, rest.length), (1.4, 1.2), "{t:?}");
    assert_eq!(
        (rest.before.from, rest.before.to, rest.before.delay),
        (0.9, 2.0, 0.0),
        "{t:?}"
    );
    assert_eq!(
        (rest.after.from, rest.after.to, rest.after.delay),
        (1.4, 2.6, 0.0),
        "{t:?}"
    );
}

/// Without marks the timeline is one section from both films' start, as they always played.
#[test]
fn without_marks_the_timeline_starts_both_films_together() {
    let c = compared(
        &Frames::boxed(64, 48, 20, 10, 10),
        &Frames::boxed(64, 48, 25, 10, 10),
    );
    let t = &c.timeline;
    assert_eq!(t.len(), 1, "{t:?}");
    assert_eq!((t[0].at, t[0].length), (0.0, 2.5), "{t:?}");
    assert_eq!(
        (t[0].before.from, t[0].before.to, t[0].before.delay),
        (0.0, 2.0, 0.0),
        "{t:?}"
    );
    assert_eq!(
        (t[0].after.from, t[0].after.to, t[0].after.delay),
        (0.0, 2.5, 0.0),
        "{t:?}"
    );
}
