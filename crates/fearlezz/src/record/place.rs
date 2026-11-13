//! Keeping a recorded window off the developer's screens: parked in a corner, where window
//! capture still sees all of it. macOS only lets an app place its own new windows, so the
//! window shows where it opened for a moment first.
//!
//! A window manager that owns window positions moves a parked window back, so it parks the
//! window itself when one answers: AeroSpace through its command line, floating on a
//! workspace it does not show. Otherwise Accessibility moves it, and a move undone by some
//! other manager is told as such. Nothing here needs a window manager.
#![expect(
    unsafe_code,
    reason = "Accessibility and AppKit are C and Objective C APIs; every call is unsafe in objc2"
)]

use std::ffi::c_void;
use std::process::{Command, Stdio};
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail};
use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
use objc2_application_services::{AXError, AXIsProcessTrusted, AXUIElement, AXValue, AXValueType};
use objc2_core_foundation::{CFArray, CFRetained, CFString, CFType, CGPoint, CGRect, CGSize};
use objc2_foundation::{
    NSData, NSDictionary, NSPropertyListReadOptions, NSPropertyListSerialization, NSString,
};

use super::{Parking, WindowInfo};

/// The AeroSpace workspace parked windows float on; never shown unless the developer goes
/// there, and gone once its windows close.
const WORKSPACE: &str = "fearlezz";

/// How long a window manager gets to undo a move before the move counts.
const UNDO: Duration = Duration::from_millis(150);

/// Parks `window` and says how, or why not yet: AeroSpace learns of windows on its own
/// schedule, after the window server lists them, and refuses ids it does not know yet.
pub(crate) fn park(window: &WindowInfo) -> Result<Parking> {
    if aerospace(&["list-workspaces", "--focused"]).is_ok() {
        let id = window.id.to_string();
        let pid = window.pid.to_string();
        if !aerospace(&[
            "list-windows",
            "--monitor",
            "all",
            "--pid",
            &pid,
            "--format",
            "%{window-id}",
        ])?
        .lines()
        .any(|l| l.trim() == id)
        {
            return Ok(Parking::NotYet(
                "AeroSpace did not pick up the window".into(),
            ));
        }
        aerospace(&["layout", "floating", "--window-id", &id])?;
        aerospace(&["move-node-to-workspace", "--window-id", &id, WORKSPACE])?;
        return Ok(Parking::Parked(format!(
            "by AeroSpace, on its workspace {WORKSPACE}"
        )));
    }
    trusted("move windows")?;
    let element = ax_window(window)?;
    let displays = displays();
    let to = corner(window, &displays)?;
    set_position(&element, to)?;
    std::thread::sleep(UNDO);
    let now = position(&element).context("reading where the window went")?;
    if (now.x - window.x).abs() < 1.0 && (now.y - window.y).abs() < 1.0 {
        bail!("a window manager moved it back; only AeroSpace is asked to park it");
    }
    let size = size(&element).unwrap_or(CGSize::new(window.width, window.height));
    let shown = visible(CGRect::new(now, size), &displays);
    if shown > MOST_SHOWN {
        bail!(
            "macOS left {:.0}% of it on screen at ({}, {})",
            shown * 100.0,
            now.x,
            now.y
        );
    }
    Ok(Parking::Parked("by Accessibility".into()))
}

/// The largest part of a parked window that may still show; macOS keeps a strip.
const MOST_SHOWN: f64 = 0.2;

/// Fails with the one time fix unless this process may use Accessibility to `what`.
fn trusted(what: &str) -> Result<()> {
    // SAFETY: reads this process's trust; never prompts.
    if unsafe { AXIsProcessTrusted() } {
        return Ok(());
    }
    let app = super::responsible_app().unwrap_or_else(|| "the app that started the agent".into());
    bail!(
        "{app} may not {what}; once: System Settings, Privacy & Security, Accessibility, \
         turn on {app}"
    )
}

/// The bounds of every active display, in global points.
fn displays() -> Vec<CGRect> {
    let mut ids = [0_u32; 32];
    let mut count = 0_u32;
    // SAFETY: the buffer holds as many ids as passed.
    let err = unsafe {
        objc2_core_graphics::CGGetActiveDisplayList(32, ids.as_mut_ptr(), &raw mut count)
    };
    if err != objc2_core_graphics::CGError::Success {
        return Vec::new();
    }
    ids.iter()
        .take(count as usize)
        .map(|id| objc2_core_graphics::CGDisplayBounds(*id))
        .collect()
}

