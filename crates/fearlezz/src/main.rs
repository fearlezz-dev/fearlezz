//! `fearlezz`: helps coding agents prove their work. See the README for the model.

mod action;
mod config;
mod counts;
mod env;
mod findings;
mod floor;
mod git;
mod github;
mod hooks;
mod host;
mod init;
mod journal;
mod lints;
mod look;
mod media;
mod overview;
mod page;
mod presets;
mod proof;
mod record;
mod red;
mod report;
mod rules;
mod run;
mod settings;
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
    /// Write the overview page of every journal.
    #[arg(long)]
    page: bool,
    /// A still of a running app's window, to look at while working (macOS): a pid, bundle
    /// id, app name or part of the window title. Prints the PNG's path.
    #[arg(long, value_name = "WINDOW")]
    still: Option<String>,
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
        /// Prove this commit, for CI: every check, fresh, with no journal. On
        /// GitHub Actions with a token it also does what the event asks: a pull request gets
        /// its pictures, status comment and status, `/fearlezz approve` and `/fearlezz accept`
        /// are answered, and closed
        /// pull requests lose their pictures.
        #[arg(long, conflicts_with_all = ["checks", "stage"])]
        ci: bool,
        /// With --ci: also write the status comment (Markdown) to this file, for a CI other
        /// than GitHub Actions to post on its merge request.
        #[arg(long, value_name = "PATH", requires = "ci")]
        comment: Option<PathBuf>,
        /// With --ci: also write everything the run showed into this folder: `proof.json`
        /// with each check's output and every picture beside it, for a page to show.
        #[arg(long, value_name = "DIR", requires = "ci")]
        proof: Option<PathBuf>,
    },
    /// Tell: record what only you know, such as a note, a question, or an approval.
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
    /// Record that the developer agreed with a check's changed stills and films.
    Approve {
        check: String,
        /// The developer's own words, quoted.
        #[arg(long, required_unless_present = "github", conflicts_with = "github")]
        said: Option<String>,
        /// Read the approval a person gave on the pull request with `/fearlezz approve`.
        #[arg(long)]
        github: bool,
    },
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

/// Proves this commit for CI. On GitHub Actions it does what the event asks: a pull request's
/// proof also pushes its changed pictures, writes its status comment and sets its status; a
/// comment, a closed pull request or the schedule need no checkout.
fn ci_run(comment: Option<PathBuf>, proof: Option<PathBuf>) -> Result<bool> {
    let workflow = action::Workflow::read();
    let actions = action::Actions::detect(&workflow)?;
    if let Some(gh) = &actions {
        let task = gh.task();
        if task != action::Task::Prove {
            return gh.act(&task);
        }
    }
    let ctx = prepared()?;
    if actions.is_some()
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
    let pull = actions.as_ref().and_then(action::Actions::pull);
    // The action keeps this folder as the run's `fearlezz-pictures` artifact.
    let pictures = actions.as_ref().and_then(|_| {
        std::env::var_os("RUNNER_TEMP").map(|t| PathBuf::from(t).join("fearlezz-pictures"))
    });
    let mut journal = Journal::in_memory("ci");
    let to = ctx.config.stages.name(ctx.config.stages.last()).to_string();
    journal.append(ctx.now, Event::Stage { to, reason: None })?;
    let out = CiOut {
        comment,
        proof,
        pictures,
        pull,
        workflow,
    };
    run_checks(&ctx, journal, &[], true, out)
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
            comment,
            proof,
        } => {
            if ci {
                return ci_run(comment, proof);
            }
            let ctx = prepared()?;
            if let Some(name) = stage {
                move_stage(&ctx, name, reason)?;
                if no_run {
                    println!("next: {}", ctx.view(&ctx.journal()?)?.next);
                    return Ok(true);
                }
            }
            run_checks(&ctx, ctx.journal()?, &checks, fresh, CiOut::default())
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
    };
    Ok(hooks::judge(&ctx.config, &work, event, input))
}

