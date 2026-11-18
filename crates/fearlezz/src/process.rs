//! Running another program and reading what it printed: the one place that starts a child
//! with no input, drains its output while it runs, stops it at a time budget and turns a
//! failure into what the program said.

use std::ffi::OsString;
use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Env vars every project command gets, so ordinary tools work. Everything else must be
/// declared. The rustup ones choose the toolchain, as PATH does for other tools.
pub(crate) const BASE_ENV: [&str; 11] = [
    "PATH",
    "HOME",
    "TMPDIR",
    "TERM",
    "LANG",
    "LC_ALL",
    "USER",
    "SHELL",
    "RUSTUP_TOOLCHAIN",
    "RUSTUP_HOME",
    "CARGO_HOME",
];

/// The project's own tool folders, put in front of PATH when they exist.
pub(crate) const TOOL_DIRS: [&str; 2] = ["node_modules/.bin", ".venv/bin"];

/// Where a project's commands run: from the checkout at `root`, inside the `[env]` `shell`
/// when there is one. Every command Fearlezz runs for a project (checks, envtest, services,
/// the preview, SecretSpec, releases) is built here, so what a command inherits is decided
/// in one place.
#[derive(Clone, Copy)]
pub(crate) struct Project<'a> {
    pub root: &'a Path,
    pub shell: Option<&'a str>,
}

impl<'a> Project<'a> {
    pub(crate) fn new(root: &'a Path, shell: Option<&'a str>) -> Self {
        Project { root, shell }
    }

    /// `script` through `sh -c` inside the shell, in the folder `dir` of the root, with only
    /// [`BASE_ENV`] and the variables `vars` names from Fearlezz's own, and the project's tool
    /// folders in front of PATH. `args` reach the script as its arguments (`"$@"`), whole,
    /// whatever they hold.
    pub(crate) fn clean(
        &self,
        dir: Option<&str>,
        script: &str,
        vars: &[String],
        args: &[String],
    ) -> Command {
        let folder = dir.map(|d| self.root.join(d));
        self.shaped(folder.as_deref(), script, Some(vars), args)
    }

    /// `script` as [`Project::clean`] runs it, but with all of Fearlezz's own variables: only
    /// for asking SecretSpec, whose provider's token must reach it and never a check.
    pub(crate) fn inherited(&self, dir: &Path, script: &str) -> Command {
        self.shaped(Some(dir), script, None, &[])
    }

    /// The command: its line, its folder, its variables (only [`BASE_ENV`] and `vars` when
    /// given, else all of Fearlezz's own) and the variables that carry the script. Inside a
    /// shell the script travels in a variable, so no quoting can break it. A command with a
    /// folder starts there; the shell itself starts at the root, where it finds its own files,
    /// and moves into the folder before the script.
    fn shaped(
        &self,
        folder: Option<&Path>,
        script: &str,
        vars: Option<&[String]>,
        args: &[String],
    ) -> Command {
        let line = match (self.shell, folder) {
            (Some(shell), None) => format!("{shell} sh -c \"$FEARLEZZ_RUN\" sh \"$@\""),
            (Some(shell), Some(_)) => {
                format!("{shell} sh -c 'cd \"$FEARLEZZ_DIR\" && eval \"$FEARLEZZ_RUN\"' sh \"$@\"")
            }
            (None, _) => script.to_string(),
        };
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(&line)
            .arg("sh")
            .args(args)
            .stdin(Stdio::null())
            .current_dir(match (self.shell, folder) {
                (None, Some(folder)) => folder,
                _ => self.root,
            });
        if let Some(vars) = vars {
            cmd.env_clear();
            for name in BASE_ENV
                .iter()
                .copied()
                .chain(vars.iter().map(String::as_str))
            {
                if let Some(value) = std::env::var_os(name) {
                    cmd.env(name, value);
                }
            }
        }
        if let Some(path) = self.path() {
            cmd.env("PATH", path);
        }
        if let Some(folder) = folder {
            cmd.env("FEARLEZZ_DIR", folder);
        }
        cmd.env("FEARLEZZ_RUN", script);
        cmd
    }

    /// PATH with the project's tool folders that exist in front, or None when there are none.
    pub(crate) fn path(&self) -> Option<OsString> {
        let dirs: Vec<PathBuf> = TOOL_DIRS
            .iter()
            .map(|d| self.root.join(d))
            .filter(|d| d.is_dir())
            .collect();
        if dirs.is_empty() {
            return None;
        }
        let rest = std::env::var_os("PATH").unwrap_or_default();
        std::env::join_paths(dirs.into_iter().chain(std::env::split_paths(&rest))).ok()
    }
}

