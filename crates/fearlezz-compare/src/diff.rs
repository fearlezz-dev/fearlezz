//! What differs between two images: thumbnails for matching states, the blurred per channel
//! difference, the masks it makes and the boxes around them.
#![expect(
    clippy::indexing_slicing,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "pixel loops index frame buffers whose size is fixed by the frame's width and \
              height, and convert between pixel coordinates and sample values in range"
)]

use crate::{Box4, Image};

/// Per pixel difference in any color channel (0 to 255) that counts as different.
pub(crate) const PIX_THRESH: i16 = 28;
/// Changed areas smaller than this many pixels are noise.
pub(crate) const MIN_BLOB: usize = 40;
const THUMB_W: usize = 48;
/// Changed areas closer than this many pixels share one box.
const BOX_GROW: u32 = 12;
/// The Gaussian blur that removes codec noise before differencing: sigma 1, radius 4,
/// normalized. Written out, since `exp` differs by a last bit between platform maths
/// libraries and the blur must be the same everywhere.
const BLUR: [f32; 9] = [
    0.000_133_830_62,
    0.004_431_861_5,
    0.053_991_124,
    0.241_971_43,
    0.398_943_45,
    0.241_971_43,
    0.053_991_124,
    0.004_431_861_5,
    0.000_133_830_62,
];

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
                for v in &mut ws {
                    *v /= sum;
                }
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
pub(crate) fn thumb(g: &Image) -> Vec<f32> {
    let th = ((THUMB_W * g.h) as f64 / g.w as f64).round().max(1.0) as usize;
    resize(&luma(&g.px), g.w, g.h, THUMB_W, th)
        .into_iter()
        .map(f32::from)
        .collect()
}

pub(crate) struct Mask {
    pub(crate) w: usize,
    pub(crate) h: usize,
    pub(crate) on: Vec<bool>,
}

