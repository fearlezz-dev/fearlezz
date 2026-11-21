//! Pictures as decoders give them, the one conversion of each to the RGB the comparison reads,
//! and the one scaler of a large film's frames.
#![expect(
    clippy::indexing_slicing,
    reason = "pixel loops index planes whose size is fixed by the picture's width and height"
)]

use crate::Unfit;

/// Films are compared no larger than this, long side by short side.
pub const FILM_MAX: (usize, usize) = (1280, 720);

/// How a picture's bytes are laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Planar YUV 4:2:0, BT.601 limited range (ffmpeg's `yuv420p`, WebCodecs' `I420`): a Y
    /// plane of w by h, then a U and a V plane of half the width and half the height rounded
    /// up, every row packed with no padding.
    Yuv420,
    /// Three bytes per pixel: red, green, blue.
    Rgb,
}

impl Format {
    /// How many bytes a picture of `w` by `h` takes.
    #[must_use]
    pub fn len(self, w: usize, h: usize) -> usize {
        match self {
            Format::Yuv420 => w * h + 2 * w.div_ceil(2) * h.div_ceil(2),
            Format::Rgb => w * h * 3,
        }
    }
}

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Format::Yuv420 => "yuv420p",
            Format::Rgb => "rgb24",
        })
    }
}

/// An RGB image, three bytes per pixel: what the comparison reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// Its width.
    pub w: usize,
    /// Its height.
    pub h: usize,
    /// Its pixels, row by row.
    pub px: Vec<u8>,
}

/// One decoded frame, borrowed from the decoder's buffer.
#[derive(Debug, Clone, Copy)]
pub struct Picture<'a> {
    format: Format,
    w: usize,
    h: usize,
    data: &'a [u8],
}

impl<'a> Picture<'a> {
    /// The picture `data` holds in `format` at `w` by `h`.
    ///
    /// # Errors
    ///
    /// When the size is empty or `data` does not fill it exactly.
    pub fn new(format: Format, w: usize, h: usize, data: &'a [u8]) -> Result<Self, Unfit> {
        if w == 0 || h == 0 || data.len() != format.len(w, h) {
            return Err(Unfit::Bytes {
                format,
                w,
                h,
                len: data.len(),
            });
        }
        Ok(Picture { format, w, h, data })
    }

    /// How its bytes are laid out.
    #[must_use]
    pub fn format(&self) -> Format {
        self.format
    }

    /// Its width and height.
    #[must_use]
    pub fn size(&self) -> (usize, usize) {
        (self.w, self.h)
    }

    /// The picture's bytes, as given.
    #[must_use]
    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    /// The picture as RGB. YUV is converted with the BT.601 limited range matrix, each chroma
    /// sample shared by the 2 by 2 luma samples it covers, in 16 bit fixed point rounded half
    /// up, so every build gives the same bytes. RGB is copied as it is.
    #[must_use]
    pub fn rgb(&self) -> Image {
        let (w, h) = (self.w, self.h);
        let px = match self.format {
            Format::Rgb => self.data.to_vec(),
            Format::Yuv420 => {
                let cw = w.div_ceil(2);
                let (luma, chroma) = self.data.split_at(w * h);
                let (u, v) = chroma.split_at(cw * h.div_ceil(2));
                let mut px = Vec::with_capacity(w * h * 3);
                for y in 0..h {
                    let row = &luma[y * w..(y + 1) * w];
                    let (ur, vr) = (&u[y / 2 * cw..][..cw], &v[y / 2 * cw..][..cw]);
                    for (x, &l) in row.iter().enumerate() {
                        let (cb, cr) = (usize::from(ur[x / 2]), usize::from(vr[x / 2]));
                        let l = Y[usize::from(l)] + HALF;
                        px.extend([
                            clamp(l + RV[cr]),
                            clamp(l + GU[cb] + GV[cr]),
                            clamp(l + BU[cb]),
                        ]);
                    }
                }
                px
            }
        };
        Image { w, h, px }
    }

