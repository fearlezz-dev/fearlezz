//! The floor of a change's proof: the config of the commit where the branch left the base
//! branch. A change may add proof but never take it away, so a full run at the final stage
//! compares the change's config with that one. Proof is about behaviour, so the floor judges
//! the checks, never which files they read.

use anyhow::Result;
use fearlezz_proof::waits::{PAST, Part, Weakening};

use crate::config::{self, Builtin, Config, Env, Ports, Widening};
use crate::findings::Scope;
use crate::state::Worktree;

/// What a change takes away from the proof the base branch requires.
#[derive(Default)]
pub(crate) struct Floor {
    /// What weakened, and how.
    pub weakened: Vec<Weakening>,
}

impl Floor {
    /// Compares the change's config with the one on its base commit. Without a config there,
    /// there is no floor.
    pub(crate) fn read(head: &Config, wt: &Worktree) -> Result<Floor> {
        let git = &wt.git;
        let (base, _hold) = git.base_worktree(&wt.base, "")?;
        if !base.root.join(config::FILE).exists() {
            return Ok(Floor::default());
        }
        let base = Config::load(&base.root, &git.common)?;
        Ok(Floor {
            weakened: weakenings(&base, head, &wt.files),
        })
    }

    pub(crate) fn holds(&self) -> bool {
        self.weakened.is_empty()
    }

    /// The lines a run prints: what fails the floor and how to get past it; when it `waits`
    /// on a pull request, that an admin's accept lets it through.
    pub(crate) fn lines(&self, waits: bool) -> Vec<String> {
        let mut lines: Vec<String> = self
            .weakened
            .iter()
            .map(|w| format!("weakens {}: {}", w.part.name(), w.how))
            .collect();
        if !self.weakened.is_empty() {
            lines.push(if waits {
                "  note: the proof passes; the fearlezz/approval status waits until a person with \
                 admin rights reads why and comments /fearlezz accept <words>"
                    .into()
            } else {
                format!("  fix: keep the proof the base branch requires; {PAST}")
            });
        }
        lines
    }
}

/// How `head` proves less than `base`: a check gone, running another command, at a later
/// stage, reporting or reading findings otherwise, taking fewer pictures (see
/// [`config::Pictures::weakenings`]), rules covering fewer of `files`, and new `no_proof` globs
/// or dropped negations. Which files a check reads is only a speed hint, so it never weakens.
fn weakenings(base: &Config, head: &Config, files: &[String]) -> Vec<Weakening> {
    let mut found = Vec::new();
    for b in &base.checks {
        let mut say = |how: String| {
            found.push(Weakening {
                part: Part::Check(b.id.clone()),
                how,
            });
        };
        let Some(h) = head.checks.iter().find(|h| h.id == b.id) else {
            say("it is gone".into());
            continue;
        };
        if h.run != b.run {
            say("it runs another command".into());
        }
        let base_stage = base.stages.name(b.stage);
        if head.stages.find(base_stage).is_none_or(|s| h.stage > s) {
            say(format!("it moves to a later stage than {base_stage}"));
        }
        match (&b.findings, &h.findings) {
            (Some(_), None) => say("it no longer reports findings".into()),
            (Some(bf), Some(hf)) => {
                if bf.read != hf.read {
                    say(format!(
                        "its findings are read another way ({}, now {})",
                        bf.read.name(),
                        hf.read.name()
                    ));
                }
                for tool in bf.tools.iter().filter(|t| !hf.tools.contains(t)) {
                    say(format!(
                        "its findings no longer get the levels of [lints.{}]",
                        tool.name()
                    ));
                }
                if bf.on == Scope::Everywhere && hf.on == Scope::Changed {
                    say("its findings count only on changed lines".into());
                }
            }
            (None, _) => {}
        }
        for how in b.pictures.weakenings(&h.pictures) {
            say(how);
        }
        if b.tests.is_some() && h.tests.is_none() {
            say("it no longer counts its tests".into());
        }
        // Which files are tests decides what runs on the base commit, so any change to it
        // could hide a fix whose tests do not fail there. Naming them the first time is fine.
        if !b.test_files.is_empty() && h.test_files != b.test_files {
            say("it names other test files".into());
        }
        let env: Vec<String> = h
            .env
            .iter()
            .filter(|v| !b.env.contains(v))
            .map(|v| format!("+{v}"))
            .chain(
                b.env
                    .iter()
                    .filter(|v| !h.env.contains(v))
                    .map(|v| format!("-{v}")),
            )
            .collect();
        if !env.is_empty() {
            say(format!(
                "it sees other environment variables ({})",
                env.join(", ")
            ));
        }
        if let (Some(bf), Some(hf)) = (&b.findings, &h.findings) {
            for tool in bf.tools.iter().filter(|t| hf.tools.contains(t)) {
                found.extend(settings(bf.levels.weakenings(&hf.levels, *tool)));
            }
        }
        if let (Some(Builtin::Rules(br)), Some(Builtin::Rules(hr))) = (&b.builtin, &h.builtin) {
            found.extend(settings(br.weakenings(hr, files)));
        }
    }
    // Checks that share a lints table would name its weakenings once each.
    let mut seen = std::collections::BTreeSet::new();
    found.retain(|w| seen.insert(w.clone()));
    found.extend(settings(base.settings.weakenings(&head.settings)));
    found.extend(settings(env_weakenings(&base.env, &head.env)));
    for w in head.no_proof.widenings(&base.no_proof) {
        let how = match w {
            Widening::Added(glob) => format!("it newly says {glob} needs no proof"),
            Widening::Dropped(glob) => format!(
                "it no longer takes {} back from needing no proof",
                glob.trim_start_matches('!')
            ),
            Widening::Reordered => "it reorders its globs so more files need no proof".into(),
        };
        found.push(Weakening {
            part: Part::Setting("no_proof".into()),
            how,
        });
    }
    found
}

