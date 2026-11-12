//! Comparing stills and films: same, only timing, or changed, on films rendered by ffmpeg.

// Clippy applies its test allowances (unwrap, expect, panic) only to cfg(test) code.
#![cfg(test)]

mod common;

use common::{box_at, fz_at, render};

fn compare(before: &std::path::Path, after: &std::path::Path) -> serde_json::Value {
    let dir = before.parent().unwrap();
    let out = fz_at(
        dir,
        &[
            "setup",
            "compare",
            before.to_str().unwrap(),
            after.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(out.ok, "{}", out.stderr);
    serde_json::from_str(&out.stdout).unwrap()
}

#[test]
fn the_same_film_rendered_twice_is_the_same() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.gif"), dir.path().join("b.gif"));
    render(&a, 2.0, &box_at(20, 1.0));
    render(&b, 2.0, &box_at(20, 1.0));
    assert_eq!(compare(&a, &b)["verdict"], "same");
}

#[test]
fn a_moved_box_is_a_change_at_the_right_moment_and_place() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.gif"), dir.path().join("b.gif"));
    render(&a, 2.0, &box_at(20, 1.0));
    render(&b, 2.0, &box_at(90, 1.0));
    let c = compare(&a, &b);
    assert_eq!(c["verdict"], "changed");
    let moments = c["moments"].as_array().unwrap();
    assert_eq!(moments.len(), 1, "{c}");
    let at = moments[0]["after"][0].as_f64().unwrap();
    assert!((at - 1.0).abs() < 0.11, "change starts at {at}");
    let area: Vec<u64> = moments[0]["box"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    // Both the old place (x 20 to 70) and the new place (x 90 to 140) changed.
    assert!(area[0] <= 20 && area[2] >= 140, "{area:?}");
    assert!(area[1] <= 30 && area[3] >= 70, "{area:?}");
}

#[test]
fn an_mp4_whose_frames_are_stored_out_of_order_has_its_change_at_the_right_moment() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.mp4"), dir.path().join("b.mp4"));
    let blue_from = |x: u32| format!("{},{}", box_at(20, 1.0), box_at(x, 2.0));
    render(&a, 3.0, &blue_from(90));
    render(&b, 3.0, &blue_from(110));
    let c = compare(&a, &b);
    assert_eq!(c["verdict"], "changed", "{c}");
    let moments = c["moments"].as_array().unwrap();
    assert_eq!(moments.len(), 1, "{c}");
    let at = moments[0]["after"][0].as_f64().unwrap();
    assert!((at - 2.0).abs() < 0.11, "change starts at {at}");
    let area = &moments[0]["box"];
    assert!(
        area[0].as_u64().unwrap() >= 70,
        "only the second box changed: {c}"
    );
}

#[test]
fn a_slower_film_with_the_same_pictures_only_changed_timing() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.gif"), dir.path().join("b.gif"));
    render(&a, 2.0, &box_at(20, 1.0));
    render(&b, 2.5, &box_at(20, 1.5));
    let c = compare(&a, &b);
    assert_eq!(c["verdict"], "timing", "{c}");
    let shift = c["max_shift"].as_f64().unwrap();
    assert!((shift - 0.5).abs() < 0.11, "shift {shift}");
}

#[test]
fn stills_are_same_or_changed_with_the_changed_area() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b, c) = (
        dir.path().join("a.png"),
        dir.path().join("b.png"),
        dir.path().join("c.png"),
    );
    render(&a, 1.0, &box_at(20, 0.0));
    render(&b, 1.0, &box_at(20, 0.0));
    render(&c, 1.0, &box_at(60, 0.0));
    assert_eq!(compare(&a, &b)["verdict"], "same");
    let changed = compare(&a, &c);
    assert_eq!(changed["verdict"], "changed");
    assert_eq!(changed["still"], true);
    let out = fz_at(dir.path(), &["setup", "compare", "a.png", "c.png"]);
    assert!(out.stdout.starts_with("changed: x "), "{}", out.stdout);
}

