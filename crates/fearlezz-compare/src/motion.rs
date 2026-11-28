//! A state caught mid motion: a scroll or a slide the two films caught at other moments shows
//! the same content at other places, which no neighbour's range explains. Such a state is no
//! change when every part of it that differs is the other film's content caught at one moment
//! of a motion that film itself makes there, between two of its states. A part farther than
//! [`REACH`] from where the other film changes is never forgiven, and a part that differs in
//! any other way stays a change.
#![expect(
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "pixel loops index frame buffers whose size is fixed by the frame's width and \
              height, and convert between pixel coordinates and sample values in range"
)]

use std::collections::BTreeMap;
use std::rc::Rc;

use crate::Image;
use crate::diff::{MIN_BLOB, Mask, PIX_THRESH, blur};

/// How far, in compared pixels, content may move between two states and still be followed. A
/// farther move is not followed, so a state caught inside it stays a change.
const REACH: isize = 200;
/// How many states on each side of a state's counterpart the other film's motion is taken from.
pub(crate) const AROUND: usize = 3;
/// Blurred samples are kept in fixed point, this many steps per level, which halves the memory
/// of floats and keeps every comparison integer.
const SCALE: f32 = 64.0;
const THRESH: i32 = PIX_THRESH as i32 * SCALE as i32;
/// How far apart, in levels of green, two neighbouring pixels are on an edge or a letter, the
/// texture a motion is measured on; flat areas match any motion.
const EDGE: i32 = 24 * SCALE as i32;

/// An image's colour channels after the blur that removes codec noise, in fixed point.
pub(crate) struct Planes {
    w: usize,
    h: usize,
    c: [Vec<u16>; 3],
}

impl Planes {
    fn of(img: &Image) -> Planes {
        let (w, h) = (img.w, img.h);
        let c = std::array::from_fn(|c| {
            let plane: Vec<f32> = (0..w * h).map(|p| f32::from(img.px[p * 3 + c])).collect();
            blur(&plane, w, h)
                .into_iter()
                .map(|v| (v * SCALE).round().clamp(0.0, f32::from(u16::MAX)) as u16)
                .collect()
        });
        Planes { w, h, c }
    }

    fn get(&self, c: usize, p: usize) -> i32 {
        i32::from(self.c[c][p])
    }

    /// Pixel `p + (dx, dy)`, when it is inside.
    fn at(&self, p: usize, (dx, dy): (isize, isize)) -> Option<usize> {
        let (x, y) = ((p % self.w) as isize + dx, (p / self.w) as isize + dy);
        (x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h)
            .then(|| y as usize * self.w + x as usize)
    }

    /// Whether pixel `p` of `self` lies outside the range of `other` at `q` and its four
    /// neighbours, give or take the threshold, in any channel: content moved by a fraction of a
    /// pixel lands between them.
    fn far_from(&self, p: usize, other: &Planes, q: usize) -> bool {
        let (x, y, w) = (q % other.w, q / other.w, other.w);
        let near = [
            q,
            if x > 0 { q - 1 } else { q },
            if x + 1 < w { q + 1 } else { q },
            if y > 0 { q - w } else { q },
            if y + 1 < other.h { q + w } else { q },
        ];
        (0..3).any(|c| {
            let (lo, hi) = near.iter().fold((i32::MAX, i32::MIN), |(lo, hi), &n| {
                (lo.min(other.get(c, n)), hi.max(other.get(c, n)))
            });
            let v = self.get(c, p);
            v < lo - THRESH || v > hi + THRESH
        })
    }

    /// Pixels that differ from `other`, without noise.
    fn changed(&self, other: &Planes) -> Mask {
        let on = (0..self.w * self.h)
            .map(|p| (0..3).any(|c| (self.get(c, p) - other.get(c, p)).abs() > THRESH))
            .collect();
        let mut m = Mask {
            w: self.w,
            h: self.h,
            on,
        }
        .open();
        m.drop_specks();
        m
    }
}

/// What moves from one state of a film to the next.
pub(crate) struct Step {
    /// The pixels that change.
    moving: Mask,
    /// How the changed area's content moved, when it shows texture to tell.
    shift: Option<(isize, isize)>,
}

