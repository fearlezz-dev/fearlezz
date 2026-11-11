//! The recorder where there is no ScreenCaptureKit: every call says it cannot record here.

use std::path::Path;

use anyhow::{Result, bail};

use super::Frame;

const NOT_HERE: &str = "recording a window needs macOS";

pub(crate) struct Window {
    pub info: super::WindowInfo,
}

pub(crate) fn allowed() -> bool {
    false
}

pub(crate) fn windows() -> Result<Vec<Window>> {
    bail!(NOT_HERE)
}

pub(crate) fn still(_window: &Window, _path: &Path, _frame: &Frame) -> Result<(usize, usize)> {
    bail!(NOT_HERE)
}

pub(crate) struct Film;

pub(crate) fn start_film(_window: &Window, _path: &Path, _frame: &Frame) -> Result<Film> {
    bail!(NOT_HERE)
}

impl Film {
    pub(crate) fn ended(&mut self) -> bool {
        false
    }

    pub(crate) fn stop(self) -> Result<()> {
        bail!(NOT_HERE)
    }
}
