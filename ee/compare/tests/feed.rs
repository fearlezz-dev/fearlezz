//! Feeding the comparison as the page does: stills as a canvas's RGBA, films as I420 planes,
//! each film read once.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

use fearlezz_page_compare::{Feed, Given, compare};

/// A white frame of `w` by `h` with a black square at `x`, as RGBA.
fn rgba(w: usize, h: usize, x: usize) -> Vec<u8> {
    let mut px = vec![255u8; w * h * 4];
    for y in 10..20 {
        for xx in x..x + 10 {
            px[(y * w + xx) * 4..][..3].fill(0);
        }
    }
    px
}

/// The same frame as I420 planes: black is luma 16, white 235, both with neutral chroma.
fn yuv(w: usize, h: usize, x: usize) -> Vec<u8> {
    let mut px = vec![235u8; w * h];
    for y in 10..20 {
        px[y * w + x..][..10].fill(16);
    }
    px.extend(vec![128u8; 2 * w.div_ceil(2) * h.div_ceil(2)]);
    px
}

fn fed(given: Given, frames: &[Vec<u8>]) -> Feed {
    let mut feed = Feed::new(64, 48, frames.len(), given);
    for (i, f) in frames.iter().enumerate() {
        feed.push(f, i as f64 / 10.0, 0.1).unwrap();
    }
    feed
}

#[test]
fn a_still_given_as_rgba_drops_its_alpha_and_finds_the_moved_square() {
    let (a, b) = (rgba(64, 48, 5), rgba(64, 48, 40));
    let c = compare(fed(Given::Rgba, &[a]), fed(Given::Rgba, &[b])).unwrap();
    assert!(c.still);
    assert_eq!(c.moments.len(), 1, "{c:?}");
    let area = c.moments[0].area;
    assert!(area[0] <= 5 && area[2] >= 50, "{area:?}");
}

/// The second read comes from the frames kept on the first, so a film of several states needs
/// no second decode.
#[test]
fn a_film_is_compared_from_the_frames_kept_as_it_was_fed() {
    let before: Vec<Vec<u8>> = (0..20)
        .map(|i| yuv(64, 48, if i < 10 { 5 } else { 20 }))
        .collect();
    let after: Vec<Vec<u8>> = (0..20)
        .map(|i| yuv(64, 48, if i < 10 { 5 } else { 40 }))
        .collect();
    let c = compare(fed(Given::Yuv420, &before), fed(Given::Yuv420, &after)).unwrap();
    assert!(!c.still);
    assert_eq!((c.states_before, c.states_after), (2, 2));
    assert_eq!(c.moments.len(), 1, "{c:?}");
    assert!((c.moments[0].after.0 - 1.0).abs() < 1e-9, "{c:?}");
}

/// A box that shows later after a load the page was told about is timed from where the load
/// ended, so the film looks the same.
#[test]
fn a_film_is_timed_from_its_marks() {
    let film = |load: usize| -> Vec<Vec<u8>> {
        (0..load + 10)
            .map(|i| yuv(64, 48, if i < load { 5 } else { 40 }))
            .collect()
    };
    let marked = |load: usize| {
        let mut feed = fed(Given::Yuv420, &film(load));
        feed.mark(
            serde_json::from_value(serde_json::json!({
                "marks": [{"name": "loaded", "at": load as f64 / 10.0}]
            }))
            .unwrap(),
        );
        feed
    };
    let plain = compare(fed(Given::Yuv420, &film(3)), fed(Given::Yuv420, &film(8))).unwrap();
    assert_eq!(
        plain.verdict,
        fearlezz_compare::Verdict::Timing,
        "{plain:?}"
    );
    let timed = compare(marked(3), marked(8)).unwrap();
    assert_eq!(timed.verdict, fearlezz_compare::Verdict::Same, "{timed:?}");
}

#[test]
fn a_frame_that_does_not_fill_the_size_is_refused() {
    let mut feed = Feed::new(64, 48, 1, Given::Rgba);
    let err = feed.push(&[0u8; 12], 0.0, 0.04).unwrap_err();
    assert!(err.to_string().contains("64x48"), "{err}");
}

/// Every frame of a film, at its own size, as the CLI's second read can give them.
struct All<'a> {
    frames: &'a [Vec<u8>],
    w: usize,
    h: usize,
    next: usize,
}

impl fearlezz_compare::Reread for All<'_> {
    type Error = fearlezz_compare::Unfit;

    fn next(
        &mut self,
    ) -> Result<Option<(usize, fearlezz_compare::Picture<'_>)>, fearlezz_compare::Unfit> {
        let Some(bytes) = self.frames.get(self.next) else {
            return Ok(None);
        };
        self.next += 1;
        let picture = fearlezz_compare::Picture::new(
            fearlezz_compare::Format::Yuv420,
            self.w,
            self.h,
            bytes,
        )?;
        Ok(Some((self.next - 1, picture)))
    }
}

/// A film larger than the size films are compared at keeps its frames scaled, and compares
/// exactly as the CLI does from frames at their own size.
#[test]
fn a_large_film_keeps_its_frames_scaled_and_compares_as_the_cli_does() {
    let (w, h) = (1400, 800);
    let film = |moved: usize| -> Vec<Vec<u8>> {
        (0..6)
            .map(|i| {
                let mut px: Vec<u8> = (0..w * h)
                    .map(|p| u8::try_from(p % w * 200 / w + 20).unwrap_or(u8::MAX))
                    .collect();
                let x = if i < 3 { 100 } else { moved };
                for y in 300..400 {
                    px[y * w + x..][..120].fill(16);
                }
                px.extend(vec![128u8; 2 * w.div_ceil(2) * h.div_ceil(2)]);
                px
            })
            .collect()
    };
    let (before, after) = (film(400), film(900));
    let feed = |frames: &[Vec<u8>]| {
        let mut feed = Feed::new(w, h, frames.len(), Given::Yuv420);
        for (i, f) in frames.iter().enumerate() {
            feed.push(f, i as f64 / 10.0, 0.1).unwrap();
        }
        feed
    };
    let page = compare(feed(&before), feed(&after)).unwrap();
    let cut = |frames: &[Vec<u8>]| {
        let mut cut = fearlezz_compare::Cut::new(w, h, frames.len());
        for (i, f) in frames.iter().enumerate() {
            let picture =
                fearlezz_compare::Picture::new(fearlezz_compare::Format::Yuv420, w, h, f).unwrap();
            cut.push(&picture, i as f64 / 10.0, 0.1).unwrap();
        }
        cut.finish()
    };
    let all = |frames| All {
        frames,
        w,
        h,
        next: 0,
    };
    let cli =
        fearlezz_compare::compare(&cut(&before), all(&before), &cut(&after), all(&after)).unwrap();
    assert_eq!(
        serde_json::to_string(&page).unwrap(),
        serde_json::to_string(&cli).unwrap()
    );
    assert!(!page.moments.is_empty(), "{page:?}");
}
