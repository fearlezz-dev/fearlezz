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

use anyhow::{Context as _, Result, bail};
use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
use objc2_application_services::{AXError, AXIsProcessTrusted, AXUIElement, AXValue, AXValueType};
use objc2_core_foundation::{CFArray, CFRetained, CFString, CFType, CGPoint, CGRect, CGSize};

use super::WindowInfo;

/// The AeroSpace workspace parked windows float on; never shown unless the developer goes
/// there, and gone once its windows close.
const WORKSPACE: &str = "fearlezz";

/// How long a window manager gets to undo a move before the move counts.
const UNDO: Duration = Duration::from_millis(150);

/// How long AeroSpace gets to pick up a new window: it learns of windows on its own
/// schedule, after the window server lists them, and refuses ids it does not know yet.
const PICK_UP: Duration = Duration::from_secs(2);

/// Parks `window` and says how.
pub(crate) fn park(window: &WindowInfo) -> Result<String> {
    if aerospace(&["list-workspaces", "--focused"]).is_ok() {
        let id = window.id.to_string();
        let since = Instant::now();
        while !aerospace(&["list-windows", "--all", "--format", "%{window-id}"])?
            .lines()
            .any(|l| l.trim() == id)
        {
            if since.elapsed() >= PICK_UP {
                bail!("AeroSpace did not pick up the window in {PICK_UP:?}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        aerospace(&["layout", "floating", "--window-id", &id])?;
        aerospace(&["move-node-to-workspace", "--window-id", &id, WORKSPACE])?;
        return Ok(format!("by AeroSpace, on its workspace {WORKSPACE}"));
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
    Ok("by Accessibility".into())
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

/// How long one AeroSpace command may take before Fearlezz gives up on it.
const AEROSPACE_TIMEOUT: Duration = Duration::from_secs(5);

/// Runs the AeroSpace command line and returns what it printed; fails when it is not
/// installed, not running, or gives no answer in [`AEROSPACE_TIMEOUT`].
fn aerospace(args: &[&str]) -> Result<String> {
    let mut child = Command::new("aerospace")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("running aerospace")?;
    let since = Instant::now();
    while child.try_wait()?.is_none() {
        if since.elapsed() >= AEROSPACE_TIMEOUT {
            drop(child.kill());
            drop(child.wait());
            bail!(
                "aerospace {} gave no answer in {AEROSPACE_TIMEOUT:?}",
                args.join(" ")
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    // Its answers are a few lines, well within a pipe's buffer, so it never blocked on them.
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!(
            "aerospace {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
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

/// Brings the app of `pid` to the front, as the developer had it.
pub(crate) fn bring_front(pid: i32) -> Result<()> {
    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .with_context(|| format!("no running app has pid {pid}"))?;
    if !app.activateWithOptions(NSApplicationActivationOptions::empty()) {
        bail!("macOS did not bring pid {pid} to the front");
    }
    Ok(())
}
