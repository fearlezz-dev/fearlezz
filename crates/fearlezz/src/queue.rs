//! The machine's heavy queue. Many agents in many worktrees run `fearlezz run` at once, and
//! every heavy tool uses all the cores it finds, so on a developer's machine that opts in with
//! `FEARLEZZ_QUEUE=<slots>` each heavy step (a check's command, a run on the base commit, a
//! picture comparison, a tool run by hand through `fearlezz run -- <command>`) takes one of
//! that many slots first, whichever worktree or repository it comes from, and waits for one
//! while all are taken. Off by default and wherever `CI` is set.
//!
//! A slot is a lock file in the user's cache folder, locked with `flock`, so the system frees
//! it when its holder ends however it ends. Its holder writes who it is into it, so a waiter
//! can say what it waits behind. Waiters poll instead of blocking, so each poll can also see a
//! result another worktree produced meanwhile for the same inputs, and none is first in line:
//! the slot goes to whichever waiter polls first.

use std::fs::File;
use std::io::{Seek as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::lease::{Lease, Mode};

/// The developer's choice for this machine: how many heavy steps may run at once.
const VAR: &str = "FEARLEZZ_QUEUE";
/// Set for every command started while this process holds a slot: a Fearlezz inside it runs
/// at once, under its parent's slot, instead of waiting for that slot forever.
pub(crate) const HELD: &str = "FEARLEZZ_QUEUE_HELD";
const POLL: Duration = Duration::from_millis(250);

/// Slots this process holds. A second step in the same process while one is held (which no
/// path takes today) runs under it, since a second flock of the same file would wait forever.
static HOLDING: AtomicUsize = AtomicUsize::new(0);

/// A taken slot, freed when dropped.
pub(crate) struct Slot {
    lease: Lease,
}

impl Drop for Slot {
    fn drop(&mut self) {
        // Emptied, so no waiter names a step that has ended.
        drop(self.lease.file().set_len(0));
        HOLDING.fetch_sub(1, Ordering::SeqCst);
    }
}

/// What waiting for a slot gave: the slot (none when the queue is off), or what `done`
/// found meanwhile.
pub(crate) enum Turn<T> {
    Go(Option<Slot>),
    Done(T),
}

/// Whether this process holds a slot now, so the commands it starts get [`HELD`].
pub(crate) fn holding() -> bool {
    HOLDING.load(Ordering::SeqCst) > 0
}

/// How many slots the machine has, or None when the queue is off here: not opted in, on CI,
/// or inside a command that runs under a slot already.
fn slots() -> Option<usize> {
    if std::env::var_os("CI").is_some() || std::env::var_os(HELD).is_some() || holding() {
        return None;
    }
    let value = std::env::var(VAR).ok()?;
    let slots = value.trim().parse::<usize>().ok().filter(|n| *n > 0);
    if slots.is_none() {
        static SAID: std::sync::Once = std::sync::Once::new();
        SAID.call_once(|| {
            eprintln!(
                "queue: {VAR}={value} is not a number of slots above zero, so the heavy queue is off"
            );
            eprintln!(
                "  fix: set {VAR} to how many heavy steps may run at once on this machine, such \
                 as 1, or unset it"
            );
        });
    }
    slots
}

/// Waits for a slot for the heavy step `what` (a check id, or a command), asking `done`
/// after every poll whether the step is no longer needed, such as when another worktree just
/// proved the same inputs. Says once that it waits and behind what, and how the wait ended.
pub(crate) fn turn<T>(what: &str, done: impl Fn() -> Option<T>) -> Result<Turn<T>> {
    let Some(slots) = slots() else {
        return Ok(Turn::Go(None));
    };
    let dir = crate::files::UserDir::Cache.path()?.join("queue");
    let paths: Vec<PathBuf> = (0..slots)
        .map(|i| dir.join(format!("slot-{i}.lock")))
        .collect();
    let clock = Instant::now();
    let mut waited = false;
    loop {
        if let Some(mut lease) = free(&paths)? {
            // A result may have landed between the last poll and the lock.
            if waited && let Some(found) = done() {
                eprintln!("wait {what}: the same inputs ran meanwhile; reused");
                return Ok(Turn::Done(found));
            }
            Holder::now(what).claim(lease.file())?;
            HOLDING.fetch_add(1, Ordering::SeqCst);
            if waited {
                eprintln!(
                    "wait {what}: its turn after {}",
                    elapsed(clock.elapsed().as_secs())
                );
            }
            return Ok(Turn::Go(Some(Slot { lease })));
        }
        if !waited {
            let now = now_secs();
            let behind: Vec<String> = paths
                .iter()
                .map(|path| behind(Holder::read(path), now))
                .collect();
            let plural = if slots == 1 { "slot" } else { "slots" };
            eprintln!(
                "wait {what}: the heavy queue is full ({slots} {plural}, {VAR}={slots}); {}",
                behind.join("; ")
            );
            eprintln!(
                "  fix: let it wait and never stop the holder; a heavy tool run by hand waits \
                 its turn too with fearlezz run -- <command>"
            );
            waited = true;
        }
        std::thread::sleep(POLL);
        if let Some(found) = done() {
            eprintln!("wait {what}: the same inputs ran meanwhile; reused");
            return Ok(Turn::Done(found));
        }
    }
}

/// Waits for a slot for the heavy step `what`, which no other run can do instead.
pub(crate) fn slot(what: &str) -> Result<Option<Slot>> {
    match turn(what, || None::<std::convert::Infallible>)? {
        Turn::Go(slot) => Ok(slot),
        Turn::Done(never) => match never {},
    }
}

/// Runs a tool an agent runs by hand (`fearlezz run -- <command>`) in a slot, with the
/// terminal's input and output, and returns its exit code, or 128 plus the signal that ended
/// it, as a shell reports it.
pub(crate) fn by_hand(command: &[String]) -> Result<u8> {
    let (program, args) = command
        .split_first()
        .context("name the command to run after --")?;
    let slot = slot(&command.join(" "))?;
    let mut cmd = Command::new(program);
    cmd.args(args);
    if slot.is_some() {
        cmd.env(HELD, "1");
    }
    let status = cmd
        .status()
        .with_context(|| format!("cannot start {program}"))?;
    #[cfg(unix)]
    if let Some(signal) = std::os::unix::process::ExitStatusExt::signal(&status) {
        return Ok(u8::try_from(128 + signal).unwrap_or(1));
    }
    Ok(status
        .code()
        .map_or(1, |code| u8::try_from(code).unwrap_or(1)))
}

/// A free slot, now leased by this process, if any.
fn free(paths: &[PathBuf]) -> Result<Option<Lease>> {
    for path in paths {
        if let Some(lease) = Lease::try_take(path, Mode::Only)? {
            return Ok(Some(lease));
        }
    }
    Ok(None)
}

/// Who holds a slot, written into its file once it is taken. The next holder rewrites it.
#[derive(Serialize, Deserialize)]
struct Holder {
    what: String,
    /// The worktree it runs for.
    folder: PathBuf,
    pid: u32,
    since: u64,
}

impl Holder {
    fn now(what: &str) -> Holder {
        let here = std::env::current_dir().unwrap_or_default();
        Holder {
            what: what.to_string(),
            folder: worktree(&here).unwrap_or(here),
            pid: std::process::id(),
            since: now_secs(),
        }
    }

    fn claim(&self, file: &mut File) -> Result<()> {
        file.set_len(0)?;
        file.rewind()?;
        serde_json::to_writer(&mut *file, self)?;
        file.flush()?;
        Ok(())
    }

    /// The holder in a slot's file; None when it is empty or unreadable, as in the moment
    /// between taking a slot and writing it.
    fn read(path: &Path) -> Option<Holder> {
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
    }
}

/// The worktree holding `dir`: the nearest folder with a `.git` entry.
fn worktree(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .find(|d| d.join(".git").exists())
        .map(Path::to_path_buf)
}

/// `behind tests in feat-x (pid 4121, 3m 10s)`.
fn behind(holder: Option<Holder>, now: u64) -> String {
    let Some(h) = holder else {
        return "behind another heavy step".to_string();
    };
    let folder = h.folder.file_name().map_or_else(
        || h.folder.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    format!(
        "behind {} in {folder} (pid {}, {})",
        h.what,
        h.pid,
        elapsed(now.saturating_sub(h.since))
    )
}

fn elapsed(secs: u64) -> String {
    match (secs / 3600, secs / 60 % 60, secs % 60) {
        (0, 0, s) => format!("{s}s"),
        (0, m, s) => format!("{m}m {s}s"),
        (h, m, _) => format!("{h}h {m}m"),
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wait_names_the_holder_its_worktree_and_how_long_it_held() {
        let holder = Holder {
            what: "tests".into(),
            folder: "/work/feat-x".into(),
            pid: 4121,
            since: 1000,
        };
        assert_eq!(
            behind(Some(holder), 1190),
            "behind tests in feat-x (pid 4121, 3m 10s)"
        );
        assert_eq!(behind(None, 0), "behind another heavy step");
        assert_eq!(elapsed(7300), "2h 1m");
        assert_eq!(elapsed(9), "9s");
    }
}
