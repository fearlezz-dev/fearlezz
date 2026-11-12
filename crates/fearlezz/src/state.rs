//! Derives everything shown about a piece of work from its journal, the result store and the
//! worktree as it is now. Nothing here is stored, so every command agrees.

use std::collections::BTreeSet;

use anyhow::Result;
use jiff::Timestamp;

use crate::config::{Config, Stage};
use crate::git::Git;
use crate::journal::{Event, Journal};
use crate::store::{Hasher, Inputs, Outcome, Store};

/// The worktree a journal's branch is checked out in, as it is right now.
pub(crate) struct Worktree {
    pub git: Git,
    /// The base commit: where the branch left the base branch. Everything compared with the
    /// base (changed files, the floor, the look, red before green) is compared with this one.
    pub base: String,
    pub changed: BTreeSet<String>,
    /// The changed lines, for checks that report findings.
    pub lines: crate::findings::Changed,
    pub files: Vec<String>,
    pub tree: String,
    pub committed: bool,
    pub hasher: Hasher,
}

impl Worktree {
    pub(crate) fn read(git: Git, config: &Config) -> Result<Worktree> {
        let base = git.base_commit(&config.base)?;
        Ok(Worktree {
            changed: git.changed(&base)?,
            lines: git.changed_lines(&base)?,
            base,
            files: git.files()?,
            tree: git.tree()?,
            committed: git.committed()?,
            git,
            hasher: Hasher::default(),
        })
    }

