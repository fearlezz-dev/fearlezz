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
/// schedule, after ScreenCaptureKit lists them, and refuses ids it does not know yet.
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
    // SAFETY: reads this process's trust; never prompts.
    if !unsafe { AXIsProcessTrusted() } {
        let app =
            super::responsible_app().unwrap_or_else(|| "the app that started the agent".into());
        bail!(
            "{app} may not move windows; once: System Settings, Privacy & Security, \
             Accessibility, turn on {app}"
        );
    }
    let element = ax_window(window)?;
    let to = corner(window);
    set_position(&element, to)?;
    std::thread::sleep(UNDO);
    let now = position(&element).context("reading where the window went")?;
    if (now.x - window.x).abs() < 1.0 && (now.y - window.y).abs() < 1.0 {
        bail!("a window manager moved it back; only AeroSpace is asked to park it");
    }
    Ok("by Accessibility".into())
}

/// Runs the AeroSpace command line and returns what it printed; fails when it is not
/// installed or not running.
fn aerospace(args: &[&str]) -> Result<String> {
    let out = Command::new("aerospace")
        .args(args)
        .stdin(Stdio::null())
        .output()
        .context("running aerospace")?;
    if !out.status.success() {
        bail!(
            "aerospace {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The bottom right corner of the display the window is on, past which macOS keeps only a
/// strip of the window on screen.
fn corner(window: &WindowInfo) -> CGPoint {
    let centre = CGPoint::new(
        window.x + window.width / 2.0,
        window.y + window.height / 2.0,
    );
    let mut display = 0_u32;
    let mut count = 0_u32;
    // SAFETY: room for one display id, as passed.
    let err = unsafe {
        objc2_core_graphics::CGGetDisplaysWithPoint(centre, 1, &raw mut display, &raw mut count)
    };
    let bounds: CGRect = if err == objc2_core_graphics::CGError::Success && count == 1 {
        objc2_core_graphics::CGDisplayBounds(display)
    } else {
        objc2_core_graphics::CGDisplayBounds(objc2_core_graphics::CGMainDisplayID())
    };
    CGPoint::new(
        bounds.origin.x + bounds.size.width,
        bounds.origin.y + bounds.size.height,
    )
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
        windows
            .iter()
            .find(|w| {
                let (Some(at), Some(size)) = (position(w), size(w)) else {
                    return false;
                };
                (at.x - window.x).abs() < 2.0
                    && (at.y - window.y).abs() < 2.0
                    && (size.width - window.width).abs() < 2.0
                    && (size.height - window.height).abs() < 2.0
            })
            .context("Accessibility shows no window of the app with the recorded window's frame")
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

/// The pid of the focused app, by Accessibility (AppKit's front app goes stale in a process
/// without a run loop); none without Accessibility.
pub(crate) fn front() -> Option<i32> {
    // SAFETY: reads this process's trust, then the system wide element's focused app; out
    // pointers are valid and the value returned is owned.
    unsafe {
        if !AXIsProcessTrusted() {
            return None;
        }
        let system = AXUIElement::new_system_wide();
        let mut value: *const CFType = std::ptr::null();
        let err = system.copy_attribute_value(
            &CFString::from_static_str("AXFocusedApplication"),
            NonNull::from(&mut value),
        );
        if err != AXError::Success || value.is_null() {
            return None;
        }
        let app: CFRetained<AXUIElement> =
            CFRetained::from_raw(NonNull::new_unchecked(value.cast_mut().cast()));
        let mut pid = 0;
        (app.pid(NonNull::from(&mut pid)) == AXError::Success).then_some(pid)
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