impl Mask {
    pub(crate) fn count(&self) -> usize {
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
    pub(crate) fn open(&self) -> Mask {
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
    pub(crate) fn parts(&self) -> Vec<Vec<usize>> {
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
    pub(crate) fn drop_specks(&mut self) {
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
        let w = self.w;
        self.grown(grow)
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

    /// The mask grown by `by` pixels (city block).
    pub(crate) fn grown(&self, by: u32) -> Mask {
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
        Mask {
            w,
            h,
            on: dist.iter().map(|d| *d <= by).collect(),
        }
    }
}

/// Separable Gaussian blur with mirrored edges, as scipy's default.
pub(crate) fn blur(px: &[f32], w: usize, h: usize) -> Vec<f32> {
    let (k, r) = (&BLUR, BLUR.len() / 2);
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

pub(crate) struct Diff {
    /// How many pixels differ; without boxes asked for, only whether any do.
    pub px: usize,
    pub boxes: Vec<Box4>,
}

/// Pixels that really differ between two images in any color channel, and with `want_boxes`
/// boxes around them. Cutting a film into states only needs to know whether frames differ.
pub(crate) fn diff(a: &Image, b: &Image, want_boxes: bool) -> Diff {
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
pub(crate) fn outside_range(x: &Image, near: &[&Image]) -> usize {
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

// ----- stills ------------------------------------------------------------------------------

/// The smallest change of a still a person sees, in pixels; fewer is render jitter (proven on
/// the proof host's own pages: an icon drawn a fraction of a pixel apart leaves up to 22).
const STILL_MIN: usize = 30;
/// The largest squared YIQ distance two colours of a still may be apart and still be the same
/// colour: pixelmatch's threshold 0.01 of the largest distance, 35215, about 2 levels of 255.
const STILL_DELTA: f64 = 35215.0 * 0.01 * 0.01;

/// Pixels that differ between two stills, and boxes around them. Stills are lossless, so
/// unlike film frames they get no codec noise allowance: every pixel more than about 2
/// levels apart counts, however faint, unless it is antialiasing, and fewer than
/// [`STILL_MIN`] count as none. The colour distance and the antialiasing rule are
/// pixelmatch's (ISC licence, Mapbox), on RGB without alpha.
pub(crate) fn still_diff(a: &Image, b: &Image) -> Diff {
    if a.w != b.w || a.h != b.h {
        return Diff {
            px: a.w * a.h,
            boxes: vec![[0, 0, a.w as u32, a.h as u32]],
        };
    }
    let (w, h) = (a.w, a.h);
    let mut mask = Mask {
        w,
        h,
        on: vec![false; w * h],
    };
    for y in 0..h {
        for x in 0..w {
            if delta(a, b, y * w + x, y * w + x, false).abs() > STILL_DELTA
                && !antialiased(a, b, x, y)
                && !antialiased(b, a, x, y)
            {
                mask.on[y * w + x] = true;
            }
        }
    }
    let px = mask.count();
    if px < STILL_MIN {
        return Diff {
            px: 0,
            boxes: Vec::new(),
        };
    }
    Diff {
        px,
        boxes: mask.boxes(BOX_GROW),
    }
}

/// Pixel `p` of `a` against pixel `q` of `b`: the squared YIQ distance, negative when `b`'s
/// is darker, or with `y_only` only the brightness difference.
fn delta(a: &Image, b: &Image, p: usize, q: usize, y_only: bool) -> f64 {
    let rgb = |img: &Image, i: usize| {
        img.px[i * 3..]
            .first_chunk::<3>()
            .copied()
            .unwrap_or_default()
    };
    let (pa, pb) = (rgb(a, p), rgb(b, q));
    if pa == pb {
        return 0.0;
    }
    let [r1, g1, b1] = pa.map(f64::from);
    let [r2, g2, b2] = pb.map(f64::from);
    let y1 = r1 * 0.298_895_31 + g1 * 0.586_622_47 + b1 * 0.114_482_23;
    let y2 = r2 * 0.298_895_31 + g2 * 0.586_622_47 + b2 * 0.114_482_23;
    let y = y1 - y2;
    if y_only {
        return y;
    }
    let i = (r1 * 0.595_977_99 - g1 * 0.274_176_10 - b1 * 0.321_801_89)
        - (r2 * 0.595_977_99 - g2 * 0.274_176_10 - b2 * 0.321_801_89);
    let q = (r1 * 0.211_470_17 - g1 * 0.522_617_11 + b1 * 0.311_146_94)
        - (r2 * 0.211_470_17 - g2 * 0.522_617_11 + b2 * 0.311_146_94);
    let d = 0.5053 * y * y + 0.299 * i * i + 0.1957 * q * q;
    if y1 > y2 { -d } else { d }
}

/// The 3 by 3 block around `(x, y)` clipped to the image, and whether it is clipped.
fn around(
    img: &Image,
    x: usize,
    y: usize,
) -> (
    std::ops::RangeInclusive<usize>,
    std::ops::RangeInclusive<usize>,
    bool,
) {
    let (x0, y0) = (x.saturating_sub(1), y.saturating_sub(1));
    let (x2, y2) = ((x + 1).min(img.w - 1), (y + 1).min(img.h - 1));
    (x0..=x2, y0..=y2, x == x0 || x == x2 || y == y0 || y == y2)
}

/// Whether pixel `(x, y)` of `img` is likely antialiasing: among its neighbours it has both a
/// darker and a brighter one, at most two of its own colour, and the darkest or the brightest
/// has three or more of its own colour around it in both images (Vysniauskas, 2009).
fn antialiased(img: &Image, other: &Image, x: usize, y: usize) -> bool {
    let (xs, ys, edge) = around(img, x, y);
    let p = y * img.w + x;
    let mut zeroes = usize::from(edge);
    let (mut min, mut max) = (0.0, 0.0);
    let (mut darkest, mut brightest) = ((0, 0), (0, 0));
    for nx in xs {
        for ny in ys.clone() {
            if nx == x && ny == y {
                continue;
            }
            let d = delta(img, img, p, ny * img.w + nx, true);
            if d == 0.0 {
                zeroes += 1;
                if zeroes > 2 {
                    return false;
                }
            } else if d < min {
                min = d;
                darkest = (nx, ny);
            } else if d > max {
                max = d;
                brightest = (nx, ny);
            }
        }
    }
    if min == 0.0 || max == 0.0 {
        return false;
    }
    let settled = |(x, y): (usize, usize)| many_siblings(img, x, y) && many_siblings(other, x, y);
    settled(darkest) || settled(brightest)
}

/// Whether pixel `(x, y)` of `img` has three or more neighbours of its exact colour.
fn many_siblings(img: &Image, x: usize, y: usize) -> bool {
    let (xs, ys, edge) = around(img, x, y);
    let p = (y * img.w + x) * 3;
    let mut zeroes = usize::from(edge);
    for nx in xs {
        for ny in ys.clone() {
            if nx == x && ny == y {
                continue;
            }
            let q = (ny * img.w + nx) * 3;
            if img.px[p..p + 3] == img.px[q..q + 3] {
                zeroes += 1;
            }
            if zeroes > 2 {
                return true;
            }
        }
    }
    false
}