/// Why a program gave no answer.
#[derive(Debug)]
pub(crate) enum Failed {
    /// It did not start, most often because it is not installed.
    Start {
        program: String,
        error: std::io::Error,
    },
    /// It was still running when its budget ran out, and was stopped.
    Late { command: String, budget: Duration },
    /// It ended unsuccessfully; what it printed on stderr, trimmed.
    Exit { command: String, stderr: String },
}

impl fmt::Display for Failed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Failed::Start { program, error } => write!(f, "running {program}: {error}"),
            Failed::Late { command, budget } => {
                write!(f, "{command} gave no answer in {budget:?}")
            }
            Failed::Exit { command, stderr } => write!(f, "{command} failed: {stderr}"),
        }
    }
}

impl std::error::Error for Failed {}

/// Runs `cmd` with no input until it ends, or until `budget` when one is given, and returns
/// what it printed on stdout. Both outputs are read while it runs, so a program that prints a
/// lot never blocks on a full pipe.
pub(crate) fn read(cmd: &mut Command, budget: Option<Duration>) -> Result<Vec<u8>, Failed> {
    read_fed(cmd, &[], budget)
}

/// [`read`], with `input` written to the program's stdin first, such as a token that must not
/// show in its arguments.
pub(crate) fn read_fed(
    cmd: &mut Command,
    input: &[u8],
    budget: Option<Duration>,
) -> Result<Vec<u8>, Failed> {
    let stdin = if input.is_empty() {
        Stdio::null()
    } else {
        Stdio::piped()
    };
    let mut child = cmd
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| Failed::Start {
            program: cmd.get_program().to_string_lossy().into_owned(),
            error,
        })?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    if let Some(mut stdin) = child.stdin.take() {
        // On a thread of its own, so a program that prints before it reads all of its input
        // never blocks; one that stops reading early has said what it will say.
        let input = input.to_vec();
        std::thread::spawn(move || drop(stdin.write_all(&input)));
    }
    let status = match budget {
        None => child.wait(),
        Some(budget) => {
            let since = Instant::now();
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => break Ok(status),
                    Ok(None) if since.elapsed() >= budget => {
                        drop(child.kill());
                        drop(child.wait());
                        return Err(Failed::Late {
                            command: shown(cmd),
                            budget,
                        });
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                    Err(e) => break Err(e),
                }
            }
        }
    };
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    match status {
        Ok(status) if status.success() => Ok(stdout),
        Ok(_) => Err(Failed::Exit {
            command: shown(cmd),
            stderr: String::from_utf8_lossy(&stderr).trim().to_string(),
        }),
        Err(error) => Err(Failed::Start {
            program: cmd.get_program().to_string_lossy().into_owned(),
            error,
        }),
    }
}

/// Reads `pipe` to its end on a thread of its own.
fn drain(pipe: Option<impl std::io::Read + Send + 'static>) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            drop(pipe.read_to_end(&mut bytes));
        }
        bytes
    })
}

/// The program and its arguments, as a person would type them (without quoting).
fn shown(cmd: &Command) -> String {
    std::iter::once(cmd.get_program())
        .chain(cmd.get_args())
        .map(|a| a.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_a_program_prints_is_read() {
        let out = read(Command::new("sh").args(["-c", "echo hi"]), None).unwrap();
        assert_eq!(out, b"hi\n");
    }

    #[test]
    fn a_failure_says_what_the_program_said() {
        let err = read(Command::new("sh").args(["-c", "echo no >&2; exit 3"]), None)
            .unwrap_err()
            .to_string();
        assert_eq!(err, "sh -c echo no >&2; exit 3 failed: no");
    }

    #[test]
    fn a_program_past_its_budget_is_stopped() {
        let since = Instant::now();
        let err = read(
            Command::new("sleep").arg("5"),
            Some(Duration::from_millis(50)),
        )
        .unwrap_err();
        assert!(matches!(err, Failed::Late { .. }), "{err}");
        assert!(since.elapsed() < Duration::from_secs(4));
    }

    #[test]
    fn a_missing_program_says_it_did_not_start() {
        let err = read(&mut Command::new("fearlezz-no-such-tool"), None).unwrap_err();
        assert!(matches!(err, Failed::Start { .. }), "{err}");
    }

    #[test]
    fn a_long_output_never_blocks_the_program() {
        let out = read(
            Command::new("sh").args(["-c", "yes x | head -c 1000000; yes y | head -c 1000000 >&2"]),
            Some(Duration::from_secs(20)),
        )
        .unwrap();
        assert_eq!(out.len(), 1_000_000);
    }
}