impl Step {
    fn between(from: &Planes, to: &Planes) -> Step {
        let moving = from.changed(to);
        let shift = shift(from, to, &moving);
        Step { moving, shift }
    }
}

/// How the content of `from` within `area` moved to show in `to`: the shift `d` with
/// `to(p + d)` most like `from(p)` over the textured pixels of `from` in the area's box grown by
/// 16, searched every fourth pixel within [`REACH`] and then pixel by pixel around the best
/// four. None without enough texture to tell.
fn shift(from: &Planes, to: &Planes, area: &Mask) -> Option<(isize, isize)> {
    let (w, h) = (from.w, from.h);
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for p in (0..w * h).filter(|&p| area.on[p]) {
        let (x, y) = (p % w, p / w);
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    if x0 == usize::MAX {
        return None;
    }
    let (x0, y0) = (x0.saturating_sub(16).max(1), y0.saturating_sub(16).max(1));
    let (x1, y1) = ((x1 + 17).min(w - 1), (y1 + 17).min(h - 1));
    let textured: Vec<usize> = (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| y * w + x))
        .filter(|&i| {
            let g = |j| from.get(1, j);
            (g(i + 1) - g(i - 1)).abs().max((g(i + w) - g(i - w)).abs()) > EDGE
        })
        .collect();
    if textured.len() < 20 {
        return None;
    }
    let step = (textured.len() / 250).max(1);
    let sampled = textured.len().div_ceil(step);
    // The share of the samples still inside after the shift that land on unlike pixels, in
    // thousandths; a shift that keeps fewer than a quarter inside is no answer.
    let unlike = |d: (isize, isize)| {
        let (mut inside, mut bad) = (0, 0);
        for &p in textured.iter().step_by(step) {
            if let Some(q) = from.at(p, d) {
                inside += 1;
                bad += usize::from(from.far_from(p, to, q));
            }
        }
        if inside * 4 < sampled {
            usize::MAX
        } else {
            bad * 1000 / inside
        }
    };
    let mut coarse: Vec<(usize, isize, isize)> = (-REACH..=REACH)
        .step_by(4)
        .flat_map(|dy| (-REACH..=REACH).step_by(4).map(move |dx| (dx, dy)))
        .map(|(dx, dy)| (unlike((dx, dy)), dy, dx))
        .collect();
    coarse.sort_unstable();
    coarse
        .iter()
        .take(4)
        .flat_map(|&(_, cy, cx)| {
            (cy - 2..=cy + 2).flat_map(move |dy| (cx - 2..=cx + 2).map(move |dx| (dx, dy)))
        })
        .map(|(dx, dy)| (unlike((dx, dy)), dy, dx))
        .min()
        .map(|(_, dy, dx)| (dx, dy))
}

/// Whether pixel `p` of `x` is unlike both `refs`, each moved by its shift (`x(p)` against
/// `r(p - d)`).
fn bad_at(x: &Planes, refs: &[(&Planes, (isize, isize)); 2], p: usize) -> bool {
    refs.iter()
        .all(|&(r, (dx, dy))| x.at(p, (-dx, -dy)).is_none_or(|q| x.far_from(p, r, q)))
}

/// Whether a sample of `part` already says it is unlike `refs`: more than a quarter of up to 256
/// of its pixels are. Most moments a frame is not caught at are this far off, and the sample
/// costs little. It can only find a part unlike, never like, so it never hides a change.
fn far_off(x: &Planes, refs: &[(&Planes, (isize, isize)); 2], part: &[usize]) -> bool {
    let stride = (part.len() / 256).max(1);
    let (sampled, bad) = part.iter().step_by(stride).fold((0, 0), |(n, b), &p| {
        (n + 1, b + usize::from(bad_at(x, refs, p)))
    });
    sampled >= 64 && bad * 4 > sampled
}

