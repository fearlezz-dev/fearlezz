//! `fearlezz`: helps coding agents prove their work. See the README for the model.

mod action;
mod base;
mod commits;
mod config;
mod counts;
mod coverage;
mod env;
mod feedback;
mod ffmpeg;
mod files;
mod findings;
mod floor;
mod generated;
mod git;
mod hooks;
mod host;
mod init;
mod journal;
mod lease;
mod legs;
mod lints;
mod look;
mod media;
mod names;
mod presets;
mod process;
mod proof;
mod prove;
mod queue;
mod read;
mod record;
mod red;
mod release;
mod report;
mod rules;
mod run;
mod settings;
mod sops;
mod state;
mod store;
mod text;
mod title;

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use clap::{Args, Parser, Subcommand};
use jiff::Timestamp;

use config::Config;
use git::Git;
use journal::{Event, Journal};
use state::{CheckState, Next, View, Worktree};
use store::{Outcome, Store};

#[derive(Parser)]
#[command(
    version,
    about = "Prove that a change is right, locally first",
    args_conflicts_with_subcommands = true,
    after_help = "Start with `fearlezz`: it says where the work stands and ends with the exact \
                  next command. Every command does."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
    #[command(flatten)]
    see: See,
}

/// What `fearlezz` on its own shows: by default this journal and what to do next.
#[derive(Args)]
#[group(multiple = false)]
struct See {
    /// Every journal in this repository, most urgent first.
    #[arg(long)]
    all: bool,
    /// Another journal, in detail.
    #[arg(long, value_name = "NAME")]
    journal: Option<String>,
    /// The checks per stage and where each stands.
    #[arg(long)]
    plan: bool,
    /// What happened since a moment: `2h`, `30m`, `1d`, or a timestamp.
    #[arg(long, value_name = "WHEN")]
    since: Option<String>,
    /// The latest result of a check in this journal, as JSON.
    #[arg(long, value_name = "CHECK")]
    check: Option<String>,
    /// A still of a running app's window, to look at while working (macOS): a pid, bundle
    /// id, app name or part of the window title. Prints the PNG's path.
    #[arg(long, value_name = "WINDOW")]
    still: Option<String>,
    /// The open notes people left on the proof host for this branch, each with its pictures
    /// saved and its region drawn. Needs `fearlezz setup login`.
    #[arg(long)]
    notes: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Prove: run the checks the current stage and change need, or the named ones.
    Run {
        checks: Vec<String>,
        /// Run even when an earlier result for the same inputs exists.
        #[arg(long)]
        fresh: bool,
        /// Move the work to this stage first, forward or back.
        #[arg(long, value_name = "NAME")]
        stage: Option<String>,
        /// Why the work moves, mostly when it goes back.
        #[arg(long, requires = "stage")]
        reason: Option<String>,
        /// Only move the stage, such as going back to fix something first.
        #[arg(long, requires = "stage")]
        no_run: bool,
        /// Prove this commit, for CI: every check not on a schedule, fresh, with no journal. It writes nothing
        /// to GitHub; on GitHub Actions it writes the job summary, and sends the proof to the
        /// proof host when the job may ask for an OIDC token.
        #[arg(long, conflicts_with = "stage")]
        ci: bool,
        /// With --ci: also write everything the run showed into this folder: `proof.json`
        /// with each check's output and every picture beside it, before and after.
        #[arg(long, value_name = "DIR", requires = "ci")]
        proof: Option<PathBuf>,
        /// Publish to every place in `[release]` that does not have its version yet, and wait
        /// until each shows it. Places that have it are skipped, so running it again finishes
        /// a half done release.
        #[arg(long, conflicts_with_all = ["checks", "stage", "ci", "fresh"])]
        release: bool,
        /// With --ci: print the legs a matrix runs, one per platform and job, as JSON for
        /// GitHub's `matrix.include`; a pull request leaves out what --reuse shows still holds.
        #[arg(long, requires = "ci", conflicts_with_all = ["leg", "gate", "proof", "checks"])]
        plan: bool,
        /// With --ci: run one leg of the plan, by id or by its plan entry (`toJSON(matrix)`),
        /// only the named checks of it when checks are named. The gate judges what the change
        /// weakens.
        #[arg(long, value_name = "LEG", requires = "ci", conflicts_with = "gate")]
        leg: Option<String>,
        /// With --ci: combine the bundles of every leg (each a folder in DIR) into one proof,
        /// the required check: a leg that did not finish is not proven.
        #[arg(long, value_name = "DIR", requires = "ci", conflicts_with = "checks")]
        gate: Option<PathBuf>,
        /// With --plan or --gate: the bundle of the base commit's CI proof, whose passes a pull
        /// request reuses for checks that read the same inputs on the same platform.
        #[arg(long, value_name = "DIR", requires = "ci", conflicts_with = "leg")]
        reuse: Option<PathBuf>,
        /// With --ci: prove the checks with `schedule = true` instead, the ones too slow,
        /// costly or reliant on real infrastructure to run for each change, from a scheduled
        /// workflow, in one job.
        #[arg(long, requires = "ci", conflicts_with_all = ["plan", "leg", "gate"])]
        scheduled: bool,
        /// After --: run this tool by hand, waiting its turn in the machine's heavy queue
        /// when `FEARLEZZ_QUEUE` is set, and exit with its code. It proves nothing.
        #[arg(last = true, value_name = "COMMAND", conflicts_with_all = ["checks", "fresh", "stage", "ci", "release"])]
        command: Vec<String>,
    },
    /// Tell: record what only you know, such as a note or a question.
    Say {
        #[command(subcommand)]
        what: Say,
    },
    /// Keep the project's tools and services in order. The output of other commands says when.
    Setup {
        #[command(subcommand)]
        what: Setup,
    },
}

