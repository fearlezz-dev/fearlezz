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

/// A version's numbers, for comparing, three of them: a part that is no number counts as 0,
/// and a missing one too, so `0.6` is `0.6.0`.
pub(crate) fn numbers(text: &str) -> [u64; 3] {
    let mut out = [0; 3];
    for (slot, part) in out.iter_mut().zip(text.split(['.', '-', '+'])) {
        *slot = part.parse().unwrap_or(0);
    }
    out
}

/// Refuses a config whose `fearlezz` key names a newer version than this binary, before
/// anything else in it is read: a newer config may use fields and pieces this one does not
/// know, and the version is the cause to name.
pub(crate) fn meets(needs: &toml::Value) -> Result<()> {
    let Some(text) = needs.as_str().filter(|t| {
        let parts: Vec<&str> = t.split('.').collect();
        parts.len() <= 3
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    }) else {
        bail!(
            "fearlezz = {needs} is no version; name the lowest Fearlezz this config needs, like fearlezz = \"{OURS}\""
        );
    };
    if numbers(text) > numbers(OURS) {
        bail!(
            "this config needs Fearlezz {text} or newer (fearlezz = \"{text}\"), and this is {}; \
             install a newer one (curl -fsSL \
             https://raw.githubusercontent.com/fearlezz-dev/fearlezz/main/install.sh | sh, or the \
             way it was installed: brew upgrade, nix, devenv update), or when one is installed, \
             put it first on PATH (which -a fearlezz lists them)",
            this()
        );
    }
    Ok(())
}
