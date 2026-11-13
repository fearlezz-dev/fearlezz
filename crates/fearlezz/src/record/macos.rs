//! ScreenCaptureKit: a still (PNG) or a film (MP4) of one window, captured on its own, so it
//! may be covered by other windows, offscreen or on a virtual display.
#![expect(
    unsafe_code,
    reason = "ScreenCaptureKit and AppKit are Objective C APIs; every call is unsafe in objc2"
)]

use std::collections::HashMap;
use std::path::Path;
use std::ptr::NonNull;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{AllocAnyThread as _, DefinedClass as _, MainThreadMarker, define_class, msg_send};
use objc2_app_kit::{NSApplication, NSBitmapImageFileType, NSBitmapImageRep, NSRunningApplication};
use objc2_core_foundation::{CFArray, CFDictionary, CFRetained, CFString, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGImage, CGPreflightScreenCaptureAccess, CGRectMakeWithDictionaryRepresentation,
    CGWindowListCopyWindowInfo, CGWindowListOption, kCGWindowBounds, kCGWindowLayer, kCGWindowName,
    kCGWindowNumber, kCGWindowOwnerName, kCGWindowOwnerPID,
};
use objc2_core_media::{CMTime, CMTimeFlags};
use objc2_foundation::{
    NSArray, NSDictionary, NSError, NSNumber, NSObject, NSObjectProtocol, NSString, NSURL,
};
use objc2_screen_capture_kit::{
    SCContentFilter, SCRecordingOutput, SCRecordingOutputConfiguration, SCRecordingOutputDelegate,
    SCScreenshotManager, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamDelegate,
    SCWindow,
};

use super::Frame;

/// How long a ScreenCaptureKit call may take before Fearlezz gives up on it.
const TIMEOUT: Duration = Duration::from_secs(15);

/// Carries an Objective C object from ScreenCaptureKit's queue to the waiting thread.
struct Sendable<T>(T);
// SAFETY: the objects sent (shareable content, images) are immutable once handed over, and
// only the receiving thread touches them afterwards.
unsafe impl<T> Send for Sendable<T> {}

/// A window ScreenCaptureKit can capture.
pub(crate) struct Window {
    inner: Retained<SCWindow>,
    pub info: super::WindowInfo,
}

/// Makes the window server connection a command line tool lacks until AppKit makes one;
/// ScreenCaptureKit asserts in `CGS_REQUIRE_INIT` without it.
fn connect() {
    if let Some(mtm) = MainThreadMarker::new() {
        drop(NSApplication::sharedApplication(mtm));
    } else {
        // Off the main thread, asking for a display connects as well.
        objc2_core_graphics::CGMainDisplayID();
    }
}

/// Whether this process may record the screen. Never prompts: it reads the permission of
/// the responsible process (the terminal or the agent's app).
pub(crate) fn allowed() -> bool {
    CGPreflightScreenCaptureAccess()
}

fn error_text(error: *mut NSError) -> String {
    // SAFETY: ScreenCaptureKit passes either null or a valid NSError.
    match unsafe { error.as_ref() } {
        Some(e) => e.localizedDescription().to_string(),
        None => "no reason given".into(),
    }
}

fn wait<T>(rx: &Receiver<Result<T, String>>, what: &str) -> Result<T> {
    rx.recv_timeout(TIMEOUT)
        .map_err(|e| anyhow!("{what}: no answer from ScreenCaptureKit in {TIMEOUT:?} ({e})"))?
        .map_err(|e| anyhow!("{what}: {e}"))
}