#[derive(Subcommand)]
enum Say {
    /// Leave a one line note for whoever catches up later.
    Note { text: String },
    /// Ask the developer something and wait for them.
    Ask { question: String },
}

#[derive(Subcommand)]
enum Setup {
    /// Write a starter fearlezz.toml from the languages this repository shows.
    Init,
    /// Write the shared settings into each tool's own file (lint levels and more).
    Sync,
    /// Move pieces from a repository to their newest tag of the same major version (or only
    /// those whose spec contains PIECE), accept tags that moved, and list every setting that
    /// changes.
    Update { piece: Option<String> },
    /// Answer an agent's hook, reading its JSON on stdin. Written into each agent's settings
    /// by `fearlezz setup sync`.
    #[command(hide = true)]
    Hook {
        #[arg(value_enum)]
        agent: hooks::Agent,
        #[arg(value_enum)]
        event: hooks::Event,
    },
    /// Sign in to the proof host through GitHub, to read the notes people leave there. The
    /// token stays in your config folder and can only read.
    Login,
    /// Sign out of the proof host and forget its token.
    Logout,
    /// Stop the services this checkout runs, or with --all every checkout's.
    Down {
        #[arg(long)]
        all: bool,
    },
    /// Compare two stills or films: same, only timing, or changed (where and when).
    Compare {
        before: PathBuf,
        after: PathBuf,
        /// Print the full comparison as JSON.
        #[arg(long)]
        json: bool,
    },
}

struct Ctx {
    git: Git,
    config: Config,
    store: Store,
    /// The base commits this command compares with, each opened once on first use.
    bases: base::Bases,
    now: Timestamp,
}

