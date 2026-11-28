//! Matching two films' states and saying what changed between them.
#![expect(
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    reason = "states and costs are indexed by the warp path, which only names states the films \
              have, and a picture's sides fit in a box's u32"
)]

use std::collections::{BTreeMap, BTreeSet};

use crate::diff::{diff, outside_range, still_diff};
use crate::film::Window;
use crate::{Box4, Comparison, Film, Moment, Pair, Reread, State, Verdict};
use fearlezz_proof::media::{Footage, Section};

/// Seconds of timing difference that still count as the same timing.
const TIMING_TOL: f64 = 0.10;
/// Seconds a moment a check said may read later than the frame it shows on: the recorder's
/// first frame comes a little after the check's clock starts, and only that well known.
const SAID_TOL: f64 = 0.10;
/// A state held at least this long is a moment worth checking for timing.
const STABLE_MIN: f64 = 0.15;

/// Matches states of two films, only where `allowed` says a pair may be; returns the path of
/// (before, after) pairs and the costs.
fn warp(
    a: &[Vec<f32>],
    b: &[Vec<f32>],
    allowed: impl Fn(usize, usize) -> bool,
) -> (Vec<(usize, usize)>, Vec<Vec<f32>>) {
    let (n, m) = (a.len(), b.len());
    let cost: Vec<Vec<f32>> = a
        .iter()
        .enumerate()
        .map(|(i, x)| {
            b.iter()
                .enumerate()
                .map(|(j, y)| {
                    if allowed(i, j) {
                        x.iter().zip(y).map(|(p, q)| (p - q).abs()).sum::<f32>() / x.len() as f32
                    } else {
                        f32::INFINITY
                    }
                })
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

/// Compares two films, reading each again through its [`Reread`] for the pixel diffs. Films
/// of different sizes are changed whole, and neither is read again.
///
/// # Errors
///
/// When a second read fails or gives other frames than the first.
pub fn compare<B, A>(
    before: &Film,
    before_again: B,
    after: &Film,
    after_again: A,
) -> Result<Comparison, B::Error>
where
    B: Reread,
    A: Reread<Error = B::Error>,
{
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
        tail: None,
        states_before: before.states.len(),
        states_after: after.states.len(),
        moments: Vec::new(),
        pairs: Vec::new(),
        marks_changed: false,
        timeline: Vec::new(),
    };
    let clock = Clock::of(before, after);
    out.marks_changed = clock.is_none();
    let clock = clock.unwrap_or_default();
    if !still {
        out.timeline = timeline(&clock.anchors, before.total, after.total);
        out.tail = Some(round3(clock.tail(before.total, after.total)));
    }
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
    // States are matched only within the same section between two anchors: a state that runs
    // across an anchor, or ends just before one by what a said moment may read late, is in both
    // sections it touches.
    let sections = |states: &[State], at: &dyn Fn(&(f64, f64)) -> f64| {
        let of = |t: f64| clock.anchors.iter().filter(|x| at(x) <= t).count();
        states
            .iter()
            .map(|k| {
                let end = (k.start + k.dur - 1e-6).max(k.start);
                (of(k.start), of(end + SAID_TOL))
            })
            .collect::<Vec<_>>()
    };
    let (so, sn) = (sections(ko, &|x| x.0), sections(kn, &|x| x.1));
    let allowed = |i: usize, j: usize| so[i].0 <= sn[j].1 && sn[j].0 <= so[i].1;
    let (path, cost) = warp(&thumbs(ko), &thumbs(kn), allowed);

    let mut windows = (
        Window::new(before, before_again),
        Window::new(after, after_again),
    );
    // A state within a free span of its film, or starting just before it by what a said moment
    // may read late, looks different each time; only the state the span ends on is compared, and a film's last state is what it ends on, even while a span
    // such as a spinner runs on to its end.
    let free = |i: usize, j: usize| {
        let within = |m: &fearlezz_proof::media::Marks, s: &[State], i: usize| {
            i + 1 < s.len() && m.free_over(s[i].start, s[i].start + s[i].dur, SAID_TOL)
        };
        within(&before.marks, ko, i) || within(&after.marks, kn, j)
    };
    out.pairs = pairs(&mut windows, &path, still, &allowed, &free)?;
    let max_shift = max_shift(ko, kn, &path, &cost, &clock);
    out.max_shift = round3(max_shift.0);
    out.max_shift_at = max_shift.1.map(round3);
    out.moments = moments(&out.pairs, ko, kn);

    out.verdict = if !out.moments.is_empty() || out.marks_changed {
        Verdict::Changed
    } else if !still
        && (clock.tail(before.total, after.total).abs() > TIMING_TOL
            || max_shift.0.abs() > TIMING_TOL)
    {
        Verdict::Timing
    } else {
        Verdict::Same
    };
    Ok(out)
}

/// Each pair of states the path visits, with what differs between them. The path only moves
/// forward, so each film needs the states one before to one after.
fn pairs<B, A>(
    (wo, wn): &mut (Window<'_, B>, Window<'_, A>),
    path: &[(usize, usize)],
    still: bool,
    allowed: &dyn Fn(usize, usize) -> bool,
    free: &dyn Fn(usize, usize) -> bool,
) -> Result<Vec<Pair>, B::Error>
where
    B: Reread,
    A: Reread<Error = B::Error>,
{
    let (ko, kn) = (&wo.film().states, &wn.film().states);
    let after = wn.film();
    let mut pairs = Vec::new();
    let visited: BTreeSet<(usize, usize)> = path.iter().copied().collect();
    let mut cache: BTreeMap<(usize, usize), (usize, Vec<Box4>)> = BTreeMap::new();
    for &(i, j) in path {
        wo.span(i.saturating_sub(1), i + 1)?;
        wn.span(j.saturating_sub(1), j + 1)?;
        cache.retain(|&(ci, cj), _| ci + 1 >= i && cj + 1 >= j);
        let mut pd = |i: usize, j: usize| -> Result<(usize, Vec<Box4>), crate::Unfit> {
            if let Some(d) = cache.get(&(i, j)) {
                return Ok(d.clone());
            }
            let d = if still {
                still_diff(wo.image(i)?, wn.image(j)?)
            } else {
                diff(wo.image(i)?, wn.image(j)?, true)
            };
            let d = (d.px, d.boxes);
            cache.insert((i, j), d.clone());
            Ok(d)
        };
        let (mut px, mut boxes) = if free(i, j) {
            (0, Vec::new())
        } else {
            pd(i, j)?
        };
        // A still has one state, so nothing it could have been caught between.
        if px > 0 && !still {
            // When the films caught a transition at different moments, each side of the pair
            // has its true counterpart next to the other: a neighbour the path also visits
            // that matches it fully, or the other film's neighbours, whose range it sits in.
            // Both sides must have one; a state only one film shows is a change. A counterpart
            // across a mark is never one, since the marks say those moments differ.
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
            before_found =
                before_found || outside_range(wo.image(i)?, &wn.near(j, |jj| allowed(i, jj))?) == 0;
            after_found =
                after_found || outside_range(wn.image(j)?, &wo.near(i, |ii| allowed(ii, j))?) == 0;
            if before_found && after_found {
                px = 0;
                boxes.clear();
            }
        }
        let boxes = boxes.into_iter().map(|b| after.own_pixels(b)).collect();
        pairs.push(Pair {
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
    Ok(pairs)
}

/// How the two films are timed against each other: from the moments their checks marked,
/// matched in order, each state's start measured from the anchor before it or the end of a free
/// span after that, whichever is later; a film without marks is timed from its start. Inside a
/// span whose timing is free nothing is timed.
#[derive(Default)]
struct Clock {
    /// Each matched anchor's moment in the before film and the after film, in order.
    anchors: Vec<(f64, f64)>,
    free: (fearlezz_proof::media::Marks, fearlezz_proof::media::Marks),
}

impl Clock {
    /// The clock of two films, or None when their marks differ: one names a moment the other
    /// does not, or in another order. A film without marks says nothing about what happened
    /// in it, such as one taken before its check marked anything, so both are timed from their
    /// start.
    fn of(before: &Film, after: &Film) -> Option<Clock> {
        let free = (before.marks.clone(), after.marks.clone());
        let (b, a) = (before.marks.anchors(), after.marks.anchors());
        if b.is_empty() || a.is_empty() {
            return Some(Clock {
                anchors: Vec::new(),
                free,
            });
        }
        let names = |v: &[(String, f64)]| v.iter().map(|x| x.0.clone()).collect::<Vec<_>>();
        if names(&b) != names(&a) {
            return None;
        }
        Some(Clock {
            anchors: b.iter().zip(&a).map(|(b, a)| (b.1, a.1)).collect(),
            free,
        })
    }

    /// What moment `b` of the before film and moment `a` of the after film are each timed
    /// from: the anchor before `b` (or just after it, by what a said moment may read late), or a
    /// later end of a free span in that film.
    fn from(&self, b: f64, a: f64) -> (f64, f64) {
        let (fb, fa) = self
            .anchors
            .iter()
            .rev()
            .find(|(x, _)| *x <= b + SAID_TOL)
            .copied()
            .unwrap_or((0.0, 0.0));
        let later = |m: &fearlezz_proof::media::Marks, t: f64, f: f64| {
            m.freed_by(t).map_or(f, |e| e.max(f))
        };
        (later(&self.free.0, b, fb), later(&self.free.1, a, fa))
    }

    /// How much later a state that starts at `b` in the before film starts at `a` in the after
    /// film, each from what it is timed from; None inside a free span of either film.
    fn shift(&self, b: f64, a: f64) -> Option<f64> {
        if self.free.0.free_at(b) || self.free.1.free_at(a) {
            return None;
        }
        let (fb, fa) = self.from(b, a);
        Some((a - fa) - (b - fb))
    }

    /// How much longer the after film runs after what its end is timed from than the before
    /// film, 0 when it ends inside a free span.
    fn tail(&self, before: f64, after: f64) -> f64 {
        let end = |m: &fearlezz_proof::media::Marks, t: f64| m.free_at((t - 1e-6).max(0.0));
        if end(&self.free.0, before) || end(&self.free.1, after) {
            return 0.0;
        }
        let (fb, fa) = self.from(before, after);
        (after - fa) - (before - fb)
    }
}

/// Timing: how much later each held state of the before film shows in the after film, from
/// the anchor before it, the largest shift and the moment of the before film it is at.
fn max_shift(
    ko: &[State],
    kn: &[State],
    path: &[(usize, usize)],
    cost: &[Vec<f32>],
    clock: &Clock,
) -> (f64, Option<f64>) {
    let mut best: BTreeMap<usize, usize> = BTreeMap::new();
    for &(i, j) in path {
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
            let Some(shift) = clock.shift(k.start, kn[*j].start) else {
                continue;
            };
            if max_shift.1.is_none() || shift.abs() > max_shift.0.abs() {
                max_shift = (shift, Some(k.start));
            }
        }
    }
    max_shift
}

/// The moments that changed: each run of pairs that differ, with the area they differ in.
fn moments(pairs: &[Pair], ko: &[State], kn: &[State]) -> Vec<Moment> {
    let mut current: Option<(Moment, Vec<Box4>)> = None;
    let mut groups = Vec::new();
    for p in pairs {
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
    groups
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
        .collect()
}

/// The films' timeline from their matched `anchors` (each one's moment in the before and the
/// after film) and their lengths: a section from each anchor to the next, as long as the longer
/// film's stretch. Before the first anchor the shorter film starts later, so both reach it
/// together; elsewhere both start the section together. Without anchors, one section from both
/// films' start.
fn timeline(anchors: &[(f64, f64)], total_before: f64, total_after: f64) -> Vec<Section> {
    let bounds = |pick: fn(&(f64, f64)) -> f64, total: f64| {
        let mut b = vec![0.0];
        b.extend(anchors.iter().map(pick));
        b.push(total.max(b.last().copied().unwrap_or(0.0)));
        b
    };
    let (bb, ba) = (bounds(|x| x.0, total_before), bounds(|x| x.1, total_after));
    let mut at = 0.0;
    let mut out = Vec::new();
    for k in 0..bb.len() - 1 {
        let (lb, la) = (bb[k + 1] - bb[k], ba[k + 1] - ba[k]);
        let length = lb.max(la);
        if length < 1e-6 {
            continue;
        }
        let lead = k == 0 && !anchors.is_empty();
        let footage = |from: f64, to: f64, own: f64| Footage {
            from: round3(from),
            to: round3(to),
            delay: if lead { round3(length - own) } else { 0.0 },
        };
        out.push(Section {
            at: round3(at),
            length: round3(length),
            before: footage(bb[k], bb[k + 1], lb),
            after: footage(ba[k], ba[k + 1], la),
        });
        at += length;
    }
    out
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}