    pub(crate) fn inputs(&mut self, check: &crate::config::Check) -> Result<Inputs> {
        // A check judged on changed lines gives another verdict when those lines change.
        let scope = check
            .findings
            .as_ref()
            .filter(|f| f.on == crate::findings::Scope::Changed)
            .map(|_| self.lines.digest(|p| check.reads(p)));
        self.hasher
            .inputs(&self.git.root, check, &self.files, scope.as_deref())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CheckState {
    Ok,
    Failed,
    /// Same inputs gave both outcomes.
    Unstable,
    /// Inputs changed since the last run; holds what changed.
    Stale(Vec<String>),
    /// Passed, but the final stage wants a fresh run of its own.
    NeedsFresh,
    /// Passed, but not yet compared with the base commit the work stands on now (its
    /// pictures, or red before green for a fix at the final stage).
    Uncompared,
    Never,
}

pub(crate) struct CheckView {
    pub id: String,
    pub stage: Stage,
    pub state: CheckState,
    /// How its stills and films compare with the base, for a passing run.
    pub look: Option<crate::look::Look>,
    /// The approval of its pictures as they are now.
    pub approved: Option<Approved>,
    /// The findings in scope of its latest run on these inputs.
    pub findings: Vec<crate::findings::Finding>,
    /// What red before green found for its passing run on these inputs, against this base.
    pub red: Option<crate::red::Mark>,
}

/// An approval of a check's changed pictures.
#[derive(Clone, Debug)]
pub(crate) struct Approved {
    /// The developer's words.
    pub said: String,
    /// Who approved on the pull request, as CI checked; none when the agent reported it.
    pub by: Option<String>,
}

impl Approved {
    /// For example `approved on GitHub by @lead ("ship it")`.
    pub(crate) fn label(&self) -> String {
        match &self.by {
            Some(by) => format!("approved on GitHub by @{by} (\"{}\")", self.said),
            None => format!("approved (\"{}\")", self.said),
        }
    }
}

impl CheckView {
    /// Changed pictures nobody approved yet.
    pub(crate) fn needs_approval(&self) -> bool {
        self.approved.is_none() && self.look.as_ref().is_some_and(|l| l.to_look_at() > 0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Group {
    NeedsYou,
    Failing,
    Active,
    Idle,
}

pub(crate) struct View {
    pub name: String,
    pub stage: Stage,
    /// False when something in the worktree is not committed; true when it is unknown.
    pub committed: bool,
    /// Why the work last went back a stage.
    pub went_back: Option<String>,
    pub checks: Vec<CheckView>,
    pub waiting: Option<String>,
    pub last_note: Option<(Timestamp, String)>,
    pub last_at: Option<Timestamp>,
    /// False when the branch is not checked out anywhere, so staleness is unknown.
    pub live: bool,
    pub group: Group,
    pub now: String,
    pub next: String,
}

impl Group {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Group::NeedsYou => "needs you",
            Group::Failing => "failing",
            Group::Active => "active",
            Group::Idle => "idle",
        }
    }
}

const ACTIVE_SECONDS: i64 = 30 * 60;

/// The next line once all proof is in at the final stage.
pub(crate) const READY: &str = "all proof is in: ready to merge";

/// The next line at the final stage when the change fails the floor of the base branch.
pub(crate) const FLOOR_NEXT: &str = "the change takes away proof the base branch requires \
                                     (above): restore it, or ask the developer whether to \
                                     merge past CI: fearlezz say ask \"<question>\"";

/// The next line after proving a commit, where no work is being judged.
pub(crate) const COMMIT_PROVEN: &str =
    "this proved the commit, not the work; fearlezz shows where the work stands";

pub(crate) fn view(
    config: &Config,
    store: &Store,
    journal: &Journal,
    worktree: Option<&mut Worktree>,
    now: Timestamp,
) -> Result<View> {
    let stages = &config.stages;
    let mut stage = stages.first();
    let mut went_back = None;
    let mut waiting = None;
    let mut last_note = None;
    let mut approvals: std::collections::BTreeMap<String, (String, Approved)> =
        std::collections::BTreeMap::default();
    for entry in &journal.entries {
        match &entry.event {
            Event::Stage { to, reason } => {
                if let Some(next) = stages.find(to) {
                    went_back = (next < stage).then(|| reason.clone()).flatten();
                    stage = next;
                }
                waiting = None;
            }
            Event::Ran { .. } => waiting = None,
            Event::Approve {
                check,
                key,
                said,
                by,
                ..
            } => {
                let approved = Approved {
                    said: said.clone(),
                    by: by.clone(),
                };
                // The agent's report of what the developer said never replaces an approval
                // a person gave on the pull request for the same inputs.
                let weaker = by.is_none()
                    && approvals
                        .get(check)
                        .is_some_and(|(k, a)| k == key && a.by.is_some());
                if !weaker {
                    approvals.insert(check.clone(), (key.clone(), approved));
                }
                waiting = None;
            }
            Event::Wait { question } => waiting = Some(question.clone()),
            Event::Note { text } => last_note = Some((entry.at, text.clone())),
            Event::Retired => {}
        }
    }
    let last_at = journal.entries.last().map(|e| e.at);
    let live = worktree.is_some();
    let final_stage = stages.last();

    let mut checks = Vec::new();
    let mut skill_out_of_date = false;
    let mut committed = true;
    let mut floor_fails = false;
    let mut red_next = None;
    if let Some(wt) = worktree {
        committed = wt.committed;
        // Only the final stage answers for what the base branch requires.
        if stage == final_stage && !wt.changed.is_empty() {
            floor_fails = !crate::floor::Floor::read(config, wt)?.holds();
        }
        // Asked once, and only when a check's red before green is in question.
        let mut fix: Option<bool> = None;
        skill_out_of_date = config.settings.skill_out_of_date(&wt.git.root)?;
        for check in &config.checks {
            let latest = latest_ran(journal, &check.id);
            // Any change needs every check; its inputs only say whether a pass still holds.
            if wt.changed.is_empty() && latest.is_none() {
                continue;
            }
            let inputs = wt.inputs(check)?;
            let passed = latest
                .as_ref()
                .filter(|ran| ran.key == inputs.key && ran.outcome == Outcome::Passed)
                .and_then(|ran| store.get(ran.key, ran.result));
            let compared = match &passed {
                Some(run) => compared(config, wt, check, run, stage == final_stage, &mut fix)?,
                None => Compared::default(),
            };
            let findings = latest
                .as_ref()
                .filter(|ran| ran.key == inputs.key)
                .and_then(|ran| store.get(ran.key, ran.result))
                .map(|r| r.findings)
                .unwrap_or_default();
            let state = match latest {
                None => CheckState::Never,
                Some(ran) if ran.key != inputs.key => {
                    CheckState::Stale(changed_parts(store, ran, &inputs))
                }
                Some(_) if store.unstable(&inputs.key) => CheckState::Unstable,
                Some(ran) if ran.outcome == Outcome::Failed => CheckState::Failed,
                Some(ran) => {
                    let fresh = store
                        .get(ran.key, ran.result)
                        .is_some_and(|r| r.made_at_final(&journal.name, stages.name(final_stage)));
                    if stage == final_stage && !fresh {
                        CheckState::NeedsFresh
                    } else if compared.missing {
                        CheckState::Uncompared
                    } else {
                        CheckState::Ok
                    }
                }
            };
            let approved = approvals
                .get(&check.id)
                .filter(|(key, _)| *key == inputs.key)
                .map(|(_, approved)| approved.clone());
            checks.push(CheckView {
                id: check.id.clone(),
                stage: check.stage,
                state,
                look: compared.look,
                approved,
                findings,
                red: compared.red,
            });
        }
        if stage == final_stage {
            red_next = red_hold(wt, &checks)?;
        }
    } else {
        for check in &config.checks {
            if let Some(ran) = latest_ran(journal, &check.id) {
                checks.push(CheckView {
                    id: check.id.clone(),
                    stage: check.stage,
                    state: match ran.outcome {
                        Outcome::Passed => CheckState::Ok,
                        Outcome::Failed => CheckState::Failed,
                    },
                    look: None,
                    approved: None,
                    findings: Vec::new(),
                    red: None,
                });
            }
        }
    }

    let mut view = View {
        name: journal.name.clone(),
        stage,
        committed,
        went_back,
        checks,
        waiting,
        last_note,
        last_at,
        live,
        group: Group::Idle,
        now: String::new(),
        next: String::new(),
    };
    let failing = view
        .checks
        .iter()
        .filter(|c| matches!(c.state, CheckState::Failed | CheckState::Unstable))
        .count();
    view.group = if view.waiting.is_some() {
        Group::NeedsYou
    } else if failing > 0 {
        Group::Failing
    } else if last_at.is_some_and(|t| now.as_second() - t.as_second() < ACTIVE_SECONDS) {
        Group::Active
    } else {
        Group::Idle
    };
    view.next = next(
        config,
        &view,
        skill_out_of_date,
        floor_fails,
        red_next.as_deref(),
    );
    view.now = now_line(config, &view);
    Ok(view)
}

pub(crate) struct Ran<'a> {
    pub key: &'a str,
    pub result: &'a str,
    pub outcome: Outcome,
}

fn latest_ran<'a>(journal: &'a Journal, check: &str) -> Option<Ran<'a>> {
    journal.entries.iter().rev().find_map(|e| match &e.event {
        Event::Ran {
            check: c,
            key,
            result,
            outcome,
            ..
        } if c == check => Some(Ran {
            key,
            result,
            outcome: *outcome,
        }),
        _ => None,
    })
}

