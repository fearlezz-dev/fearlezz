//! Running another program and reading what it printed: the one place that starts a child
//! with no input, drains its output while it runs, stops it at a time budget and turns a
//! failure into what the program said.

use std::fmt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| Failed::Start {
            program: cmd.get_program().to_string_lossy().into_owned(),
            error,
        })?;
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
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
