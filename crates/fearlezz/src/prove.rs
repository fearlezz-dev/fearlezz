//! Proving checks: a run of the checks a piece of work or a commit needs, in the change's
//! worktree ([`run`]), and one check where it runs ([`prove`]), the change's worktree or a
//! checkout of its base commit. Both sites take what a check needs in one fixed order (the
//! services' hold before a slot of the heavy queue, then the checkout prepared once, its
//! services, its preview, and the command with its secrets inside [`run::execute`]), and both
//! ask one [`Reuse`] whether a result for the same inputs answers instead.

use std::collections::BTreeMap;
use std::time::Instant;

use anyhow::{Result, bail};
use fearlezz_proof::why::{Generated, Why};

use crate::base::{Base, Checkout};
use crate::config::{Check, Env};
use crate::env::{EnvCtx, Hold};
use crate::journal::{Event, Journal};
use crate::proof::{self, Away};
use crate::queue::{self, Turn};
use crate::report::{self, Came, CiOut, Report};
use crate::state::{Next, View, Worktree};
use crate::store::{self, Inputs, Outcome, RunResult, Store};
use crate::{Ctx, floor, legs, look, record, red, run};

/// The journal name base runs are recorded under.
const BASE_JOURNAL: &str = "(base)";

/// Which stored result for a check's inputs answers instead of running it.
pub(crate) enum Reuse<'a> {
    /// None: `--fresh` runs every check.
    Never,
    /// The latest result, when it passed and its inputs are not unstable. At the final stage
    /// (`at_final` names the journal and that stage) only a pass that journal made there, so
    /// the proof before a merge is fresh.
    Pass {
        at_final: Option<(&'a str, &'a str)>,
    },
    /// The latest result that got going, passed or failed: what the base commit gives is
    /// compared either way. One that could not find its command or get a secret may go
    /// another way the next time.
    Ran,
}

impl Reuse<'_> {
    pub(crate) fn found(&self, store: &Store, key: &str) -> Option<RunResult> {
        match self {
            Reuse::Never => None,
            Reuse::Pass { .. } if store.unstable(key) => None,
            Reuse::Pass { at_final } => store.for_key(key).pop().filter(|r| {
                r.outcome == Outcome::Passed
                    && at_final.is_none_or(|(journal, stage)| r.made_at_final(journal, stage))
            }),
            Reuse::Ran => store
                .for_key(key)
                .into_iter()
                .rev()
                .find(|r| !r.proves_nothing()),
        }
    }
}

/// A checkout a check runs in.
pub(crate) struct Site<'a> {
    pub wt: &'a Worktree,
    /// The `[env]` that prepares it, runs its services and deploys its preview.
    pub env: &'a Env,
    pub stage: &'a str,
    /// Whether it was prepared in this command yet.
    pub prepared: &'a mut bool,
    pub of: Of<'a>,
}

/// Whose checkout it is.
pub(crate) enum Of<'a> {
    /// The change's worktree. Its services stay up between checks, held from the first check
    /// that needs them to the end of the run, and restart fresh at the final stage; its
    /// preview is named by `branch`.
    Change {
        journal: &'a str,
        fresh: bool,
        branch: Option<&'a str>,
        held: &'a mut Option<Hold>,
        /// When the run started: services count as idle from when a check stopped using them.
        started: Instant,
    },
    /// A checkout of the base commit, by the base commit's own `[env]`: its services last one
    /// run, its preview is named by its tree, and what it says starts with `base`.
    Base,
}