fn changed_parts(store: &Store, ran: Ran<'_>, now: &Inputs) -> Vec<String> {
    let Some(before) = store.get(ran.key, ran.result) else {
        return vec!["its inputs".into()];
    };
    let mut changed: Vec<String> = now
        .parts
        .iter()
        .filter(|(k, v)| before.parts.get(*k) != Some(v))
        .map(|(k, _)| k.clone())
        .chain(
            before
                .parts
                .keys()
                .filter(|k| !now.parts.contains_key(*k))
                .cloned(),
        )
        .collect();
    if changed.is_empty() {
        changed.push("its command".into());
    }
    changed
}

/// How a passing run compares with the base commit the work stands on now.
#[derive(Default)]
struct Compared {
    look: Option<crate::look::Look>,
    red: Option<crate::red::Mark>,
    /// A comparison it needs is missing: its pictures, or red before green for a fix at the
    /// final stage.
    missing: bool,
}

/// How `run`, a passing run of `check`, compares with the base commit of `wt`. `fix` caches
/// whether the change is a fix, asked only when red before green is in question.
fn compared(
    config: &Config,
    wt: &Worktree,
    check: &crate::config::Check,
    run: &crate::store::RunResult,
    at_final: bool,
    fix: &mut Option<bool>,
) -> Result<Compared> {
    let against = crate::store::AgainstBase::of(run, &wt.base).unwrap_or_default();
    let look = match &against.look {
        Some(crate::store::Looked::At(dir)) => crate::look::Look::at(dir),
        _ => None,
    };
    let look_missing = match &against.look {
        None => true,
        Some(crate::store::Looked::At(_)) => look.is_none(),
        Some(crate::store::Looked::Nothing) => false,
    };
    let wants_look = crate::look::has_media(run) || config.takes_pictures(check);
    let wants_red = at_final
        && against.red.is_none()
        && !crate::red::test_files(check, wt).is_empty()
        && match *fix {
            Some(fix) => fix,
            None => *fix.insert(crate::red::is_fix_change(&wt.git, &wt.base)?),
        };
    Ok(Compared {
        look,
        // A look recorded but gone or unreadable is missing, so the next run makes it again
        // rather than the check reading ok with nothing to approve.
        missing: (wants_look && look_missing) || wants_red,
        red: against.red,
    })
}