/// The part of `rect` that shows on any of `displays`, from 0 to 1.
fn visible(rect: CGRect, displays: &[CGRect]) -> f64 {
    let area = rect.size.width * rect.size.height;
    if area <= 0.0 {
        return 0.0;
    }
    let shown: f64 = displays
        .iter()
        .map(|d| {
            let w = (rect.origin.x + rect.size.width).min(d.origin.x + d.size.width)
                - rect.origin.x.max(d.origin.x);
            let h = (rect.origin.y + rect.size.height).min(d.origin.y + d.size.height)
                - rect.origin.y.max(d.origin.y);
            w.max(0.0) * h.max(0.0)
        })
        .sum();
    shown / area
}

/// How long one command (AeroSpace, `defaults`) may take before Fearlezz gives up on it.
const TOOL_TIMEOUT: Duration = Duration::from_secs(5);

/// Runs the AeroSpace command line and returns what it printed; fails when it is not
/// installed, not running, or gives no answer in [`TOOL_TIMEOUT`].
fn aerospace(args: &[&str]) -> Result<String> {
    tool("aerospace", args)
}

/// Runs `program` and returns what it printed, or fails with what it said, or when it gives
/// no answer in [`TOOL_TIMEOUT`]. Its output is read while it runs, so a long answer never
/// blocks it.
pub(super) fn tool(program: &str, args: &[&str]) -> Result<String> {
    use std::io::Read as _;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("running {program}"))?;
    let mut out = child.stdout.take().context("no output pipe")?;
    let mut err = child.stderr.take().context("no error pipe")?;
    let stdout = std::thread::spawn(move || {
        let mut text = Vec::new();
        drop(out.read_to_end(&mut text));
        text
    });
    let stderr = std::thread::spawn(move || {
        let mut text = Vec::new();
        drop(err.read_to_end(&mut text));
        text
    });
    let since = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if since.elapsed() >= TOOL_TIMEOUT {
            drop(child.kill());
            drop(child.wait());
            bail!(
                "{program} {} gave no answer in {TOOL_TIMEOUT:?}",
                args.join(" ")
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    if !status.success() {
        bail!(
            "{program} {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&stdout).into_owned())
}

/// Where to move the window: past the bottom corner of a display that leaves the least of
/// it on any display, before macOS pulls a strip of it back on screen.
fn corner(window: &WindowInfo, displays: &[CGRect]) -> Result<CGPoint> {
    let size = CGSize::new(window.width, window.height);
    displays
        .iter()
        .flat_map(|d| {
            let bottom = d.origin.y + d.size.height;
            [
                CGPoint::new(d.origin.x + d.size.width, bottom),
                CGPoint::new(d.origin.x - window.width, bottom),
            ]
        })
        .map(|to| (to, visible(CGRect::new(to, size), displays)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .filter(|(_, shown)| *shown <= MOST_SHOWN)
        .map(|(to, _)| to)
        .context("every bottom corner of every display has another display beyond it")
}

/// The Accessibility element of `window`: the window of its app with the same frame.
fn ax_window(window: &WindowInfo) -> Result<CFRetained<AXUIElement>> {
    // SAFETY: Accessibility reads on an element for a live pid; out pointers are valid and
    // the value returned is owned, so CFRetained releases it.
    unsafe {
        let app = AXUIElement::new_application(window.pid);
        let mut value: *const CFType = std::ptr::null();
        let err = app.copy_attribute_value(
            &CFString::from_static_str("AXWindows"),
            NonNull::from(&mut value),
        );
        if err != AXError::Success || value.is_null() {
            bail!("the app's windows cannot be read (AXError {})", err.0);
        }
        let windows: CFRetained<CFArray> =
            CFRetained::from_raw(NonNull::new_unchecked(value.cast_mut().cast()));
        let windows: &CFArray<AXUIElement> = windows.cast_unchecked();
        // A window still opening has not reached its listed frame, so the title decides
        // first and the nearest frame breaks ties.
        let titled: Vec<CFRetained<AXUIElement>> = windows
            .iter()
            .filter(|w| title(w).as_deref() == Some(window.title.as_str()))
            .collect();
        let candidates = if titled.is_empty() {
            windows.iter().collect()
        } else {
            titled
        };
        candidates
            .into_iter()
            .min_by(|a, b| distance(a, window).total_cmp(&distance(b, window)))
            .context("Accessibility shows no window of the app")
    }
}

fn title(window: &AXUIElement) -> Option<String> {
    // SAFETY: out pointers are valid; the value returned is owned and released by CFRetained.
    unsafe {
        let mut value: *const CFType = std::ptr::null();
        let err = window.copy_attribute_value(
            &CFString::from_static_str("AXTitle"),
            NonNull::from(&mut value),
        );
        if err != AXError::Success || value.is_null() {
            return None;
        }
        let value: CFRetained<CFType> =
            CFRetained::from_raw(NonNull::new_unchecked(value.cast_mut()));
        value.downcast_ref::<CFString>().map(ToString::to_string)
    }
}

/// How far a window's frame is from the listed one, in points; unknown is farthest.
fn distance(element: &AXUIElement, window: &WindowInfo) -> f64 {
    match (position(element), size(element)) {
        (Some(at), Some(size)) => {
            (at.x - window.x).abs()
                + (at.y - window.y).abs()
                + (size.width - window.width).abs()
                + (size.height - window.height).abs()
        }
        _ => f64::MAX,
    }
}

fn set_position(window: &AXUIElement, to: CGPoint) -> Result<()> {
    let mut point = to;
    // SAFETY: the value reads a CGPoint from a valid pointer during the call.
    unsafe {
        let value = AXValue::new(
            AXValueType::CGPoint,
            NonNull::from(&mut point).cast::<c_void>(),
        )
        .context("making the position")?;
        let err = window.set_attribute_value(&CFString::from_static_str("AXPosition"), &value);
        if err != AXError::Success {
            bail!("the window did not move (AXError {})", err.0);
        }
    }
    Ok(())
}

fn position(window: &AXUIElement) -> Option<CGPoint> {
    let mut point = CGPoint::default();
    read(
        window,
        "AXPosition",
        AXValueType::CGPoint,
        NonNull::from(&mut point).cast(),
    )
    .then_some(point)
}

fn size(window: &AXUIElement) -> Option<CGSize> {
    let mut size = CGSize::default();
    read(
        window,
        "AXSize",
        AXValueType::CGSize,
        NonNull::from(&mut size).cast(),
    )
    .then_some(size)
}

/// Reads an AXValue attribute of `kind` into `into`, which points at a value of that kind.
fn read(window: &AXUIElement, attribute: &str, kind: AXValueType, into: NonNull<c_void>) -> bool {
    // SAFETY: out pointers are valid; the value returned is owned and released by CFRetained;
    // `into` points at a value of `kind`, as the callers pass.
    unsafe {
        let mut value: *const CFType = std::ptr::null();
        let err =
            window.copy_attribute_value(&CFString::from_str(attribute), NonNull::from(&mut value));
        if err != AXError::Success || value.is_null() {
            return false;
        }
        let value: CFRetained<AXValue> =
            CFRetained::from_raw(NonNull::new_unchecked(value.cast_mut().cast()));
        value.value(kind, into)
    }
}

/// The pid of the focused app: by Accessibility (AppKit's front app goes stale in a process
/// without a run loop), else by AeroSpace when it runs.
pub(crate) fn front() -> Result<i32> {
    if trusted("read which app is in front").is_err()
        && let Ok(out) = aerospace(&["list-windows", "--focused", "--format", "%{app-pid}"])
    {
        return out
            .trim()
            .parse()
            .with_context(|| format!("AeroSpace named no app in front: {out}"));
    }
    trusted("read which app is in front")?;
    // SAFETY: reads the system wide element's focused app; out pointers are valid and the
    // value returned is owned.
    unsafe {
        let system = AXUIElement::new_system_wide();
        let mut value: *const CFType = std::ptr::null();
        let err = system.copy_attribute_value(
            &CFString::from_static_str("AXFocusedApplication"),
            NonNull::from(&mut value),
        );
        if err != AXError::Success || value.is_null() {
            bail!("no app is in front (AXError {})", err.0);
        }
        let app: CFRetained<AXUIElement> =
            CFRetained::from_raw(NonNull::new_unchecked(value.cast_mut().cast()));
        let mut pid = 0;
        let err = app.pid(NonNull::from(&mut pid));
        if err != AXError::Success {
            bail!("the app in front has no pid (AXError {})", err.0);
        }
        Ok(pid)
    }
}

/// Whether [`front`] reads through Accessibility, in process, rather than asking AeroSpace.
pub(crate) fn reads_front_cheaply() -> bool {
    // SAFETY: reads this process's trust; never prompts.
    unsafe { AXIsProcessTrusted() }
}

/// Brings the app of `pid` to the front, as the developer had it.
pub(crate) fn bring_front(pid: i32) -> Result<()> {
    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .with_context(|| format!("no running app has pid {pid}"))?;
    if !app.activateWithOptions(NSApplicationActivationOptions::empty()) {
        bail!("macOS did not bring pid {pid} to the front");
    }
    Ok(())
}

/// The window positions an app had saved (its `NSWindow Frame` preferences) before its
/// window was parked: macOS saves a frame on every move, so without them the app's next
/// normal launch would open jammed in the parking corner. Read and written with `defaults`,
/// which reaches a sandboxed app's container and goes through the same preferences service
/// as the app.
pub(crate) struct Saved {
    bundle: String,
    /// Why they could not be read, when they could not: nothing is put back then, since
    /// an empty list would remove positions the app saved long before.
    frames: std::result::Result<Vec<(String, String)>, String>,
}

/// The window positions saved for `bundle`; none when it has no preferences yet (`defaults`
/// exports an empty dictionary then).
fn frames(bundle: &str) -> Result<Vec<(String, String)>> {
    let plist = tool("defaults", &["export", bundle, "-"])?;
    let data = NSData::with_bytes(plist.as_bytes());
    // SAFETY: reads a property list from bytes; no format is asked for.
    let parsed = unsafe {
        NSPropertyListSerialization::propertyListWithData_options_format_error(
            &data,
            NSPropertyListReadOptions::empty(),
            std::ptr::null_mut(),
        )
    };
    let dict = parsed
        .ok()
        .and_then(|p| p.downcast::<NSDictionary>().ok())
        .with_context(|| format!("defaults exported no dictionary for {bundle}"))?;
    Ok(dict
        .allKeys()
        .into_iter()
        .filter_map(|key| {
            let value = dict.objectForKey(&key)?.downcast::<NSString>().ok()?;
            let name = key.downcast::<NSString>().ok()?.to_string();
            name.starts_with("NSWindow Frame ")
                .then(|| (name, value.to_string()))
        })
        .collect())
}

/// What the app of `bundle` saved, read before its window is parked; none for an app
/// without a bundle id, which has no preferences of its own.
pub(crate) fn saved(bundle: &str) -> Option<Saved> {
    (!bundle.is_empty()).then(|| Saved {
        bundle: bundle.to_string(),
        frames: frames(bundle).map_err(|e| format!("{e:#}")),
    })
}

/// Puts back the positions that are parked now, once the app has saved its last frame;
/// returns how many it changed. See [`repairs`] for which and to what.
pub(crate) fn put_back(saved: &Saved) -> Result<usize> {
    let before = saved
        .frames
        .as_ref()
        .map_err(|e| anyhow!("they could not be read before parking: {e}"))?;
    let repairs = repairs(before, &frames(&saved.bundle)?);
    for (key, was) in &repairs {
        match was {
            Some(was) => tool("defaults", &["write", &saved.bundle, key, "-string", was])?,
            None => tool("defaults", &["delete", &saved.bundle, key])?,
        };
    }
    Ok(repairs.len())
}

/// What to do to each saved position that is parked `now` and changed during the run: write back its value from
/// `before`, or remove it (`None`) when it had none that was not parked. A position that is
/// not parked is left alone, whoever saved it: the developer moving the app's windows, or
/// another recording that put its own back first. A parked value in `before` (from another
/// recording still running, or one that was killed) is never written back.
fn repairs(before: &[(String, String)], now: &[(String, String)]) -> Vec<(String, Option<String>)> {
    now.iter()
        .filter(|(key, frame)| {
            looks_parked(frame) && !before.iter().any(|(k, was)| k == key && was == frame)
        })
        .map(|(key, _)| {
            let was = before
                .iter()
                .find(|(k, was)| k == key && !looks_parked(was))
                .map(|(_, was)| was.clone());
            (key.clone(), was)
        })
        .collect()
}

/// Whether a saved `NSWindow Frame` value ("x y w h" of the window, then of its screen)
/// puts the window mostly off its screen, as parking does.
fn looks_parked(frame: &str) -> bool {
    let n: Vec<f64> = frame
        .split_whitespace()
        .map_while(|n| n.parse().ok())
        .collect();
    let [x, y, w, h, sx, sy, sw, sh, ..] = n[..] else {
        return false;
    };
    let window = CGRect::new(CGPoint::new(x, y), CGSize::new(w, h));
    let screen = CGRect::new(CGPoint::new(sx, sy), CGSize::new(sw, sh));
    visible(window, &[screen]) < 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "19 5 1489 939 0 0 1512 948 ";
    const MOVED: &str = "300 200 1489 939 0 0 1512 948 ";
    const PARKED: &str = "1472 -907 1489 939 0 0 1512 948 ";
    const PARKED_ELSEWHERE: &str = "1511 -907 1474 939 0 0 1512 948 ";

    fn saved(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn a_frame_mostly_off_its_screen_looks_parked() {
        assert!(looks_parked(PARKED));
        assert!(!looks_parked(HOME));
        assert!(!looks_parked("not a frame"));
    }

    #[test]
    fn only_parked_positions_are_put_back_and_never_to_a_parked_value() {
        let before = saved(&[
            ("main", HOME),
            ("mine", HOME),
            ("other", PARKED),
            ("hidden", PARKED),
        ]);
        let now = saved(&[
            ("main", PARKED),
            ("mine", MOVED),
            ("other", PARKED_ELSEWHERE),
            ("hidden", PARKED),
            ("new", PARKED),
        ]);
        assert_eq!(
            repairs(&before, &now),
            [
                ("main".to_string(), Some(HOME.to_string())),
                ("other".to_string(), None),
                ("new".to_string(), None),
            ]
        );
    }
}
