//! Notes on the proof host: what a person says should change on a picture, drawn as a region on
//! its before, its after or both, at a moment of a film, in their words. A note is the feedback
//! loop's unit: the agent reads it through `fearlezz --notes` with the pictures and the region
//! drawn, and it shows on every later run of the same work beside the new picture until a
//! person resolves it.
//!
//! A note is on a picture by check and name, so it is the same picture across runs, and was
//! made on one CI run, whose pictures it describes.

use serde::{Deserialize, Serialize};

use crate::events::Work;
use crate::review::Review;

/// Which picture of a pair a note marks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "NoteSide"))]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// The picture on the base commit.
    Before,
    /// The picture at the change's head.
    After,
    /// The same region on both.
    Both,
}

impl Side {
    /// `before`, `after` or `both sides`, as a person reads it.
    pub fn label(self) -> &'static str {
        match self {
            Side::Before => "before",
            Side::After => "after",
            Side::Both => "both sides",
        }
    }
}

/// A rectangle on a picture in fractions of its width and height, so it holds at any size the
/// picture shows at: `x` and `y` its top left corner, `w` and `h` its size, all in 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Region {
    /// Its left edge.
    pub x: f64,
    /// Its top edge.
    pub y: f64,
    /// Its width.
    pub w: f64,
    /// Its height.
    pub h: f64,
}

impl Region {
    /// Whether it lies on the picture and covers some of it.
    pub fn fits(&self) -> bool {
        let unit = |v: f64| v.is_finite() && (0.0..=1.0).contains(&v);
        unit(self.x)
            && unit(self.y)
            && unit(self.w)
            && unit(self.h)
            && self.w > 0.0
            && self.h > 0.0
            && self.x + self.w <= 1.0 + 1e-9
            && self.y + self.h <= 1.0 + 1e-9
    }
}

/// Whether a note still waits for the agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum NoteState {
    /// It still asks for a change.
    Open,
    /// A person resolved it.
    Resolved,
}

/// A person's note on a picture.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Note {
    /// Its id on the host.
    pub id: String,
    /// The check that took the picture.
    pub check: String,
    /// The picture's name in its look.
    pub picture: String,
    /// The CI run it was made on, whose pictures it describes.
    pub run: String,
    /// The commit that run proved.
    pub head: String,
    /// Which picture of the pair it marks.
    pub side: Side,
    /// Where on the picture.
    pub region: Region,
    /// For a film, the moment it marks, in seconds from its start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<f64>,
    /// What should change there.
    pub words: String,
    /// The GitHub login of who made it.
    pub by: String,
    /// When, in seconds since the epoch.
    pub at: f64,
    /// Whether it still asks for a change.
    pub state: NoteState,
    /// Who resolved it, once someone did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_by: Option<String>,
    /// The note it was made from, such as the same note on another variant of the screen: the
    /// first of them, so a group's notes share one link and an edit can reach them all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

impl Note {
    /// Whether it still asks for a change.
    pub fn is_open(&self) -> bool {
        self.state == NoteState::Open
    }
}

/// What a person sends to make a note on a picture of a run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ask {
    /// The check that took the picture.
    pub check: String,
    /// The picture's name in its look.
    pub picture: String,
    /// Which picture of the pair it marks.
    pub side: Side,
    /// Where on the picture.
    pub region: Region,
    /// For a film, the moment it marks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<f64>,
    /// What should change there.
    pub words: String,
    /// The note it is made from, to keep them linked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

/// What a person sends to change a note's words: on that note alone, or with `linked` on every
/// open note linked with it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(rename = "NoteEdit"))]
pub struct Edit {
    /// What should change there now.
    pub words: String,
    /// Whether the notes linked with it take the words too.
    #[serde(default)]
    pub linked: bool,
}

/// The longest words a note takes.
pub const WORDS_MAX: usize = 2000;

/// An open note with where its pictures are: the paths of the before and after of its picture
/// in the run it was made on, for the CLI to fetch and draw the region on.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Noted {
    /// The note.
    pub note: Note,
    /// `still` or `film`.
    pub kind: String,
    /// The picture on the base commit, by its path in the run's files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    /// The picture at the run's head, by its path in the run's files.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
}

/// What waits on the host for a work, as the CLI reads it with its token
/// (`GET /api/v2/work/<owner>/<repo>/<branch>`): the work, its page, the review of its head and
/// its open notes with their pictures.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Feedback {
    /// The work.
    pub work: Work,
    /// The work's page on the host.
    pub url: String,
    /// Where review of its head stands, once a run named a head.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<Review>,
    /// Its open notes, oldest first.
    #[serde(default)]
    pub notes: Vec<Noted>,
}