/// Whether some pixels of `part` of `x` are unlike both `refs`, in an area larger than noise.
fn unlike(x: &Planes, refs: &[(&Planes, (isize, isize)); 2], part: &[usize]) -> bool {
    let bad: Vec<usize> = part
        .iter()
        .copied()
        .filter(|&p| bad_at(x, refs, p))
        .collect();
    if bad.len() < MIN_BLOB {
        return false;
    }
    let w = x.w;
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for &p in &bad {
        let (x, y) = (p % w, p / w);
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    let (bw, bh) = (x1 - x0 + 1, y1 - y0 + 1);
    let mut m = Mask {
        w: bw,
        h: bh,
        on: vec![false; bw * bh],
    };
    for p in bad {
        m.on[(p / w - y0) * bw + (p % w - x0)] = true;
    }
    let mut m = m.open();
    m.drop_specks();
    m.count() > 0
}

/// Whether every one of `parts` of `x` shows the content of consecutive `frames` at one moment
/// of one of their `steps`: for a step from `k` to `k + 1` whose content moves by `m`, `x(p)` is
/// `frames[k](p - t m)` for one `t` from 0 to 1, or `frames[k + 1](p - (t - 1) m)`, which shows
/// what the earlier state hid, such as content under a fixed bar. One moment for the whole
/// frame, so a part that changed some other way (gone blank, say) finds no moment of its own.
/// A moment that is a state itself (`t` 0 or 1) counts only for the state `center` matched and
/// its neighbours, as a transition's ends do.
fn seen_moving(
    x: &Planes,
    center: usize,
    frames: &[Rc<Planes>],
    steps: &[Rc<Step>],
    parts: &[Vec<usize>],
) -> bool {
    steps.iter().enumerate().any(|(k, step)| {
        let Some((mx, my)) = step.shift else {
            return false;
        };
        let n = mx.abs().max(my.abs()).max(1);
        let near = |s: usize| s.abs_diff(center) <= 1;
        let from = isize::from(!near(k));
        let to = n - isize::from(!near(k + 1));
        (from..=to).any(|t| {
            let d = |m: isize| ((m * t) as f64 / n as f64).round() as isize;
            let (dx, dy) = (d(mx), d(my));
            let refs = [
                (&*frames[k], (dx, dy)),
                (&*frames[k + 1], (dx - mx, dy - my)),
            ];
            !parts.iter().any(|part| far_off(x, &refs, part))
                && !parts.iter().any(|part| unlike(x, &refs, part))
        })
    })
}

/// Whether every part of `x` that differs from `frames[center]` is content of the consecutive
/// `frames` (the other film's states around the counterpart) caught part of the way through one
/// of their `steps`, within reach of where that film moves.
pub(crate) fn only_moved(
    x: &Planes,
    center: usize,
    frames: &[Rc<Planes>],
    steps: &[Rc<Step>],
) -> bool {
    if frames.iter().any(|f| f.w != x.w || f.h != x.h) {
        return false;
    }
    let changed = x.changed(&frames[center]);
    let mut moving = Mask {
        w: x.w,
        h: x.h,
        on: vec![false; x.w * x.h],
    };
    for step in steps {
        for (o, v) in moving.on.iter_mut().zip(&step.moving.on) {
            *o |= *v;
        }
    }
    let near_moving = moving.grown(REACH as u32);
    let parts = changed.parts();
    if parts.is_empty() {
        return true;
    }
    parts.iter().flatten().all(|&p| near_moving.on[p])
        && seen_moving(x, center, frames, steps, &parts)
}

/// One film's blurred states and steps, kept while the comparison is near them.
#[derive(Default)]
pub(crate) struct Motion {
    planes: BTreeMap<usize, Rc<Planes>>,
    steps: BTreeMap<usize, Rc<Step>>,
}

impl Motion {
    /// State `s`, whose first frame is `img`, blurred.
    pub(crate) fn planes(&mut self, s: usize, img: &Image) -> Rc<Planes> {
        Rc::clone(
            self.planes
                .entry(s)
                .or_insert_with(|| Rc::new(Planes::of(img))),
        )
    }

    /// What moves from state `k`, blurred as `from`, to the next, blurred as `to`.
    pub(crate) fn step(&mut self, k: usize, from: &Planes, to: &Planes) -> Rc<Step> {
        Rc::clone(
            self.steps
                .entry(k)
                .or_insert_with(|| Rc::new(Step::between(from, to))),
        )
    }

    /// Forgets the states before `s`.
    pub(crate) fn forget_before(&mut self, s: usize) {
        self.planes.retain(|&k, _| k >= s);
        self.steps.retain(|&k, _| k >= s);
    }
}