/// Weakenings of parts of the config other than a check, each as its path and how.
fn settings(found: Vec<(String, String)>) -> impl Iterator<Item = Weakening> {
    found.into_iter().map(|(path, how)| Weakening {
        part: Part::Setting(path),
        how,
    })
}

/// How `head` makes checks run in another environment: any command, variable, cloned folder
/// or port source changed, since Fearlezz cannot tell which way that weakens them. Each as
/// `env.<field>` and how.
fn env_weakenings(base: &Env, head: &Env) -> Vec<(String, String)> {
    let ports = |p: &Option<Ports>| {
        p.as_ref().map(|p| match p {
            Ports::Command(cmd) => cmd.clone(),
            Ports::Devenv => "read devenv".to_string(),
        })
    };
    let commands = [
        ("prepare", base.prepare.clone(), head.prepare.clone()),
        ("shell", base.shell.clone(), head.shell.clone()),
        ("up", base.up.clone(), head.up.clone()),
        ("down", base.down.clone(), head.down.clone()),
        ("reset", base.reset.clone(), head.reset.clone()),
        ("ports", ports(&base.ports), ports(&head.ports)),
    ];
    let mut found = Vec::new();
    for (field, was, now) in commands {
        let how = match (was, now) {
            (Some(b), Some(h)) if b != h => format!("it changes from {b} to {h}"),
            (None, Some(h)) => format!("it is added ({h})"),
            (Some(b), None) => format!("it is removed (was {b})"),
            _ => continue,
        };
        found.push((format!("env.{field}"), how));
    }
    let lists = [
        ("clone", &base.clone, &head.clone),
        ("clone_when", &base.clone_when, &head.clone_when),
        ("env", &base.vars, &head.vars),
    ];
    for (field, was, now) in lists {
        let changes: Vec<String> = now
            .iter()
            .filter(|v| !was.contains(v))
            .map(|v| format!("+{v}"))
            .chain(
                was.iter()
                    .filter(|v| !now.contains(v))
                    .map(|v| format!("-{v}")),
            )
            .collect();
        if !changes.is_empty() {
            found.push((
                format!("env.{field}"),
                format!("it changes ({})", changes.join(", ")),
            ));
        }
    }
    found
}
