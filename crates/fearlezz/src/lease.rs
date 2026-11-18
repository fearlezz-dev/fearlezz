//! Leases: a lock on a file, held by this process until dropped and freed by the system
//! however the process ends. The services of a worktree, base checkouts, the heavy queue's
//! slots and the simulator pool are each a lease on a file of their own.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;

use anyhow::{Context as _, Result};

/// How a lease shares its file with other holders.
#[derive(Clone, Copy)]
pub(crate) enum Mode {
    /// No other holder.
    Only,
    /// Other shared holders, but no holder in [`Mode::Only`].
    Shared,
}

/// A held lock on a file. Its contents stay as they were; a holder may write into it, such as
/// who holds it.
pub(crate) struct Lease {
    file: File,
}

impl Lease {
    /// Takes the lease on `path`, waiting while another process holds it. The file and its
    /// folder are made when missing.
    pub(crate) fn take(path: &Path, mode: Mode) -> Result<Lease> {
        let file = open(path)?;
        match mode {
            Mode::Only => file.lock(),
            Mode::Shared => file.lock_shared(),
        }
        .with_context(|| format!("locking {}", path.display()))?;
        Ok(Lease { file })
    }

    /// Takes the lease on `path` when no other process holds it; None when one does.
    pub(crate) fn try_take(path: &Path, mode: Mode) -> Result<Option<Lease>> {
        let file = open(path)?;
        let taken = match mode {
            Mode::Only => file.try_lock(),
            Mode::Shared => file.try_lock_shared(),
        };
        match taken {
            Ok(()) => Ok(Some(Lease { file })),
            Err(TryLockError::WouldBlock) => Ok(None),
            Err(TryLockError::Error(e)) => {
                Err(e).with_context(|| format!("locking {}", path.display()))
            }
        }
    }

    /// Whether another process holds a lease on `path` right now.
    pub(crate) fn held(path: &Path) -> bool {
        Lease::try_take(path, Mode::Only).is_ok_and(|lease| lease.is_none())
    }

    /// The leased file, to write into.
    pub(crate) fn file(&mut self) -> &mut File {
        &mut self.file
    }
}

/// `path`, made with its folder when missing and never truncated, so a waiter can read what
/// its holder wrote.
fn open(path: &Path) -> Result<File> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("making {}", dir.display()))?;
    }
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_only_lease_keeps_out_every_other_holder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a/lock");
        let lease = Lease::take(&path, Mode::Only).unwrap();
        assert!(Lease::try_take(&path, Mode::Only).unwrap().is_none());
        assert!(Lease::try_take(&path, Mode::Shared).unwrap().is_none());
        assert!(Lease::held(&path));
        drop(lease);
        assert!(!Lease::held(&path));
    }

    #[test]
    fn shared_leases_hold_together_and_keep_out_an_only_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lock");
        let _one = Lease::take(&path, Mode::Shared).unwrap();
        let _two = Lease::try_take(&path, Mode::Shared).unwrap().unwrap();
        assert!(Lease::try_take(&path, Mode::Only).unwrap().is_none());
    }

    #[test]
    fn taking_a_lease_keeps_what_its_file_holds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lock");
        std::fs::write(&path, "holder").unwrap();
        let _lease = Lease::take(&path, Mode::Only).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "holder");
    }
}