impl<'a> Site<'a> {
    /// A checkout of `base` for one check at `stage`.
    pub(crate) fn base(base: &'a Base, checkout: &'a mut Checkout, stage: &'a str) -> Site<'a> {
        Site {
            wt: &checkout.wt,
            env: base.env(),
            stage,
            prepared: &mut checkout.prepared,
            of: Of::Base,
        }
    }

    fn say(&self, line: &str) {
        let base = if matches!(self.of, Of::Base) {
            "base "
        } else {
            ""
        };
        println!("{base}{line}");
    }
}

/// The result of `check` on `inputs` at `site`, and whether it was reused: found by `reuse`
/// before or while it waits for the heavy queue, or run.
pub(crate) fn prove(
    store: &Store,
    env: &EnvCtx<'_>,
    check: &Check,
    inputs: &Inputs,
    mut site: Site<'_>,
    reuse: &Reuse<'_>,
) -> Result<(RunResult, bool)> {
    if let Some(found) = reuse.found(store, &inputs.key) {
        return Ok((found, true));
    }
    // The worktree's services (which its preview deploy shares) before a slot, always: a run
    // waiting for them holds no slot, so the run holding them can always get one.
    let root = &site.wt.git.root;
    if let Of::Change { held, .. } = &mut site.of
        && held.is_none()
        && (check.needs.services || check.needs.preview)
    {
        **held = Some(if let Some(hold) = env.state.try_hold(root)? {
            hold
        } else {
            eprintln!(
                "wait {}: another run in this worktree holds its services; it goes on once that \
                 run ends",
                check.id
            );
            env.state.hold(root)?
        });
    }
    // Heavy work waits its turn on this machine, and another worktree may prove the same
    // inputs meanwhile. The slot is freed once the check's command ends.
    let label = match site.of {
        Of::Base => format!("{} on the base commit", check.id),
        Of::Change { .. } => check.id.clone(),
    };
    let _slot = if check.light {
        None
    } else {
        match queue::turn(&label, || reuse.found(store, &inputs.key))? {
            Turn::Go(slot) => slot,
            Turn::Done(found) => return Ok((found, true)),
        }
    };
    Ok((run_at(store, env, check, inputs, &mut site)?, false))
}

/// Runs `check` at `site`, with the services and preview it needs.
fn run_at(
    store: &Store,
    env: &EnvCtx<'_>,
    check: &Check,
    inputs: &Inputs,
    site: &mut Site<'_>,
) -> Result<RunResult> {
    let root = &site.wt.git.root;
    if !*site.prepared {
        if let Some(line) = env.state.prepare(site.env, &env.main, root)? {
            site.say(&line);
        }
        *site.prepared = true;
    }
    let (journal, fresh) = match &site.of {
        Of::Change { journal, fresh, .. } => (*journal, *fresh),
        Of::Base => (BASE_JOURNAL, false),
    };
    let mut vars = BTreeMap::new();
    if check.needs.services {
        let (services, line) = env
            .state
            .ensure_up(site.env, root, journal, fresh, env.now)?;
        if let Some(line) = line {
            site.say(&line);
        }
        vars = services.ports;
    }
    let mut undeployed = None;
    if check.needs.preview {
        // A base checkout is named by its tree: a red before green checkout of the same
        // commit holds the change's tests, so it is another deploy. Base checkouts are shared
        // by worktrees, so the deploy holds the checkout.
        let (name, _held) = match &site.of {
            Of::Change { branch, .. } => (
                crate::env::preview_name(*branch, &site.wt.git.head_short()?),
                None,
            ),
            Of::Base => (
                format!("base-{}", site.wt.tree.get(..7).unwrap_or(&site.wt.tree)),
                Some(env.state.hold(root)?),
            ),
        };
        match env.state.preview(site.env, root, &site.wt.tree, &name)? {
            Ok(deployed) => {
                if let Some(line) = deployed.line {
                    site.say(&line);
                }
                vars.insert("FEARLEZZ_PREVIEW_URL".to_string(), deployed.url);
            }
            Err(u) => undeployed = Some(u),
        }
    }
    let at = run::Where {
        root,
        shell: site.wt.env.shell.as_deref(),
        tree: &site.wt.tree,
        journal,
        stage: site.stage,
        vars: &vars,
        changed: &site.wt.lines,
        no_preview: undeployed.as_ref(),
    };
    let result = run::execute(store, check, inputs, &at);
    let after = || -> Result<()> {
        match &site.of {
            _ if !check.needs.services => Ok(()),
            // Idle time counts from when the check stopped using them.
            Of::Change { started, .. } => {
                let took = jiff::SignedDuration::try_from(started.elapsed())?;
                env.state.touch(root, env.now.checked_add(took)?)
            }
            Of::Base => env.state.stop(site.env, root),
        }
    };
    let after = after();
    let result = result?;
    after?;
    Ok(result)
}

/// What a run answers for, stated once.
#[derive(Clone, Copy)]
struct Mode {
    /// Proving a commit for CI, in a journal of its own, not a piece of work.
    commit: bool,
    /// Every check due, not only the ones named.
    whole: bool,
    /// At the final stage, where a pass must be made there, services restart fresh, red
    /// before green runs and the floor of the base branch counts. Proving a commit always is.
    last: bool,
}

impl Mode {
    /// A whole run at the final stage answers for what the base branch requires, not only
    /// for its checks; for a leg, the gate does.
    fn floor(self) -> bool {
        self.whole && self.last
    }