    /// The picture's bytes at `nw` by `nh`, in its own format, each sample the average of the
    /// samples it covers weighted by how much of each it covers (an area filter), rounded half
    /// up. Chroma planes are scaled to half the new size, rounded up.
    #[must_use]
    pub fn area(&self, nw: usize, nh: usize) -> Vec<u8> {
        let (w, h) = (self.w, self.h);
        match self.format {
            Format::Rgb => area(self.data, (w, h), 3, (nw, nh)),
            Format::Yuv420 => {
                let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
                let (ncw, nch) = (nw.div_ceil(2), nh.div_ceil(2));
                let (luma, chroma) = self.data.split_at(w * h);
                let (u, v) = chroma.split_at(cw * ch);
                let mut out = area(luma, (w, h), 1, (nw, nh));
                out.extend(area(u, (cw, ch), 1, (ncw, nch)));
                out.extend(area(v, (cw, ch), 1, (ncw, nch)));
                out
            }
        }
    }

    /// The picture as RGB at `seen`, scaled first when that is not its own size.
    pub(crate) fn seen(&self, seen: (usize, usize)) -> Image {
        if seen == (self.w, self.h) {
            return self.rgb();
        }
        let data = self.area(seen.0, seen.1);
        Picture {
            format: self.format,
            w: seen.0,
            h: seen.1,
            data: &data,
        }
        .rgb()
    }
}

/// Fixed point: 16 fraction bits, and a half to round with.
const HALF: i32 = 1 << 15;

/// Each luma value's part of every channel: 255/219 (Y minus 16).
const Y: [i32; 256] = table(76_309, 16);
/// Each chroma value's part of red, green and blue: 1.596027 (V minus 128) to red, 0.391762
/// (U minus 128) and 0.812968 (V minus 128) off green, 2.017232 (U minus 128) to blue.
const RV: [i32; 256] = table(104_597, 128);
const GU: [i32; 256] = table(-25_675, 128);
const GV: [i32; 256] = table(-53_279, 128);
const BU: [i32; 256] = table(132_201, 128);

/// `c` times (each byte minus `offset`).
const fn table(c: i32, offset: i32) -> [i32; 256] {
    let mut t = [0; 256];
    let (mut i, mut v) = (0, -offset);
    while i < 256 {
        t[i] = c * v;
        i += 1;
        v += 1;
    }
    t
}

/// A fixed point sample, already rounded, as a byte.
fn clamp(v: i32) -> u8 {
    u8::try_from((v >> 16).clamp(0, 255)).unwrap_or(u8::MAX)
}

/// The largest size within `max` (long side by short side) with the aspect of `w` by `h`;
/// `w` by `h` itself when it fits.
#[must_use]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a scaled side is positive and never larger than the side it scales"
)]
pub fn fit(w: usize, h: usize, max: (usize, usize)) -> (usize, usize) {
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

/// For each of `to` samples along a line of `from`, its first source sample and how much of
/// each source sample from there it covers. Measured in units where a source sample is `to`
/// long and a new one `from` long, every overlap is a whole number and each sample's weights
/// add up to `from`.
fn spans(from: usize, to: usize) -> Vec<(usize, Vec<u64>)> {
    (0..to)
        .map(|x| {
            let (lo, hi) = (x * from, (x + 1) * from);
            let first = lo / to;
            let weights = (first..=(hi - 1) / to)
                .map(|i| (hi.min((i + 1) * to) - lo.max(i * to)) as u64)
                .collect();
            (first, weights)
        })
        .collect()
}

/// A plane of `ch` interleaved channels at `size`, scaled to `to` by area.
fn area(px: &[u8], size: (usize, usize), ch: usize, to: (usize, usize)) -> Vec<u8> {
    let ((w, h), (nw, nh)) = (size, to);
    let (wx, wy) = (spans(w, nw), spans(h, nh));
    let mut tmp = vec![0u64; nw * h * ch];
    for y in 0..h {
        for (x, (lo, ws)) in wx.iter().enumerate() {
            for c in 0..ch {
                tmp[(y * nw + x) * ch + c] = ws
                    .iter()
                    .enumerate()
                    .map(|(k, wt)| u64::from(px[(y * w + lo + k) * ch + c]) * wt)
                    .sum();
            }
        }
    }
    let total = (w * h) as u64;
    let mut out = vec![0u8; nw * nh * ch];
    for (y, (lo, ws)) in wy.iter().enumerate() {
        for x in 0..nw {
            for c in 0..ch {
                let acc: u64 = ws
                    .iter()
                    .enumerate()
                    .map(|(k, wt)| tmp[((lo + k) * nw + x) * ch + c] * wt)
                    .sum();
                out[(y * nw + x) * ch + c] =
                    u8::try_from((acc + total / 2) / total).unwrap_or(u8::MAX);
            }
        }
    }
    out
}
