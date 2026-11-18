//! The floor of a change's proof: the config of the commit where the branch left the base
//! branch. A change may add proof but never take it away, so a full run at the final stage
//! compares the change's config with that one. Proof is about behaviour, so the floor judges
//! the checks, never which files they read.

use anyhow::Result;
use fearlezz_proof::floor::{Change, PAST, Part, Weakening, names_diff};
use fearlezz_proof::review::Accept;

use crate::base::Bases;
use crate::config::{self, Builtin, Config, Env, Ports};
use crate::findings::Scope;
use crate::names::Named;
use crate::state::Worktree;

/// Where what a change weakens gets past the job that judges it.
pub(crate) enum Gate {
    /// Nowhere but the job: an admin merges past the required check.
    Job,
    /// The proof host took the run: an admin's accept on its page runs the job again.
    Host,
    /// The proof host took the run but could not be asked, or did not get its end, so no
    /// accept can reach it: the job is the gate, as without a host.
    Unasked,
    /// An admin accepted exactly this on the proof page.
    Accepted(Accept),
}

/// What a change takes away from the proof the base branch requires.
#[derive(Default)]
pub(crate) struct Floor {
    /// What weakened, one per part of the config, with every way it did.
    pub weakened: Vec<Weakening>,
}

impl Floor {
    /// Compares the change's config with the one on its base commit. Without a config there,
    /// there is no floor.
    pub(crate) fn read(head: &Config, wt: &Worktree, bases: &Bases) -> Result<Floor> {
        let opened = bases.at(&wt.git, &wt.base)?;
        let Some(base) = &opened.config else {
            return Ok(Floor::default());
        };
        let mut weakened: Vec<Weakening> = Vec::new();
        for (part, change) in weakenings(base, head, &wt.files) {
            let how = change.to_string();
            if let Some(w) = weakened.iter_mut().find(|w| w.part == part) {
                if !w.hows.contains(&how) {
                    w.hows.push(how);
                }
            } else {
                let path = part.path();
                weakened.push(Weakening {
                    was: base.part(&path),
                    now: head.part(&path),
                    part,
                    hows: vec![how],
                });
            }
        }
        Ok(Floor { weakened })
    }

    pub(crate) fn holds(&self) -> bool {
        self.weakened.is_empty()
    }

    /// Whether it holds at `gate`: nothing weakens, or an admin accepted what does.
    pub(crate) fn holds_at(&self, gate: &Gate) -> bool {
        self.holds() || matches!(gate, Gate::Accepted(_))
    }

    /// The lines a run prints: what weakens, and how it gets past `gate` or that it did.
    pub(crate) fn lines(&self, gate: &Gate) -> Vec<String> {
        let mut lines: Vec<String> = self
            .weakened
            .iter()
            .flat_map(|w| {
                w.hows
                    .iter()
                    .map(|how| format!("weakens {}: {how}", w.part.name()))
            })
            .collect();
        if !self.weakened.is_empty() {
            lines.push(match gate {
                Gate::Accepted(accept) => {
                    format!("  accepted by {} on the proof page", accept.by)
                }
                Gate::Host => "  fix: keep the proof the base branch requires; when it must \
                               change, an admin accepts it on the proof page, which runs this \
                               job again"
                    .into(),
                Gate::Job => format!("  fix: keep the proof the base branch requires; {PAST}"),
                Gate::Unasked => format!(
                    "  fix: the proof host could not be asked, so it fails as without a host: \
                     keep the proof the base branch requires; {PAST}"
                ),
            });
        }
        lines
    }
}

/// Whether `check` is the copy `each` made for a project folder that, in the change, is no
/// longer a project of that check (renamed, removed, or now a member of a workspace above it):
/// the check goes with its folder. Removing `each` itself, or one folder's check while its
/// folder stays, is gone.
fn gone_with_its_folder(base: &Config, head: &Config, check: &config::Check) -> bool {
    let folder = check.dir.as_deref().unwrap_or(".");
    base.fanned.iter().any(|(id, folders)| {
        let named = if folder == "." {
            check.id == *id
        } else {
            check.id == format!("{id}/{folder}")
        };
        named
            && folders.iter().any(|f| f == folder)
            && head
                .fanned
                .get(id)
                .is_some_and(|now| !now.iter().any(|f| f == folder))
    })
}

