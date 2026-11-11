//! The floor of a change's proof: the config of the commit where the branch left the base
//! branch. A change may add proof but never take it away, so a full run at the final stage
//! compares the change's config with that one, and every changed file must be read by a check
//! or declared by the base branch as needing no proof.

use anyhow::Result;

use crate::config::{self, Builtin, Config};
use crate::findings::Scope;
use crate::git::Git;
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

impl Floor {
    /// Compares the change's config with the base branch's. Without a base branch, or with
    /// no config on it, there is no floor and the change's own `no_proof` counts.
    pub(crate) fn read(git: &Git, head: &Config, wt: &Worktree) -> Result<Floor> {
        let base = match git.merge_base(&head.base) {
            Some(commit) => {
                let (base, _hold) = git.base_worktree(&commit)?;
                if base.root.join(config::FILE).exists() {
                    let config = Config::load(&base.root, &git.common)?;
                    Some((config, presets::local_pieces(&base.root)?))
                } else {
                    None
                }
            }
            None => None,
        };
        let mut config_files = vec![config::FILE.to_string(), presets::LOCK.to_string()];
        config_files.extend(presets::local_pieces(&wt.git.root)?);
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
            .filter(|p| !config_files.contains(p) && !crate::work::is_work_file(p))
            .filter(|p| !head.checks.iter().any(|c| c.reads(p)))
            .filter(|p| !floor.needs_no_proof(p))
            .cloned()
            .collect();
        Ok(Floor { weakened, unproven })
    }

    pub(crate) fn holds(&self) -> bool {
        self.weakened.is_empty() && self.unproven.is_empty()
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
                "unproven {path}: no check reads it and the base branch does not declare it in \
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
/// stage, reading fewer of `files` or no longer reporting findings, and new `no_proof` globs.
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
        match (&b.findings, &h.findings) {
            (Some(_), None) => say("it no longer reports findings".into()),
            (Some(bf), Some(hf)) => {
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
        if b.tests.is_some() && h.tests.is_none() {
            say("it no longer counts its tests".into());
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
    for glob in head.no_proof.iter().filter(|g| !base.no_proof.contains(g)) {
        found.push(("no_proof".into(), glob.clone()));
    }
    found
}
