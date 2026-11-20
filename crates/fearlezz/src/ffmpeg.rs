//! Which ffmpeg and ffprobe Fearlezz runs itself, to compare, record and draw pictures: found
//! once per run, in the project's `[env]` shell first (where a devenv or Nix project keeps
//! its tools), then on PATH, then on GitHub Actions the pinned Linux build. Every call runs
//! the found program by its absolute path, so the project's ffmpeg is used even though
//! Fearlezz itself runs outside that shell, and with the clean environment a check gets,
//! since the project chose that program.

use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::Result;

/// How long the project's shell may take to answer: enough to build a devenv shell the first
/// time. Past it, PATH is searched instead.
const SHELL_BUDGET: Duration = Duration::from_secs(300);

/// The project whose shell is asked first: its root and its `[env]` shell.
static PROJECT: OnceLock<(PathBuf, Option<String>)> = OnceLock::new();
/// What the one search found, or why it found nothing.
static FOUND: OnceLock<Result<Found, Missing>> = OnceLock::new();

/// The programs found and where they came from.
struct Found {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    from: String,
    /// The pinned build's folder, when that is what was found.
    pinned: Option<PathBuf>,
}

/// Why Fearlezz has no ffmpeg, and what to do about it.
#[derive(Debug, Clone)]
pub(crate) struct Missing {
    pub why: String,
    pub fix: String,
}

impl fmt::Display for Missing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}; {}", self.why, self.fix)
    }
}

impl std::error::Error for Missing {}

/// Names the project whose `[env]` shell is asked for ffmpeg, once the repository is read.
/// Without it only PATH is searched, as `setup compare` outside a repository does.
pub(crate) fn within(root: &Path, shell: Option<&str>) {
    drop(PROJECT.set((root.to_path_buf(), shell.map(str::to_owned))));
}

/// ffmpeg, ready for its arguments.
pub(crate) fn ffmpeg() -> Result<Command> {
    found().map(|f| clean(&f.ffmpeg))
}

/// ffprobe, ready for its arguments.
pub(crate) fn ffprobe() -> Result<Command> {
    found().map(|f| clean(&f.ffprobe))
}

/// None when Fearlezz has ffmpeg, else why not and what to do about it.
pub(crate) fn missing() -> Option<Missing> {
    FOUND.get_or_init(search).as_ref().err().cloned()
}

/// On GitHub Actions, when a check takes pictures: finds ffmpeg up front (the search itself
/// installs the pinned build there when nothing else has one) and says which it uses. Returns
/// the pinned build's folder, which the checks' own commands get in front of PATH.
pub(crate) fn on_runner() -> Option<PathBuf> {
    match FOUND.get_or_init(search) {
        Ok(f) => {
            println!("ffmpeg for the pictures: {}", f.from);
            f.pinned.clone()
        }
        Err(e) => {
            println!("::warning::{e}");
            None
        }
    }
}

fn found() -> Result<&'static Found> {
    Ok(FOUND.get_or_init(search).as_ref().map_err(Missing::clone)?)
}

/// `program` with the environment a project's command gets.
fn clean(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    crate::process::cleared(&mut cmd, &[]);
    cmd
}

/// The project's shell, then PATH, then on GitHub Actions the pinned build.
fn search() -> Result<Found, Missing> {
    let project = PROJECT.get();
    let shell = project.and_then(|(root, shell)| Some((root, shell.as_deref()?)));
    let mut failed = None;
    if let Some((root, shell)) = shell {
        match in_shell(root, shell) {
            Ok(Some(f)) => return Ok(f),
            Ok(None) => {}
            Err(e) => failed = Some(e),
        }
    }
    if let (Some(ffmpeg), Some(ffprobe)) = (on_path("ffmpeg"), on_path("ffprobe")) {
        return Ok(Found {
            ffmpeg,
            ffprobe,
            from: "the one on PATH".into(),
            pinned: None,
        });
    }
    let nowhere = "Fearlezz needs ffmpeg and ffprobe to read stills and films, and found them \
                   nowhere on PATH";
    let mut missing = match (project, shell) {
        (_, Some((_, shell))) => Missing {
            why: format!(
                "Fearlezz needs ffmpeg and ffprobe to read stills and films, and found them \
                 neither in the project's shell (`{shell}`) nor on PATH{}",
                failed.map_or_else(String::new, |e| format!("; the shell failed: {e}"))
            ),
            fix: "add ffmpeg to the project's shell, or install it".into(),
        },
        (Some(_), None) => Missing {
            why: nowhere.into(),
            fix: "install ffmpeg, or add it to the project's [env] shell".into(),
        },
        (None, None) => Missing {
            why: nowhere.into(),
            fix: "install ffmpeg".into(),
        },
    };
    if !crate::action::on_actions() {
        return Err(missing);
    }
    match crate::action::pinned_ffmpeg() {
        Ok(Some(dir)) => Ok(Found {
            ffmpeg: dir.join("ffmpeg"),
            ffprobe: dir.join("ffprobe"),
            from: format!("7.0.2, installed into {}", dir.display()),
            pinned: Some(dir),
        }),
        Ok(None) => Err(missing),
        Err(e) => {
            missing.why = format!("{}; installing the pinned one failed: {e:#}", missing.why);
            Err(missing)
        }
    }
}

/// Both programs as `command -v` finds them inside the shell, run as every project command
/// is: None when the shell has neither, an error when the shell itself failed or gave no
/// answer in time.
fn in_shell(root: &Path, shell: &str) -> Result<Option<Found>, String> {
    let project = crate::process::Project::new(root, Some(shell));
    // Each answer on a line of its own, marked, so whatever else the shell prints on stdout
    // (a banner, a path it entered) is never taken for one; the script itself always passes,
    // so a failure is the shell's.
    let script = "echo \"fearlezz-ffmpeg=$(command -v ffmpeg)\"; \
                  echo \"fearlezz-ffprobe=$(command -v ffprobe)\"";
    let out = crate::process::read(&mut project.script(script), Some(SHELL_BUDGET)).map_err(
        |e| match e {
            // The command line is Fearlezz's own wrapping; what the shell said is the news.
            crate::process::Failed::Exit { stderr, .. } => stderr,
            e @ (crate::process::Failed::Start { .. } | crate::process::Failed::Late { .. }) => {
                e.to_string()
            }
        },
    )?;
    let text = String::from_utf8_lossy(&out);
    let answer = |name: &str| {
        let mark = format!("fearlezz-{name}=");
        text.lines()
            .rev()
            .find_map(|l| l.trim().strip_prefix(&mark).map(PathBuf::from))
            .filter(|p| p.is_absolute() && p.is_file())
    };
    let (Some(ffmpeg), Some(ffprobe)) = (answer("ffmpeg"), answer("ffprobe")) else {
        return Ok(None);
    };
    Ok(Some(Found {
        ffmpeg,
        ffprobe,
        from: format!("the project's shell (`{shell}`)"),
        pinned: None,
    }))
}

fn on_path(tool: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(tool))
        .find(|p| p.is_file())
}