fn main() -> ExitCode {
    // Like other command line tools, stop quietly when the reader of our output goes away
    // (`fearlezz | head`) instead of panicking on the broken pipe.
    #[cfg(unix)]
    // SAFETY: called first thing in main, before any other thread exists, with a valid
    // signal number and the default disposition.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let cli = Cli::parse();
    if let Some(Cmd::Run { command, .. }) = &cli.command
        && !command.is_empty()
    {
        return match queue::by_hand(command) {
            Ok(code) => ExitCode::from(code),
            Err(e) => {
                eprintln!("error: {e:#}");
                ExitCode::from(2)
            }
        };
    }
    let done = match cli.command {
        Some(cmd) => dispatch(cmd),
        None => match &cli.see.still {
            Some(window) => still(window),
            None if cli.see.notes => notes(),
            None => see(&cli.see),
        },
    };
    match done {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn ctx() -> Result<Ctx> {
    let git = Git::discover(&std::env::current_dir()?)?;
    let config = Config::load(&git.root, &git.common)?;
    ffmpeg::within(&git.root, config.env.shell.as_deref());
    let store = Store::open(&git.common);
    let now = match std::env::var("FEARLEZZ_NOW") {
        Ok(s) => s.parse().context("FEARLEZZ_NOW is not a timestamp")?,
        Err(_) => Timestamp::now(),
    };
    Ok(Ctx {
        git,
        config,
        store,
        bases: base::Bases::default(),
        now,
    })
}

impl Ctx {
    fn env(&self) -> env::EnvCtx<'_> {
        env::EnvCtx {
            cfg: &self.config.env,
            state: env::EnvState::open(&self.git.common),
            main: env::EnvCtx::main_checkout(&self.git.common, &self.git.root),
            now: self.now,
        }
    }

    /// The journal of the work in this worktree: named by `FEARLEZZ_JOURNAL` when set (CI, or
    /// a thread that spans branches), otherwise by the branch.
    fn journal(&self) -> Result<Journal> {
        let name = match std::env::var("FEARLEZZ_JOURNAL") {
            Ok(name) if !name.is_empty() => name,
            _ => self.git.branch()?,
        };
        Journal::open(&self.git.common, &name)
    }

    fn worktree(&self) -> Result<Worktree> {
        Worktree::read(self.git.clone(), &self.config)
    }

    fn view(&self, journal: &Journal) -> Result<View> {
        self.view_in(journal, Some(&mut self.worktree()?))
    }

    /// The view of `journal` with `wt`, the worktree its branch is checked out in, if any.
    fn view_in(&self, journal: &Journal, wt: Option<&mut Worktree>) -> Result<View> {
        state::view(
            &self.config,
            &self.store,
            journal,
            wt,
            &self.bases,
            self.now,
        )
    }

    /// Views of every journal; live where its branch is checked out somewhere.
    fn all_views(&self) -> Result<Vec<(View, Journal)>> {
        let mut worktrees: HashMap<String, Git> = self.git.worktrees()?.into_iter().collect();
        let mut views = Vec::new();
        for journal in Journal::all(&self.git.common)? {
            let mut wt = match worktrees.remove(&journal.name) {
                Some(git) => Some(Worktree::read(git, &self.config)?),
                None => None,
            };
            let view = self.view_in(&journal, wt.as_mut())?;
            views.push((view, journal));
        }
        views.sort_by_key(|(v, _)| (v.group, std::cmp::Reverse(v.last_at)));
        Ok(views)
    }

    fn stage_name(&self, view: &View) -> &str {
        self.config.stages.name(view.stage)
    }
}

/// The repository read, after tidying up what other commands left.
fn prepared() -> Result<Ctx> {
    let ctx = ctx()?;
    if ctx.config.env.up.is_some() {
        // No background process keeps watch, so each command tidies up after others.
        let env = ctx.env();
        for (s, outcome) in env.state.stop_idle(env.cfg, &ctx.git.root, ctx.now) {
            let journal = &s.journal;
            match outcome {
                Ok(env::Idle::Stopped) => eprintln!("stopped the idle services of {journal}"),
                Ok(env::Idle::Forgotten) => eprintln!(
                    "the checkout of {journal}'s services is gone ({}); if they still run, stop \
                     them by hand",
                    s.root.display()
                ),
                Err(e) => eprintln!("could not stop the idle services of {journal}: {e:#}"),
            }
        }
    }
    Ok(ctx)
}

/// Proves this commit for CI. On GitHub Actions it finds ffmpeg when checks take pictures
/// and names the pull request the event is for; it writes nothing to GitHub.
fn ci_run(proof: Option<PathBuf>, leg: Option<legs::Asked>, scheduled: bool) -> Result<bool> {
    let workflow = action::Workflow::read();
    let mut ctx = prepared()?;
    if scheduled {
        ctx.config.scheduled = true;
        // A scheduled workflow that proves nothing must not stay green.
        if ctx.config.due().next().is_none() {
            bail!(
                "no check has schedule = true, so a scheduled run proves nothing; give the slow \
                 checks schedule = true, or remove the scheduled workflow"
            );
        }
    }
    if workflow.actions
        && needs_ffmpeg(&ctx.config)
        && let Some(dir) = ffmpeg::on_runner()
    {
        let mut path = std::ffi::OsString::from(&dir);
        path.push(":");
        path.push(std::env::var_os("PATH").unwrap_or_default());
        #[expect(
            unsafe_code,
            reason = "the checks' own commands find the pinned ffmpeg through PATH"
        )]
        // SAFETY: Fearlezz runs no other thread yet, so nothing reads the environment while it
        // changes.
        unsafe {
            std::env::set_var("PATH", path);
        }
    }
    let mut journal = Journal::in_memory("ci");
    let to = ctx.config.stages.name(ctx.config.stages.last()).to_string();
    journal.append(ctx.now, Event::Stage { to, reason: None })?;
    prove::run(
        &ctx,
        journal,
        &[],
        true,
        Some(report::CiOut {
            proof,
            workflow,
            leg,
        }),
    )
}

/// Prints the legs a matrix runs, as JSON for GitHub's `matrix.include`. A pull request leaves
/// out each check whose pass the base commit's CI proof shows for the same inputs on the same
/// platform, and every leg left with nothing to run.
fn ci_plan(reuse: Option<&Path>) -> Result<bool> {
    let workflow = action::Workflow::read();
    let ctx = ctx()?;
    let mut wt = ctx.worktree()?;
    let reuse = legs::Reuse::read(reuse, workflow.pull().is_some())?;
    let mut planned = Vec::new();
    for mut leg in legs::legs(&ctx.config) {
        if let Some(reuse) = &reuse {
            let platform = leg.platform_name();
            let mut keep = Vec::new();
            for check in leg.checks {
                let key = wt.inputs(check)?.key;
                if reuse.pass(&check.id, platform.as_deref(), &key).is_none() {
                    keep.push(check);
                }
            }
            leg.checks = keep;
        }
        if !leg.checks.is_empty() {
            planned.push(leg);
        }
    }
    let ids: Vec<String> = planned.iter().map(|l| l.id.clone()).collect();
    let entries: Vec<serde_json::Value> = planned.iter().map(|l| legs::entry(l, &ids)).collect();
    // Output for a program: the JSON alone, with no next: line.
    println!("{}", serde_json::to_string(&entries)?);
    Ok(true)
}