    /// The next line: the view's, unless the run proved a commit or the floor does not hold.
    fn next(self, view: &View, floor_holds: bool) -> Next {
        if self.commit {
            Next::CommitProven
        } else if !floor_holds {
            Next::Floor
        } else {
            view.next.clone()
        }
    }

    /// Whether the work is settled after it: the floor holds (nothing weakens, or an admin
    /// accepted exactly that on the proof host), and a whole run of a piece of work at the
    /// final stage leaves it ready.
    fn settled(self, view: &View, floor_holds: bool) -> bool {
        let answers_for_work = !self.commit && self.floor();
        floor_holds && (!answers_for_work || view.next == Next::Ready || view.checks.is_empty())
    }
}

/// A run of checks in the change's worktree, while it goes.
struct Run<'a> {
    ctx: &'a Ctx,
    env: EnvCtx<'a>,
    mode: Mode,
    wt: Worktree,
    journal: Journal,
    stage: String,
    final_stage: &'a str,
    fresh: bool,
    /// What names the preview: the pull request's number in CI, otherwise the branch.
    branch: Option<String>,
    prepared: bool,
    held: Option<Hold>,
    started: Instant,
    report: Report,
}

/// Runs the checks `ids` names, or those the stage needs, and says what is next. `out` is
/// given when proving a commit for CI.
pub(crate) fn run(
    ctx: &Ctx,
    journal: Journal,
    ids: &[String],
    fresh: bool,
    out: Option<CiOut>,
) -> Result<bool> {
    let mut wt = ctx.worktree()?;
    let before = ctx.view_in(&journal, Some(&mut wt))?;
    let stages = &ctx.config.stages;
    let stage = stages.name(before.stage).to_string();
    let final_stage = stages.name(stages.last());
    let mode = Mode {
        commit: out.is_some(),
        whole: ids.is_empty(),
        last: stage == final_stage,
    };
    let (leg, targets, left_out) = take_on(ctx, &before, ids, mode, out.as_ref())?;
    let proven_here = left_out.is_empty();
    let floor = if mode.floor() {
        floor::Floor::read(&ctx.config, &wt, &ctx.bases)?
    } else {
        floor::Floor::default()
    };
    // A pull request's CI checks out a merge commit with no branch; its number names the
    // preview, so each push updates one preview, and two forks' branches never share one.
    let branch = out
        .as_ref()
        .and_then(|o| o.workflow.pull())
        .map(|pull| format!("pr-{}", pull.number))
        .or_else(|| wt.git.branch().ok());
    let mut report = match out {
        Some(out) => Report::ci(ctx, &wt, &targets, out, leg.as_ref())?,
        None => Report::local(ctx, &targets)?,
    };
    report.proof.checks.extend(left_out);
    if targets.is_empty() {
        return nothing_to_run(ctx, &before, mode, report, &floor).map(|s| proven_here && s);
    }
    let mut run = Run {
        ctx,
        env: ctx.env(),
        mode,
        wt,
        journal,
        stage,
        final_stage,
        fresh,
        branch,
        prepared: false,
        held: None,
        started: Instant::now(),
        report,
    };
    let mut came = Vec::new();
    for check in &targets {
        let one = run.one(check)?;
        run.report.add(check, stages.name(check.stage), &one);
        came.push((*check, one));
    }
    let passed = run.end(&came)?;
    let Run {
        mut wt,
        journal,
        report,
        env,
        ..
    } = run;
    let floor_holds = report.close(&floor)?;
    // Only now: the commit report reads this run's results, which no journal names in CI, so
    // a check sharing its inputs with another could lose its output to the prune.
    crate::prune_store(ctx);
    let after = ctx.view_in(&journal, Some(&mut wt))?;
    if mode.last
        && (after.next == Next::Ready || mode.commit)
        && env.state.services(&wt.git.root).is_some()
    {
        env.state.stop(env.cfg, &wt.git.root)?;
        println!("services stopped: all proof is in");
    }
    if let Some(line) = after.pictures_to_review(&ctx.config) {
        println!("{line}");
    }
    let cannot = |c: &Came| matches!(c, Came::Away(Away::Cannot { .. }));
    let next = if came.iter().any(|(_, c)| cannot(c)) {
        Next::Recorder
    } else {
        mode.next(&after, floor_holds)
    };
    println!("next: {next}");
    Ok(proven_here && passed && mode.settled(&after, floor_holds))
}

