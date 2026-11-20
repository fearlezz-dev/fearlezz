//! Which ffmpeg and ffprobe Fearlezz runs itself, to compare, record and draw pictures: found
//! once per run, in the project's `[env]` shell first (where a devenv or Nix project keeps
//! its tools), then on PATH, then on GitHub Actions the pinned Linux build. Every call runs
//! the found program by its absolute path, so the project's ffmpeg is used even though
//! Fearlezz itself runs outside that shell.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use anyhow::{Result, anyhow};

/// The project whose shell is asked first: its root and its `[env]` shell.
static PROJECT: OnceLock<(PathBuf, Option<String>)> = OnceLock::new();
/// What the one search found, or why it found nothing.
static FOUND: OnceLock<Result<Found, String>> = OnceLock::new();

/// The programs found and where they came from.
struct Found {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    from: String,
}

/// Names the project whose `[env]` shell is asked for ffmpeg, once the repository is read.
/// Without it only PATH is searched, as `setup compare` outside a repository does.
pub(crate) fn within(root: &Path, shell: Option<&str>) {
    drop(PROJECT.set((root.to_path_buf(), shell.map(str::to_owned))));
}

/// ffmpeg, ready for its arguments.
pub(crate) fn ffmpeg() -> Result<Command> {
    found().map(|f| Command::new(&f.ffmpeg))
}

/// ffprobe, ready for its arguments.
pub(crate) fn ffprobe() -> Result<Command> {
    found().map(|f| Command::new(&f.ffprobe))
}

/// None when Fearlezz has ffmpeg, else what to do about it.
pub(crate) fn missing() -> Option<String> {
    found().err().map(|e| format!("{e:#}"))
}

/// On GitHub Actions, when a check takes pictures: finds ffmpeg up front, installing the
/// pinned Linux build when neither the shell nor PATH has one, and says which it uses. Returns
/// the pinned build's folder, which the checks' own commands get in front of PATH.
pub(crate) fn on_runner() -> Option<PathBuf> {
    let mut installed = None;
    let found = FOUND.get_or_init(|| {
        search().or_else(|none| match crate::action::pinned_ffmpeg() {
            Ok(Some(dir)) => {
                installed = Some(dir.clone());
                Ok(Found {
                    ffmpeg: dir.join("ffmpeg"),
                    ffprobe: dir.join("ffprobe"),
                    from: format!("7.0.2, installed into {}", dir.display()),
                })
            }
            Ok(None) => Err(none),
            Err(e) => Err(format!("{none}; installing the pinned one failed: {e:#}")),
        })
    });
    match found {
        Ok(f) => println!("ffmpeg for the pictures: {}", f.from),
        Err(e) => println!("::warning::{e}"),
    }
    installed
}

fn found() -> Result<&'static Found> {
    FOUND
        .get_or_init(search)
        .as_ref()
        .map_err(|e| anyhow!("{e}"))
}

/// The project's shell, then PATH.
fn search() -> Result<Found, String> {
    let project = PROJECT.get();
    let shell = project.and_then(|(root, shell)| Some((root, shell.as_deref()?)));
    if let Some((root, shell)) = shell
        && let Some(f) = in_shell(root, shell)
    {
        return Ok(f);
    }
    if let (Some(ffmpeg), Some(ffprobe)) = (on_path("ffmpeg"), on_path("ffprobe")) {
        return Ok(Found {
            ffmpeg,
            ffprobe,
            from: "the one on PATH".into(),
        });
    }
    Err(match shell {
        Some((_, shell)) => format!(
            "Fearlezz needs ffmpeg and ffprobe to read stills and films, and found them neither \
             in the project's shell (`{shell}`) nor on PATH; add ffmpeg to the project's shell, \
             or install it"
        ),
        None => "Fearlezz needs ffmpeg and ffprobe to read stills and films, and found them \
                 nowhere on PATH; install ffmpeg, or add it to the project's [env] shell"
            .into(),
    })
}

/// Both programs as `command -v` finds them inside the shell, run as every project command is.
fn in_shell(root: &Path, shell: &str) -> Option<Found> {
    let project = crate::process::Project::new(root, Some(shell));
    let out = crate::process::read(
        &mut project.script("command -v ffmpeg && command -v ffprobe"),
        None,
    )
    .ok()?;
    let text = String::from_utf8_lossy(&out);
    // A shell may say more on stdout before the script runs; the paths are its last lines.
    let mut paths = text
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('/'))
        .rev()
        .map(PathBuf::from);
    let ffprobe = paths.next().filter(|p| p.is_file())?;
    let ffmpeg = paths.next().filter(|p| p.is_file())?;
    Some(Found {
        ffmpeg,
        ffprobe,
        from: format!("the project's shell (`{shell}`)"),
    })
}

fn on_path(tool: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(tool))
        .find(|p| p.is_file())
}