/// Combines the bundles of every leg in `dir` into one proof of the commit: each check on each
/// of its platforms from its leg, or reused from the base commit's CI proof by a pull request,
/// and otherwise not proven. Judges what the change weakens, writes the bundle when asked and
/// the job summary GitHub Actions names.
fn ci_gate(dir: &Path, bundle: Option<&Path>, reuse: Option<&Path>) -> Result<bool> {
    let workflow = action::Workflow::read();
    let ctx = ctx()?;
    let mut wt = ctx.worktree()?;
    let reuse = legs::Reuse::read(reuse, workflow.pull().is_some())?;
    let found = legs::read_legs(dir)?;
    let mut proof = report::ci_proof(&ctx, &workflow)?;
    println!(
        "commit {}: the gate over {} legs in {}",
        proof.commit,
        found.len(),
        dir.display()
    );
    for leg in legs::legs(&ctx.config) {
        let platform = leg.platform_name();
        for check in &leg.checks {
            let from_leg = found.iter().find_map(|l| {
                l.proof
                    .checks
                    .iter()
                    .find(|c| c.id == check.id && c.platform == platform)
                    .map(|c| legs::from_leg(l, c))
            });
            let entry = if let Some(entry) = from_leg {
                entry
            } else {
                let key = wt.inputs(check)?.key;
                let reused = reuse
                    .as_ref()
                    .and_then(|r| r.pass(&check.id, platform.as_deref(), &key));
                reused.unwrap_or_else(|| {
                    let away =
                        proof::Away::Unproven(legs::missing(check, platform.as_deref(), &leg.id));
                    let stage = ctx.config.stages.name(check.stage);
                    proof::Check {
                        platform: platform.clone(),
                        ..proof::not_run(check, stage, &away)
                    }
                })
            };
            println!("{} {}", entry.word(), entry.name());
            proof.checks.push(entry);
        }
    }
    for leg in &found {
        proof.looks.extend(legs::looks_of(leg));
    }
    // Each leg that needs the preview deploys the same tree; the first one found names it.
    proof.preview = found.iter().find_map(|l| l.proof.preview.clone());
    let floor = floor::Floor::read(&ctx.config, &wt, &ctx.bases)?;
    proof.weakened.clone_from(&floor.weakened);
    let gate = if floor.holds() {
        floor::Gate::Job
    } else {
        gate_accept(&ctx, &found, &mut proof, &floor)
    };
    if let Some(dir) = bundle {
        proof::write(&proof, dir)?;
    }
    report::summary(&proof, None, &gate)?;
    let blockers = proof.blockers();
    // What weakens is said below, with how it gets past.
    let weakened: Vec<String> = proof
        .weakened
        .iter()
        .map(fearlezz_proof::floor::Weakening::what)
        .collect();
    for item in blockers.iter().filter(|i| !weakened.contains(&i.what)) {
        println!("blocked: {}: {}", item.what, item.why);
        println!("  fix: {}", item.fix);
    }
    for line in floor.lines(&gate) {
        println!("{line}");
    }
    println!("next: {}", Next::CommitProven);
    Ok(blockers.is_empty())
}

/// Where what a split proof weakens gets past: an admin's accept of exactly it at the head, as
/// the host answers it through the run of a leg that streamed to it. Without such a leg the
/// gate fails as without a host.
fn gate_accept(
    ctx: &Ctx,
    found: &[legs::LegProof],
    proof: &mut proof::Proof,
    floor: &floor::Floor,
) -> floor::Gate {
    let run = found
        .iter()
        .find_map(|l| std::fs::read_to_string(l.dir.join(legs::HOST_RUN)).ok());
    let (Some(run), Some(host)) = (run, host::Host::from_actions(ctx.config.host.as_deref()))
    else {
        return floor::Gate::Job;
    };
    match report::ask_accept(&host.resume(run.trim()), proof, floor) {
        Ok(()) => proof
            .accepted
            .clone()
            .map_or(floor::Gate::Host, floor::Gate::Accepted),
        Err(e) => {
            println!("proof host not asked: {e:#}");
            floor::Gate::Unasked
        }
    }
}

/// Publishes the release to every place that does not have it yet, inside the checkout's
/// environment.
fn release_run() -> Result<bool> {
    let workflow = action::Workflow::read();
    let ctx = prepared()?;
    let env = ctx.env();
    if !ctx.config.releases.is_empty()
        && let Some(line) = env.state.prepare(env.cfg, &env.main, &ctx.git.root)?
    {
        println!("{line}");
    }
    Ok(release::run(
        &ctx.config.releases,
        &release::At {
            root: &ctx.git.root,
            shell: ctx.config.env.shell.as_deref(),
            github: release::GitHub {
                repository: workflow
                    .repository
                    .clone()
                    .or_else(|| ctx.git.github_repository()),
                api: workflow
                    .api_url
                    .clone()
                    .unwrap_or_else(|| "https://api.github.com".into()),
            },
        },
    ))
}