fn say(ctx: &Ctx, what: Say) -> Result<bool> {
    match what {
        Say::Approve {
            check,
            github: true,
            ..
        } => approve_from_github(ctx, &check)?,
        Say::Approve { check, said, .. } => {
            let said = said.unwrap_or_default().trim().to_string();
            if said.is_empty() {
                bail!("approving needs the developer's own words: --said \"<what they said>\"");
            }
            let mut j = ctx.journal()?;
            let v = ctx.view(&j)?;
            let view = v.checks.iter().find(|c| c.id == check).with_context(|| {
                format!(
                    "nothing to approve: {check} has not run in journal {}",
                    j.name
                )
            })?;
            let look = match (&view.state, &view.look) {
                (CheckState::Ok | CheckState::NeedsFresh, Some(l)) if l.to_look_at() > 0 => {
                    l.clone()
                }
                _ => bail!(
                    "nothing to approve: {check} has no changed stills or films on the current inputs"
                ),
            };
            let key = ctx.worktree()?.inputs(ctx.config.check(&check)?)?.key;
            j.append(
                ctx.now,
                Event::Approve {
                    check: check.clone(),
                    key,
                    said: said.clone(),
                    by: None,
                    url: None,
                },
            )?;
            println!("approved {check}: {} (\"{said}\")", look.counts());
            println!("next: {}", ctx.view(&j)?.next);
        }
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

/// Records the approval given on the pull request for the commit checked out here, by the
/// rule CI settles the status with (`github::Pending`), for a check whose pictures waited.
fn approve_from_github(ctx: &Ctx, check: &str) -> Result<()> {
    let config = ctx.config.check(check)?;
    let mut wt = ctx.worktree()?;
    if !wt.committed {
        bail!(
            "an approval on the pull request is for a commit: commit and push the work, wait for \
             its proof, then run this again"
        );
    }
    let head = ctx.git.head()?;
    let approval = github::approval_of_commit(&ctx.git.root, &head, check)?;
    let key = wt.inputs(config)?.key;
    let mut j = ctx.journal()?;
    j.append(
        ctx.now,
        Event::Approve {
            check: check.to_string(),
            key,
            said: approval.said.clone(),
            by: Some(approval.by.clone()),
            url: Some(approval.url),
        },
    )?;
    println!(
        "approved {check} on GitHub by @{} (\"{}\")",
        approval.by, approval.said
    );
    println!("next: {}", ctx.view(&j)?.next);
    Ok(())
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

fn setup(what: Setup) -> Result<bool> {
    match what {
        Setup::Init => setup_init()?,
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
        print_status(&ctx, &v);
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
    } else if see.page {
        let views = ctx.all_views()?;
        let html = overview::render(&ctx.config, &views, ctx.now)?;
        let dir = ctx.git.common.join("fearlezz");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("overview.html");
        std::fs::write(&path, html)?;
        println!("wrote {}", path.display());
        println!("next: open it in a browser");
    } else {
        let j = ctx.journal()?;
        print_status(&ctx, &ctx.view(&j)?);
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
    println!("next: {}", v.next);
    Ok(())
}

/// The checks a run takes on: the named ones, every check when proving a commit, and otherwise
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
        if let Some(tool) = f
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
        } else {
            println!("fail {} (exit {code})", f.check);
        }
        for line in run::tail(&f.log, 12) {
            println!("  {line}");
        }
        println!("  log: {}", f.log.display());
    }
    print_findings(f);
}

/// What proving a commit shows besides the usual lines, so a reader can tell the checks ran:
/// the commit and each check with its command and time, and for GitHub the check page (the
/// job summary) and the short status for the pull request. Only CI writes these, so the agent
/// whose work they judge cannot change them.
struct CommitReport {
    proof: proof::Proof,
    /// The proof host the config names, when it is not the default one.
    host: Option<String>,
    /// Where the short status, the bundle and the changed pictures go, when asked.
    out: CiOut,
}

/// What proving a commit writes for CI besides its output, when asked.
#[derive(Default)]
struct CiOut {
    /// A file for the status comment, for a CI other than GitHub Actions.
    comment: Option<PathBuf>,
    /// A folder for the proof bundle.
    proof: Option<PathBuf>,
    /// On GitHub Actions: a folder for the before and after of each changed picture, kept as
    /// the run's artifact.
    pictures: Option<PathBuf>,
    /// The pull request on GitHub the proof reports to.
    pull: Option<action::Pull>,
    /// What GitHub Actions says of the run: the pull request and pages the proof names.
    workflow: action::Workflow,
}

impl CiOut {
    /// Whether what the change weakens waits for an admin's accept rather than failing the
    /// proof: only on a pull request from the repository itself, whose `fearlezz/approval`
    /// status makes it wait. A run of it without a token (another job of the same pull request)
    /// waits too, since the run with the token holds that status; a fork's run cannot.
    fn weakenings_wait(&self) -> bool {
        self.workflow.own_pull_request()
    }
}

impl CommitReport {
    fn start(ctx: &Ctx, wt: &Worktree, to_run: usize, out: CiOut) -> Result<CommitReport> {
        let merge = ctx.git.head_short()?;
        // A pull request's run proves the merge GitHub made of its head into the base branch;
        // the head is the commit people know.
        let full = out.workflow.head().map(str::to_string);
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
        println!(
            "commit {commit}: {} changed files, {to_run} of {} checks to run",
            wt.changed.len(),
            ctx.config.checks.len()
        );
        let proof = proof::Proof {
            repository: out.workflow.repository.clone(),
            pull: out.workflow.pull(),
            run_url: out.workflow.run_url.clone(),
            ..proof::Proof::new(commit, head, ctx.config.base.clone())
        };
        Ok(CommitReport {
            proof,
            host: ctx.config.host.clone(),
            out,
        })
    }

    fn ran(&mut self, check: &config::Check, stage: &str, result: &store::RunResult) {
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
        println!("{word} {}  {secs:.1} s{tests}  {run}", check.id);
        let failure = (result.outcome == Outcome::Failed).then(|| failure_item(result));
        self.proof
            .checks
            .push(proof::ran(check, stage, result, failure));
    }

    /// Writes the check page into the job summary GitHub Actions names, when it names one,
    /// and the status comment to the file and the pull request, when asked.
    fn finish(mut self, floor: &floor::Floor) -> Result<()> {
        use std::io::Write as _;
        let waits = self.out.weakenings_wait();
        self.proof.weakenings_wait = waits;
        self.proof.weakened.clone_from(&floor.weakened);
        // A host that cannot be reached never fails the proof: CI proved the commit anyway.
        let host = host::Host::from_actions(self.host.as_deref());
        let temp = match (&self.out.proof, &host) {
            (None, Some(_)) => Some(tempfile::tempdir()?),
            _ => None,
        };
        let bundle = self
            .out
            .proof
            .clone()
            .or_else(|| temp.as_ref().map(|t| t.path().to_path_buf()));
        if let Some(dir) = &bundle {
            proof::write(&self.proof, dir)?;
        }
        let page = match (&host, &bundle) {
            (Some(host), Some(dir)) => match host.send(dir) {
                Ok(host::Sent::Page(url)) => {
                    println!("proof page: {url}");
                    Some(url)
                }
                Ok(host::Sent::NotInstalled(install)) => {
                    println!(
                        "proof page: the Fearlezz app is not installed on this repository, so \
                         nothing was sent; install it to get the page: {install}"
                    );
                    None
                }
                Err(e) => {
                    println!("proof page not sent: {e:#}");
                    None
                }
            },
            _ => None,
        };
        let report = github::Report::new(
            &self.proof,
            self.out.pull.as_ref().and_then(|p| p.url.as_deref()),
            self.out.pictures.is_some(),
            page,
        );
        if let Some(dir) = &self.out.pictures {
            github::copy_pictures(report.waits(), dir)?;
        }
        if let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") {
            let page = report.page()?;
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(Path::new(&path))?
                .write_all(page.as_bytes())?;
        }
        let comment = report.comment()?;
        if let Some(path) = &self.out.comment {
            std::fs::write(path, comment.render())
                .with_context(|| format!("writing {}", path.display()))?;
        }
        if let Some(pull) = &self.out.pull {
            pull.report(&comment, self.out.pictures.as_deref());
        }
        Ok(())
    }
}

