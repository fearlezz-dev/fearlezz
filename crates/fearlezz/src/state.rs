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
    /// What of the `[env]` its checks run in shapes their results (its shell, services and
    /// preview): the change's own for the change's worktree, the base commit's for a base
    /// checkout (see `base::Checkout`).
    pub env: crate::config::EnvKey,
    pub hasher: Hasher,
}

impl Worktree {
    /// The worktree `git` names, whose checks run in `config`'s `[env]`.
    pub(crate) fn read(git: Git, config: &Config) -> Result<Worktree> {
        Worktree::read_in(git, config, &config.env)
    }

    /// The worktree `git` names, compared with `config`'s base branch, whose checks run in
    /// `env`.
    pub(crate) fn read_in(git: Git, config: &Config, env: &crate::config::Env) -> Result<Worktree> {
        let base = git.base_commit(&config.base)?;
        Ok(Worktree {
            changed: git.changed(&base)?,
            lines: git.changed_lines(&base)?,
            base,
            files: git.files()?,
            tree: git.tree()?,
            committed: git.committed()?,
            git,
            env: env.key(),
            hasher: Hasher::default(),
        })
    }

    pub(crate) fn inputs(&mut self, check: &crate::config::Check) -> Result<Inputs> {
        // A check judged on changed lines gives another verdict when those lines change.
        let judged = check
            .findings
            .as_ref()
            .is_some_and(|f| f.on == crate::findings::Scope::Changed);
        let scope =
            (judged || check.coverage.is_some()).then(|| self.lines.digest(|p| check.reads(p)));
        self.hasher.inputs(
            &self.git.root,
            check,
            &self.env,
            &self.files,
            scope.as_deref(),
        )
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
    /// How its stills and films compare with the base, for a passing run. A person reviews
    /// what changed on the pull request; it never holds the work.
    pub look: Option<crate::look::Look>,
    /// The findings in scope of its latest run on these inputs.
    pub findings: Vec<crate::findings::Finding>,
    /// What red before green found for its passing run on these inputs, against this base.
    pub red: Option<crate::red::Mark>,
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
    pub next: Next,
}

impl View {
    /// At the final stage, how many pictures differ from the base and in which checks, for a
    /// person to review on the pull request. They never hold the work, so this only informs.
    /// Flaky pictures do not count. None at other stages or when nothing changed.
    pub(crate) fn pictures_to_review(&self, config: &Config) -> Option<String> {
        if self.stage != config.stages.last() {
            return None;
        }
        let looks: Vec<&crate::look::Look> = self
            .checks
            .iter()
            .filter_map(|c| c.look.as_ref())
            .filter(|l| l.to_look_at() > 0)
            .collect();
        let count: usize = looks.iter().map(|l| l.to_look_at()).sum();
        let checks: Vec<&str> = looks.iter().map(|l| l.check.as_str()).collect();
        let noun = if count == 1 { "picture" } else { "pictures" };
        (count > 0).then(|| {
            format!(
                "{count} {noun} changed ({}): a person reviews them on the pull request",
                checks.join(", ")
            )
        })
    }
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

/// What the agent should do next: the `next:` line every command ends with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Next {
    /// The developer has a question to answer.
    Waiting(String),
    /// The agent skill is older than this Fearlezz.
    Sync,
    /// The check gave passed and failed on the same inputs.
    Stabilize(String),
    /// The check failed.
    Fix(String),
    /// These checks need a run.
    Run(Vec<String>),
    /// The change takes away proof the base branch requires.
    Floor,
    /// Nothing changed yet.
    Change,
    /// Red before green holds a fix: what to do, as [`crate::red::Mark::next`] says it.
    Red(String),
    /// The stage is done; the next one is called this.
    WhenReady(String),
    /// Ready but for work that is not committed.
    Commit,
    /// All proof is in at the final stage.
    Ready,
    /// A run proved a commit, where no work is judged.
    CommitProven,
    /// Every check waits in the stage called this.
    Stage(String),
    /// A check could not record: its recorder lacks a permission or a tool.
    Recorder,
}

impl std::fmt::Display for Next {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Next::Waiting(q) => write!(f, "waiting for the developer: {q}"),
            Next::Sync => f.write_str(
                "write the agent skill for this Fearlezz version: fearlezz setup sync, then \
                 commit it",
            ),
            // The same inputs stay unstable, so only a change to them can clear it.
            Next::Stabilize(id) => write!(
                f,
                "make {id} stable (fix the flake, or add the input it misses to its inputs), \
                 then fearlezz run {id}"
            ),
            Next::Fix(id) => write!(f, "fix {id}, then fearlezz run {id}"),
            Next::Run(todo) => write!(
                f,
                "fearlezz run   ({} needed: {})",
                todo.len(),
                todo.join(", ")
            ),
            Next::Floor => f.write_str(
                "the change takes away proof the base branch requires (above): restore it, or \
                 ask the developer whether a person with admin rights should merge past the \
                 required check: fearlezz say ask \"<question>\"",
            ),
            Next::Change => {
                f.write_str("make a change; fearlezz --plan shows the proof it will need")
            }
            Next::Red(line) => f.write_str(line),
            Next::WhenReady(stage) => write!(f, "when ready: fearlezz run --stage {stage}"),
            // Proof counts only for what is committed.
            Next::Commit => f.write_str("commit the work: ready means everything is committed"),
            Next::Ready => f.write_str("all proof is in: ready to merge"),
            Next::CommitProven => f.write_str(
                "this proved the commit, not the work; fearlezz shows where the work stands",
            ),
            Next::Stage(stage) => write!(f, "fearlezz run --stage {stage}"),
            Next::Recorder => f.write_str(
                "give the recorder its permission or install its tool (fix: above), then \
                 fearlezz run",
            ),
        }
    }
}