/// Whether a check takes pictures, which needs ffmpeg.
fn needs_ffmpeg(config: &Config) -> bool {
    config.due().any(|c| config.needs_ffmpeg(c))
}

fn dispatch(cmd: Cmd) -> Result<bool> {
    match cmd {
        Cmd::Run {
            checks,
            fresh,
            stage,
            reason,
            no_run,
            ci,
            proof,
            release,
            plan,
            leg,
            gate,
            reuse,
            scheduled,
            ..
        } => {
            if plan {
                return ci_plan(reuse.as_deref());
            }
            if let Some(dir) = gate {
                return ci_gate(&dir, proof.as_deref(), reuse.as_deref());
            }
            if ci {
                let leg = match leg {
                    Some(arg) => Some(legs::Asked::read(&arg, checks)?),
                    None if checks.is_empty() => None,
                    None => bail!(
                        "--ci proves every check; to run some of them, name the leg they are in                          with --leg"
                    ),
                };
                return ci_run(proof, leg, scheduled);
            }
            if release {
                return release_run();
            }
            let ctx = prepared()?;
            if let Some(name) = stage {
                move_stage(&ctx, name, reason)?;
                if no_run {
                    println!("next: {}", ctx.view(&ctx.journal()?)?.next);
                    return Ok(true);
                }
            }
            prove::run(&ctx, ctx.journal()?, &checks, fresh, None)
        }
        Cmd::Say { what } => say(&prepared()?, what),
        Cmd::Setup { what } => setup(what),
    }
}

/// Moves the journal to the stage called `name`.
fn move_stage(ctx: &Ctx, name: String, reason: Option<String>) -> Result<()> {
    let stages = &ctx.config.stages;
    let to = stages
        .find(&name)
        .with_context(|| format!("unknown stage \"{name}\"; stages are {stages}"))?;
    let mut j = ctx.journal()?;
    let from = ctx.view(&j)?.stage;
    j.append(ctx.now, Event::Stage { to: name, reason })?;
    println!("stage {} -> {}", stages.name(from), stages.name(to));
    Ok(())
}

/// What Fearlezz tells an agent at `event`, from the repository the hook input names.
fn hook_answer(event: hooks::Event, input: &hooks::Input) -> Result<hooks::Answer> {
    if let Some(dir) = &input.dir {
        std::env::set_current_dir(dir)?;
    }
    let ctx = ctx()?;
    let journal = ctx.journal()?;
    let wt = ctx.worktree()?;
    let view = ctx.view(&journal)?;
    let work = hooks::Work {
        name: journal.name.clone(),
        ready: view.next == Next::Ready,
        waiting: view.waiting.is_some(),
        needs_proof: wt.changed.iter().any(|p| ctx.config.needs_proof(p)),
        now: view.now.clone(),
        next: view.next.to_string(),
        // Asked of the host only where it can hold the agent: a stop it has not held yet.
        open_notes: if matches!(event, hooks::Event::Stop) && !input.again {
            feedback::open_notes(&ctx.git, &host_url(Some(&ctx.config)))
        } else {
            0
        },
    };
    Ok(hooks::judge(&ctx.config, &work, event, input))
}

fn say(ctx: &Ctx, what: Say) -> Result<bool> {
    match what {
        Say::Ask { question } => {
            let mut j = ctx.journal()?;
            j.append(
                ctx.now,
                Event::Wait {
                    question: question.clone(),
                },
            )?;
            println!("asked the developer: {question}");
            println!("next: {}", ctx.view(&j)?.next);
        }
        Say::Note { text } => {
            let mut j = ctx.journal()?;
            j.append(ctx.now, Event::Note { text })?;
            println!("noted in {}", j.name);
            println!("next: {}", ctx.view(&j)?.next);
        }
    }
    Ok(true)
}

/// Writes the starter config and says what it found.
fn setup_init() -> Result<()> {
    let git = Git::discover(&std::env::current_dir()?)?;
    let found = init::init(&git.root)?;
    if found.iter().all(|p| p.file.is_none()) {
        println!("wrote fearlezz.toml: no language found, so fill in its checks");
    }
    let names: Vec<String> = found.iter().map(ToString::to_string).collect();
    println!("wrote fearlezz.toml: uses {}", names.join(", "));
    println!("next: fearlezz setup sync");
    Ok(())
}

/// The proof host this repository sends its proof to, or the default one outside a repository.
pub(crate) fn host_url(config: Option<&Config>) -> String {
    config
        .and_then(|c| c.host.as_deref())
        .unwrap_or(host::DEFAULT)
        .trim_end_matches('/')
        .to_owned()
}

/// The proof host of the repository here, if any.
fn here_host_url() -> String {
    let config = Git::discover(&std::env::current_dir().unwrap_or_default())
        .ok()
        .and_then(|git| Config::load(&git.root, &git.common).ok());
    host_url(config.as_ref())
}

