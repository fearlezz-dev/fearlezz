//! `fearlezz`: helps coding agents prove their work. See the README for the model.

mod config;
mod counts;
mod env;
mod findings;
mod floor;
mod git;
mod hooks;
mod init;
mod journal;
mod lints;
mod look;
mod media;
mod overview;
mod presets;
mod report;
mod rules;
mod run;
mod settings;
mod state;
mod store;

use std::collections::{BTreeSet, HashMap};
use std::fmt::Write as _;
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
    /// The proof this change needs, per stage.
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
        /// Prove this commit, for CI: every check the change needs, fresh, with no journal.
        #[arg(long, conflicts_with_all = ["checks", "stage"])]
        ci: bool,
        /// With --ci: also write the short status for the pull request to this file.
        #[arg(long, value_name = "PATH", requires = "ci")]
        comment: Option<PathBuf>,
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
        #[arg(long)]
        said: String,
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
        None => see(&cli.see),
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
        } => {
            let ctx = prepared()?;
            if ci {
                let mut journal = Journal::in_memory("ci");
                let to = ctx.config.stages.name(ctx.config.stages.last()).to_string();
                journal.append(ctx.now, Event::Stage { to, reason: None })?;
                return run_checks(&ctx, journal, &[], true, comment);
            }
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
    };
    Ok(hooks::judge(&ctx.config, &work, event, input))
}

fn say(ctx: &Ctx, what: Say) -> Result<bool> {
    match what {
        Say::Approve { check, said } => {
            let said = said.trim().to_string();
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

/// Writes the starter config and says what it found.
fn setup_init() -> Result<()> {
    let git = Git::discover(&std::env::current_dir()?)?;
    let found = init::init(&git.root)?;
    if found.is_empty() {
        println!("wrote fearlezz.toml: no language found, so fill in its checks");
    } else {
        let names: Vec<String> = found.iter().map(ToString::to_string).collect();
        println!("wrote fearlezz.toml: uses {}", names.join(", "));
    }
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
            let (b, a) = std::thread::scope(|s| {
                let b = s.spawn(|| media::decode(&before));
                let a = media::decode(&after);
                let b = b.join().map_err(|panic| {
                    anyhow::anyhow!(
                        "decoding {} crashed: {}",
                        before.display(),
                        panic_text(&*panic)
                    )
                });
                (b, a)
            });
            let c = media::compare(&b??, &a?)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&c)?);
            } else {
                println!("{} ({:.1} s)", c.summary(), clock.elapsed().as_secs_f64());
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
            .filter(|c| c.needed && c.stage == stage)
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

/// The checks a run takes on: the named ones, every check when proving a commit on the base
/// branch itself, and otherwise those the change needs in the current stage.
fn targets<'a>(
    ctx: &'a Ctx,
    before: &View,
    wt: &Worktree,
    ids: &[String],
    in_memory: bool,
) -> Result<Vec<&'a config::Check>> {
    if !ids.is_empty() {
        return ids.iter().map(|id| ctx.config.check(id)).collect();
    }
    if in_memory && wt.changed.is_empty() {
        return Ok(ctx.config.checks.iter().collect());
    }
    let due: Vec<&str> = before
        .checks
        .iter()
        .filter(|c| c.needed && c.stage <= before.stage)
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
    commit: String,
    base: String,
    rows: Vec<Row>,
    /// Where the short status for the pull request goes, when asked.
    comment: Option<PathBuf>,
}

/// One check in the report.
struct Row {
    id: String,
    outcome: &'static str,
    /// How many tests ran, for a check that counts them.
    tests: Option<u64>,
    secs: f64,
    /// The first line of the command, marked when there is more.
    run: String,
}

/// The first line of the status comment, so CI finds its own comment to replace.
const COMMENT_MARKER: &str = "<!-- fearlezz -->";

impl CommitReport {
    fn start(
        ctx: &Ctx,
        wt: &Worktree,
        to_run: usize,
        comment: Option<PathBuf>,
    ) -> Result<CommitReport> {
        let commit = ctx.git.head_short()?;
        println!(
            "commit {commit}: {} changed files, {to_run} of {} checks to run",
            wt.changed.len(),
            ctx.config.checks.len()
        );
        Ok(CommitReport {
            commit,
            base: ctx.config.base.clone(),
            rows: Vec::new(),
            comment,
        })
    }

    fn ran(&mut self, check: &config::Check, result: &store::RunResult) {
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
        self.rows.push(Row {
            id: check.id.clone(),
            outcome: word,
            tests: result.tests,
            secs,
            run,
        });
    }

