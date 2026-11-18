//! The base commit a change is compared with, opened once per command: its checkout, its
//! config and the hold that keeps other commands from removing the checkout. The floor, the
//! look and red before green borrow it instead of each opening their own.

use std::cell::{RefCell, RefMut};
use std::fs;
use std::rc::Rc;

use anyhow::Result;

use crate::config::{Config, Env};
use crate::git::Git;
use crate::lease::Lease;
use crate::state::Worktree;

/// The base commits a command reached, each opened on first use. A command about one piece
/// of work reaches one; listing every journal may reach one per worktree.
#[derive(Default)]
pub(crate) struct Bases {
    open: RefCell<Vec<Rc<Base>>>,
}

impl Bases {
    /// The base `commit`, checked out in the per user cache (see [`Git::bases_dir`]) the
    /// first time it is asked for.
    pub(crate) fn at(&self, git: &Git, commit: &str) -> Result<Rc<Base>> {
        if let Some(base) = self.open.borrow().iter().find(|b| b.commit == commit) {
            return Ok(Rc::clone(base));
        }
        let base = Rc::new(Base::open(git, commit)?);
        self.open.borrow_mut().push(Rc::clone(&base));
        Ok(base)
    }
}

/// The base commit in its plain checkout.
pub(crate) struct Base {
    pub commit: String,
    /// Its `fearlezz.toml`, None when it has none.
    pub config: Option<Config>,
    git: Git,
    /// The checkout as a place to run checks, read the first time a check runs there.
    plain: RefCell<Option<Checkout>>,
    /// Keeps other commands from removing the checkout while this one uses it.
    _hold: Lease,
}

impl Base {
    fn open(git: &Git, commit: &str) -> Result<Base> {
        let (base, hold) = git.base_worktree(commit, "")?;
        let config = if base.root.join(crate::config::FILE).exists() {
            Some(Config::load(&base.root, &git.common)?)
        } else {
            None
        };
        Ok(Base {
            commit: commit.to_string(),
            config,
            git: base,
            plain: RefCell::new(None),
            _hold: hold,
        })
    }

    /// The base commit's own `[env]`: its code ran in that environment, so its checkouts are
    /// prepared, their services run and their checks run inside its shell by it, never by
    /// the change's. None when it has no config.
    pub(crate) fn env(&self) -> Option<&Env> {
        self.config.as_ref().map(|c| &c.env)
    }

    /// The plain checkout to run checks in, compared by `head`. Look and red before green
    /// share it, so it is prepared once per command.
    pub(crate) fn plain(&self, head: &Config) -> Result<RefMut<'_, Checkout>> {
        let mut plain = self.plain.borrow_mut();
        let checkout = match plain.take() {
            Some(checkout) => checkout,
            None => Checkout::new(self.read(self.git.clone(), head)?, None),
        };
        Ok(RefMut::map(plain, |p| p.insert(checkout)))
    }

    /// A checkout of its own, told apart by `name`, with `files` laid over it (None removes
    /// one). Red before green runs the change's tests there; the plain checkout never holds
    /// them.
    pub(crate) fn overlay(
        &self,
        git: &Git,
        head: &Config,
        name: &str,
        files: Vec<(&String, Option<Vec<u8>>)>,
    ) -> Result<Checkout> {
        let (over, hold) = git.base_worktree(&self.commit, name)?;
        for (path, content) in files {
            let to = over.root.join(path);
            match content {
                Some(bytes) => {
                    if let Some(dir) = to.parent() {
                        fs::create_dir_all(dir)?;
                    }
                    fs::write(&to, bytes)?;
                }
                None if to.exists() => fs::remove_file(&to)?,
                None => {}
            }
        }
        Ok(Checkout::new(self.read(over, head)?, Some(hold)))
    }

    fn read(&self, git: Git, head: &Config) -> Result<Worktree> {
        Worktree::read_in(git, head, self.env().unwrap_or(&Env::default()))
    }
}

/// A checkout of the base commit that checks run in.
pub(crate) struct Checkout {
    pub wt: Worktree,
    pub prepared: bool,
    /// An overlay's own hold; the plain checkout is held by its [`Base`].
    _hold: Option<Lease>,
}

impl Checkout {
    fn new(wt: Worktree, hold: Option<Lease>) -> Checkout {
        Checkout {
            wt,
            prepared: false,
            _hold: hold,
        }
    }
}
