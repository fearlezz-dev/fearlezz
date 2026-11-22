//! How a still or film compares with the one on the base commit.

use serde::{Deserialize, Serialize};

/// A box in full resolution pixels: x0, y0, x1, y1 with x1 and y1 exclusive.
pub type Box4 = [u32; 4];

/// What comparing two pictures found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "Likeness"))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
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
    /// The films' marks differ: one has a mark or span the other does not, or in another order.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub marks_changed: bool,
}

/// What a check said happened in a film, as it recorded it: marks at moments, and spans of time
/// with how their timing counts. Two films are timed from their marks rather than from their
/// start, so a run that loads slower or waits longer on an answer is not a retime.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Marks {
    /// Moments, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<Mark>,
    /// Spans of time, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<Span>,
}

/// A moment of a film a check named, such as `pressed y`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Mark {
    /// What happened, as the check wrote it.
    pub name: String,
    /// Seconds into the film.
    pub at: f64,
    /// Only for timing, not drawn on the film's timeline.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
}

/// A stretch of a film a check named, such as `loading`, and how its timing counts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Span {
    /// What happens in it, as the check wrote it.
    pub name: String,
    /// Seconds into the film it starts.
    pub start: f64,
    /// Seconds into the film it ends.
    pub end: f64,
    /// How its timing counts: the same by default.
    #[serde(default)]
    pub timing: Timing,
}

/// How a span's timing counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum Timing {
    /// Its states start as long after its start as before, as everywhere a span does not cover.
    #[default]
    Same,
    /// It takes as long as it takes, such as loading: its pictures are compared, its timing not.
    Free,
}

impl Marks {
    /// Whether a check said nothing about its film.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.marks.is_empty() && self.spans.is_empty()
    }

    /// The moments a film is timed from, in order: each mark, and each span's start and end.
    #[must_use]
    pub fn anchors(&self) -> Vec<(String, f64)> {
        let mut all: Vec<(String, f64)> =
            self.marks.iter().map(|m| (m.name.clone(), m.at)).collect();
        for s in &self.spans {
            all.push((format!("{} starts", s.name), s.start));
            all.push((format!("{} ends", s.name), s.end));
        }
        all.sort_by(|a, b| a.1.total_cmp(&b.1));
        all
    }

    /// Whether moment `t` is inside a span whose timing is free.
    #[must_use]
    pub fn free_at(&self, t: f64) -> bool {
        self.spans
            .iter()
            .any(|s| s.timing == Timing::Free && s.start <= t && t < s.end)
    }
}