    /// Writes the check page into the job summary GitHub Actions names, when it names one,
    /// and the status for the pull request, when asked.
    fn finish(
        self,
        floor: &floor::Floor,
        failures: &[store::RunResult],
        unstable: &[String],
    ) -> Result<()> {
        use std::io::Write as _;
        let mut items: Vec<floor::Item> = failures.iter().map(failure_item).collect();
        items.extend(unstable.iter().map(|id| floor::Item {
            what: format!("check `{id}` is unstable"),
            why: "the same inputs gave passed and failed".into(),
            fix: "make it give one answer: a flaky test, or an input it does not declare".into(),
        }));
        items.extend(floor.items(&self.base));
        if let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") {
            let page = self.page(&items, failures)?;
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(Path::new(&path))?
                .write_all(page.as_bytes())?;
        }
        if let Some(path) = &self.comment {
            std::fs::write(path, self.status(&items)?)
                .with_context(|| format!("writing {}", path.display()))?;
        }
        Ok(())
    }

    /// The headline: proven or not, with the commit and how many checks passed.
    fn headline(&self, items: &[floor::Item]) -> String {
        let passed = self.rows.iter().filter(|r| r.outcome == "passed").count();
        let word = if items.is_empty() {
            "Proven"
        } else {
            "Not proven"
        };
        format!(
            "**{word}**: commit {}, {passed} of {} checks passed",
            self.commit,
            self.rows.len()
        )
    }

    /// The short status for the pull request: the headline, what blocks it, the checks folded
    /// away, and a link to the check page.
    fn status(&self, items: &[floor::Item]) -> Result<String, std::fmt::Error> {
        let mut text = String::new();
        writeln!(
            text,
            "{COMMENT_MARKER}\n### Fearlezz\n\n{}\n",
            self.headline(items)
        )?;
        blockers(&mut text, items)?;
        if !self.rows.is_empty() {
            writeln!(
                text,
                "<details><summary>{} checks</summary>\n",
                self.rows.len()
            )?;
            self.table(&mut text)?;
            writeln!(text, "\n</details>\n")?;
        }
        if let (Ok(server), Ok(repo), Ok(run)) = (
            std::env::var("GITHUB_SERVER_URL"),
            std::env::var("GITHUB_REPOSITORY"),
            std::env::var("GITHUB_RUN_ID"),
        ) {
            writeln!(
                text,
                "[The check page]({server}/{repo}/actions/runs/{run}) has each check's output."
            )?;
        }
        Ok(text)
    }

    /// The check page: the headline, what blocks it, every check, and the end of each failing
    /// output.
    fn page(
        &self,
        items: &[floor::Item],
        failures: &[store::RunResult],
    ) -> Result<String, std::fmt::Error> {
        let mut text = String::new();
        writeln!(text, "### Fearlezz\n\n{}\n", self.headline(items))?;
        blockers(&mut text, items)?;
        self.table(&mut text)?;
        if self.rows.is_empty() {
            writeln!(text, "\nNo check reads what this commit changed.")?;
        }
        for f in failures {
            writeln!(
                text,
                "\n<details><summary>The end of what {} printed</summary>\n\n```",
                f.check
            )?;
            for line in run::tail(&f.log, 40) {
                writeln!(text, "{}", line.replace("```", "'''"))?;
            }
            writeln!(text, "```\n\n</details>")?;
        }
        Ok(text)
    }

    fn table(&self, text: &mut String) -> std::fmt::Result {
        writeln!(text, "| check | result | tests | time | command |")?;
        writeln!(text, "| --- | --- | --- | --- | --- |")?;
        for row in &self.rows {
            let run = row.run.replace('|', "\\|");
            let tests = row.tests.map(|n| n.to_string()).unwrap_or_default();
            writeln!(
                text,
                "| {} | {} | {tests} | {:.1} s | `{run}` |",
                row.id, row.outcome, row.secs
            )?;
        }
        Ok(())
    }
}

/// The table of what keeps the commit from being proven, if anything does.
fn blockers(text: &mut String, items: &[floor::Item]) -> std::fmt::Result {
    if items.is_empty() {
        return Ok(());
    }
    writeln!(text, "| what | why | what to do |\n| --- | --- | --- |")?;
    let cell = |s: &str| s.replace('|', "\\|").replace('\n', " ");
    for item in items {
        writeln!(
            text,
            "| {} | {} | {} |",
            cell(&item.what),
            cell(&item.why),
            cell(&item.fix)
        )?;
    }
    writeln!(text)
}

/// A failed check as a blocker, for a reader who sees neither the agent nor the logs.
/// How to make a missing tool reachable for checks.
fn install_fix(tool: &str) -> String {
    format!(
        "install {tool} where checks find it: the project's dev dependencies ({}), its devenv, \
         or PATH",
        run::TOOL_DIRS.join(", ")
    )
}