impl Run<'_> {
    /// Proves one check, or says why it does not run here.
    fn one(&mut self, check: &Check) -> Result<Came> {
        let ctx = self.ctx;
        let inputs = self.wt.inputs(check)?;
        let at_final = self
            .mode
            .last
            .then_some((self.journal.name.as_str(), self.final_stage));
        let reuse = if self.fresh {
            Reuse::Never
        } else {
            Reuse::Pass { at_final }
        };
        // A screen this machine can never have, in a leg: its `on` names the wrong platform.
        if let Some(why) = check.needs.screen.never_here() {
            return Ok(Came::Away(Away::Cannot {
                why: why.to_string(),
                said: format!("{why}, and this leg runs on {}", std::env::consts::OS),
                fix: format!(
                    "put it on a platform whose os is {}, by its on or a [platform] with that os",
                    check.needs.screen.describe_os()
                ),
            }));
        }
        // A check runs only where its screen can be had: a window where something may record,
        // the simulator or a Linux display where their tools are. A window cannot be granted
        // on CI's runners (looks of app windows are proven on a developer's machine), so there
        // it is not run; anywhere else a missing tool fails with the one time fix, CI
        // included, since a runner of the right system is the one that proves it. Neither is
        // stored, so it is not called unstable once the tool is there.
        if (check.needs.screen.owned() || check.pictures.record.is_some())
            && let Some(u) = record::unavailable(&check.needs.screen)
            && reuse.found(&ctx.store, &inputs.key).is_none()
        {
            return Ok(Came::Away(
                if self.mode.commit && !check.needs.screen.owned() {
                    Away::Skipped(u.why)
                } else {
                    Away::Cannot {
                        said: u.why.clone(),
                        why: u.why,
                        fix: u.fix,
                    }
                },
            ));
        }
        self.report.starts(check);
        let site = Site {
            wt: &self.wt,
            env: &ctx.config.env,
            stage: &self.stage,
            prepared: &mut self.prepared,
            of: Of::Change {
                journal: &self.journal.name,
                fresh: self.mode.last,
                branch: self.branch.as_deref(),
                held: &mut self.held,
                started: self.started,
            },
        };
        let (result, reused) = prove(&ctx.store, &self.env, check, &inputs, site, &reuse)?;
        // A generator that rewrote files changed the worktree: later checks are keyed on the
        // files as they are now.
        if matches!(result.why, Some(Why::Generated(Generated::Stale(_)))) {
            self.wt = ctx.worktree()?;
        }
        self.journal.append(
            ctx.now,
            Event::Ran {
                check: check.id.clone(),
                key: inputs.key,
                result: result.id.clone(),
                outcome: result.outcome,
                tree: self.wt.tree.clone(),
                reused,
            },
        )?;
        // Compared as soon as the check passed, so its look goes with it to the proof host.
        let look = if result.outcome == Outcome::Passed {
            look_at(ctx, &self.wt, check, &result, &self.stage)?
        } else {
            None
        };
        Ok(Came::Ran {
            result,
            reused,
            look,
        })
    }

    /// Says what the checks came to beyond their own lines (unstable ones, failures, failed
    /// and flaky tests, findings, the counts) and completes the proof with the looks, the
    /// preview and red before green. Whether every check passed and red before green holds
    /// nothing.
    fn end(&mut self, came: &[(&Check, Came)]) -> Result<bool> {
        let ctx = self.ctx;
        let proof = &mut self.report.proof;
        let (mut failures, mut green, mut unstable) = (Vec::new(), Vec::new(), Vec::new());
        let (mut reused, mut skipped, mut cannot) = (0, 0, 0);
        for (check, c) in came {
            match c {
                Came::Ran {
                    result,
                    reused: r,
                    look,
                } => {
                    reused += usize::from(*r);
                    proof.looks.extend(look.clone());
                    if ctx.store.unstable(&result.key) {
                        unstable.push(check.id.as_str());
                    }
                    match result.outcome {
                        Outcome::Passed => green.push((*check, result)),
                        Outcome::Failed => failures.push(result),
                    }
                }
                Came::Away(Away::Skipped(_)) => skipped += 1,
                Came::Away(_) => cannot += 1,
            }
        }
        for id in &unstable {
            println!(
                "unstable {id}: same inputs gave passed and failed (flaky, or an input not \
                 declared); it stays unstable until its inputs change"
            );
            if let Some(c) = proof.check_mut(id) {
                c.unstable = true;
            }
        }
        failures.iter().copied().for_each(report::print_failure);
        for (_, r) in &green {
            report::print_cases(r);
            report::print_findings(r);
        }
        // The preview of the tree this run proved, deployed now or by an earlier run.
        if came.iter().any(|(c, _)| c.needs.preview) {
            proof.preview = self.env.state.preview_of(&self.wt.git.root, &self.wt.tree);
        }
        // In CI, and at the final stage of a journal so the agent learns it before CI. It
        // marks each run, so `fearlezz` and the hooks hold the work to the same verdict.
        let red_blocks =
            self.mode.last && red_before_green(ctx, &self.wt, &green, &self.stage, proof)?;
        let not_run = if skipped > 0 {
            format!(", {skipped} not run")
        } else {
            String::new()
        };
        println!(
            "{} checks: {} passed, {} failed, {reused} reused{not_run}, {:.1} s",
            came.len(),
            green.len(),
            failures.len() + cannot,
            self.started.elapsed().as_secs_f64()
        );
        Ok(failures.is_empty() && cannot == 0 && !red_blocks)
    }
}

