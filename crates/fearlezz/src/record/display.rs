//! A virtual display for recording, so an app's window stays off the developer's screens.
//! Experimental: CGVirtualDisplay is a private CoreGraphics class (the one DeskPad and
//! BetterDisplay use), reached through the Objective C runtime and checked before each use,
//! so a macOS without it fails with a message instead of a crash. Moving another app's
//! window onto it needs Accessibility. The display lives as long as this value, and the
//! window server removes it when the process ends, even when it is killed.
#![expect(
    unsafe_code,
    reason = "CGVirtualDisplay and Accessibility are C and Objective C APIs"
)]

use std::ffi::c_void;
use std::ptr::NonNull;

use anyhow::{Context as _, Result, bail};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Sel};
use objc2::{msg_send, sel};
use objc2_application_services::{AXError, AXIsProcessTrusted, AXUIElement, AXValue, AXValueType};
use objc2_core_foundation::{CFArray, CFRetained, CFString, CFType, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBeginDisplayConfiguration, CGCancelDisplayConfiguration, CGCompleteDisplayConfiguration,
    CGConfigureDisplayOrigin, CGConfigureOption, CGError, CGGetActiveDisplayList,
};
use objc2_foundation::{NSArray, NSString};

/// Pixels of the display's one mode; at 2x it is 1280 by 800 points.
const WIDTH: u32 = 2560;
const HEIGHT: u32 = 1600;

pub(crate) struct Virtual {
    _display: Retained<AnyObject>,
    pub id: u32,
}

fn class(name: &std::ffi::CStr) -> Result<&'static AnyClass> {
    AnyClass::get(name).with_context(|| {
        format!(
            "this macOS has no {} (a private class), so there is no virtual display",
            name.to_string_lossy()
        )
    })
}

/// Checks that `class` has the instance method `sel` with these argument type encodings
/// (after self and _cmd), so a private class that changed fails here instead of in the call.
fn signature(class: &AnyClass, sel: Sel, args: &[&str]) -> Result<()> {
    let method = class
        .instance_method(sel)
        .with_context(|| format!("CGVirtualDisplay changed: {class} has no {sel}"))?;
    for (i, want) in args.iter().enumerate() {
        let got = method.argument_type(i + 2);
        let got = got.as_deref().map(|c| c.to_string_lossy().into_owned());
        if got.as_deref() != Some(*want) {
            bail!(
                "CGVirtualDisplay changed: {class} {sel} argument {i} is {got:?}, not \"{want}\""
            );
        }
    }
    Ok(())
}

