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
    /// How much longer the after film runs past what its end is timed from (its last anchor
    /// or free span) than the before film, in seconds. None for a still, and from a CLI that
    /// did not write it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail: Option<f64>,
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
    /// The films' one timeline, aligned at their marks: a section from each anchor to the next,
    /// in order. States are only matched within one section, and a page plays the films on it,
    /// so what plays side by side is what was compared. Empty for stills.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub timeline: Vec<Section>,
}

/// How late a film with the same pictures may come, in seconds, before a person looks at it:
/// within it, timing is noise from run to run (an animation or a load starting a little
/// later); past it, the film is slower in a way a person may care about.
pub const TIMING_BUDGET: f64 = 1.0;

impl Comparison {
    /// Whether its timing differs by no more than [`TIMING_BUDGET`]: in each held state, in
    /// how long it runs past its last anchor, and in how long it runs in all, since each
    /// section between marks may come a little late and add up. Unknown, so never within it,
    /// while its tail is not known.
    pub fn within_budget(&self) -> bool {
        let within = |s: f64| s.abs() <= TIMING_BUDGET;
        self.tail.is_some_and(|t| {
            within(t) && within(self.max_shift) && within(self.total_after - self.total_before)
        })
    }
}

/// One section of two films' timeline, between two anchors their checks marked (or a film's
/// start or end): where it starts on the timeline, how long it lasts, and what each film shows
/// in it. Its length is the longer film's stretch; the shorter one has no footage for the rest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Section {
    /// Where it starts on the timeline, in seconds.
    pub at: f64,
    /// How long it lasts, in seconds.
    pub length: f64,
    /// What the before film shows in it.
    pub before: Footage,
    /// What the after film shows in it.
    pub after: Footage,
}

/// The stretch of one film a section shows: from `from` to `to` in the film's own seconds,
/// starting `delay` seconds into the section. Before it starts the film holds its frame at
/// `from`, and once it ends its frame at `to`: it has no footage there.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Footage {
    /// Where it starts in the film, in seconds.
    pub from: f64,
    /// Where it ends in the film, in seconds.
    pub to: f64,
    /// How long into the section it starts, in seconds.
    pub delay: f64,
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
    /// It takes as long as it takes and looks different each time, such as loading or a fade:
    /// only the state it ends on is compared, its timing is not, and what comes after it is
    /// timed from its end. It is no anchor, so a film may have more or fewer of them.
    Free,
}

impl Marks {
    /// Whether a check said nothing about its film.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.marks.is_empty() && self.spans.is_empty()
    }

    /// The moments a film is timed from, in order: each mark, and each start and end of a span
    /// whose timing is the same. A free span is none, since how often one happens is free too.
    #[must_use]
    pub fn anchors(&self) -> Vec<(String, f64)> {
        let mut all: Vec<(String, f64)> =
            self.marks.iter().map(|m| (m.name.clone(), m.at)).collect();
        for s in self.spans.iter().filter(|s| s.timing == Timing::Same) {
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

    /// Whether the stretch from `start` to `end` lies within one free span, or starts at most
    /// `early` seconds before it, so its pictures are not compared: a state that runs on past a
    /// span's end is the state it ends on.
    #[must_use]
    pub fn free_over(&self, start: f64, end: f64, early: f64) -> bool {
        self.spans
            .iter()
            .any(|s| s.timing == Timing::Free && s.start <= start + early && end <= s.end + 1e-3)
    }

    /// Where the last free span that ended by moment `t` ended, which what follows is timed from.
    #[must_use]
    pub fn freed_by(&self, t: f64) -> Option<f64> {
        self.spans
            .iter()
            .filter(|s| s.timing == Timing::Free && s.end <= t)
            .map(|s| s.end)
            .max_by(f64::total_cmp)
    }
}