/// What a run takes on: the leg of a plan a CI run proves, which proves its platform only on
/// a machine that fits it; the checks it runs; and outside a leg the entries of those left to
/// CI that it cannot prove (see [`left_to_ci`]).
fn take_on<'a>(
    ctx: &'a Ctx,
    before: &View,
    ids: &[String],
    mode: Mode,
    out: Option<&CiOut>,
) -> Result<(Option<legs::Leg<'a>>, Vec<&'a Check>, Vec<proof::Check>)> {
    let Some(asked) = out.and_then(|o| o.leg.as_ref()) else {
        let targets = targets(ctx, before, ids, mode.commit)?
            .into_iter()
            .filter(|check| legs::left_to_ci(&ctx.config, check).is_none())
            .collect();
        return Ok((None, targets, left_to_ci(ctx, before, ids, mode.commit)));
    };
    let leg = legs::leg(&ctx.config, &asked.id, &asked.checks)?;
    if let Some(p) = leg.platform.filter(|p| !p.fits_here())
        && let Some(os) = &p.os
    {
        bail!(
            "leg {} needs {os}, and this machine runs {}; give platform {} a runner of {os} \
             (runs-on)",
            leg.id,
            std::env::consts::OS,
            p.name
        );
    }
    let targets = leg.checks.clone();
    Ok((Some(leg), targets, Vec::new()))
}

/// The checks a run takes on: the named ones, every check of this run when proving a commit,
/// and otherwise every check of the current stage and earlier ones.
fn targets<'a>(
    ctx: &'a Ctx,
    before: &View,
    ids: &[String],
    commit: bool,
) -> Result<Vec<&'a Check>> {
    if !ids.is_empty() {
        return ids.iter().map(|id| ctx.config.check(id)).collect();
    }
    if commit {
        return Ok(ctx.config.due().collect());
    }
    let due: Vec<&str> = before
        .checks
        .iter()
        .filter(|c| c.stage <= before.stage)
        .map(|c| c.id.as_str())
        .collect();
    Ok(ctx
        .config
        .due()
        .filter(|c| due.contains(&c.id.as_str()))
        .collect())
}