pub(crate) fn view(
    config: &Config,
    store: &Store,
    journal: &Journal,
    worktree: Option<&mut Worktree>,
    bases: &crate::base::Bases,
    now: Timestamp,
) -> Result<View> {
    let stages = &config.stages;
    let mut stage = stages.first();
    let mut went_back = None;
    let mut waiting = None;
    let mut last_note = None;
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
            floor_fails = !crate::floor::Floor::read(config, wt, bases)?.holds();
        }
        // Asked once, and only when a check's red before green is in question.
        let mut fix: Option<bool> = None;
        skill_out_of_date = config.settings.skill_out_of_date(&wt.git.root)?;
        // A check this machine cannot stand in for is left to CI: it never holds the work.
        let here = config
            .due()
            .filter(|c| crate::legs::left_to_ci(config, c).is_none());
        for check in here {
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
            checks.push(CheckView {
                id: check.id.clone(),
                stage: check.stage,
                state,
                look: compared.look,
                findings,
                red: compared.red,
            });
        }
        if stage == final_stage {
            red_next = red_hold(wt, &checks)?;
        }
    } else {
        for check in config.due() {
            if let Some(ran) = latest_ran(journal, &check.id) {
                checks.push(CheckView {
                    id: check.id.clone(),
                    stage: check.stage,
                    state: match ran.outcome {
                        Outcome::Passed => CheckState::Ok,
                        Outcome::Failed => CheckState::Failed,
                    },
                    look: None,
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
        next: Next::Change,
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
    view.next = next(config, &view, skill_out_of_date, floor_fails, red_next);
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
        Some(crate::store::Looked::At(dir)) => crate::look::saved(dir),
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
        // rather than the check reading ok with no pictures to show.
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

/// What the agent should do next. Order: the developer, the agent skill, failures, needed
/// runs, what the base branch requires, moving on.
fn next(
    config: &Config,
    v: &View,
    skill_out_of_date: bool,
    floor_fails: bool,
    red: Option<String>,
) -> Next {
    let stages = &config.stages;
    if let Some(q) = &v.waiting {
        return Next::Waiting(q.clone());
    }
    if skill_out_of_date {
        return Next::Sync;
    }
    let due: Vec<&CheckView> = v.checks.iter().filter(|c| c.stage <= v.stage).collect();
    if let Some(c) = due
        .iter()
        .find(|c| matches!(c.state, CheckState::Failed | CheckState::Unstable))
    {
        return if c.state == CheckState::Unstable {
            Next::Stabilize(c.id.clone())
        } else {
            Next::Fix(c.id.clone())
        };
    }
    let todo: Vec<String> = due
        .iter()
        .filter(|c| c.state != CheckState::Ok)
        .map(|c| c.id.clone())
        .collect();
    if !todo.is_empty() {
        return Next::Run(todo);
    }
    if floor_fails {
        return Next::Floor;
    }
    if v.checks.is_empty() {
        return Next::Change;
    }
    if let Some(line) = red {
        return Next::Red(line);
    }
    match stages.after(v.stage) {
        Some(s) => Next::WhenReady(stages.name(s).to_string()),
        None if !v.committed => Next::Commit,
        None => Next::Ready,
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