/// Every normal window of every app, off screen ones included, from the window server:
/// about 2 ms a listing, against about 70 ms for ScreenCaptureKit, so a new window is found
/// within a few frames of showing. Only windows of an app count, as ScreenCaptureKit lists
/// them. `apps` remembers each app's bundle id by pid.
pub(crate) fn windows(apps: &mut HashMap<i32, String>) -> Result<Vec<super::WindowInfo>> {
    connect();
    let list = CGWindowListCopyWindowInfo(
        CGWindowListOption::OptionAll | CGWindowListOption::ExcludeDesktopElements,
        0,
    )
    .context("the window server listed no windows")?;
    // SAFETY: CFArray and NSArray are toll free bridged, and the window server's list holds
    // dictionaries keyed by strings; the reference lives no longer than `list`.
    let list: &NSArray<NSDictionary<NSString, AnyObject>> =
        unsafe { &*std::ptr::from_ref::<CFArray>(&list).cast() };
    let mut out = Vec::new();
    for w in list {
        // SAFETY: the keys are the window server's own constant strings.
        let (number, layer, pid, owner, name, bounds) = unsafe {
            (
                kCGWindowNumber,
                kCGWindowLayer,
                kCGWindowOwnerPID,
                kCGWindowOwnerName,
                kCGWindowName,
                kCGWindowBounds,
            )
        };
        if self::number(&w, layer) != Some(0) {
            continue;
        }
        let (Some(id), Some(pid), Some(frame)) = (
            self::number(&w, number),
            self::number(&w, pid),
            rect(&w, bounds),
        ) else {
            continue;
        };
        let (Ok(id), Ok(pid)) = (u32::try_from(id), i32::try_from(pid)) else {
            continue;
        };
        // An app that just started may not be one yet, so only an app is remembered.
        let bundle = match apps.get(&pid) {
            Some(bundle) => bundle.clone(),
            None => {
                let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
                else {
                    continue;
                };
                let bundle = app
                    .bundleIdentifier()
                    .map(|b| b.to_string())
                    .unwrap_or_default();
                apps.insert(pid, bundle.clone());
                bundle
            }
        };
        out.push(super::WindowInfo {
            id,
            pid,
            bundle,
            app: text(&w, owner).unwrap_or_default(),
            title: text(&w, name).unwrap_or_default(),
            x: frame.origin.x,
            y: frame.origin.y,
            width: frame.size.width,
            height: frame.size.height,
        });
    }
    Ok(out)
}

/// The value of `key` in a window server dictionary, as a CF key bridged to an NS one.
fn value(w: &NSDictionary<NSString, AnyObject>, key: &CFString) -> Option<Retained<AnyObject>> {
    // SAFETY: CFString and NSString are toll free bridged.
    let key: &NSString = unsafe { &*std::ptr::from_ref(key).cast() };
    w.objectForKey(key)
}

fn number(w: &NSDictionary<NSString, AnyObject>, key: &CFString) -> Option<i64> {
    value(w, key)?
        .downcast::<NSNumber>()
        .ok()
        .map(|n| n.as_i64())
}

fn text(w: &NSDictionary<NSString, AnyObject>, key: &CFString) -> Option<String> {
    value(w, key)?
        .downcast::<NSString>()
        .ok()
        .map(|t| t.to_string())
}

fn rect(w: &NSDictionary<NSString, AnyObject>, key: &CFString) -> Option<CGRect> {
    let bounds = value(w, key)?;
    let mut rect = CGRect::default();
    // SAFETY: a window's bounds are a CFDictionary, toll free bridged with the NSDictionary
    // returned; it lives during the call.
    let read = unsafe {
        CGRectMakeWithDictionaryRepresentation(
            Some(&*Retained::as_ptr(&bounds).cast::<CFDictionary>()),
            &raw mut rect,
        )
    };
    read.then_some(rect)
}

/// The window `id` as ScreenCaptureKit can capture it, listed afresh.
pub(crate) fn capturable(id: u32) -> Result<Window> {
    connect();
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            // SAFETY: a non null content is a valid object that the block does not own.
            let answer = match unsafe { Retained::retain(content) } {
                Some(c) => Ok(Sendable(c)),
                None => Err(error_text(error)),
            };
            drop(tx.send(answer));
        },
    );
    // SAFETY: the block lives until the call returns and is copied by the callee.
    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            true, false, &block,
        );
    }
    let content = wait(&rx, "listing windows")?.0;
    // SAFETY: plain property reads on objects ScreenCaptureKit returned.
    unsafe {
        let w = content
            .windows()
            .into_iter()
            .find(|w| w.windowID() == id)
            .with_context(|| format!("window {id} is gone"))?;
        let frame = w.frame();
        let app = w.owningApplication();
        Ok(Window {
            info: super::WindowInfo {
                id,
                pid: app.as_ref().map_or(0, |a| a.processID()),
                bundle: app
                    .as_ref()
                    .map(|a| a.bundleIdentifier().to_string())
                    .unwrap_or_default(),
                app: app
                    .as_ref()
                    .map(|a| a.applicationName().to_string())
                    .unwrap_or_default(),
                title: w.title().map(|t| t.to_string()).unwrap_or_default(),
                x: frame.origin.x,
                y: frame.origin.y,
                width: frame.size.width,
                height: frame.size.height,
            },
            inner: w,
        })
    }
}

