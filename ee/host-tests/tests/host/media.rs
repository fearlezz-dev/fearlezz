//! Real pictures and films for the page to show, made with ffmpeg: a card with a button as a
//! still, and a film of such cards one after another.

use std::process::Command;

/// A box filled with a colour: x, y, width, height and an ffmpeg colour.
pub type Fill = (u32, u32, u32, u32, &'static str);

/// The ffmpeg source that draws `fills` on a `w` by `h` page, with `more` options of the page.
fn drawn(w: u32, h: u32, more: &str, fills: &[Fill]) -> String {
    let mut filter = format!("color=c=0xeef0f3:s={w}x{h}{more}");
    for (x, y, bw, bh, color) in fills {
        filter.push_str(&format!(
            ",drawbox=x={x}:y={y}:w={bw}:h={bh}:color={color}:t=fill"
        ));
    }
    filter
}

fn ffmpeg(args: &[&str]) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    let made = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-y"])
        .args(args)
        .arg(&out)
        .output()
        .expect("the browser tests make their pictures with ffmpeg");
    assert!(
        made.status.success(),
        "ffmpeg failed: {}",
        String::from_utf8_lossy(&made.stderr)
    );
    std::fs::read(out).unwrap()
}

/// A card on a page, its button `button` pixels wide, in `accent`.
pub fn card(button: u32, accent: &'static str) -> Vec<Fill> {
    vec![
        (40, 40, 560, 320, "white"),
        (72, 72, 56, 56, "0xc5ccd8"),
        (148, 80, 220, 16, "0x2a3240"),
        (148, 106, 140, 12, "0x8a93a3"),
        (72, 160, 496, 12, "0xd5dbe4"),
        (72, 184, 440, 12, "0xd5dbe4"),
        (72, 208, 470, 12, "0xd5dbe4"),
        (72, 280, button, 48, accent),
    ]
}

/// A 640 by 400 PNG of `fills`.
pub fn still(fills: &[Fill]) -> Vec<u8> {
    let filter = drawn(640, 400, "", fills);
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &filter,
        "-frames:v",
        "1",
        "-f",
        "image2",
        "-c:v",
        "png",
    ])
}

/// A 640 by 400 WebM of each scene for its seconds, one after another.
pub fn film(scenes: &[(f64, Vec<Fill>)]) -> Vec<u8> {
    let mut args: Vec<String> = Vec::new();
    for (seconds, fills) in scenes {
        args.extend(["-f".into(), "lavfi".into(), "-i".into()]);
        args.push(drawn(640, 400, &format!(":d={seconds}:r=25"), fills));
    }
    let inputs: String = (0..scenes.len()).map(|i| format!("[{i}:v]")).collect();
    args.extend([
        "-filter_complex".into(),
        format!(
            "{inputs}concat=n={}:v=1:a=0,format=yuv420p[v]",
            scenes.len()
        ),
        "-map".into(),
        "[v]".into(),
        "-c:v".into(),
        "libvpx".into(),
        "-b:v".into(),
        "400k".into(),
        "-deadline".into(),
        "realtime".into(),
        "-f".into(),
        "webm".into(),
    ]);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    ffmpeg(&args)
}