/// `fearlezz --notes`: the open notes on the proof host, with their pictures.
fn notes() -> Result<bool> {
    let ctx = ctx()?;
    feedback::notes(&ctx.git, &host_url(Some(&ctx.config)))
}

fn setup(what: Setup) -> Result<bool> {
    match what {
        Setup::Init => setup_init()?,
        Setup::Login => feedback::login(&here_host_url())?,
        Setup::Logout => feedback::logout(&here_host_url())?,
        Setup::Update { piece } => setup_update(piece.as_deref())?,
        Setup::Compare {
            before,
            after,
            json,
        } => setup_compare(&before, &after, json)?,
        Setup::Sync => setup_sync()?,
        Setup::Hook { agent, event } => setup_hook(agent, event)?,
        Setup::Down { all } => setup_down(all)?,
    }
    Ok(true)
}

/// `fearlezz setup update`: moves pinned pieces to their newest tag and lists what changed.
fn setup_update(piece: Option<&str>) -> Result<()> {
    let git = Git::discover(&std::env::current_dir()?)?;
    let updated = presets::update(&git.root, &git.common, piece)?;
    for (spec, tag) in &updated.pieces {
        match spec.rsplit_once('@') {
            Some((_, old)) if old != tag => println!("{spec} → {tag}"),
            _ => println!("{spec}: still the newest tag"),
        }
    }
    for (spec, tag) in &updated.majors {
        println!(
            "{spec}: {tag} is out, a new major version that update does not move to; to \
                 take it, change the tag in fearlezz.toml"
        );
    }
    for id in &updated.moved {
        println!("{id}: now pins where its tag points");
    }
    match &updated.changes {
        None => println!(
            "the old pins can no longer be read, so the changes cannot be listed; compare \
                 the pieces by hand"
        ),
        Some(changes) if changes.is_empty() => println!("no changes to the settings"),
        Some(changes) => {
            for change in changes {
                println!("  {change}");
            }
        }
    }
    println!("next: review the changes, then commit fearlezz.toml and fearlezz.lock");
    Ok(())
}

/// `fearlezz setup compare`: two stills or films compared, as a summary or as JSON.
fn setup_compare(before: &Path, after: &Path, json: bool) -> Result<()> {
    let clock = std::time::Instant::now();
    let (b, a) = media::decode_both(before, after)?;
    let c = media::compare(&b, &a)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&c)?);
    } else {
        println!(
            "{} ({:.1} s)",
            media::summary(&c),
            clock.elapsed().as_secs_f64()
        );
        println!("next: fearlezz run shows changes for the checks that record stills and films");
    }
    Ok(())
}

/// `fearlezz setup sync`: writes the shared tool settings into each tool's own file.
fn setup_sync() -> Result<()> {
    let ctx = prepared()?;
    let written = ctx.config.settings.sync(&ctx.git.root)?;
    if written.is_empty() {
        println!("settings already in sync");
    }
    let wrote = !written.is_empty();
    for path in written {
        println!("wrote {path}");
    }
    if wrote {
        println!("next: commit the written files with the change");
    } else {
        println!("next: {}", ctx.view(&ctx.journal()?)?.next);
    }
    Ok(())
}

/// `fearlezz setup hook`: an agent's hook event answered from its input on stdin.
fn setup_hook(agent: hooks::Agent, event: hooks::Event) -> Result<()> {
    let mut text = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)?;
    let input = hooks::Input::parse(agent, &text);
    // A hook that cannot judge says nothing: it is a guardrail, and CI is the boundary.
    let answer = hook_answer(event, &input).unwrap_or_else(|e| {
        eprintln!("fearlezz hook: {e:#}");
        hooks::Answer::Nothing
    });
    if let Some(out) = hooks::render(agent, event, answer) {
        println!("{out}");
    }
    Ok(())
}

/// `fearlezz setup down`: stops the services of this worktree, or of every worktree.
fn setup_down(all: bool) -> Result<()> {
    let ctx = prepared()?;
    let env = ctx.env();
    let services = if all {
        env.state.all_services()
    } else {
        env.state.services(&ctx.git.root).into_iter().collect()
    };
    let roots: Vec<PathBuf> = services.into_iter().map(|s| s.root).collect();
    for root in &roots {
        env.state.stop_unless_in_use(env.cfg, root)?;
        println!("stopped services in {}", root.display());
    }
    if roots.is_empty() {
        println!("no services running");
    }
    println!("next: fearlezz run starts them again when a check needs them");
    Ok(())
}

