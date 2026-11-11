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
    assert!(area[0].as_u64().unwrap() >= 70, "only the second box changed: {c}");
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

#[test]
fn a_file_ffmpeg_cannot_read_says_so() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.png"), "not an image").unwrap();
    render(&dir.path().join("b.png"), 1.0, "");
    let out = fz_at(dir.path(), &["setup", "compare", "a.png", "b.png"]);
    assert!(!out.ok);
    assert!(out.stderr.contains("a.png"), "{}", out.stderr);
}
