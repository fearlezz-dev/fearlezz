//! Versions of Fearlezz as they compare, and how a CLI names its own to the proof host: the
//! CLI compares a config's pin and the host's newest release with its own, and the host
//! compares the caller's with the lowest version it takes.

use serde::{Deserialize, Serialize};

/// What every call of the CLI names itself as before its version (`fearlezz/0.5.4`).
pub const AGENT: &str = "fearlezz";

/// A version as it compares: three numbers, a missing one counting as 0 (`0.6` is `0.6.0`),
/// and a prerelease (`0.6.0-rc.1`) before its release. Build metadata (`+…`) counts for
/// nothing, and a part that is no number counts as 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    numbers: [u64; 3],
    released: bool,
}

impl Version {
    /// `text` as it compares; never fails, so a version no one can read counts as `0.0.0`.
    pub fn parse(text: &str) -> Version {
        let text = text.trim().trim_start_matches('v');
        let text = text.split('+').next().unwrap_or_default();
        let (release, pre) = match text.split_once('-') {
            Some((release, _)) => (release, true),
            None => (text, false),
        };
        let mut numbers = [0; 3];
        for (slot, part) in numbers.iter_mut().zip(release.split('.')) {
            *slot = part.parse().unwrap_or(0);
        }
        Version {
            numbers,
            released: !pre,
        }
    }

    /// The version a caller's `User-Agent` names (`fearlezz/0.5.4`). A caller that names none,
    /// as every CLI before 0.5.4 did, counts as `0.0.0`: older than any lowest version but
    /// `0.0.0`.
    pub fn of_agent(user_agent: &str) -> Version {
        Version::parse(named(user_agent).unwrap_or("0.0.0"))
    }
}

/// The version a caller's `User-Agent` names after `fearlezz/`, as it spells it.
pub fn named(user_agent: &str) -> Option<&str> {
    user_agent
        .split_whitespace()
        .find_map(|part| part.strip_prefix(AGENT)?.strip_prefix('/'))
        .filter(|named| !named.is_empty())
}

/// What `GET /api/v2/cli` answers anyone: the newest release of Fearlezz, when the host could
/// ask GitHub, and the lowest version whose runs it takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cli {
    /// The newest release, without its `v`; none when GitHub could not say.
    pub latest: Option<String>,
    /// The lowest version whose runs the host takes.
    pub minimum: String,
}