fn failure_item(f: &store::RunResult) -> floor::Item {
    let what = format!("check `{}` failed", f.check);
    if let Some(tool) = f
        .not_found()
        .then(|| run::missing_command(&f.log))
        .flatten()
    {
        return floor::Item {
            what,
            why: format!("{tool} was not found"),
            fix: install_fix(&tool),
        };
    }
    if let Some(found) = f.findings.iter().find(|x| x.level == findings::Level::Gate) {
        return floor::Item {
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
        return floor::Item {
            what,
            why: format!("it ran no tests, so it proves nothing{detail}"),
            fix: "make it run its tests, or give it the tests reader its runner needs".into(),
        };
    }
    let code = f
        .exit_code
        .map_or_else(|| "a signal".to_string(), |c| format!("exit code {c}"));
    floor::Item {
        what,
        why: format!("its command ended with {code}"),
        fix: "fix what it reports; the check page shows the end of its output".into(),
    }
}

/// The next line of a final run whose change fails the floor of the base branch.
const FLOOR_NEXT: &str = "the change takes away proof the base branch requires (above): restore \
                          it, or ask the developer whether to merge past CI: fearlezz say ask \
                          \"<question>\"";

fn run_checks(
    ctx: &Ctx,
    mut j: Journal,
    ids: &[String],
    fresh: bool,
    comment: Option<PathBuf>,
) -> Result<bool> {
    let mut wt = ctx.worktree()?;
    let before = state::view(&ctx.config, &ctx.store, &j, Some(&mut wt), ctx.now)?;
    let stages = &ctx.config.stages;
    let stage = stages.name(before.stage).to_string();
    let final_stage = stages.name(stages.last()).to_string();
    let in_memory = j.is_in_memory();
    let targets = targets(ctx, &before, &wt, ids, in_memory)?;
    // A full run at the final stage answers for the whole work, and for what the base
    // branch requires, not only for its checks.
    let floor = if ids.is_empty() && stage == final_stage {
        floor::Floor::read(&ctx.git, &ctx.config, &wt)?
    } else {
        floor::Floor::default()
    };
    let lines = floor.lines();
    let answers_for_work = !in_memory && ids.is_empty() && stage == final_stage;
    let settled = |view: &View| {
        floor.holds() && (!answers_for_work || view.next == state::READY || view.checks.is_empty())
    };
    let next_line = |view: &View| {
        if in_memory {
            state::COMMIT_PROVEN.to_string()
        } else if !floor.holds() {
            FLOOR_NEXT.to_string()
        } else {
            view.next.clone()
        }
    };
    let mut report = in_memory
        .then(|| CommitReport::start(ctx, &wt, targets.len(), comment))
        .transpose()?;
    if targets.is_empty() {
        println!("nothing to run in {stage}");
        for line in &lines {
            println!("{line}");
        }
        report.map_or(Ok(()), |r| r.finish(&floor, &[], &[]))?;
        println!("next: {}", next_line(&before));
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
            r.ran(check, &result);
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
    look_at_media(ctx, &green, &stage);
    prune_store(ctx);
    println!(
        "{} checks: {passed} passed, {} failed, {reused} reused, {:.1} s",
        targets.len(),
        failures.len(),
        clock.elapsed().as_secs_f64()
    );
    for line in &lines {
        println!("{line}");
    }
    report.map_or(Ok(()), |r| r.finish(&floor, &failures, &unstable))?;
    let after = state::view(&ctx.config, &ctx.store, &j, Some(&mut wt), ctx.now)?;
    if stage == final_stage
        && (after.next == state::READY || in_memory)
        && env.state.services(&wt.git.root).is_some()
    {
        env.state.stop(env.cfg, &wt.git.root)?;
        println!("services stopped: all proof is in");
    }
    println!("next: {}", next_line(&after));
    Ok(failures.is_empty() && settled(&after))
}

/// Compares the stills and films of green checks with the base commit, one line per check.
fn look_at_media(ctx: &Ctx, green: &[(&config::Check, store::RunResult)], stage: &str) {
    // Opened once, on the first check with pictures.
    let mut base: Option<Result<Option<look::Base>>> = None;
    for (check, result) in green {
        if !look::has_media(result) {
            continue;
        }
        let base = base.get_or_insert_with(|| look::Base::open(&ctx.git, &ctx.config));
        let outcome = match base {
            Ok(Some(base)) => look::look(&ctx.store, check, result, base, stage, &ctx.env()),
            Ok(None) => Err(anyhow::anyhow!(
                "there is no base branch \"{}\" to compare with",
                ctx.config.base
            )),
            Err(e) => Err(anyhow::anyhow!("the base checkout failed: {e:#}")),
        };
        // Without a before every picture is new; a failed look never ends the run.
        let outcome = outcome.or_else(|e| {
            look::without_before(check, result, &ctx.git.common, &format!("{e:#}"))
                .map(look::LookOutcome::Done)
        });
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
                println!("  report: {}", l.report.display());
            }
            Err(e) => println!("look {} failed: {e:#}", check.id),
        }
    }
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
                Some(said) => format!("approved (\"{said}\")"),
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
        CheckState::Never => "never run",
    }
}