/// A failed check as a blocker, for a reader who sees neither the agent nor the logs.
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

fn failure_item(f: &store::RunResult) -> proof::Item {
    let what = format!("check `{}` failed", f.check);
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
    let code = f
        .exit_code
        .map_or_else(|| "a signal".to_string(), |c| format!("exit code {c}"));
    proof::Item {
        what,
        why: format!("its command ended with {code}"),
        fix: "fix what it reports; the check page shows the end of its output".into(),
    }
}

fn run_checks(ctx: &Ctx, mut j: Journal, ids: &[String], fresh: bool, out: CiOut) -> Result<bool> {
    let mut wt = ctx.worktree()?;
    let before = state::view(&ctx.config, &ctx.store, &j, Some(&mut wt), ctx.now)?;
    let stages = &ctx.config.stages;
    let stage = stages.name(before.stage).to_string();
    let final_stage = stages.name(stages.last()).to_string();
    let in_memory = j.is_in_memory();
    let targets = targets(ctx, &before, ids, in_memory)?;
    // A full run at the final stage answers for the whole work, and for what the base
    // branch requires, not only for its checks.
    let floor = if ids.is_empty() && stage == final_stage {
        floor::Floor::read(&ctx.config, &wt)?
    } else {
        floor::Floor::default()
    };
    // On a pull request a weakening waits for an admin's accept; elsewhere it blocks.
    let waits = out.weakenings_wait();
    let lines = floor.lines(waits);
    let floor_holds = floor.holds() || waits;
    let answers_for_work = !in_memory && ids.is_empty() && stage == final_stage;
    let settled = |view: &View| {
        floor_holds && (!answers_for_work || view.next == state::READY || view.checks.is_empty())
    };
    let next_line = |view: &View| {
        if in_memory {
            state::COMMIT_PROVEN.to_string()
        } else if !floor_holds {
            state::FLOOR_NEXT.to_string()
        } else {
            view.next.clone()
        }
    };
    let mut report = in_memory
        .then(|| CommitReport::start(ctx, &wt, targets.len(), out))
        .transpose()?;
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
        for line in &lines {
            println!("{line}");
        }
        report.map_or(Ok(()), |r| r.finish(&floor))?;
        match &later {
            Some((next_stage, _)) if floor_holds => {
                println!("next: fearlezz run --stage {next_stage}");
            }
            _ => println!("next: {}", next_line(&before)),
        }
        return Ok(settled(&before));
    }

    let clock = std::time::Instant::now();
    let (mut passed, mut reused) = (0, 0);
    let mut failures = Vec::new();
    let mut unstable = Vec::new();
    let mut green = Vec::new();
    let env = ctx.env();
    let mut prepared = false;
    let mut held = None;
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
            if let Some(r) = report.as_mut() {
                println!("not run {}: {}", check.id, u.why);
                r.proof
                    .checks
                    .push(proof::not_run(check, stages.name(check.stage), &u.why));
                not_run += 1;
            } else {
                println!("fail {}: {}", check.id, u.why);
                println!("  fix: {}", u.fix);
                cannot_record += 1;
            }
            continue;
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
            let at = run::Where {
                root: &wt.git.root,
                tree: &wt.tree,
                journal: &j.name,
                stage: &stage,
                vars: &vars,
                changed: &wt.lines,
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
        if let Some(r) = report.as_mut() {
            r.ran(check, stages.name(check.stage), &result);
        }
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
        print_findings(r);
    }
    let looks = look_at_media(ctx, &wt, &green, &stage)?;
    if let Some(r) = report.as_mut() {
        r.proof.looks = looks;
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
    for line in &lines {
        println!("{line}");
    }
    report.map_or(Ok(()), |r| r.finish(&floor))?;
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
    if cannot_record > 0 {
        println!("next: give the recorder its permission (fix: above), then fearlezz run");
    } else {
        println!("next: {}", next_line(&after));
    }
    Ok(failures.is_empty() && cannot_record == 0 && !red_blocks && settled(&after))
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
    mut report: Option<&mut CommitReport>,
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

/// Compares the stills and films of green checks with the base commit, one line per check,
/// and records each comparison against it. Returns each look it made.
fn look_at_media(
    ctx: &Ctx,
    wt: &Worktree,
    green: &[(&config::Check, store::RunResult)],
    stage: &str,
) -> Result<Vec<look::Look>> {
    let mut looks = Vec::new();
    // Opened once, on the first check with pictures.
    let mut base: Option<Result<look::Base>> = None;
    for (check, result) in green {
        // A check that takes pictures is compared even without any, so all gone shows.
        let has_media = look::has_media(result);
        if !has_media && !ctx.config.takes_pictures(check) {
            continue;
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
            Ok(look::LookOutcome::Done(l)) => Some(store::Looked::At(l.dir().to_path_buf())),
            Err(_) => None,
        };
        if let Some(looked) = looked {
            store::AgainstBase::update(result, &wt.base, |a| a.look = Some(looked))?;
        }
        match outcome {
            Ok(look::LookOutcome::NoMedia) => {}
            Ok(look::LookOutcome::SameInputs) => println!(
                "look {}: inputs are the same as on the base, so the pictures are too",
                check.id
            ),
            Ok(look::LookOutcome::Done(l)) => {
                match &l.no_before {
                    Some(why) => println!("look {}: no before ({why}); {}", check.id, l.counts()),
                    None => println!("look {}: {}", check.id, l.counts()),
                }
                if let Some(note) = &l.note {
                    println!("  note: {note}");
                }
                println!("  report: {}", l.report.display());
                looks.push(l);
            }
            Err(e) => println!("look {} failed: {e:#}", check.id),
        }
    }
    Ok(looks)
}

fn print_status(ctx: &Ctx, v: &View) {
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
    for c in &v.checks {
        if let Some(l) = c.look.as_ref().filter(|l| l.to_look_at() > 0) {
            let verdict = match &c.approved {
                Some(a) => a.label(),
                None => "needs the developer's approval".to_string(),
            };
            println!(
                "look {}: {}, {verdict}  {}",
                l.check,
                l.counts(),
                l.report.display()
            );
        }
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
    println!("next: {}", v.next);
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
                Event::Approve {
                    check,
                    said,
                    by: Some(by),
                    ..
                } => println!("  approved {check} on GitHub by @{by}: \"{said}\""),
                Event::Approve { check, said, .. } => println!("  approved {check}: \"{said}\""),
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