/// `fearlezz --still`: a still of a running app's window, kept with this repository's
/// Fearlezz files (outside the worktree, so it is never a change), or in the temp folder.
fn still(window: &str) -> Result<bool> {
    let dir = match Git::discover(&std::env::current_dir()?) {
        Ok(git) => git.common.join("fearlezz").join("stills"),
        Err(_) => std::env::temp_dir().join("fearlezz-stills"),
    };
    let s = record::still_of(window, &dir, 0.0)?;
    println!(
        "still {}x{} of {} in {:.2} s: {}",
        s.size.0,
        s.size.1,
        s.window.describe(),
        s.took.as_secs_f64(),
        s.path.display()
    );
    println!(
        "next: look at the PNG and judge the UI; for proof, a check records it with a record \
         table and fearlezz run compares it with the base"
    );
    Ok(true)
}

/// `fearlezz` on its own: where the work stands, in the view asked for.
fn see(see: &See) -> Result<bool> {
    let ctx = prepared()?;
    if see.all {
        for (v, _) in ctx.all_views()? {
            println!(
                "{:<9} {}  {}  {}",
                v.group.label(),
                v.name,
                ctx.stage_name(&v),
                v.now
            );
        }
        println!("next: fearlezz --journal <name> shows one in detail");
    } else if let Some(name) = &see.journal {
        let found = ctx.all_views()?.into_iter().find(|(v, _)| v.name == *name);
        let (v, _) = found.with_context(|| format!("no journal \"{name}\""))?;
        print_status(&ctx, &v, (Vec::new(), 0));
    } else if see.plan {
        plan(&ctx)?;
    } else if let Some(when) = &see.since {
        since(&ctx, when)?;
    } else if let Some(check) = &see.check {
        let j = ctx.journal()?;
        let ran = j.entries.iter().rev().find_map(|e| {
            if let Event::Ran {
                check: c,
                key,
                result,
                ..
            } = &e.event
                && c == check
            {
                Some((key, result))
            } else {
                None
            }
        });
        let (key, id) =
            ran.with_context(|| format!("no run of {check} in journal {} yet", j.name))?;
        let result = ctx
            .store
            .get(key, id)
            .context("the result is missing from the store")?;
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        let j = ctx.journal()?;
        let host = feedback::status(&ctx.git, &host_url(Some(&ctx.config)));
        print_status(&ctx, &ctx.view(&j)?, host);
    }
    Ok(true)
}

fn plan(ctx: &Ctx) -> Result<()> {
    let j = ctx.journal()?;
    let v = ctx.view(&j)?;
    let stages = &ctx.config.stages;
    println!("plan {}  stage {}", v.name, ctx.stage_name(&v));
    let mut any = false;
    for stage in stages.all() {
        let here: Vec<String> = v
            .checks
            .iter()
            .filter(|c| c.stage == stage)
            .map(|c| format!("{} {}", c.id, state_label(&c.state)))
            .collect();
        if here.is_empty() {
            continue;
        }
        any = true;
        let mark = if stage <= v.stage { '*' } else { ' ' };
        println!("{mark} {:<9} {}", stages.name(stage), here.join(", "));
    }
    if !any {
        println!("  nothing touched yet");
    }
    let scheduled: Vec<&str> = ctx
        .config
        .checks
        .iter()
        .filter(|c| c.schedule)
        .map(|c| c.id.as_str())
        .collect();
    if !scheduled.is_empty() {
        println!(
            "  scheduled {}: never needed by a change; fearlezz run --ci --scheduled proves them",
            scheduled.join(", ")
        );
    }
    println!("next: {}", v.next);
    Ok(())
}

/// Where a work stands, then what waits on the proof host (`host`: its lines and how many notes
/// are open), then what to do next.
fn print_status(ctx: &Ctx, v: &View, host: (Vec<String>, usize)) {
    println!(
        "{}  stage {}  [{}]",
        v.name,
        ctx.stage_name(v),
        v.group.label()
    );
    println!("now: {}", v.now);
    if v.live
        && let Some(s) = env::EnvState::open(&ctx.git.common).services(&ctx.git.root)
    {
        let ports: Vec<String> = s.ports.iter().map(|(k, v)| format!("{k}={v}")).collect();
        // Fearlezz reads the ports when the services start, so it cannot say they still run.
        println!(
            "services: started {}{}  {}",
            ago(ctx.now, s.started_at),
            if s.ready {
                ""
            } else {
                ", start did not finish"
            },
            ports.join(" ")
        );
    }
    if let Some(reason) = &v.went_back {
        println!("went back to {}: {reason}", ctx.stage_name(v));
    }
    if !v.checks.is_empty() {
        let list: Vec<String> = v
            .checks
            .iter()
            .map(|c| format!("{} {}", c.id, state_label(&c.state)))
            .collect();
        println!("checks: {}", list.join(", "));
    }
    for c in v.checks.iter().filter(|c| !c.findings.is_empty()) {
        println!("findings {}:", c.id);
        for f in c.findings.iter().take(report::SHOWN) {
            println!("  {f}");
        }
    }
    for l in v.checks.iter().filter_map(|c| c.look.as_ref()) {
        if l.items.iter().any(|i| i.change != look::Change::Same) {
            println!("look {}: {}", l.check, l.counts());
            for line in look::lines(l) {
                println!("{line}");
            }
        }
    }
    if let Some(line) = v.pictures_to_review(&ctx.config) {
        println!("{line}");
    }
    let stale = v.checks.iter().filter_map(|c| {
        if let CheckState::Stale(why) = &c.state {
            Some((&c.id, why))
        } else {
            None
        }
    });
    for (id, why) in stale.take(2) {
        let shown: Vec<&str> = why.iter().take(3).map(String::as_str).collect();
        println!("stale: {id}, {} changed", shown.join(", "));
    }
    if let Some((at, text)) = &v.last_note {
        println!("note ({}): {text}", ago(ctx.now, *at));
    }
    let (lines, open) = host;
    for line in lines {
        println!("{line}");
    }
    if open > 0 {
        let notes = if open == 1 {
            "the open note".to_owned()
        } else {
            format!("the {open} open notes")
        };
        println!(
            "next: fearlezz --notes shows {notes} a person left, with their pictures; then {}",
            v.next
        );
    } else {
        println!("next: {}", v.next);
    }
}

