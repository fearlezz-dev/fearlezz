//! Pictures: the one YUV to RGB conversion, the area scaler and the size films are compared at.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

use fearlezz_compare::{Format, Picture, Unfit, fit};

/// A YUV 4:2:0 picture from its three planes.
fn yuv(y: &[u8], u: &[u8], v: &[u8]) -> Vec<u8> {
    [y, u, v].concat()
}

fn rgb_of(w: usize, h: usize, data: &[u8]) -> Vec<u8> {
    Picture::new(Format::Yuv420, w, h, data).unwrap().rgb().px
}

#[test]
fn black_white_and_gray_are_the_limited_range_ends_and_middle() {
    assert_eq!(rgb_of(1, 1, &[16, 128, 128]), [0, 0, 0]);
    assert_eq!(rgb_of(1, 1, &[235, 128, 128]), [255, 255, 255]);
    assert_eq!(rgb_of(1, 1, &[126, 128, 128]), [128, 128, 128]);
    // Below black and above white clamp.
    assert_eq!(rgb_of(1, 1, &[0, 128, 128]), [0, 0, 0]);
    assert_eq!(rgb_of(1, 1, &[255, 128, 128]), [255, 255, 255]);
}

/// BT.601 red, green and blue in limited range, against the matrix in floating point
/// (1.164383 (Y minus 16) plus 1.596027 (V minus 128) for red, and so on), rounded.
#[test]
fn colours_follow_the_bt601_matrix() {
    assert_eq!(rgb_of(1, 1, &[81, 90, 240]), [254, 0, 0]);
    assert_eq!(rgb_of(1, 1, &[145, 54, 34]), [0, 255, 1]);
    assert_eq!(rgb_of(1, 1, &[41, 240, 110]), [0, 0, 255]);
    assert_eq!(rgb_of(1, 1, &[100, 150, 160]), [149, 63, 142]);
}

/// Each chroma sample colours the 2 by 2 luma samples it covers; an odd side's last column
/// or row has a chroma sample of its own.
#[test]
fn a_chroma_sample_covers_two_by_two_pixels_and_an_odd_edge_has_its_own() {
    let data = yuv(&[81; 9], &[128, 128, 128, 90], &[128, 128, 128, 240]);
    let px = rgb_of(3, 3, &data);
    let at = |x: usize, y: usize| &px[(y * 3 + x) * 3..][..3];
    assert_eq!(at(2, 2), [254, 0, 0]);
    for (x, y) in [(0, 0), (1, 1), (2, 1), (1, 2)] {
        assert_eq!(at(x, y), [76, 76, 76], "({x}, {y})");
    }
}

#[test]
fn bytes_that_do_not_fill_the_size_are_refused() {
    let err = Picture::new(Format::Yuv420, 3, 3, &[0; 16]).unwrap_err();
    assert_eq!(
        err,
        Unfit::Bytes {
            format: Format::Yuv420,
            w: 3,
            h: 3,
            len: 16
        }
    );
    assert_eq!(
        err.to_string(),
        "a 3x3 yuv420p picture takes 17 bytes, not 16"
    );
    assert!(
        Picture::new(Format::Rgb, 0, 3, &[]).is_err(),
        "an empty size"
    );
}

#[test]
fn rgb_is_taken_as_it_is() {
    let px = [1, 2, 3, 250, 251, 252];
    assert_eq!(Picture::new(Format::Rgb, 2, 1, &px).unwrap().rgb().px, px);
}

/// Halving averages each 2 by 2 block per channel, rounding a half up.
#[test]
fn halving_averages_each_block_rounding_half_up() {
    let px = [0, 0, 0, 255, 255, 255, 10, 20, 1, 11, 21, 0];
    let half = Picture::new(Format::Rgb, 2, 2, &px).unwrap().area(1, 1);
    // (0 + 255 + 10 + 11) / 4 = 69, (0 + 255 + 20 + 21) / 4 = 74, (0 + 255 + 1 + 0) / 4 = 64.
    assert_eq!(half, [69, 74, 64]);
    let gray = |a: u8, b: u8, c: u8, d: u8| {
        let px = [a, a, a, b, b, b, c, c, c, d, d, d];
        Picture::new(Format::Rgb, 2, 2, &px).unwrap().area(1, 1)[0]
    };
    assert_eq!(gray(0, 0, 0, 1), 0, "a quarter rounds down");
    assert_eq!(gray(1, 1, 0, 0), 1, "a half rounds up");
}

/// Three samples to two: each new sample covers one and a half old ones, so it weighs the one
/// it covers whole twice the one it shares.
#[test]
fn a_fractional_scale_weighs_each_sample_by_how_much_it_covers() {
    let px = [0, 0, 0, 90, 90, 90, 180, 180, 180];
    let two = Picture::new(Format::Rgb, 3, 1, &px).unwrap().area(2, 1);
    assert_eq!(two, [30, 30, 30, 150, 150, 150]);
}

/// A YUV picture is scaled plane by plane, chroma to half the new size rounded up.
#[test]
fn yuv_planes_scale_each_to_its_own_size() {
    let data = yuv(&[0, 100, 20, 40, 200, 100, 60, 80], &[100, 200], &[10, 31]);
    let small = Picture::new(Format::Yuv420, 4, 2, &data)
        .unwrap()
        .area(2, 1);
    // Luma blocks: (0 + 100 + 200 + 100) / 4, (20 + 40 + 60 + 80) / 4; one chroma sample each.
    assert_eq!(small, [100, 50, 150, 21]);
    let odd = Picture::new(Format::Yuv420, 4, 2, &data)
        .unwrap()
        .area(3, 1);
    assert_eq!(odd.len(), Format::Yuv420.len(3, 1));
}

#[test]
fn films_fit_within_720p_by_their_long_and_short_side() {
    assert_eq!(fit(1920, 1080, (1280, 720)), (1280, 720));
    assert_eq!(fit(1080, 1920, (1280, 720)), (720, 1280));
    assert_eq!(fit(800, 600, (1280, 720)), (800, 600));
    assert_eq!(fit(2000, 100, (1280, 720)), (1280, 64));
}
