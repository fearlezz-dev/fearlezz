//! The floor of a change's proof: the config of the commit where the branch left the base
//! branch. A change may add proof but never take it away, so a full run at the final stage
//! compares the change's config with that one, and every changed file must be read by a check
//! or declared by the base branch as needing no proof.

use anyhow::Result;

use crate::config::{self, Builtin, Config, Env, Ports, Widening};
use crate::findings::Scope;
use crate::presets;
use crate::state::Worktree;

/// What a change takes away from the proof the base branch requires.
#[derive(Default)]
pub(crate) struct Floor {
    /// What weakened, and how.
    pub weakened: Vec<(String, String)>,
    /// Changed files nothing proves.
    pub unproven: Vec<String>,
}

/// One thing that keeps a commit from being proven: what, why, and what to do about it.
#[derive(Clone)]
pub(crate) struct Item {
    pub what: String,
    pub why: String,
    pub fix: String,
}

impl Floor {
    /// Compares the change's config with the one on its base commit. Without a config there,
    /// there is no floor and the change's own `no_proof` counts.
    pub(crate) fn read(head: &Config, wt: &Worktree) -> Result<Floor> {
        let git = &wt.git;
        let (base, _hold) = git.base_worktree(&wt.base, "")?;
        let base = if base.root.join(config::FILE).exists() {
            let config = Config::load(&base.root, &git.common)?;
            Some((config, presets::config_files(&base.root)?))
        } else {
            None
        };
        let mut config_files = presets::config_files(&wt.git.root)?;
        let floor = base.as_ref().map_or(head, |(config, _)| config);
        if let Some((_, pieces)) = &base {
            config_files.extend(pieces.iter().cloned());
        }
        let weakened = base
            .as_ref()
            .map(|(config, _)| weakenings(config, head, &wt.files))
            .unwrap_or_default();
        let unproven = wt
            .changed
            .iter()
            .filter(|p| !config_files.contains(p))
            .filter(|p| !head.checks.iter().any(|c| c.proves(p)))
            .filter(|p| !floor.needs_no_proof(p))
            .cloned()
            .collect();
        Ok(Floor { weakened, unproven })
    }

    pub(crate) fn holds(&self) -> bool {
        self.weakened.is_empty() && self.unproven.is_empty()
    }

    /// What fails the floor, one item each, for a reader who sees nothing else.
    pub(crate) fn items(&self, base: &str) -> Vec<Item> {
        let weakened = self.weakened.iter().map(|(what, how)| Item {
            what: if what.contains('.') || what == "no_proof" {
                format!("`{what}` is weaker")
            } else {
                format!("check `{what}` is weaker")
            },
            why: how.clone(),
            fix: format!(
                "keep the proof {base} requires; when it must change, a person with admin rights \
                 merges past CI after reading this"
            ),
        });
        let unproven = self.unproven.iter().map(|path| Item {
            what: format!("`{path}` has no proof"),
            why: format!("no check proves it and {base} does not declare it in no_proof"),
            fix: format!(
                "add it to a check's inputs, or declare it in no_proof in {} on {base} first",
                config::FILE
            ),
        });
        weakened.chain(unproven).collect()
    }

    /// The lines that say what fails the floor and how to get past it.
    pub(crate) fn lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for (what, how) in &self.weakened {
            lines.push(format!("weakens {what}: {how}"));
        }
        if !self.weakened.is_empty() {
            lines.push(
                "  fix: keep the proof the base branch requires; when it must change, a person \
                 with admin rights merges past CI after reading this"
                    .into(),
            );
        }
        for path in &self.unproven {
            lines.push(format!(
                "unproven {path}: no check proves it and the base branch does not declare it in \
                 no_proof"
            ));
        }
        if !self.unproven.is_empty() {
            lines.push(format!(
                "  fix: add it to a check's inputs, or declare it in no_proof in {} on the base \
                 branch first",
                config::FILE
            ));
        }
        lines
    }
}

/// How `head` proves less than `base`: a check gone, running another command, at a later
/// stage, reading fewer of `files`, reporting or reading findings otherwise, taking fewer pictures
/// (see [`config::Pictures::weakenings`]), and new `no_proof` globs or dropped negations.
fn weakenings(base: &Config, head: &Config, files: &[String]) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for b in &base.checks {
        let mut say = |how: String| found.push((b.id.clone(), how));
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
        let lost: Vec<&String> = files.iter().filter(|f| b.reads(f) && !h.reads(f)).collect();
        if let Some(first) = lost.first() {
            say(match lost.len() {
                1 => format!("it no longer reads {first}"),
                n => format!("it no longer reads {first} and {} more files", n - 1),
            });
        }
        // Reading a file it no longer proves is named once, as reading.
        let unproven: Vec<&String> = files
            .iter()
            .filter(|f| b.proves(f) && h.reads(f) && !h.proves(f))
            .collect();
        if let Some(first) = unproven.first() {
            say(match unproven.len() {
                1 => format!("it no longer proves {first}"),
                n => format!("it no longer proves {first} and {} more files", n - 1),
            });
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
                found.extend(bf.levels.weakenings(&hf.levels, *tool));
            }
        }
        if let (Some(Builtin::Rules(br)), Some(Builtin::Rules(hr))) = (&b.builtin, &h.builtin) {
            found.extend(br.weakenings(hr, files));
        }
    }
    // Checks that share a lints table would name its weakenings once each.
    let mut seen = std::collections::BTreeSet::new();
    found.retain(|w| seen.insert(w.clone()));
    found.extend(base.settings.weakenings(&head.settings));
    found.extend(env_weakenings(&base.env, &head.env));
    for w in head.no_proof.widenings(&base.no_proof) {
        let how = match w {
            Widening::Added(glob) => format!("it newly says {glob} needs no proof"),
            Widening::Dropped(glob) => format!(
                "it no longer takes {} back from needing no proof",
                glob.trim_start_matches('!')
            ),
            Widening::Reordered => "it reorders its globs so more files need no proof".into(),
        };
        found.push(("no_proof".into(), how));
    }
    found
}

/// How `head` makes checks run in another environment: any command, variable, cloned folder
/// or port source changed, since Fearlezz cannot tell which way that weakens them, and any
/// input dropped. Each as `env.<field>` and how.
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
    for input in base.inputs.iter().filter(|i| !head.inputs.contains(i)) {
        found.push(("env.inputs".into(), format!("it no longer has {input}")));
    }
    found
}