fn filter(window: &Window) -> Retained<SCContentFilter> {
    // SAFETY: the window came from the current shareable content.
    unsafe {
        SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &window.inner)
    }
}

/// The capture of `window` without `frame.crop_top` points at the top, at two pixels a
/// point wherever it is, so a window on a 1x or virtual display gives the same picture as on
/// a Retina display, and a look compares like with like.
fn configuration(window: &Window, frame: &Frame) -> Result<Retained<SCStreamConfiguration>> {
    let (width, height) = (window.info.width, window.info.height - frame.crop_top);
    if height < 1.0 {
        bail!(
            "crop_top {} leaves nothing of a window {} points high",
            frame.crop_top,
            window.info.height
        );
    }
    // SAFETY: plain property reads and writes on fresh objects.
    unsafe {
        let scale = 2.0;
        let c = SCStreamConfiguration::new();
        c.setSourceRect(CGRect::new(
            CGPoint::new(0.0, frame.crop_top),
            CGSize::new(width, height),
        ));
        c.setWidth((width * scale).round() as usize);
        c.setHeight((height * scale).round() as usize);
        c.setScalesToFit(true);
        c.setShowsCursor(false);
        c.setIgnoreShadowsSingleWindow(true);
        c.setMinimumFrameInterval(CMTime {
            value: 1,
            timescale: 30,
            flags: CMTimeFlags::Valid,
            epoch: 0,
        });
        Ok(c)
    }
}

/// Writes a still of `window` to `path` as PNG; returns its size in pixels.
pub(crate) fn still(window: &Window, path: &Path, frame: &Frame) -> Result<(usize, usize)> {
    connect();
    let filter = filter(window);
    let config = configuration(window, frame)?;
    let (tx, rx) = mpsc::channel();
    let block = RcBlock::new(move |image: *mut CGImage, error: *mut NSError| {
        let answer = match NonNull::new(image) {
            // SAFETY: a non null image is valid; retaining keeps it past the block.
            Some(image) => Ok(Sendable(unsafe { CFRetained::retain(image) })),
            None => Err(error_text(error)),
        };
        drop(tx.send(answer));
    });
    // SAFETY: filter, configuration and block outlive the call; the block is copied.
    unsafe {
        SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
            &filter,
            &config,
            Some(&block),
        );
    }
    let image = wait(&rx, "taking a still")?.0;
    let size = (CGImage::width(Some(&image)), CGImage::height(Some(&image)));
    let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &image);
    // SAFETY: an empty properties dictionary is valid for every file type.
    let data = unsafe {
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }
    .context("encoding the still as PNG")?;
    std::fs::write(path, data.to_vec()).with_context(|| format!("writing {}", path.display()))?;
    Ok(size)
}

/// What the recording's delegate reports.
enum Event {
    /// The film's file is written.
    Finished,
    /// The film failed.
    Failed(String),
    /// The stream stopped by itself, such as when the window closed. The file is still
    /// finished afterwards.
    Stopped(String),
}

struct Ivars {
    events: Mutex<Sender<Event>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "FearlezzRecording"]
    #[ivars = Ivars]
    struct Recording;

    unsafe impl NSObjectProtocol for Recording {}

    unsafe impl SCRecordingOutputDelegate for Recording {
        #[unsafe(method(recordingOutputDidFinishRecording:))]
        fn finished(&self, _output: &SCRecordingOutput) {
            self.report(Event::Finished);
        }

        #[unsafe(method(recordingOutput:didFailWithError:))]
        fn failed(&self, _output: &SCRecordingOutput, error: &NSError) {
            self.report(Event::Failed(error.localizedDescription().to_string()));
        }
    }

    unsafe impl SCStreamDelegate for Recording {
        #[unsafe(method(stream:didStopWithError:))]
        fn stopped(&self, _stream: &SCStream, error: &NSError) {
            self.report(Event::Stopped(error.localizedDescription().to_string()));
        }
    }
);

impl Recording {
    fn new(events: Sender<Event>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars {
            events: Mutex::new(events),
        });
        // SAFETY: NSObject's init on a freshly allocated object.
        unsafe { msg_send![super(this), init] }
    }

    fn report(&self, event: Event) {
        if let Ok(tx) = self.ivars().events.lock() {
            drop(tx.send(event));
        }
    }
}

/// A film being recorded.
pub(crate) struct Film {
    stream: Retained<SCStream>,
    /// The window filmed, held while the stream runs.
    _window: Retained<SCWindow>,
    _output: Retained<SCRecordingOutput>,
    _delegate: Retained<Recording>,
    events: Receiver<Event>,
    /// Why the stream stopped by itself, once it did.
    stopped: Option<String>,
    /// How the file ended, once it did.
    done: Option<Result<(), String>>,
}

