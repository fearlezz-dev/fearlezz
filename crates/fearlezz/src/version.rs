//! Which Fearlezz this is: its version and where it runs from, and the lowest version a config
//! needs (`fearlezz = "0.5.1"` at the top of `fearlezz.toml` or a piece). An older Fearlezz
//! first on PATH is a common reason a word or a config field is unknown, so errors name both.
//! And whether it is behind: the proof host says which release is newest and the lowest
//! version it takes, asked at most once a day.

use anyhow::{Result, bail};
use fearlezz_proof::version::Cli;
pub(crate) use fearlezz_proof::version::Version;

/// This binary's version.
pub(crate) const OURS: &str = env!("CARGO_PKG_VERSION");

/// This binary as an error names it: `Fearlezz 0.5.1 at /usr/local/bin/fearlezz`.
pub(crate) fn this() -> String {
    let path = std::env::current_exe()
        .map_or_else(|_| "an unknown path".into(), |p| p.display().to_string());
    format!("Fearlezz {OURS} at {path}")
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

/// How to update Fearlezz, after "update it" or "update Fearlezz": the agent knows how this
/// machine installed it, and the developer should know it moved.
pub(crate) const UPDATE: &str = "the way it was installed (devenv update, nix, brew upgrade or \
                                 the install script), and tell the developer";

/// How long the proof host's answer is kept before it is asked again.
const DAY: u64 = 24 * 60 * 60;

/// How this Fearlezz is behind what its proof host says.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Behind {
    /// A newer release is out.
    Newer(String),
    /// The host takes runs only from this version or newer, so it sends none there.
    TooOld(String),
}

impl std::fmt::Display for Behind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Behind::Newer(latest) => {
                write!(
                    f,
                    "Fearlezz {latest} is out and this is {OURS}; update it {UPDATE}"
                )
            }
            Behind::TooOld(minimum) => write!(
                f,
                "Fearlezz {OURS} is older than the proof host takes ({minimum} or newer), so it \
                 sends no runs there; update it {UPDATE}"
            ),
        }
    }
}

/// What the proof host said, kept in the user's cache folder: which host, when, and its answer
/// (none when it could not say, so a host that is down is not asked again that day either).
#[derive(serde::Serialize, serde::Deserialize)]
struct Kept {
    host: String,
    asked: u64,
    cli: Option<Cli>,
}

/// How this Fearlezz is behind the proof host of `config`: below the lowest version it takes,
/// or older than the newest release. The host is asked at most once a day, within a second or
/// two. Never on CI, where the action picks the version, never with the host off, where
/// nothing goes to it, and never with `FEARLEZZ_NO_UPDATE_CHECK` set.
pub(crate) fn behind(config: &crate::config::Config) -> Option<Behind> {
    let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
    if config.host_off || set("CI") || set("FEARLEZZ_NO_UPDATE_CHECK") {
        return None;
    }
    let cli = said(&crate::host_url(Some(config)))?;
    let ours = Version::parse(OURS);
    if plain(&cli.minimum) && ours < Version::parse(&cli.minimum) {
        return Some(Behind::TooOld(cli.minimum));
    }
    cli.latest
        .filter(|latest| plain(latest) && Version::parse(latest) > ours)
        .map(Behind::Newer)
}

/// Whether the host's `text` is a plain version (`0.5.4`, `1.0.0-rc.1`), the only words of a
/// host an agent is shown: a repository may name any host.
fn plain(text: &str) -> bool {
    let release = text.split(['-', '+']).next().unwrap_or_default();
    let parts: Vec<&str> = release.split('.').collect();
    text.len() <= 32
        && parts.len() <= 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'))
}

/// What the host at `host` says of the CLI: kept from less than a day ago, or asked now and
/// kept.
fn said(host: &str) -> Option<Cli> {
    let file = crate::files::UserDir::Cache
        .path()
        .ok()?
        .join("latest.json");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let kept = crate::files::read_if_exists(&file)
        .ok()
        .flatten()
        .and_then(|text| serde_json::from_str::<Kept>(&text).ok())
        // A clock that was set ahead and back makes a kept answer old, not fresh for longer.
        .filter(|kept| kept.host == host && now.abs_diff(kept.asked) < DAY);
    if let Some(kept) = kept {
        return kept.cli;
    }
    let cli = crate::host::cli(host).ok();
    let kept = Kept {
        host: host.to_owned(),
        asked: now,
        cli,
    };
    // Not kept only costs asking again next time.
    if let Ok(text) = serde_json::to_string(&kept) {
        drop(crate::files::write_atomic(&file, text));
    }
    kept.cli
}