/// How `head` proves less than `base`: a check gone, running another command, at a later
/// stage, reporting or reading findings otherwise, taking fewer pictures (see
/// [`config::Pictures::weakenings`]), rules covering fewer of `files`, and new `no_proof` globs
/// or dropped negations, each as the part and how, in the order found. Which files a check
/// reads is only a speed hint, so it never weakens.
fn weakenings(base: &Config, head: &Config, files: &[String]) -> Vec<(Part, Change)> {
    // A scheduled check has a floor too: its run proves the default branch on a schedule.
    let mut found: Vec<(Part, Change)> = base
        .checks
        .iter()
        .flat_map(|b| check_weakenings(base, head, b, files))
        .collect();
    // Checks that share a lints table would name its weakenings once each.
    let mut seen = std::collections::BTreeSet::new();
    found.retain(|w: &(Part, Change)| seen.insert(w.clone()));
    found.extend(settings(base.settings.weakenings(&head.settings)));
    found.extend(settings(env_weakenings(&base.env, &head.env)));
    for b in &base.platforms {
        match head.platforms.iter().find(|h| h.name == b.name) {
            Some(h) if h.runs_on != b.runs_on || h.os != b.os || h.matrix != b.matrix => {
                found.push((
                    Part::Setting(format!("platform.{}", b.name)),
                    "it runs on another machine".into(),
                ));
            }
            _ => {}
        }
    }
    for name in head.no_proof.widenings(&base.no_proof) {
        found.push((Part::Setting("no_proof".into()), Change::Exempts(name)));
    }
    found
}

/// How `head` proves less than `base` of the base's check `b`, each as the part and how.
fn check_weakenings(
    base: &Config,
    head: &Config,
    b: &config::Check,
    files: &[String],
) -> Vec<(Part, Change)> {
    let mut found = Vec::new();
    let mut say = |how: Change| found.push((Part::Check(b.id.clone()), how));
    let Some(h) = head.checks.iter().find(|h| h.id == b.id) else {
        if !gone_with_its_folder(base, head, b) {
            say("it is gone".into());
        }
        return found;
    };
    if h.steps != b.steps {
        say("it runs another command".into());
    }
    for platform in b.on.iter().filter(|p| !h.on.contains(p)) {
        say(format!("it no longer runs on {platform}").into());
    }
    if h.schedule && !b.schedule {
        say("it runs on a schedule instead of for each change".into());
    }
    if h.dir != b.dir {
        let at = |dir: &Option<String>| dir.clone().unwrap_or_else(|| "the root".into());
        say(Change::replaced(
            "it runs in another folder",
            at(&b.dir),
            at(&h.dir),
        ));
    }
    let base_stage = base.stages.name(b.stage);
    if head.stages.find(base_stage).is_none_or(|s| h.stage > s) {
        say(format!("it moves to a later stage than {base_stage}").into());
    }
    for how in findings(b, h) {
        say(how);
    }
    // Leaving the simulator or a Linux display moves the check to a screen CI cannot
    // record (this machine's window) or another system's runner.
    if b.screen.owned() && std::mem::discriminant(&b.screen) != std::mem::discriminant(&h.screen) {
        say(Change::replaced(
            "it runs on another screen",
            b.screen.describe(),
            h.screen.describe(),
        ));
    }
    for how in b.pictures.weakenings(&h.pictures) {
        say(how);
    }
    if let Some(how) = h.flaky_tests.widened(&b.flaky_tests, "tests") {
        say(how);
    }
    let (bg, hg) = (b.generates.patterns(), h.generates.patterns());
    if !bg.is_empty() && !b.generates.widenings(&h.generates).is_empty() {
        say(if hg.is_empty() {
            format!("it no longer checks generated files ({})", bg.join(", ")).into()
        } else {
            Change::replaced(
                "it checks fewer generated files",
                bg.join(", "),
                hg.join(", "),
            )
        });
    }
    if b.tests.is_some() && h.tests.is_none() {
        say("it no longer counts its tests".into());
    }
    // Which files are tests decides what runs on the base commit, so any change to it
    // could hide a fix whose tests do not fail there. Naming them the first time is fine,
    // unless the check measures coverage, which never counts a test file.
    let measured = b.coverage.is_some() && h.coverage.is_some();
    if (measured || !b.test_files.patterns().is_empty())
        && h.test_files.patterns() != b.test_files.patterns()
    {
        say("it names other test files".into());
    }
    // What a check sees can change what it proves (a variable that skips the slow tests,
    // a key without which they skip themselves), so either list changing weakens it.
    for (how, was, now) in [
        ("it sees other environment variables", &b.env, &h.env),
        ("it gets other secrets", &b.secrets, &h.secrets),
    ] {
        if let Some(how) = Change::names(how, names_diff(was, now)) {
            say(how);
        }
    }
    if let (Some(bf), Some(hf)) = (&b.findings, &h.findings) {
        for tool in bf.tools.iter().filter(|t| hf.tools.contains(t)) {
            found.extend(settings(bf.levels.weakenings(&hf.levels, *tool)));
        }
    }
    found.extend(coverage(b, h));
    if let (Some(Builtin::Rules(br)), Some(Builtin::Rules(hr))) = (&b.builtin, &h.builtin) {
        found.extend(settings(br.weakenings(hr, files)));
    }
    found
}

