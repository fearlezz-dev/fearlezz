//! `fearlezz`: helps coding agents prove their work. See the README for the model.

mod action;
mod commits;
mod config;
mod counts;
mod coverage;
mod env;
mod feedback;
mod findings;
mod floor;
mod git;
mod hooks;
mod host;
mod init;
mod journal;
mod legs;
mod lints;
mod look;
mod media;
mod presets;
mod proof;
mod record;
mod red;
mod release;
mod rules;
mod run;
mod settings;
mod sops;
mod state;
mod store;

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use clap::{Args, Parser, Subcommand};
use jiff::Timestamp;

use config::Config;
use git::Git;
use journal::{Event, Journal};
use state::{CheckState, View, Worktree};
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
    let store = Store::open(&git.common);
    let now = match std::env::var("FEARLEZZ_NOW") {
        Ok(s) => s.parse().context("FEARLEZZ_NOW is not a timestamp")?,
        Err(_) => Timestamp::now(),
    };
    Ok(Ctx {
        git,
        config,
        store,
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
        let mut wt = self.worktree()?;
        state::view(&self.config, &self.store, journal, Some(&mut wt), self.now)
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
            let view = state::view(&self.config, &self.store, &journal, wt.as_mut(), self.now)?;
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

/// Proves this commit for CI. On GitHub Actions it installs ffmpeg when checks take pictures
/// and names the pull request the event is for; it writes nothing to GitHub.
fn ci_run(proof: Option<PathBuf>, leg: Option<legs::Asked>, scheduled: bool) -> Result<bool> {
    let workflow = action::Workflow::read();
    let mut ctx = prepared()?;
    if scheduled {
        ctx.config.on_schedule();
        // A scheduled workflow that proves nothing must not stay green.
        if ctx.config.checks.is_empty() {
            bail!(
                "no check has schedule = true, so a scheduled run proves nothing; give the slow \
                 checks schedule = true, or remove the scheduled workflow"
            );
        }
    }
    if workflow.actions
        && needs_ffmpeg(&ctx.config)
        && let Some(dir) = action::install_ffmpeg()?
    {
        let mut path = std::ffi::OsString::from(&dir);
        path.push(":");
        path.push(std::env::var_os("PATH").unwrap_or_default());
        #[expect(
            unsafe_code,
            reason = "the checks find the installed ffmpeg through PATH"
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
    run_checks(
        &ctx,
        journal,
        &[],
        true,
        Some(CiOut {
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
fn ci_gate(dir: &Path, bundle: Option<PathBuf>, reuse: Option<&Path>) -> Result<bool> {
    use std::io::Write as _;
    let workflow = action::Workflow::read();
    let ctx = ctx()?;
    let mut wt = ctx.worktree()?;
    let reuse = legs::Reuse::read(reuse, workflow.pull().is_some())?;
    let found = legs::read_legs(dir)?;
    let mut proof = ci_proof(&ctx, &workflow)?;
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
            let entry = match from_leg {
                Some(entry) => entry,
                None => {
                    let key = wt.inputs(check)?.key;
                    match reuse
                        .as_ref()
                        .and_then(|r| r.pass(&check.id, platform.as_deref(), &key))
                    {
                        Some(entry) => entry,
                        None => legs::missing(
                            check,
                            platform.as_deref(),
                            ctx.config.stages.name(check.stage),
                            &leg.id,
                        ),
                    }
                }
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
    let floor = floor::Floor::read(&ctx.config, &wt)?;
    proof.weakened.clone_from(&floor.weakened);
    let gate = if floor.holds() {
        floor::Gate::Job
    } else {
        gate_accept(&ctx, &found, &mut proof, &floor)
    };
    if let Some(dir) = &bundle {
        proof::write(&proof, dir)?;
    }
    if let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        let summary = action::summary(&proof, None, &gate)?;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(Path::new(&path))?
            .write_all(summary.as_bytes())?;
    }
    let blockers = proof.blockers();
    // What weakens is said below, with how it gets past.
    let weakened: Vec<String> = proof.weakened.iter().map(|w| w.what()).collect();
    for item in blockers.iter().filter(|i| !weakened.contains(&i.what)) {
        println!("blocked: {}: {}", item.what, item.why);
        println!("  fix: {}", item.fix);
    }
    for line in floor.lines(&gate) {
        println!("{line}");
    }
    println!("next: {}", state::COMMIT_PROVEN);
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
    match host.resume(run.trim()).accepted(&floor.weakened) {
        Ok(accepted) => match accepted.filter(|a| a.covers(&proof.head, &floor.weakened)) {
            Some(accept) => {
                proof.accepted = Some(accept.clone());
                floor::Gate::Accepted(accept)
            }
            None => floor::Gate::Host,
        },
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
    config.checks.iter().any(|c| config.takes_pictures(c))
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
        } => {
            if plan {
                return ci_plan(reuse.as_deref());
            }
            if let Some(dir) = gate {
                return ci_gate(&dir, proof, reuse.as_deref());
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
            run_checks(&ctx, ctx.journal()?, &checks, fresh, None)
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
        ready: view.next == state::READY,
        waiting: view.waiting.is_some(),
        needs_proof: wt.changed.iter().any(|p| ctx.config.needs_proof(p)),
        now: view.now.clone(),
        next: view.next.clone(),
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
fn host_url(config: Option<&Config>) -> String {
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
        Setup::Update { piece } => {
            let git = Git::discover(&std::env::current_dir()?)?;
            let updated = presets::update(&git.root, &git.common, piece.as_deref())?;
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
        }
        Setup::Compare {
            before,
            after,
            json,
        } => {
            let clock = std::time::Instant::now();
            let (b, a) = media::decode_both(&before, &after)?;
            let c = media::compare(&b, &a)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&c)?);
            } else {
                println!(
                    "{} ({:.1} s)",
                    media::summary(&c),
                    clock.elapsed().as_secs_f64()
                );
                println!(
                    "next: fearlezz run shows changes for the checks that record stills and films"
                );
            }
        }
        Setup::Sync => {
            let ctx = prepared()?;
            let ctx = &ctx;
            {
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
            }
        }
        Setup::Hook { agent, event } => {
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
        }
        Setup::Down { all } => {
            let ctx = prepared()?;
            {
                let env = ctx.env();
                let roots: Vec<PathBuf> = if all {
                    env.state
                        .all_services()
                        .into_iter()
                        .map(|s| s.root)
                        .collect()
                } else {
                    env.state
                        .services(&ctx.git.root)
                        .into_iter()
                        .map(|s| s.root)
                        .collect()
                };
                for root in &roots {
                    env.state.stop_unless_in_use(env.cfg, root)?;
                    println!("stopped services in {}", root.display());
                }
                if roots.is_empty() {
                    println!("no services running");
                }
                println!("next: fearlezz run starts them again when a check needs them");
            }
        }
    }
    Ok(true)
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
        let ran = j.entries.iter().rev().find_map(|e| match &e.event {
            Event::Ran {
                check: c,
                key,
                result,
                ..
            } if c == check => Some((key, result)),
            _ => None,
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

/// The findings a run counted, worst first, with how many were left out.
fn print_findings(r: &store::RunResult) {
    const SHOWN: usize = 10;
    for f in r.findings.iter().take(SHOWN) {
        println!("  {f}");
    }
    if r.findings.len() > SHOWN {
        println!(
            "  and {} more: fearlezz --check {}",
            r.findings.len() - SHOWN,
            r.check
        );
    }
    if r.findings_elsewhere > 0 {
        println!(
            "  ({} more on lines this change did not touch)",
            r.findings_elsewhere
        );
    }
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
    let scheduled: Vec<&str> = ctx.config.set_aside.iter().map(|c| c.id.as_str()).collect();
    if !scheduled.is_empty() {
        println!(
            "  scheduled {}: never needed by a change; fearlezz run --ci --scheduled proves them",
            scheduled.join(", ")
        );
    }
    println!("next: {}", v.next);
    Ok(())
}

/// The checks a run takes on: the named ones, every check of this run when proving a commit, and otherwise
/// every check of the current stage and earlier ones.
fn targets<'a>(
    ctx: &'a Ctx,
    before: &View,
    ids: &[String],
    in_memory: bool,
) -> Result<Vec<&'a config::Check>> {
    if !ids.is_empty() {
        return ids.iter().map(|id| ctx.config.check(id)).collect();
    }
    if in_memory {
        return Ok(ctx.config.checks.iter().collect());
    }
    let due: Vec<&str> = before
        .checks
        .iter()
        .filter(|c| c.stage <= before.stage)
        .map(|c| c.id.as_str())
        .collect();
    Ok(ctx
        .config
        .checks
        .iter()
        .filter(|c| due.contains(&c.id.as_str()))
        .collect())
}

fn print_failure(f: &store::RunResult) {
    let code = f
        .exit_code
        .map_or_else(|| "signal".to_string(), |c| c.to_string());
    if f.findings.iter().any(|x| x.level == findings::Level::Gate) {
        println!("fail {}: blocking findings", f.check);
    } else {
        if let Some(name) = &f.missing_secret {
            println!("fail {}: SecretSpec could not give it {name}", f.check);
            println!("  fix: {}", secret_fix(name));
        } else if let Some(why) = &f.no_preview {
            let (why, fix) = no_preview(why);
            println!("fail {}: {why}", f.check);
            println!("  fix: {fix}");
        } else if let Some(tool) = f
            .not_found()
            .then(|| run::missing_command(&f.log))
            .flatten()
        {
            println!("fail {}: {tool} was not found", f.check);
            println!("  fix: {}", install_fix(&tool));
        } else if f.tests == Some(0) && f.exit_code == Some(0) {
            println!("fail {}: it ran no tests, so it proves nothing", f.check);
            for why in &f.proof_errors {
                println!("  {why}");
            }
        } else if let (Some(0), Some(why)) = (f.exit_code, f.proof_errors.first()) {
            // Its command passed, but what it had to leave (a coverage report) is missing.
            println!("fail {}: {why}", f.check);
        } else {
            println!("fail {} (exit {code})", f.check);
        }
        for line in run::tail(&f.log, 12) {
            println!("  {line}");
        }
        println!("  log: {}", f.log.display());
    }
    print_cases(f);
    print_findings(f);
}

/// Each test a run names as failed or flaky, the failed ones first and cut short, and what to
/// do about a flaky one its check does not list.
fn print_cases(r: &store::RunResult) {
    const SHOWN: usize = 10;
    let failed: Vec<&counts::Case> = r.cases.iter().filter(|c| c.failed).collect();
    for case in failed.iter().take(SHOWN) {
        println!("  {}", case.line().unwrap_or_default());
    }
    if failed.len() > SHOWN {
        println!("  and {} more failed", failed.len() - SHOWN);
    }
    for case in r.cases.iter().filter(|c| c.flaky && !c.failed) {
        println!("  {}", case.line().unwrap_or_default());
    }
    if let Some(case) = failed.iter().find(|c| c.flaky) {
        println!(
            "  fix: make it pass every time; only with the developer's OK, flaky_tests = [\"{}\"] on \
             check {} lets it pass on a rerun, which weakens the proof",
            case.name, r.check
        );
    }
}

/// A run's proof as it is built, and the run on the proof host while it takes it. Proving a
/// commit for CI shows more besides the usual lines, so a reader can tell the checks ran: the
/// commit and each check with its command and time, and on GitHub Actions the job summary.
/// Only CI writes these, so the agent whose work they judge cannot change them. A local run
/// has a report only while the host takes it, signed in: the host shows it unproven.
struct Report {
    proof: proof::Proof,
    /// The run on the proof host, while it takes it.
    stream: Option<host::Stream>,
    /// The host's page of the work, once the host opened the run.
    page: Option<String>,
    /// A folder for the proof bundle, when asked.
    bundle: Option<PathBuf>,
    /// Proving a commit for CI, not a local run.
    ci: bool,
    /// The platform of the leg it runs, which names each of its checks.
    platform: Option<String>,
}

/// What proving a commit for CI takes besides the checks.
struct CiOut {
    /// A folder for the proof bundle.
    proof: Option<PathBuf>,
    /// What GitHub Actions says of the run: the pull request and pages the proof names.
    workflow: action::Workflow,
    /// The leg of a plan it runs, for a commit proven in several jobs.
    leg: Option<legs::Asked>,
}

/// The proof of the commit CI checked out, before any check: the pull request's head merged
/// into the base branch, or HEAD.
fn ci_proof(ctx: &Ctx, workflow: &action::Workflow) -> Result<proof::Proof> {
    let merge = ctx.git.head_short()?;
    // A pull request's run proves the merge GitHub made of its head into the base branch;
    // the head is the commit people know.
    let full = workflow.head().map(str::to_string);
    let short = full.as_ref().map(|sha| {
        // A shallow checkout may lack the head itself.
        ctx.git
            .short(sha)
            .unwrap_or_else(|_| sha.chars().take(7).collect())
    });
    let head = match full {
        Some(sha) => sha,
        None => ctx.git.head()?,
    };
    let commit = match short {
        Some(head) if head != merge => {
            format!("{head} (merged with {} as {merge})", ctx.config.base)
        }
        _ => merge,
    };
    Ok(proof::Proof {
        repository: workflow.repository.clone(),
        pull: workflow.pull(),
        run_url: workflow.run_url.clone(),
        ..proof::Proof::new(commit, head, ctx.config.base.clone())
    })
}

impl Report {
    fn start(
        ctx: &Ctx,
        wt: &Worktree,
        targets: &[&config::Check],
        out: CiOut,
        leg: Option<&legs::Leg<'_>>,
    ) -> Result<Report> {
        let to_run = targets.len();
        let mut proof = ci_proof(ctx, &out.workflow)?;
        let of = match leg {
            Some(leg) => format!("leg {}: ", leg.id),
            None => String::new(),
        };
        println!(
            "commit {}: {} changed files, {of}{to_run} of {} checks to run",
            proof.commit,
            wt.changed.len(),
            ctx.config.checks.len()
        );
        proof.leg = leg.map(|l| l.id.clone());
        let planned = out.leg.map(|a| a.planned).unwrap_or_default();
        let stream = host::Host::from_actions(ctx.config.host.as_deref())
            .and_then(|host| open_run(host, &proof, &out.workflow, targets, &planned));
        Ok(Report {
            page: stream.as_ref().map(|s| s.url.clone()),
            stream,
            proof,
            bundle: out.proof,
            ci: true,
            platform: leg.and_then(legs::Leg::platform_name),
        })
    }

    /// A local run on the proof host, when the developer signed in to it and the branch is on
    /// GitHub: opened before its checks run, so a person can leave notes on its pictures
    /// before there is a pull request. None otherwise, and then nothing leaves the machine.
    fn local(ctx: &Ctx, targets: &[&config::Check]) -> Result<Option<Report>> {
        let url = host_url(Some(&ctx.config));
        let Some(login) = feedback::login_of(&url) else {
            return Ok(None);
        };
        let (Some(repository), Ok(branch)) = (ctx.git.github_repository(), ctx.git.branch()) else {
            return Ok(None);
        };
        let head = ctx.git.head()?;
        let proof = proof::Proof {
            repository: Some(repository.clone()),
            ..proof::Proof::new(ctx.git.head_short()?, head.clone(), ctx.config.base.clone())
        };
        let open = fearlezz_proof::events::Open {
            repository,
            branch,
            pull: None,
            head,
            base: ctx.config.base.clone(),
            source: fearlezz_proof::events::Source::Local { login: login.login },
            checks: targets.iter().map(|c| c.id.clone()).collect(),
        };
        let stream = match host::Host::from_login(&url, &login.token).open(&open) {
            Ok(host::Opening::Opened(stream)) => {
                println!(
                    "proof page: {} shows this run, unproven; fearlezz setup logout stops sending \
                     local runs",
                    stream.url
                );
                stream
            }
            Ok(host::Opening::NotInstalled(install)) => {
                println!(
                    "proof page: the Fearlezz app is not installed on this repository, so this \
                     run was not sent; install it to see local runs on the host: {install}"
                );
                return Ok(None);
            }
            Err(e) => {
                println!("proof page not sent: {e:#}");
                return Ok(None);
            }
        };
        Ok(Some(Report {
            page: Some(stream.url.clone()),
            stream: Some(stream),
            proof,
            bundle: None,
            ci: false,
            platform: None,
        }))
    }

    /// A check starts.
    fn starts(&mut self, check: &config::Check) {
        self.stream_step(|s| s.started(&check.id));
    }

    /// A check could not run here.
    fn not_run(&mut self, check: &config::Check, stage: &str, why: &str) {
        let mut entry = proof::not_run(check, stage, why);
        entry.platform.clone_from(&self.platform);
        self.stream_step(|s| s.finished(&entry, None));
        self.proof.checks.push(entry);
    }

    /// Sends one step of the run to the host; a host that fails is said once and sent
    /// nothing more, and never fails the proof.
    fn stream_step(&mut self, step: impl FnOnce(&host::Stream) -> Result<()>) {
        if let Some(stream) = &self.stream
            && let Err(e) = step(stream)
        {
            println!("proof page not sent: {e:#}");
            self.stream = None;
        }
    }

    /// A check ended, with its pictures compared with the base when it takes pictures.
    fn ran(
        &mut self,
        check: &config::Check,
        stage: &str,
        result: &store::RunResult,
        look: Option<&look::Look>,
    ) {
        let word = match result.outcome {
            Outcome::Passed => "passed",
            Outcome::Failed => "failed",
        };
        #[expect(clippy::cast_precision_loss, reason = "a duration shown in seconds")]
        let secs = result.duration_ms as f64 / 1000.0;
        let run = match check.run.trim().split_once('\n') {
            Some((first, _)) => format!("{first} ..."),
            None => check.run.trim().to_string(),
        };
        let tests = match result.tests {
            Some(1) => "  1 test".to_string(),
            Some(n) => format!("  {n} tests"),
            None => String::new(),
        };
        let coverage = result
            .coverage
            .map(|m| format!("  {}", m.summary()))
            .unwrap_or_default();
        if self.ci {
            println!("{word} {}  {secs:.1} s{tests}{coverage}  {run}", check.id);
        }
        let failure = (result.outcome == Outcome::Failed).then(|| failure_item(result));
        let mut entry = proof::ran(check, stage, result, failure);
        entry.platform.clone_from(&self.platform);
        self.stream_step(|s| s.finished(&entry, look));
        self.proof.checks.push(entry);
    }

    /// Judges what the change weakens, writes the bundle when asked, closes the run on the
    /// proof host with it, and writes the job summary GitHub Actions names, when it names one.
    /// Before it judges, a CI run the host took asks it whether an admin accepted exactly what
    /// the change weakens at this head; a host that does not answer means not accepted. A
    /// local run never asks. The gate it answers says where the weakening got past, or can: a
    /// host that failed the ask or done holds no run an accept can reach.
    fn finish(mut self, floor: &floor::Floor) -> Result<floor::Gate> {
        use std::io::Write as _;
        self.proof.weakened.clone_from(&floor.weakened);
        let hosted = self.ci && self.stream.is_some();
        if hosted && !floor.holds() {
            let head = self.proof.head.clone();
            let mut accepted = None;
            self.stream_step(|s| {
                accepted = s
                    .accepted(&floor.weakened)?
                    .filter(|a| a.covers(&head, &floor.weakened));
                Ok(())
            });
            self.proof.accepted = accepted;
        }
        let temp = match (&self.bundle, &self.stream) {
            (None, Some(_)) => Some(tempfile::tempdir()?),
            _ => None,
        };
        let bundle = self
            .bundle
            .clone()
            .or_else(|| temp.as_ref().map(|t| t.path().to_path_buf()));
        if let Some(dir) = &bundle {
            proof::write(&self.proof, dir)?;
        }
        if let Some(dir) = &bundle {
            self.stream_step(|s| s.done(dir));
        }
        // The gate of a split proof streams nothing; it asks the host through a leg's run.
        if let (Some(dir), Some(stream), Some(_)) = (&self.bundle, &self.stream, &self.proof.leg) {
            std::fs::write(dir.join(legs::HOST_RUN), stream.run())?;
        }
        let gate = match self.proof.accepted.clone() {
            Some(accept) => floor::Gate::Accepted(accept),
            None if hosted && self.stream.is_some() => floor::Gate::Host,
            None if hosted => floor::Gate::Unasked,
            None => floor::Gate::Job,
        };
        if let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY").filter(|_| self.ci) {
            let summary = action::summary(&self.proof, self.page.as_deref(), &gate)?;
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(Path::new(&path))?
                .write_all(summary.as_bytes())?;
        }
        Ok(gate)
    }
}

/// Opens the run on the proof host, printing its page or why there is none. A host that
/// fails never fails the proof: CI proves the commit anyway.
fn open_run(
    host: host::Host,
    proof: &proof::Proof,
    workflow: &action::Workflow,
    targets: &[&config::Check],
    planned: &[String],
) -> Option<host::Stream> {
    let (Some(repository), Some(branch)) = (&workflow.repository, workflow.branch()) else {
        println!(
            "proof page not sent: the run names no repository and branch (GITHUB_REPOSITORY, \
             GITHUB_REF)"
        );
        return None;
    };
    let Some(ci_job) = &workflow.ci_job else {
        println!(
            "proof page not sent: the run names no job of a workflow run (GITHUB_RUN_ID, \
             GITHUB_RUN_ATTEMPT, RUNNER_NAME)"
        );
        return None;
    };
    let open = fearlezz_proof::events::Open {
        repository: repository.clone(),
        branch: branch.to_string(),
        pull: proof.pull.clone(),
        head: proof.head.clone(),
        base: proof.base.clone(),
        source: fearlezz_proof::events::Source::Ci {
            job: workflow.job.clone().unwrap_or_default(),
            run_url: workflow.run_url.clone(),
            run_id: ci_job.run_id,
            run_attempt: ci_job.run_attempt,
            runner_name: ci_job.runner_name.clone(),
            leg: proof.leg.clone(),
            planned: planned.to_vec(),
        },
        checks: targets.iter().map(|c| c.id.clone()).collect(),
    };
    match host.open(&open) {
        Ok(host::Opening::Opened(stream)) => {
            println!("proof page: {}", stream.url);
            Some(stream)
        }
        Ok(host::Opening::NotInstalled(install)) => {
            println!(
                "proof page: the Fearlezz app is not installed on this repository, so nothing \
                 was sent; install it to get the page: {install}"
            );
            None
        }
        Err(e) => {
            println!("proof page not sent: {e:#}");
            None
        }
    }
}

/// How to make a missing tool reachable for checks.
fn install_fix(tool: &str) -> String {
    let nix = if run::NIX_TOOLS.contains(&tool) {
        format!(" (in devenv.nix: add `pkgs.{tool}` to packages)")
    } else {
        String::new()
    };
    format!(
        "install {tool} where checks find it: the project's dev dependencies ({}), its devenv{nix}, \
         or PATH",
        run::TOOL_DIRS.join(", ")
    )
}

/// How to let SecretSpec give a check the secret `name`.
fn secret_fix(name: &str) -> String {
    format!(
        "declare {name} in secretspec.toml and store it with its provider (1Password: \
         `secretspec set {name}`); sign in to the provider locally (`op signin`), and give CI \
         its token (1Password: the OP_SERVICE_ACCOUNT_TOKEN secret in the job's env)"
    )
}

/// Why a check's preview could not be deployed, and what to do about it.
fn no_preview(why: &env::NoPreview) -> (String, String) {
    let by_hand = "run `[env] preview` by hand at the checkout root, with FEARLEZZ_PREVIEW_NAME set, \
                   and read its log (above)";
    match why {
        env::NoPreview::Failed { code } => (
            format!(
                "the preview failed (exit {})",
                code.map_or_else(|| "signal".to_string(), |c| c.to_string())
            ),
            format!("make it deploy: {by_hand}"),
        ),
        env::NoPreview::NoUrl => (
            "the preview printed no URL".to_string(),
            format!(
                "make [env] preview print the URL of its deploy (the last http:// or https:// URL \
                 it prints is taken): {by_hand}"
            ),
        ),
        env::NoPreview::Secret { name } => (
            format!("SecretSpec could not give the preview {name}"),
            secret_fix(name),
        ),
        env::NoPreview::NoSecretSpec => (
            "secretspec was not found for the preview's secrets".to_string(),
            install_fix("secretspec"),
        ),
        env::NoPreview::Unset => (
            "its checkout's [env] has no preview command".to_string(),
            "add a preview command to [env] in fearlezz.toml".to_string(),
        ),
    }
}

fn failure_item(f: &store::RunResult) -> proof::Item {
    let what = format!("check `{}` failed", f.check);
    if let Some(why) = &f.no_preview {
        let (why, fix) = no_preview(why);
        return proof::Item { what, why, fix };
    }
    if let Some(name) = &f.missing_secret {
        return proof::Item {
            what,
            why: format!("SecretSpec could not give it {name}"),
            fix: secret_fix(name),
        };
    }
    if let Some(tool) = f
        .not_found()
        .then(|| run::missing_command(&f.log))
        .flatten()
    {
        return proof::Item {
            what,
            why: format!("{tool} was not found"),
            fix: install_fix(&tool),
        };
    }
    if let Some(found) = f.findings.iter().find(|x| x.level == findings::Level::Gate) {
        return proof::Item {
            what,
            why: format!("blocking findings, such as {found}"),
            fix: "fix the findings; the check page lists them".into(),
        };
    }
    if f.tests == Some(0) && f.exit_code == Some(0) {
        let detail = f
            .proof_errors
            .first()
            .map(|e| format!(": {e}"))
            .unwrap_or_default();
        return proof::Item {
            what,
            why: format!("it ran no tests, so it proves nothing{detail}"),
            fix: "make it run its tests, or give it the tests reader its runner needs".into(),
        };
    }
    if let (Some(0), Some(why)) = (f.exit_code, f.proof_errors.first()) {
        return proof::Item {
            what,
            why: why.clone(),
            fix: "make its command leave what the check reads".into(),
        };
    }
    let code = f
        .exit_code
        .map_or_else(|| "a signal".to_string(), |c| format!("exit code {c}"));
    proof::Item {
        what,
        why: format!("its command ended with {code}"),
        fix: "fix what it reports; the check page shows the end of its output".into(),
    }
}

/// Runs the checks `ids` names, or those the stage needs, and says what is next. `out` is
/// given when proving a commit for CI.
fn run_checks(
    ctx: &Ctx,
    mut j: Journal,
    ids: &[String],
    fresh: bool,
    out: Option<CiOut>,
) -> Result<bool> {
    let mut wt = ctx.worktree()?;
    let before = state::view(&ctx.config, &ctx.store, &j, Some(&mut wt), ctx.now)?;
    let stages = &ctx.config.stages;
    let stage = stages.name(before.stage).to_string();
    let final_stage = stages.name(stages.last()).to_string();
    let in_memory = j.is_in_memory();
    let leg = match out.as_ref().and_then(|o| o.leg.as_ref()) {
        Some(asked) => Some(legs::leg(&ctx.config, &asked.id, &asked.checks)?),
        None => None,
    };
    // A leg proves its platform only on a machine that fits it.
    if let Some(p) = leg
        .as_ref()
        .and_then(|l| l.platform)
        .filter(|p| !p.fits_here())
        && let Some(os) = &p.os
    {
        bail!(
            "leg {} needs {os}, and this machine runs {}; give platform {} a runner of {os} \
             (runs-on)",
            leg.as_ref().map_or("", |l| l.id.as_str()),
            std::env::consts::OS,
            p.name
        );
    }
    let mut targets = match &leg {
        Some(leg) => leg.checks.clone(),
        None => targets(ctx, &before, ids, in_memory)?,
    };
    // Outside a leg, a check this machine can stand in for none of the platforms of is left
    // to CI, where its leg proves it.
    // CI without a leg cannot prove them: each is not proven.
    let mut left_out = Vec::new();
    if leg.is_none() {
        targets.retain(|check| legs::left_to_ci(&ctx.config, check).is_none());
        let due = ctx.config.checks.iter().filter(|c| {
            (in_memory || c.stage <= before.stage) && (ids.is_empty() || ids.contains(&c.id))
        });
        for check in due {
            if let Some(on) = legs::left_to_ci(&ctx.config, check) {
                if out.is_some() {
                    for platform in &on {
                        let entry = legs::left_out(check, platform, stages.name(check.stage));
                        if let Some(item) = &entry.failure {
                            println!("blocked: {}: {}", item.what, item.why);
                            println!("  fix: {}", item.fix);
                        }
                        left_out.push(entry);
                    }
                } else {
                    println!("left to CI {}: runs on {}", check.id, on.join(", "));
                }
            }
        }
    }
    let proven_here = left_out.is_empty();
    // A full run at the final stage answers for the whole work, and for what the base
    // branch requires, not only for its checks; for a leg, the gate does.
    let floor = if ids.is_empty() && stage == final_stage {
        floor::Floor::read(&ctx.config, &wt)?
    } else {
        floor::Floor::default()
    };
    let in_leg = leg.is_some();
    let answers_for_work = !in_memory && ids.is_empty() && stage == final_stage;
    // The floor holds when nothing weakens, or when an admin accepted on the proof host
    // exactly what weakens at this head (the gate the commit report answers).
    let settled = |view: &View, floor_holds: bool| {
        floor_holds && (!answers_for_work || view.next == state::READY || view.checks.is_empty())
    };
    let next_line = |view: &View, floor_holds: bool| {
        if in_memory {
            state::COMMIT_PROVEN.to_string()
        } else if !floor_holds {
            state::FLOOR_NEXT.to_string()
        } else {
            view.next.clone()
        }
    };
    let finish = |report: Option<Report>| -> Result<bool> {
        let gate = report.map_or(Ok(floor::Gate::Job), |r| r.finish(&floor))?;
        // A leg's proof names what the change weakens, so the host shows it; the gate judges.
        if in_leg {
            if !floor.holds() {
                println!("the gate judges what the change weakens");
            }
            return Ok(true);
        }
        for line in floor.lines(&gate) {
            println!("{line}");
        }
        Ok(floor.holds_at(&gate))
    };
    // A pull request's CI checks out a merge commit with no branch; its number names the
    // preview, so each push updates one preview, and two forks' branches never share one.
    let branch = out
        .as_ref()
        .and_then(|o| o.workflow.pull())
        .map(|pull| format!("pr-{}", pull.number))
        .or_else(|| wt.git.branch().ok());
    let mut report = match out {
        Some(out) => Some(Report::start(ctx, &wt, &targets, out, leg.as_ref())?),
        None if targets.is_empty() => None,
        None => Report::local(ctx, &targets)?,
    };
    if let Some(r) = report.as_mut() {
        r.proof.checks.extend(left_out);
    }
    if targets.is_empty() {
        // Every check waits in a later stage: say where.
        let later = (ids.is_empty() && !in_memory)
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
        let floor_holds = finish(report)?;
        match &later {
            Some((next_stage, _)) if floor_holds => {
                println!("next: fearlezz run --stage {next_stage}");
            }
            _ => println!("next: {}", next_line(&before, floor_holds)),
        }
        return Ok(proven_here && settled(&before, floor_holds));
    }

    let clock = std::time::Instant::now();
    let (mut passed, mut reused) = (0, 0);
    let mut failures = Vec::new();
    let mut unstable = Vec::new();
    let mut green = Vec::new();
    let env = ctx.env();
    let mut prepared = false;
    let mut held = None;
    let mut looks = Vec::new();
    // Opened once, on the first check with pictures.
    let mut base = None;
    let (mut not_run, mut cannot_record) = (0, 0);
    for check in &targets {
        let inputs = wt.inputs(check)?;
        // Reuse only a pass, never across an unstable key, and at the final stage only a run
        // this journal made at that stage.
        let reuse = if fresh || ctx.store.unstable(&inputs.key) {
            None
        } else {
            ctx.store.for_key(&inputs.key).pop().filter(|r| {
                r.outcome == Outcome::Passed
                    && (stage != final_stage || r.made_at_final(&j.name, &final_stage))
            })
        };
        // A check that records a window runs only where something can record. CI does not
        // need it (looks of app windows are proven on a developer's machine); locally it
        // fails with the one time fix. Neither is stored, so it is not called unstable once
        // recording works.
        if reuse.is_none()
            && check.pictures.record.is_some()
            && let Some(u) = record::unavailable()
        {
            if let Some(r) = report.as_mut().filter(|r| r.ci) {
                println!("not run {}: {}", check.id, u.why);
                r.not_run(check, stages.name(check.stage), &u.why);
                not_run += 1;
            } else {
                println!("fail {}: {}", check.id, u.why);
                println!("  fix: {}", u.fix);
                if let Some(r) = report.as_mut() {
                    r.not_run(check, stages.name(check.stage), &u.why);
                }
                cannot_record += 1;
            }
            continue;
        }
        if let Some(r) = report.as_mut() {
            r.starts(check);
        }
        let (result, was_reused) = if let Some(r) = reuse {
            (r, true)
        } else {
            if !prepared {
                if let Some(line) = env.state.prepare(env.cfg, &env.main, &wt.git.root)? {
                    println!("{line}");
                }
                prepared = true;
            }
            let mut vars = std::collections::BTreeMap::new();
            if check.needs_services() {
                if held.is_none() {
                    held = Some(env.state.hold(&wt.git.root)?);
                }
                let fresh = stage == final_stage;
                let (services, line) =
                    env.state
                        .ensure_up(env.cfg, &wt.git.root, &j.name, fresh, ctx.now)?;
                if let Some(line) = line {
                    println!("{line}");
                }
                vars = services.ports;
            }
            let mut undeployed = None;
            if check.needs_preview() {
                if held.is_none() {
                    held = Some(env.state.hold(&wt.git.root)?);
                }
                let name = env::preview_name(branch.as_deref(), &wt.git.head_short()?);
                match env.state.preview(env.cfg, &wt.git.root, &wt.tree, &name)? {
                    Ok(deployed) => {
                        if let Some(line) = deployed.line {
                            println!("{line}");
                        }
                        vars.insert("FEARLEZZ_PREVIEW_URL".to_string(), deployed.url);
                    }
                    Err(u) => undeployed = Some(u),
                }
            }
            let at = run::Where {
                root: &wt.git.root,
                shell: wt.shell.as_deref(),
                tree: &wt.tree,
                journal: &j.name,
                stage: &stage,
                vars: &vars,
                changed: &wt.lines,
                no_preview: undeployed.as_ref(),
            };
            let result = run::execute(&ctx.store, check, &inputs, &at)?;
            if check.needs_services() {
                // Idle time counts from when the check stopped using them.
                let used = ctx
                    .now
                    .checked_add(jiff::SignedDuration::try_from(clock.elapsed())?)?;
                env.state.touch(&wt.git.root, used)?;
            }
            (result, false)
        };
        j.append(
            ctx.now,
            Event::Ran {
                check: check.id.clone(),
                key: inputs.key.clone(),
                result: result.id.clone(),
                outcome: result.outcome,
                tree: wt.tree.clone(),
                reused: was_reused,
            },
        )?;
        reused += usize::from(was_reused);
        // Compared as soon as the check passed, so its look goes with it to the proof host.
        let look = if result.outcome == Outcome::Passed {
            look_at(ctx, &wt, check, &result, &stage, &mut base)?
        } else {
            None
        };
        if let Some(r) = report.as_mut() {
            r.ran(check, stages.name(check.stage), &result, look.as_ref());
        }
        looks.extend(look);
        match result.outcome {
            Outcome::Passed => {
                passed += 1;
                green.push((*check, result));
            }
            Outcome::Failed => failures.push(result),
        }
        if ctx.store.unstable(&inputs.key) {
            unstable.push(check.id.clone());
        }
    }
    for id in &unstable {
        println!(
            "unstable {id}: same inputs gave passed and failed (flaky, or an input not declared); \
             it stays unstable until its inputs change"
        );
    }
    failures.iter().for_each(print_failure);
    for (_, r) in &green {
        print_cases(r);
        print_findings(r);
    }
    if let Some(r) = report.as_mut() {
        r.proof.looks = looks;
        // The preview of the tree this run proved, deployed now or by an earlier run.
        if targets.iter().any(|c| c.needs_preview()) {
            r.proof.preview = env.state.preview_of(&wt.git.root, &wt.tree);
        }
        for id in &unstable {
            if let Some(c) = r.proof.check_mut(id) {
                c.unstable = true;
            }
        }
    }
    // In CI, and at the final stage of a journal so the agent learns it before CI.
    // It marks each run, so `fearlezz` and the hooks hold the work to the same verdict.
    let red_blocks = (in_memory || stage == final_stage)
        && red_before_green(ctx, &wt, &green, &stage, report.as_mut())?;
    let not_run_text = if not_run > 0 {
        format!(", {not_run} not run")
    } else {
        String::new()
    };
    println!(
        "{} checks: {passed} passed, {} failed, {reused} reused{not_run_text}, {:.1} s",
        targets.len(),
        failures.len() + cannot_record,
        clock.elapsed().as_secs_f64()
    );
    let floor_holds = finish(report)?;
    // Only now: the commit report reads this run's results, which no journal names in CI, so
    // a check sharing its inputs with another could lose its output to the prune.
    prune_store(ctx);
    let after = state::view(&ctx.config, &ctx.store, &j, Some(&mut wt), ctx.now)?;
    if stage == final_stage
        && (after.next == state::READY || in_memory)
        && env.state.services(&wt.git.root).is_some()
    {
        env.state.stop(env.cfg, &wt.git.root)?;
        println!("services stopped: all proof is in");
    }
    if let Some(line) = after.pictures_to_review(&ctx.config) {
        println!("{line}");
    }
    if cannot_record > 0 {
        println!("next: give the recorder its permission (fix: above), then fearlezz run");
    } else {
        println!("next: {}", next_line(&after, floor_holds));
    }
    Ok(proven_here
        && failures.is_empty()
        && cannot_record == 0
        && !red_blocks
        && settled(&after, floor_holds))
}

/// Runs each green check that reads changed test files on the base commit with those files,
/// one line per check, noted in the commit report when there is one, and marks each run with
/// what it found. True when the change is a fix and its tests do not fail there, or could not
/// run there: what cannot be decided is not proven.
fn red_before_green(
    ctx: &Ctx,
    wt: &Worktree,
    green: &[(&config::Check, store::RunResult)],
    stage: &str,
    mut report: Option<&mut Report>,
) -> Result<bool> {
    let fix = red::is_fix_change(&wt.git, &wt.base)?;
    let mut blocks = false;
    for (check, head) in green {
        let ran = red::run(
            &ctx.git,
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
            if let Some(c) = report.as_mut().and_then(|r| r.proof.check_mut(id)) {
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

/// Compares the stills and films of a green check with the base commit (`base`, opened on the
/// first check that needs it), says what it found, and records the comparison against it. The
/// look it made, if any.
fn look_at(
    ctx: &Ctx,
    wt: &Worktree,
    check: &config::Check,
    result: &store::RunResult,
    stage: &str,
    base: &mut Option<Result<look::Base>>,
) -> Result<Option<look::Look>> {
    // A check that takes pictures is compared even without any, so all gone shows.
    let has_media = look::has_media(result);
    if !has_media && !ctx.config.takes_pictures(check) {
        return Ok(None);
    }
    let base =
        base.get_or_insert_with(|| look::Base::at(&ctx.git, &ctx.config, wt.base.clone(), ""));
    let outcome = match base {
        Ok(base) => look::look(&ctx.store, check, result, base, stage, &ctx.env()),
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
                "look {}: inputs are the same as on the base, so the pictures are too",
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
        for f in c.findings.iter().take(10) {
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
    let stale = v.checks.iter().filter_map(|c| match &c.state {
        CheckState::Stale(why) => Some((&c.id, why)),
        _ => None,
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
fn prune_store(ctx: &Ctx) {
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