/// Runs ffmpeg with `args` and asserts it wrote `path`.
fn ffmpeg(path: &std::path::Path, args: &[&str]) -> std::path::PathBuf {
    let out = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-y"])
        .args(args)
        .arg(path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    path.to_path_buf()
}

/// Found by the web stills spike: a blue accent turned green on every page, and the look
/// said same, because pictures were compared in gray and the two colors are about 14 apart.
#[test]
fn a_change_of_color_alone_is_a_change() {
    let dir = tempfile::tempdir().unwrap();
    let solid = |name: &str, color: &str| {
        let source = format!("color=c={color}:s=200x100");
        ffmpeg(
            &dir.path().join(name),
            &["-f", "lavfi", "-i", &source, "-frames:v", "1"],
        )
    };
    let blue = solid("blue.png", "0x2563eb");
    let green = solid("green.png", "0x16a34a");
    assert_eq!(compare(&blue, &green)["verdict"], "changed");
    let red = solid("red.png", "red");
    let dark_green = solid("dark-green.png", "0x008a00");
    assert_eq!(compare(&red, &dark_green)["verdict"], "changed");
}

/// Turning every hue half way round keeps each pixel's brightness, so only color tells. Soft
/// colors stay in gamut when turned, so their brightness holds exactly.
#[test]
fn a_still_with_every_hue_turned_round_is_a_change() {
    let dir = tempfile::tempdir().unwrap();
    let still = |name: &str, filter: &str| {
        let args = [
            "-f",
            "lavfi",
            "-i",
            "testsrc2=s=320x240",
            "-frames:v",
            "1",
            "-vf",
            filter,
        ];
        ffmpeg(&dir.path().join(name), &args)
    };
    let a = still("a.png", "hue=s=0.5");
    let b = still("b.png", "hue=h=180:s=0.5");
    assert_eq!(compare(&a, &b)["verdict"], "changed");
}

/// A green button turned red: about 25 apart in gray, under the gray threshold of 28.
#[test]
fn a_button_turned_from_green_to_red_is_a_change_in_its_place() {
    let dir = tempfile::tempdir().unwrap();
    let button = |name: &str, color: &str| {
        let path = dir.path().join(name);
        render(
            &path,
            1.0,
            &format!("drawbox=x=40:y=40:w=60:h=24:color={color}:t=fill"),
        );
        path
    };
    let c = compare(&button("a.png", "0x22aa66"), &button("b.png", "0xcc3333"));
    assert_eq!(c["verdict"], "changed", "{c}");
    let area = &c["moments"][0]["box"];
    assert!(
        area[0].as_u64().unwrap() <= 40 && area[2].as_u64().unwrap() >= 100,
        "{c}"
    );
}

/// A header turned from blue to magenta in a compressed film, with a box that appears at 1 s.
#[test]
fn a_header_turned_from_blue_to_magenta_in_a_film_is_a_change() {
    let dir = tempfile::tempdir().unwrap();
    let film = |name: &str, color: &str| {
        let path = dir.path().join(name);
        let header = format!("drawbox=x=0:y=0:w=160:h=20:color={color}:t=fill");
        render(&path, 2.0, &format!("{header},{}", box_at(20, 1.0)));
        path
    };
    let c = compare(&film("a.mp4", "0x1f5fd1"), &film("b.mp4", "0xc8106f"));
    assert_eq!(c["verdict"], "changed", "{c}");
    let area = &c["moments"][0]["box"];
    assert!(
        area[1].as_u64().unwrap() == 0 && area[3].as_u64().unwrap() <= 40,
        "{c}"
    );
}

/// Codec noise is not a change: the same colorful film with sharp edges and digits, encoded
/// once nearly lossless and once at a low quality.
#[test]
fn the_same_colorful_film_at_two_qualities_is_not_a_change() {
    let dir = tempfile::tempdir().unwrap();
    let film = |name: &str, crf: &str| {
        let source = "testsrc2=s=320x240:r=10:d=2";
        let args = [
            "-f", "lavfi", "-i", source, "-crf", crf, "-pix_fmt", "yuv420p",
        ];
        ffmpeg(&dir.path().join(name), &args)
    };
    let c = compare(&film("a.mp4", "12"), &film("b.mp4", "30"));
    assert_ne!(c["verdict"], "changed", "{c}");
}

/// Found by the look report spike: a film whose after only adds a state at its end read as
/// the same.
#[test]
fn a_film_that_adds_a_state_at_its_end_is_a_change_there() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.gif"), dir.path().join("b.gif"));
    render(&a, 3.0, &box_at(20, 1.0));
    render(&b, 3.0, &format!("{},{}", box_at(20, 1.0), box_at(90, 2.0)));
    let c = compare(&a, &b);
    assert_eq!(c["verdict"], "changed", "{c}");
    let at = c["moments"][0]["after"][0].as_f64().unwrap();
    assert!((at - 2.0).abs() < 0.11, "change starts at {at}");
}

/// Found by the look report spike: a button that grows read as only timing.
#[test]
fn a_button_that_grows_is_a_change() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.gif"), dir.path().join("b.gif"));
    let button =
        |w: u32| format!("drawbox=x=0:y=40:w={w}:h=30:color=blue:t=fill:enable='gte(t,1)'");
    render(&a, 3.0, &button(100));
    render(&b, 3.0, &button(130));
    assert_eq!(compare(&a, &b)["verdict"], "changed");
}

/// A film larger than 720p is compared smaller, and still says when and where, in its own
/// pixels.
#[test]
fn a_large_film_has_its_change_at_the_right_moment_and_place() {
    let dir = tempfile::tempdir().unwrap();
    let film = |name: &str, x: u32| {
        let boxes = format!(
            "drawbox=x=200:y=200:w=300:h=200:color=blue:t=fill:enable='gte(t,1)',\
             drawbox=x={x}:y=600:w=300:h=200:color=red:t=fill:enable='gte(t,2.3)'"
        );
        let source = "color=c=white:s=1920x1080:r=10:d=3";
        let args = [
            "-f", "lavfi", "-i", source, "-vf", &boxes, "-pix_fmt", "yuv420p",
        ];
        ffmpeg(&dir.path().join(name), &args)
    };
    let c = compare(&film("a.mp4", 900), &film("b.mp4", 1300));
    assert_eq!(c["verdict"], "changed", "{c}");
    assert_eq!(c["size_after"], serde_json::json!([1920, 1080]));
    let moments = c["moments"].as_array().unwrap();
    assert_eq!(moments.len(), 1, "{c}");
    let at = moments[0]["after"][0].as_f64().unwrap();
    assert!((at - 2.3).abs() < 0.11, "change starts at {at}");
    let area: Vec<u64> = moments[0]["box"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    assert!(
        area[0] <= 900 && (1600..=1920).contains(&area[2]),
        "{area:?}"
    );
    assert!(
        area[1] <= 600 && (800..=1080).contains(&area[3]),
        "{area:?}"
    );
}

#[test]
fn a_file_ffmpeg_cannot_read_says_so() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.png"), "not an image").unwrap();
    render(&dir.path().join("b.png"), 1.0, "");
    let out = fz_at(dir.path(), &["setup", "compare", "a.png", "b.png"]);
    assert!(!out.ok);
    assert!(out.stderr.contains("a.png"), "{}", out.stderr);
}