/// Outside a leg, the checks due that this machine can stand in for none of the platforms of
/// are left to CI, where their legs prove them. CI without a leg cannot prove them: the
/// entries of each on each platform, not proven.
fn left_to_ci(ctx: &Ctx, before: &View, ids: &[String], commit: bool) -> Vec<proof::Check> {
    let mut left_out = Vec::new();
    let due = ctx
        .config
        .due()
        .filter(|c| (commit || c.stage <= before.stage) && (ids.is_empty() || ids.contains(&c.id)));
    for check in due {
        let Some(on) = legs::left_to_ci(&ctx.config, check) else {
            continue;
        };
        // Without platforms, the OS its screen needs stands for where it runs.
        let needs = check.needs.screen.never_here().unwrap_or_default();
        if !commit {
            match (on.is_empty(), needs.is_empty()) {
                (true, _) => println!("left to CI {}: {needs}", check.id),
                (false, true) => println!("left to CI {}: runs on {}", check.id, on.join(", ")),
                (false, false) => println!(
                    "left to CI {}: {needs}, runs on {}",
                    check.id,
                    on.join(", ")
                ),
            }
            continue;
        }
        let stage = ctx.config.stages.name(check.stage);
        let platforms = if on.is_empty() {
            vec![check.needs.screen.describe_os()]
        } else {
            on.clone()
        };
        for platform in platforms {
            let mut item = legs::left_out(check, platform);
            // Without platforms, legs cannot place it: one of its OS can.
            if on.is_empty() {
                item.why = format!("this machine cannot have its screen: {needs}");
                item.fix = format!(
                    "add a [platform.<name>] with os = \"{}\" and prove the commit in legs",
                    check.needs.screen.os().unwrap_or_default()
                );
            }
            let away = Away::Unproven(item);
            for line in away.lines(&check.id) {
                println!("{line}");
            }
            let mut entry = proof::not_run(check, stage, &away);
            entry.platform = Some(platform.to_string());
            left_out.push(entry);
        }
    }
    left_out
}

/// Every check due waits in a later stage: says where, closes the report and says what is
/// next. Whether the work is settled.
fn nothing_to_run(
    ctx: &Ctx,
    before: &View,
    mode: Mode,
    report: Report,
    floor: &floor::Floor,
) -> Result<bool> {
    let stages = &ctx.config.stages;
    let stage = stages.name(before.stage);
    let later = (mode.whole && !mode.commit)
        .then(|| {
            let waiting = before.checks.iter().filter(|c| c.stage > before.stage);
            let first = waiting.clone().map(|c| c.stage).min()?;
            let names: Vec<&str> = waiting
                .filter(|c| c.stage == first)
                .map(|c| c.id.as_str())
                .collect();
            Some((stages.name(first).to_string(), names))
        })
        .flatten();
    match &later {
        Some((next_stage, names)) => println!(
            "nothing to run in {stage}; {} {} in {next_stage}",
            names.join(", "),
            if names.len() == 1 { "waits" } else { "wait" }
        ),
        None => println!("nothing to run in {stage}"),
    }
    let floor_holds = report.close(floor)?;
    let next = match later {
        Some((next_stage, _)) if floor_holds => Next::Stage(next_stage),
        _ => mode.next(before, floor_holds),
    };
    println!("next: {next}");
    Ok(mode.settled(before, floor_holds))
}

