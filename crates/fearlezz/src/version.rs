//! Which Fearlezz this is: its version and where it runs from, and the lowest version a config
//! needs (`fearlezz = "0.5.1"` at the top of `fearlezz.toml` or a piece). An older Fearlezz
//! first on PATH is a common reason a word or a config field is unknown, so errors name both.

use anyhow::{Result, bail};

/// This binary's version.
pub(crate) const OURS: &str = env!("CARGO_PKG_VERSION");

/// This binary as an error names it: `Fearlezz 0.5.1 at /usr/local/bin/fearlezz`.
pub(crate) fn this() -> String {
    let path = std::env::current_exe()
        .map_or_else(|_| "an unknown path".into(), |p| p.display().to_string());
    format!("Fearlezz {OURS} at {path}")
}

/// A version as it compares: three numbers, a missing one counting as 0 (`0.6` is `0.6.0`),
/// and a prerelease (`0.6.0-rc.1`) before its release. Build metadata (`+…`) counts for
/// nothing, and a part that is no number counts as 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Version {
    numbers: [u64; 3],
    released: bool,
}

impl Version {
    pub(crate) fn parse(text: &str) -> Version {
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
}

/// This binary's release, its version without a prerelease or build part: the pin `setup
/// init` writes, which [`meets`] accepts.
pub(crate) fn release() -> &'static str {
    OURS.split(['-', '+']).next().unwrap_or(OURS)
}

/// Refuses a config whose `fearlezz` key names a newer version than this binary, before
/// anything else in it is read: a newer config may use fields and pieces this one does not
/// know, and the version is the cause to name. `who` names what holds the key, such as
/// `this config` or `the piece "flux"`. A prerelease of the pinned version is older than it.
pub(crate) fn meets(needs: &toml::Value, who: &str) -> Result<()> {
    let Some(text) = needs.as_str().filter(|t| {
        let parts: Vec<&str> = t.split('.').collect();
        parts.len() <= 3
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    }) else {
        bail!(
            "{who}: fearlezz = {needs} is no version; name the lowest Fearlezz it needs, like \
             fearlezz = \"{}\"",
            release()
        );
    };
    if Version::parse(text) > Version::parse(OURS) {
        bail!(
            "{who} needs Fearlezz {text} or newer (fearlezz = \"{text}\"), and this is {}; \
             install a newer one (curl -fsSL \
             https://raw.githubusercontent.com/fearlezz-dev/fearlezz/main/install.sh | sh, or the \
             way it was installed: brew upgrade, nix, devenv update), or when one is installed, \
             put it first on PATH (which -a fearlezz lists them)",
            this()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fewer_parts_are_zeros() {
        assert_eq!(Version::parse("1.0"), Version::parse("1.0.0"));
        assert_eq!(Version::parse("1"), Version::parse("1.0.0"));
        assert!(Version::parse("1.0") < Version::parse("1.0.1"));
        assert!(Version::parse("0.10.0") > Version::parse("0.9.9"));
    }

    #[test]
    fn a_prerelease_comes_before_its_release() {
        assert!(Version::parse("0.6.0-rc.1") < Version::parse("0.6.0"));
        assert!(Version::parse("0.6.0-rc.1") > Version::parse("0.5.9"));
        assert_eq!(Version::parse("0.6.0+build"), Version::parse("0.6.0"));
        assert_eq!(Version::parse("v0.6.0"), Version::parse("0.6.0"));
    }
}