/// What holds a fix at the final stage: a check whose run on these inputs red before green
/// marked, against this base commit, as not showing the fix. None when the change is no fix.
/// A check not yet compared with this base is [`CheckState::Uncompared`], so it needs a run.
fn red_hold(wt: &Worktree, checks: &[CheckView]) -> Result<Option<String>> {
    let held: Vec<(&str, &crate::red::Mark)> = checks
        .iter()
        .filter(|c| c.state == CheckState::Ok)
        .filter_map(|c| Some((c.id.as_str(), c.red.as_ref()?)))
        .filter(|(_, mark)| mark.unshown.is_some())
        .collect();
    if held.is_empty() || !crate::red::is_fix_change(&wt.git, &wt.base)? {
        return Ok(None);
    }
    Ok(held.into_iter().find_map(|(id, mark)| mark.next(id)))
}

/// What the agent should do next, as one line. Order: the developer, the agent skill, failures,
/// needed runs, what the base branch requires, moving on.
fn next(
    config: &Config,
    v: &View,
    skill_out_of_date: bool,
    floor_fails: bool,
    red: Option<&str>,
) -> String {
    let stages = &config.stages;
    if let Some(q) = &v.waiting {
        return format!("waiting for the developer: {q}");
    }
    if skill_out_of_date {
        return "write the agent skill for this Fearlezz version: fearlezz setup sync, then \
                commit it"
            .into();
    }
    let due: Vec<&CheckView> = v.checks.iter().filter(|c| c.stage <= v.stage).collect();
    if let Some(c) = due
        .iter()
        .find(|c| matches!(c.state, CheckState::Failed | CheckState::Unstable))
    {
        // The same inputs stay unstable, so only a change to them can clear it.
        return if c.state == CheckState::Unstable {
            format!(
                "make {0} stable (fix the flake, or add the input it misses to its inputs), then \
                 fearlezz run {0}",
                c.id
            )
        } else {
            format!("fix {0}, then fearlezz run {0}", c.id)
        };
    }
    let todo: Vec<&str> = due
        .iter()
        .filter(|c| c.state != CheckState::Ok)
        .map(|c| c.id.as_str())
        .collect();
    if !todo.is_empty() {
        return format!(
            "fearlezz run   ({} needed: {})",
            todo.len(),
            todo.join(", ")
        );
    }
    if floor_fails {
        return FLOOR_NEXT.into();
    }
    if v.checks.is_empty() {
        return "make a change; fearlezz --plan shows the proof it will need".into();
    }
    if let Some(l) = due
        .iter()
        .find(|c| c.needs_approval())
        .and_then(|c| c.look.as_ref())
    {
        return format!(
            "look at {0} ({1}): {2}; when the developer agrees: fearlezz say approve {0} --said \
             \"<their words>\", or once they approved on the pull request: fearlezz say approve \
             {0} --github",
            l.check,
            l.counts(),
            l.report.display(),
        );
    }
    if let Some(line) = red {
        return line.to_string();
    }
    match stages.after(v.stage) {
        Some(s) => format!("when ready: fearlezz run --stage {}", stages.name(s)),
        // Proof counts only for what is committed.
        None if !v.committed => "commit the work: ready means everything is committed".into(),
        None => READY.into(),
    }
}

fn now_line(config: &Config, v: &View) -> String {
    if let Some(q) = &v.waiting {
        return format!("Waiting for you: {q}");
    }
    let bad: Vec<String> = v
        .checks
        .iter()
        .filter_map(|c| match c.state {
            CheckState::Failed => Some(format!("{} failed", c.id)),
            CheckState::Unstable => Some(format!("{} unstable", c.id)),
            _ => None,
        })
        .collect();
    if !bad.is_empty() {
        return bad.join(", ");
    }
    let ok = v
        .checks
        .iter()
        .filter(|c| c.state == CheckState::Ok)
        .count();
    format!(
        "in {}: {ok} of {} checks ok",
        config.stages.name(v.stage),
        v.checks.len()
    )
}