/// Runs each green check that reads changed test files on the base commit with those files,
/// one line per check, noted in the proof, and marks each run with what it found. True when
/// the change is a fix and its tests do not fail there, or could not run there: what cannot
/// be decided is not proven.
fn red_before_green(
    ctx: &Ctx,
    wt: &Worktree,
    green: &[(&Check, &RunResult)],
    stage: &str,
    proof: &mut proof::Proof,
) -> Result<bool> {
    let fix = red::is_fix_change(&wt.git, &wt.base)?;
    let mut blocks = false;
    for &(check, head) in green {
        let ran = red::run(
            &ctx.bases,
            &ctx.config,
            &ctx.store,
            (check, head),
            wt,
            stage,
            &ctx.env(),
        );
        let id = &check.id;
        let mark = |unshown: Option<red::Unshown>| {
            store::AgainstBase::update(head, &wt.base, |a| a.red = Some(red::Mark { unshown }))
        };
        let mut record = |red: proof::Red| {
            for line in red.lines(id) {
                println!("{line}");
            }
            blocks |= red.blocks;
            if let Some(c) = proof.check_mut(id) {
                c.red = Some(red);
            }
        };
        let mut red = match ran {
            Ok(Some(red)) => red,
            Ok(None) => continue,
            Err(e) => {
                let why = format!("{e:#}");
                mark(Some(red::Unshown::DidNotRun(why.clone())))?;
                record(proof::Red {
                    verdict: proof::Verdict::DidNotRun,
                    files: Vec::new(),
                    tests: Vec::new(),
                    why: Some(why.replace('\n', " ")),
                    blocks: fix,
                    log: None,
                });
                continue;
            }
        };
        mark(red::unshown(&red))?;
        // Tests that pass on the base too show nothing of a fix.
        red.blocks = fix && red.verdict == proof::Verdict::NotRed;
        record(red);
    }
    Ok(blocks)
}

/// Compares the stills and films of a green check with the base commit, says what it found,
/// and records the comparison against it. The look it made, if any.
fn look_at(
    ctx: &Ctx,
    wt: &Worktree,
    check: &Check,
    result: &RunResult,
    stage: &str,
) -> Result<Option<look::Look>> {
    // A check that takes pictures is compared even without any, so all gone shows.
    let has_media = look::has_media(result);
    if !has_media && !ctx.config.takes_pictures(check) {
        return Ok(None);
    }
    let base = ctx.bases.at(&wt.git, &wt.base);
    let site = match &base {
        Ok(b) => b.plain(&ctx.config).map(|site| (b, site)),
        Err(e) => Err(anyhow::anyhow!("{e:#}")),
    };
    let outcome = match site {
        Ok((base, mut site)) => look::look(
            &ctx.store,
            check,
            result,
            (base, &mut site),
            stage,
            &ctx.env(),
        ),
        Err(e) => Err(anyhow::anyhow!("the base checkout failed: {e:#}")),
    };
    // Without a before every picture is new; a failed look never ends the run. A run
    // without pictures whose base cannot be compared has nothing to show.
    let outcome = outcome.or_else(|e| {
        if !has_media {
            return Ok(look::LookOutcome::NoMedia);
        }
        look::without_before(check, result, &ctx.git.common, &format!("{e:#}"))
            .map(look::LookOutcome::Done)
    });
    let looked = match &outcome {
        Ok(look::LookOutcome::NoMedia | look::LookOutcome::SameInputs) => {
            Some(store::Looked::Nothing)
        }
        Ok(look::LookOutcome::Done(l)) => Some(store::Looked::At(l.dir.clone())),
        Err(_) => None,
    };
    if let Some(looked) = looked {
        store::AgainstBase::update(result, &wt.base, |a| a.look = Some(looked))?;
    }
    match outcome {
        Ok(look::LookOutcome::NoMedia) => Ok(None),
        Ok(look::LookOutcome::SameInputs) => {
            println!(
                "look {}: inputs are the same as on the base, so is what it shows",
                check.id
            );
            Ok(None)
        }
        Ok(look::LookOutcome::Done(l)) => {
            match &l.no_before {
                Some(why) => println!("look {}: no before ({why}); {}", check.id, l.counts()),
                None => println!("look {}: {}", check.id, l.counts()),
            }
            if let Some(note) = &l.note {
                println!("  note: {note}");
            }
            for line in look::lines(&l) {
                println!("{line}");
            }
            Ok(Some(l))
        }
        Err(e) => {
            println!("look {} failed: {e:#}", check.id);
            Ok(None)
        }
    }
}