impl Virtual {
    pub(crate) fn create() -> Result<Virtual> {
        let descriptor_class = class(c"CGVirtualDisplayDescriptor")?;
        let display_class = class(c"CGVirtualDisplay")?;
        let settings_class = class(c"CGVirtualDisplaySettings")?;
        let mode_class = class(c"CGVirtualDisplayMode")?;
        for (sel, args) in [
            (sel!(setName:), &["@"][..]),
            (sel!(setMaxPixelsWide:), &["I"]),
            (sel!(setMaxPixelsHigh:), &["I"]),
            (sel!(setVendorID:), &["I"]),
            (sel!(setProductID:), &["I"]),
            (sel!(setSerialNum:), &["I"]),
            (sel!(setQueue:), &["@"]),
        ] {
            signature(descriptor_class, sel, args)?;
        }
        signature(descriptor_class, sel!(setSizeInMillimeters:), &[])?;
        signature(display_class, sel!(initWithDescriptor:), &["@"])?;
        signature(display_class, sel!(applySettings:), &["@"])?;
        signature(display_class, sel!(displayID), &[])?;
        signature(settings_class, sel!(setHiDPI:), &["I"])?;
        signature(settings_class, sel!(setModes:), &["@"])?;
        signature(
            mode_class,
            sel!(initWithWidth:height:refreshRate:),
            &["I", "I", "d"],
        )?;
        // SAFETY: every selector and its argument types were checked above against the
        // runtime, and the values sent have exactly those types.
        unsafe {
            let descriptor: Retained<AnyObject> = msg_send![descriptor_class, new];
            let name = NSString::from_str("Fearlezz recording");
            let _: () = msg_send![&descriptor, setName: &*name];
            let _: () = msg_send![&descriptor, setMaxPixelsWide: WIDTH];
            let _: () = msg_send![&descriptor, setMaxPixelsHigh: HEIGHT];
            let _: () = msg_send![&descriptor, setSizeInMillimeters: CGSize::new(340.0, 212.0)];
            // One fixed identity: macOS remembers settings and writes a color profile per
            // identity, so a new one each run would leave a trail.
            let _: () = msg_send![&descriptor, setVendorID: 0xfe42_u32];
            let _: () = msg_send![&descriptor, setProductID: 0x0001_u32];
            let _: () = msg_send![&descriptor, setSerialNum: 0x0001_u32];
            let queue = dispatch2::DispatchQueue::new("fearlezz.display", None);
            let _: () = msg_send![&descriptor, setQueue: &*queue];

            let display: Option<Retained<AnyObject>> =
                msg_send![msg_send![display_class, alloc], initWithDescriptor: &*descriptor];
            let display = display.context("the window server refused the virtual display")?;
            let id: u32 = msg_send![&display, displayID];
            if id == 0 {
                bail!("the virtual display has no id");
            }

            let settings: Retained<AnyObject> = msg_send![settings_class, new];
            let _: () = msg_send![&settings, setHiDPI: 1_u32];
            let mode: Retained<AnyObject> = msg_send![
                msg_send![mode_class, alloc],
                initWithWidth: WIDTH,
                height: HEIGHT,
                refreshRate: 60.0_f64
            ];
            let modes = NSArray::from_retained_slice(&[mode]);
            let _: () = msg_send![&settings, setModes: &*modes];
            let applied: bool = msg_send![&display, applySettings: &*settings];
            if !applied {
                bail!("the virtual display did not take its mode");
            }
            let created = Virtual {
                _display: display,
                id,
            };
            // The display shows up asynchronously.
            for _ in 0..50 {
                if created.bounds().size.width > 0.0 {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            created.park();
            Ok(created)
        }
    }

    /// Moves the display to the far corner of the others, touching them only at that corner,
    /// so the developer's cursor rarely wanders onto it. For this process only: the window
    /// server forgets it when the process ends.
    fn park(&self) {
        let mut ids = [0_u32; 16];
        let mut count = 0_u32;
        // SAFETY: the buffer holds as many ids as passed.
        let err = unsafe { CGGetActiveDisplayList(16, ids.as_mut_ptr(), &raw mut count) };
        if err != CGError::Success {
            return;
        }
        let (mut right, mut bottom) = (0.0_f64, 0.0_f64);
        for id in ids.iter().take(count as usize).filter(|id| **id != self.id) {
            let b = objc2_core_graphics::CGDisplayBounds(*id);
            right = right.max(b.origin.x + b.size.width);
            bottom = bottom.max(b.origin.y + b.size.height);
        }
        let mut config = std::ptr::null_mut();
        // SAFETY: a configuration begun here is completed or cancelled below.
        unsafe {
            if CGBeginDisplayConfiguration(&raw mut config) != CGError::Success {
                return;
            }
            CGConfigureDisplayOrigin(config, self.id, right as i32, bottom as i32);
            if CGCompleteDisplayConfiguration(config, CGConfigureOption::ForAppOnly)
                != CGError::Success
            {
                CGCancelDisplayConfiguration(config);
            }
        }
    }

    /// Where the display sits in global coordinates, in points.
    pub(crate) fn bounds(&self) -> CGRect {
        objc2_core_graphics::CGDisplayBounds(self.id)
    }

    /// Moves the largest window of `pid` (the one recorded, `width` by `height` points) to
    /// the display's top left corner. Needs Accessibility.
    pub(crate) fn take(&self, pid: i32, width: f64, height: f64) -> Result<()> {
        // SAFETY: AXIsProcessTrusted reads this process's trust; it never prompts.
        if !unsafe { AXIsProcessTrusted() } {
            bail!(
                "moving another app's window needs Accessibility for the app that started the \
                 agent (System Settings, Privacy & Security, Accessibility)"
            );
        }
        let to = self.bounds().origin;
        // SAFETY: Accessibility calls on an element for a live pid; out pointers are valid.
        unsafe {
            let app = AXUIElement::new_application(pid);
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
            let mut best = None;
            let mut best_diff = f64::MAX;
            for w in windows {
                let size = size_of(&w).unwrap_or_default();
                let diff = (size.width - width).abs() + (size.height - height).abs();
                if diff < best_diff {
                    best_diff = diff;
                    best = Some(w);
                }
            }
            let window = best.context("the app has no window Accessibility can see")?;
            let mut point = CGPoint::new(to.x + 20.0, to.y + 20.0);
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
}

/// Clicks at `at` (global points) in the app `pid` without moving the developer's cursor:
/// the events go to that process only. Prototype, not used by checks. Measured on macOS 27:
/// the cursor stays put, but a Flutter app in the background on a virtual display ignored
/// the clicks.
pub(crate) fn click(pid: i32, at: CGPoint) -> Result<()> {
    use objc2_core_graphics::{CGEvent, CGEventType, CGMouseButton};
    for kind in [CGEventType::LeftMouseDown, CGEventType::LeftMouseUp] {
        let event = CGEvent::new_mouse_event(None, kind, at, CGMouseButton::Left)
            .context("making a mouse event")?;
        CGEvent::post_to_pid(pid, Some(&event));
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    Ok(())
}

/// Where the developer's cursor is, in global points.
pub(crate) fn cursor() -> CGPoint {
    let event = objc2_core_graphics::CGEvent::new(None);
    objc2_core_graphics::CGEvent::location(event.as_deref())
}

/// A window's size in points, by Accessibility.
fn size_of(window: &AXUIElement) -> Option<CGSize> {
    // SAFETY: out pointers are valid; the value is released by CFRetained.
    unsafe {
        let mut value: *const CFType = std::ptr::null();
        let err = window.copy_attribute_value(
            &CFString::from_static_str("AXSize"),
            NonNull::from(&mut value),
        );
        if err != AXError::Success || value.is_null() {
            return None;
        }
        let value: CFRetained<AXValue> =
            CFRetained::from_raw(NonNull::new_unchecked(value.cast_mut().cast()));
        let mut size = CGSize::default();
        value
            .value(AXValueType::CGSize, NonNull::from(&mut size).cast())
            .then_some(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Frames = Vec<(u32, String, i64, i64)>;

    fn frames() -> Frames {
        super::super::sys::windows()
            .unwrap()
            .into_iter()
            .map(|w| {
                (
                    w.info.id,
                    w.info.app.clone(),
                    w.info.x as i64,
                    w.info.y as i64,
                )
            })
            .collect()
    }

    fn moved(a: &Frames, b: &Frames) -> Vec<String> {
        b.iter()
            .filter(|w| a.iter().any(|v| v.0 == w.0 && v != *w))
            .map(|w| {
                let v = a.iter().find(|v| v.0 == w.0).unwrap();
                format!("{} ({},{})->({},{})", w.1, v.2, v.3, w.2, w.3)
            })
            .collect()
    }

    /// Moves a running Flutter counter app (`look_app`, from `flutter create`) onto a virtual
    /// display, taps its button three times without the cursor, and writes a still of it to
    /// the temp folder: `cargo test clicks -- --ignored --nocapture` with the app open.
    #[test]
    #[ignore = "needs a running counter app named look_app"]
    fn clicks_reach_an_app_on_a_virtual_display_without_the_cursor() {
        use super::super::{Frame, sys};
        let find = || {
            sys::windows()
                .unwrap()
                .into_iter()
                .filter(|w| w.info.app == "look_app" && w.info.height > 50.0)
                .max_by(|a, b| a.info.width.total_cmp(&b.info.width))
                .expect("a look_app window")
        };
        let w = find();
        let d = Virtual::create().unwrap();
        d.take(w.info.pid, w.info.width, w.info.height).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(500));
        let w = find();
        println!(
            "window at ({}, {}) {}x{}",
            w.info.x, w.info.y, w.info.width, w.info.height
        );
        let before = cursor();
        // The floating button sits 16 points from the bottom right corner, 56 points wide.
        let fab = CGPoint::new(
            w.info.x + w.info.width - 44.0,
            w.info.y + w.info.height - 44.0,
        );
        for _ in 0..3 {
            click(w.info.pid, fab).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        let after = cursor();
        println!("cursor before {before:?}, after {after:?}");
        let path = std::env::temp_dir().join("fearlezz-click.png");
        sys::still(&w, &path, &Frame { crop_top: 0.0 }).unwrap();
        println!("still: {}", path.display());
    }

    /// Measures what a virtual display does to the developer's windows. Needs a desktop
    /// session with Screen Recording: `cargo test display -- --ignored --nocapture`.
    #[test]
    #[ignore = "creates a display on the developer's desktop"]
    fn a_virtual_display_and_the_developers_windows() {
        let quiet = frames();
        std::thread::sleep(std::time::Duration::from_secs(1));
        let still_quiet = frames();
        println!("moved with no display: {:?}", moved(&quiet, &still_quiet));
        let clock = std::time::Instant::now();
        let d = Virtual::create().unwrap();
        println!(
            "created {} at {:?} in {:?}",
            d.id,
            d.bounds(),
            clock.elapsed()
        );
        std::thread::sleep(std::time::Duration::from_secs(1));
        let with = frames();
        println!("moved by the display: {:?}", moved(&still_quiet, &with));
        drop(d);
        std::thread::sleep(std::time::Duration::from_secs(2));
        let after = frames();
        println!("moved after it went: {:?}", moved(&with, &after));
        println!(
            "not back where they were: {:?}",
            moved(&still_quiet, &after)
        );
    }
}