fn since(ctx: &Ctx, when: &str) -> Result<()> {
    let from = parse_when(ctx.now, when)?;
    for journal in Journal::all(&ctx.git.common)? {
        let recent: Vec<&journal::Entry> = journal.entries.iter().filter(|e| e.at > from).collect();
        if recent.is_empty() {
            continue;
        }
        println!("{}:", journal.name);
        let (mut runs, mut red) = (0, 0);
        for e in &recent {
            match &e.event {
                Event::Note { text } => println!("  note: {text}"),
                Event::Wait { question } => println!("  asked you: {question}"),
                Event::Stage { to, reason } => match reason {
                    Some(r) => println!("  stage {to}: {r}"),
                    None => println!("  stage {to}"),
                },
                Event::Retired => {}
                Event::Ran { outcome, .. } => {
                    runs += 1;
                    red += usize::from(*outcome == Outcome::Failed);
                }
            }
        }
        if runs > 0 {
            println!("  {runs} runs, {red} red");
        }
    }
    println!("next: fearlezz --all");
    Ok(())
}

fn parse_when(now: Timestamp, when: &str) -> Result<Timestamp> {
    if let Ok(t) = when.parse::<Timestamp>() {
        return Ok(t);
    }
    let bad = || format!("\"{when}\" is not like 2h, 30m, 1d or a timestamp");
    let split = when.char_indices().last().map_or(0, |(i, _)| i);
    let (num, unit) = when.split_at(split);
    let n: i64 = num.parse().with_context(bad)?;
    let seconds = match unit {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        "d" => n * 86400,
        _ => bail!(bad()),
    };
    Ok(Timestamp::from_second(now.as_second() - seconds)?)
}

/// The message a crashed thread panicked with.
pub(crate) fn panic_text(panic: &(dyn std::any::Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("no message")
}

/// How long ago `then` was, in words.
/// Deletes results nothing reads again; see [`store::Store::prune`]. A failure only warns:
/// the run itself is done.
pub(crate) fn prune_store(ctx: &Ctx) {
    let named = || -> Result<BTreeSet<(String, String)>> {
        let since = ctx.now.as_second() - store::KEEP_DAYS * 24 * 3600;
        let mut named = BTreeSet::new();
        for journal in Journal::all(&ctx.git.common)? {
            if journal
                .entries
                .last()
                .is_none_or(|e| e.at.as_second() < since)
            {
                continue;
            }
            // Only the latest run of each check is read.
            let mut seen = BTreeSet::new();
            for e in journal.entries.iter().rev() {
                if let Event::Ran {
                    check, key, result, ..
                } = &e.event
                    && seen.insert(check.clone())
                {
                    named.insert((key.clone(), result.clone()));
                }
            }
        }
        Ok(named)
    };
    if let Err(e) = named().and_then(|n| ctx.store.prune(&n, ctx.now)) {
        eprintln!("could not clean up old results: {e:#}");
    }
    if let Err(e) = ctx.git.prune_bases(store::KEEP_DAYS) {
        eprintln!("could not clean up old base checkouts: {e:#}");
    }
}

/// How long ago `then` was, as a person says it: "just now", "5 min ago", "2 d ago".
pub fn ago(now: Timestamp, then: Timestamp) -> String {
    let s = now.as_second() - then.as_second();
    match s {
        ..60 => "just now".into(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86400 => format!("{} h ago", s / 3600),
        _ => format!("{} d ago", s / 86400),
    }
}

/// The word for a check's state, as status shows it.
pub fn state_label(state: &CheckState) -> &'static str {
    match state {
        CheckState::Ok => "ok",
        CheckState::Failed => "failed",
        CheckState::Unstable => "unstable",
        CheckState::Stale(_) => "stale",
        CheckState::NeedsFresh => "needs a fresh run",
        CheckState::Uncompared => "not compared with this base commit yet",
        CheckState::Never => "never run",
    }
}
