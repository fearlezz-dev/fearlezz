//! How a still or film compares with the one on the base commit.

use serde::{Deserialize, Serialize};

/// A box in full resolution pixels: x0, y0, x1, y1 with x1 and y1 exclusive.
pub type Box4 = [u32; 4];

/// What comparing two pictures found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The same pictures.
    Same,
    /// Same pictures, different timing.
    Timing,
    /// Other pictures.
    Changed,
}

/// A stretch of the films where the pictures differ.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Moment {
    /// Start and end in the before film, in seconds.
    pub before: (f64, f64),
    /// Start and end in the after film, in seconds.
    pub after: (f64, f64),
    /// Where they differ.
    #[serde(rename = "box")]
    pub area: Box4,
    /// How many pixels differ.
    pub px: usize,
}

/// One matched pair of states.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pair {
    /// The state's index in the before film.
    pub before: usize,
    /// The state's index in the after film.
    pub after: usize,
    /// Frame index of the state in the before film.
    pub before_frame: usize,
    /// Frame index of the state in the after film.
    pub after_frame: usize,
    /// Start time of the state in the before film, in seconds.
    pub before_at: f64,
    /// Start time of the state in the after film, in seconds.
    pub after_at: f64,
    /// How many pixels differ.
    pub px: usize,
    /// Where they differ.
    pub boxes: Vec<Box4>,
}

/// Everything comparing a still or film with its before found.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Comparison {
    /// What it comes to.
    pub verdict: Verdict,
    /// A still rather than a film.
    pub still: bool,
    /// Width and height before.
    pub size_before: (usize, usize),
    /// Width and height after.
    pub size_after: (usize, usize),
    /// Length of the before film, in seconds.
    pub total_before: f64,
    /// Length of the after film, in seconds.
    pub total_after: f64,
    /// The largest shift of a held state, in seconds, and where it is in the before film.
    pub max_shift: f64,
    /// Where the largest shift is in the before film, in seconds.
    pub max_shift_at: Option<f64>,
    /// How many stable states the before film holds.
    pub states_before: usize,
    /// How many stable states the after film holds.
    pub states_after: usize,
    /// Where the pictures differ.
    pub moments: Vec<Moment>,
    /// The states matched between the films.
    pub pairs: Vec<Pair>,
}