/// Starts filming `window` into `path` (MP4, H.264).
pub(crate) fn start_film(window: &Window, path: &Path, frame: &Frame) -> Result<Film> {
    connect();
    drop(std::fs::remove_file(path));
    let filter = filter(window);
    let config = configuration(window, frame)?;
    let (tx, events) = mpsc::channel();
    let delegate = Recording::new(tx);
    let path = path
        .to_str()
        .with_context(|| format!("{} is not UTF-8", path.display()))?;
    // SAFETY: plain object setup; the delegate outlives the stream because Film holds both.
    unsafe {
        let rc = SCRecordingOutputConfiguration::new();
        rc.setOutputURL(&NSURL::fileURLWithPath(&NSString::from_str(path)));
        // AVFileTypeMPEG4 and AVVideoCodecTypeH264.
        rc.setOutputFileType(&NSString::from_str("public.mpeg-4"));
        rc.setVideoCodecType(&NSString::from_str("avc1"));
        let output = SCRecordingOutput::initWithConfiguration_delegate(
            SCRecordingOutput::alloc(),
            &rc,
            ProtocolObject::from_ref(&*delegate),
        );
        let stream = SCStream::initWithFilter_configuration_delegate(
            SCStream::alloc(),
            &filter,
            &config,
            Some(ProtocolObject::from_ref(&*delegate)),
        );
        stream
            .addRecordingOutput_error(&output)
            .map_err(|e| anyhow!("adding the film output: {}", e.localizedDescription()))?;
        let (started_tx, started) = mpsc::channel();
        let block = RcBlock::new(move |error: *mut NSError| {
            let answer = if error.is_null() {
                Ok(())
            } else {
                Err(error_text(error))
            };
            drop(started_tx.send(answer));
        });
        stream.startCaptureWithCompletionHandler(Some(&block));
        wait(&started, "starting the film")?;
        Ok(Film {
            stream,
            _window: window.inner.clone(),
            _output: output,
            _delegate: delegate,
            events,
            stopped: None,
            done: None,
        })
    }
}

impl Film {
    fn take(&mut self, event: Event) {
        match event {
            Event::Finished => self.done = Some(Ok(())),
            Event::Failed(e) => self.done = Some(Err(e)),
            Event::Stopped(e) => self.stopped = Some(e),
        }
    }
}

impl super::Filming for Film {
    fn ended(&mut self) -> bool {
        while let Ok(event) = self.events.try_recv() {
            self.take(event);
        }
        self.stopped.is_some() || self.done.is_some()
    }

    fn stop(mut self) -> Result<()> {
        if self.stopped.is_none() && self.done.is_none() {
            let (tx, rx) = mpsc::channel();
            let block = RcBlock::new(move |error: *mut NSError| {
                tx.send(error.is_null()).unwrap_or_default();
            });
            // SAFETY: the stream is running; the block is copied by the callee.
            unsafe { self.stream.stopCaptureWithCompletionHandler(Some(&block)) };
            rx.recv_timeout(TIMEOUT).unwrap_or_default();
        }
        let until = std::time::Instant::now() + TIMEOUT;
        while self.done.is_none() {
            let left = until.saturating_duration_since(std::time::Instant::now());
            match self.events.recv_timeout(left) {
                Ok(event) => self.take(event),
                Err(_) => break,
            }
        }
        match self.done {
            Some(Ok(())) => Ok(()),
            Some(Err(e)) => bail!("finishing the film: {e}"),
            None => match self.stopped {
                Some(why) => bail!("the film stopped ({why}) and its file was not finished"),
                None => bail!("finishing the film: no answer from ScreenCaptureKit in {TIMEOUT:?}"),
            },
        }
    }
}

/// The pid of `replayd`, the macOS service behind every capture: the same pid across
/// failing films means it is stuck, a new one that it restarted.
pub(crate) fn capture_service() -> Option<u32> {
    super::place::tool("pgrep", &["-x", "replayd"])
        .ok()?
        .lines()
        .next()?
        .trim()
        .parse()
        .ok()
}

/// The machine's load over the last minute.
pub(crate) fn load() -> Option<f64> {
    // Prints "{ 1.23 4.56 7.89 }".
    super::place::tool("sysctl", &["-n", "vm.loadavg"])
        .ok()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}