/// How the head's check reports findings less than the base's: not at all, read another way,
/// without a lints table's levels, or only on changed lines.
fn findings(b: &config::Check, h: &config::Check) -> Vec<Change> {
    let mut found = Vec::new();
    match (&b.findings, &h.findings) {
        (Some(_), None) => found.push("it no longer reports findings".into()),
        (Some(bf), Some(hf)) => {
            if bf.read != hf.read {
                found.push(Change::replaced(
                    "its findings are read another way",
                    bf.read.name(),
                    hf.read.name(),
                ));
            }
            for tool in bf.tools.iter().filter(|t| !hf.tools.contains(t)) {
                found.push(
                    format!(
                        "its findings no longer get the levels of [lints.{}]",
                        tool.name()
                    )
                    .into(),
                );
            }
            if bf.on == Scope::Everywhere && hf.on == Scope::Changed {
                found.push("its findings count only on changed lines".into());
            }
        }
        (None, _) => {}
    }
    found
}

/// What the head's check measures of coverage less than the base's: none at all, another
/// report format, a lower target, or a lower level for `[lints.coverage]`.
fn coverage(b: &config::Check, h: &config::Check) -> Vec<(Part, Change)> {
    let part = || Part::Check(b.id.clone());
    match (&b.coverage, &h.coverage) {
        (Some(_), None) => vec![(part(), "it no longer measures coverage".into())],
        (Some(bc), Some(hc)) if bc.read.name() != hc.read.name() => vec![(
            part(),
            format!(
                "it reads its coverage as {} instead of {}",
                hc.read.name(),
                bc.read.name()
            )
            .into(),
        )],
        (Some(bc), Some(hc)) => (hc.target < bc.target)
            .then(|| {
                let how = Change::replaced(
                    "its coverage target is lower",
                    format!("{}%", bc.target),
                    format!("{}%", hc.target),
                );
                (part(), how)
            })
            .into_iter()
            .chain(settings(
                bc.levels
                    .weakenings(&hc.levels, crate::lints::Tool::Coverage),
            ))
            .collect(),
        (None, _) => Vec::new(),
    }
}

/// Weakenings of parts of the config other than a check, each as its path and how.
fn settings(found: Vec<(String, Change)>) -> impl Iterator<Item = (Part, Change)> {
    found
        .into_iter()
        .map(|(path, how)| (Part::Setting(path), how))
}

/// How `head` makes checks run in another environment: any command, variable, cloned folder
/// or port source changed, since Fearlezz cannot tell which way that weakens them. Each as
/// `env.<field>` and how.
fn env_weakenings(base: &Env, head: &Env) -> Vec<(String, Change)> {
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
        ("preview", base.preview.clone(), head.preview.clone()),
    ];
    let mut found = Vec::new();
    for (field, was, now) in commands {
        if let Some(how) = Change::set(was, now) {
            found.push((format!("env.{field}"), how));
        }
    }
    let lists = [
        ("clone", &base.clone, &head.clone),
        ("clone_when", &base.clone_when, &head.clone_when),
        ("env", &base.vars, &head.vars),
        ("secrets", &base.secrets, &head.secrets),
    ];
    for (field, was, now) in lists {
        if let Some(how) = Change::names("it changes", names_diff(was, now)) {
            found.push((format!("env.{field}"), how));
        }
    }
    found
}
