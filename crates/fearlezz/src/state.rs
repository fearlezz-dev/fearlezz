//! Derives everything shown about a piece of work from its journal, the result store and the
//! worktree as it is now. Nothing here is stored, so every command agrees.

use std::collections::BTreeSet;

use anyhow::Result;
use jiff::Timestamp;

use crate::config::{Config, Stage};
use crate::git::Git;
use crate::journal::{Event, Journal, Kind};
use crate::store::{Hasher, Inputs, Outcome, Store};

/// The worktree a journal's branch is checked out in, as it is right now.
pub(crate) struct Worktree {
    pub git: Git,
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
        Ok(Worktree {
            changed: git.changed(&config.base)?,
            lines: git.changed_lines(&config.base)?,
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
    Never,
}

pub(crate) struct CheckView {
    pub id: String,
    pub stage: Stage,
    /// The change touched one of its inputs.
    pub needed: bool,
    pub state: CheckState,
    /// How its stills and films compare with the base, for a passing run.
    pub look: Option<crate::look::Look>,
    /// What the developer said when approving its pictures as they are now.
    pub approved: Option<String>,
    /// The findings in scope of its latest run on these inputs.
    pub findings: Vec<crate::findings::Finding>,
}

impl CheckView {
    /// Changed pictures nobody approved yet.
    pub(crate) fn needs_approval(&self) -> bool {
        self.approved.is_none() && self.look.as_ref().is_some_and(|l| l.to_look_at() > 0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClaimState {
    NoProof,
    Unproven,
    NeedsRedFirst,
    Proven,
}

pub(crate) struct ClaimView {
    pub id: String,
    pub text: String,
    pub check: Option<String>,
    pub state: ClaimState,
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
    pub kind: Option<Kind>,
    pub goal: Option<String>,
    pub stage: Stage,
    /// False when something in the worktree is not committed; true when it is unknown.
    pub committed: bool,
    /// Why the work last went back a stage.
    pub went_back: Option<String>,
    pub checks: Vec<CheckView>,
    pub claims: Vec<ClaimView>,
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

/// The next line after proving a commit, where no work is being judged.
pub(crate) const COMMIT_PROVEN: &str =
    "this proved the commit, not the work; fearlezz shows where the work stands";

const SAY_START: &str = "say what this work is: fearlezz say start --kind \
                         bug|feature|refactor|other --goal \"<the goal in the developer's words>\"";

pub(crate) fn view(
    config: &Config,
    store: &Store,
    journal: &Journal,
    worktree: Option<&mut Worktree>,
    now: Timestamp,
) -> Result<View> {
    let stages = &config.stages;
    let mut kind = None;
    let mut goal = None;
    let mut stage = stages.first();
    let mut went_back = None;
    let mut waiting = None;
    let mut last_note = None;
    let mut claims: Vec<(String, String, Option<String>)> = Vec::new();
    let mut approvals: std::collections::BTreeMap<String, (String, String)> =
        std::collections::BTreeMap::default();
    for entry in &journal.entries {
        match &entry.event {
            Event::Started { kind: k, goal: g } => {
                kind = Some(*k);
                goal.clone_from(g);
            }
            Event::Claim { id, text, check } => {
                claims.push((id.clone(), text.clone(), check.clone()));
                waiting = None;
            }
            Event::Link { claim, check } => {
                if let Some(c) = claims.iter_mut().find(|c| &c.0 == claim) {
                    c.2 = Some(check.clone());
                }
                waiting = None;
            }
            Event::Stage { to, reason } => {
                if let Some(next) = stages.find(to) {
                    went_back = (next < stage).then(|| reason.clone()).flatten();
                    stage = next;
                }
                waiting = None;
            }
            Event::Ran { .. } => waiting = None,
            Event::Approve { check, key, said } => {
                approvals.insert(check.clone(), (key.clone(), said.clone()));
                waiting = None;
            }
            Event::Wait { question } => waiting = Some(question.clone()),
            Event::Note { text } => last_note = Some((entry.at, text.clone())),
        }
    }
    let last_at = journal.entries.last().map(|e| e.at);
    let live = worktree.is_some();
    let final_stage = stages.last();

    let mut checks = Vec::new();
    let mut tree = None;
    let mut skill_out_of_date = false;
    let mut committed = true;
    if let Some(wt) = worktree {
        committed = wt.committed;
        skill_out_of_date = config.settings.skill_out_of_date(&wt.git.root)?;
        tree = Some(wt.tree.clone());
        for check in &config.checks {
            let needed = wt.changed.iter().any(|p| check.reads(p));
            let latest = latest_ran(journal, &check.id);
            if !needed && latest.is_none() {
                continue;
            }
            let inputs = wt.inputs(check)?;
            let look = latest
                .as_ref()
                .filter(|ran| ran.key == inputs.key && ran.outcome == Outcome::Passed)
                .and_then(|ran| store.get(ran.key, ran.result))
                .and_then(|r| crate::look::Look::of(&r));
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
                    } else {
                        CheckState::Ok
                    }
                }
            };
            let approved = approvals
                .get(&check.id)
                .filter(|(key, _)| *key == inputs.key)
                .map(|(_, said)| said.clone());
            checks.push(CheckView {
                id: check.id.clone(),
                stage: check.stage,
                needed,
                state,
                look,
                approved,
                findings,
            });
        }
    } else {
        for check in &config.checks {
            if let Some(ran) = latest_ran(journal, &check.id) {
                checks.push(CheckView {
                    id: check.id.clone(),
                    stage: check.stage,
                    needed: true,
                    state: match ran.outcome {
                        Outcome::Passed => CheckState::Ok,
                        Outcome::Failed => CheckState::Failed,
                    },
                    look: None,
                    approved: None,
                    findings: Vec::new(),
                });
            }
        }
    }

    let claims = claims
        .into_iter()
        .map(|(id, text, check)| {
            let state = claim_state(journal, check.as_deref(), tree.as_deref(), kind);
            ClaimView {
                id,
                text,
                check,
                state,
            }
        })
        .collect();

    let mut view = View {
        name: journal.name.clone(),
        kind,
        goal,
        stage,
        committed,
        went_back,
        checks,
        claims,
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
    view.next = next(config, &view, skill_out_of_date);
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

/// A claim is proven by a green run of its check on the tree as it is now. A bug claim also
/// needs that check to have been red before.
fn claim_state(
    journal: &Journal,
    check: Option<&str>,
    tree: Option<&str>,
    kind: Option<Kind>,
) -> ClaimState {
    let Some(check) = check else {
        return ClaimState::NoProof;
    };
    let runs: Vec<(Outcome, &str)> = journal
        .entries
        .iter()
        .filter_map(|e| match &e.event {
            Event::Ran {
                check: c,
                outcome,
                tree,
                ..
            } if c == check => Some((*outcome, tree.as_str())),
            _ => None,
        })
        .collect();
    let green_now = runs
        .iter()
        .rposition(|(o, t)| *o == Outcome::Passed && tree.is_none_or(|now| now == *t));
    let Some(green) = green_now else {
        return ClaimState::Unproven;
    };
    if kind == Some(Kind::Bug) && !runs[..green].iter().any(|(o, _)| *o == Outcome::Failed) {
        return ClaimState::NeedsRedFirst;
    }
    ClaimState::Proven
}

/// What the agent should do next, as one line. Order: the developer, the agent skill, failures,
/// needed runs, claims, moving on.
fn next(config: &Config, v: &View, skill_out_of_date: bool) -> String {
    let stages = &config.stages;
    if let Some(q) = &v.waiting {
        return format!("waiting for the developer: {q}");
    }
    if skill_out_of_date {
        return "write the agent skill for this Fearlezz version: fearlezz setup sync, then \
                commit it"
            .into();
    }
    let due: Vec<&CheckView> = v
        .checks
        .iter()
        .filter(|c| c.needed && c.stage <= v.stage)
        .collect();
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
    if v.stage > stages.first() || stages.after(v.stage).is_none() {
        for c in &v.claims {
            match c.state {
                ClaimState::NoProof => {
                    return format!("link a check to {0}: fearlezz say link {0} <check>", c.id);
                }
                ClaimState::Unproven => {
                    return format!(
                        "prove {}: fearlezz run {}",
                        c.id,
                        c.check.as_deref().unwrap_or("<check>")
                    );
                }
                ClaimState::NeedsRedFirst => {
                    return format!(
                        "show {} failing before the fix (a bug needs red, then green)",
                        c.id
                    );
                }
                ClaimState::Proven => {}
            }
        }
    }
    if v.checks.is_empty() {
        return if v.kind.is_none() {
            SAY_START.into()
        } else {
            "make a change; fearlezz --plan shows the proof it will need".into()
        };
    }
    if let Some(l) = due
        .iter()
        .find(|c| c.needs_approval())
        .and_then(|c| c.look.as_ref())
    {
        return format!(
            "look at {} ({}): {}; when the developer agrees: fearlezz say approve {} --said \"<their words>\"",
            l.check,
            l.counts(),
            l.report.display(),
            l.check
        );
    }
    match stages.after(v.stage) {
        Some(s) => format!("when ready: fearlezz run --stage {}", stages.name(s)),
        None => missing_for_ready(v).unwrap_or_else(|| READY.into()),
    }
}

/// What the work still lacks, beyond its checks, to be called ready: proof needs a goal and
/// something to prove, and it counts only for what is committed.
fn missing_for_ready(v: &View) -> Option<String> {
    if v.goal.is_none() {
        return Some(SAY_START.into());
    }
    if v.claims.is_empty() {
        return Some(
            "say what this work must prove: fearlezz say claim \"<behavior>\" --check <check>"
                .into(),
        );
    }
    (!v.committed).then(|| "commit the work: ready means everything is committed".into())
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
    let proven = v
        .claims
        .iter()
        .filter(|c| c.state == ClaimState::Proven)
        .count();
    let mut line = format!(
        "in {}: {ok} of {} checks ok",
        config.stages.name(v.stage),
        v.checks.len()
    );
    if !v.claims.is_empty() {
        line = format!("{line}, {proven} of {} claims proven", v.claims.len());
    }
    line
}
