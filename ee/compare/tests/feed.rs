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

#[test]
fn a_frame_that_does_not_fill_the_size_is_refused() {
    let mut feed = Feed::new(64, 48, 1, Given::Rgba);
    let err = feed.push(&[0u8; 12], 0.0, 0.04).unwrap_err();
    assert!(err.to_string().contains("64x48"), "{err}");
}
