//! A run's proof as it is built, the run on the proof host while it takes it, and what the
//! terminal says of each check. Every run builds the proof, local ones too; proving a commit
//! for CI shows more besides the usual lines, so a reader can tell the checks ran: the commit
//! and each check with its command and time, and on GitHub Actions the job summary. Only CI
//! writes these, so the agent whose work they judge cannot change them. A local run goes to
//! the host only while the developer is signed in to it: the host shows it unproven.

use std::path::{Path, PathBuf};

use anyhow::Result;
use fearlezz_proof::events::{Open, Source};
use fearlezz_proof::why::Why;

use crate::config::Check;
use crate::floor::{Floor, Gate};
use crate::look::Look;
use crate::proof::{self, Away, Proof};
use crate::state::Worktree;
use crate::store::{Outcome, RunResult};
use crate::{Ctx, action, counts, feedback, generated, host, legs, run, text};

/// What proving a commit for CI takes besides the checks.
pub(crate) struct CiOut {
    /// A folder for the proof bundle.
    pub proof: Option<PathBuf>,
    /// What GitHub Actions says of the run: the pull request and pages the proof names.
    pub workflow: action::Workflow,
    /// The leg of a plan it runs, for a commit proven in several jobs.
    pub leg: Option<legs::Asked>,
}

/// What one check of a run came to.
#[expect(clippy::large_enum_variant, reason = "one per check of a run")]
pub(crate) enum Came {
    /// It ran, or a result for its inputs answered (`reused`), with its look at the base when
    /// it passed and takes pictures.
    Ran {
        result: RunResult,
        reused: bool,
        look: Option<Look>,
    },
    /// It did not run here.
    Away(Away),
}

pub(crate) struct Report {
    pub proof: Proof,
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

/// The proof of the commit CI checked out, before any check: the pull request's head merged
/// into the base branch, or HEAD.
pub(crate) fn ci_proof(ctx: &Ctx, workflow: &action::Workflow) -> Result<Proof> {
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
    Ok(Proof {
        repository: workflow.repository.clone(),
        pull: workflow.pull(),
        run_url: workflow.run_url.clone(),
        ..Proof::new(commit, head, ctx.config.base.clone())
    })
}

impl Report {
    /// The proof of the commit CI checked out, opened on the proof host when the job may.
    pub(crate) fn ci(
        ctx: &Ctx,
        wt: &Worktree,
        targets: &[&Check],
        out: CiOut,
        leg: Option<&legs::Leg<'_>>,
    ) -> Result<Report> {
        let mut proof = ci_proof(ctx, &out.workflow)?;
        let of = match leg {
            Some(leg) => format!("leg {}: ", leg.id),
            None => String::new(),
        };
        println!(
            "commit {}: {} changed files, {of}{} of {} checks to run",
            proof.commit,
            wt.changed.len(),
            targets.len(),
            ctx.config.due().count()
        );
        proof.leg = leg.map(|l| l.id.clone());
        let workflow = &out.workflow;
        let open = match (&workflow.repository, workflow.branch(), &workflow.ci_job) {
            (None, ..) | (_, None, _) => Err("the run names no repository and branch \
                                              (GITHUB_REPOSITORY, GITHUB_REF)"),
            (.., None) => Err("the run names no job of a workflow run (GITHUB_RUN_ID, \
                               GITHUB_RUN_ATTEMPT, RUNNER_NAME)"),
            (Some(repository), Some(branch), Some(ci_job)) => Ok(Open {
                repository: repository.clone(),
                branch: branch.to_string(),
                pull: proof.pull.clone(),
                head: proof.head.clone(),
                base: proof.base.clone(),
                source: Source::Ci {
                    job: workflow.job.clone().unwrap_or_default(),
                    run_url: workflow.run_url.clone(),
                    run_id: ci_job.run_id,
                    run_attempt: ci_job.run_attempt,
                    runner_name: ci_job.runner_name.clone(),
                    leg: proof.leg.clone(),
                    planned: out.leg.map(|a| a.planned).unwrap_or_default(),
                },
                checks: targets.iter().map(|c| c.id.clone()).collect(),
            }),
        };
        // A host that fails never fails the proof: CI proves the commit anyway.
        let stream =
            host::Host::from_actions(ctx.config.host.as_deref()).and_then(|host| match open {
                Ok(open) => open_on(host, &open),
                Err(why) => {
                    println!("proof page not sent: {why}");
                    None
                }
            });
        Ok(Report {
            page: stream.as_ref().map(|s| s.url.clone()),
            stream,
            proof,
            bundle: out.proof,
            ci: true,
            platform: leg.and_then(legs::Leg::platform_name),
        })
    }

    /// The proof of a local run. It goes to the proof host when the developer signed in to
    /// it, the branch is on GitHub and there are checks to run: opened before they run, so a
    /// person can leave notes on its pictures before there is a pull request. Otherwise
    /// nothing leaves the machine.
    pub(crate) fn local(ctx: &Ctx, targets: &[&Check]) -> Result<Report> {
        let head = ctx.git.head()?;
        let mut proof = Proof::new(ctx.git.head_short()?, head.clone(), ctx.config.base.clone());
        let url = crate::host_url(Some(&ctx.config));
        let login = feedback::login_of(&url).filter(|_| !targets.is_empty());
        let stream = login.and_then(|login| {
            let (Some(repository), Ok(branch)) = (ctx.git.github_repository(), ctx.git.branch())
            else {
                return None;
            };
            proof.repository = Some(repository.clone());
            let open = Open {
                repository,
                branch,
                pull: None,
                head,
                base: ctx.config.base.clone(),
                source: Source::Local { login: login.login },
                checks: targets.iter().map(|c| c.id.clone()).collect(),
            };
            open_on(host::Host::from_login(&url, &login.token), &open)
        });
        Ok(Report {
            page: stream.as_ref().map(|s| s.url.clone()),
            stream,
            proof,
            bundle: None,
            ci: false,
            platform: None,
        })
    }

    /// A check starts.
    pub(crate) fn starts(&mut self, check: &Check) {
        sent(&mut self.stream, |s| s.started(&check.id));
    }

    /// What `check` came to, as the terminal says it at once and the proof holds it.
    pub(crate) fn add(&mut self, check: &Check, stage: &str, came: &Came) {
        let (mut entry, look) = match came {
            Came::Away(away) => {
                for line in away.lines(&check.id) {
                    println!("{line}");
                }
                (proof::not_run(check, stage, away), None)
            }
            Came::Ran { result, look, .. } => {
                if self.ci {
                    println!("{}", ci_line(check, result));
                }
                let failure = (result.outcome == Outcome::Failed)
                    .then(|| result.failure().item(&result.check, run::install));
                (proof::ran(check, stage, result, failure), look.as_ref())
            }
        };
        entry.platform.clone_from(&self.platform);
        sent(&mut self.stream, |s| s.finished(&entry, look));
        self.proof.checks.push(entry);
    }

    /// Judges what the change weakens, writes the bundle when asked, closes the run on the
    /// proof host with it, writes the job summary GitHub Actions names, when it names one, and
    /// says how the floor stands. Before it judges, a CI run the host took asks it whether an
    /// admin accepted exactly what the change weakens at this head; a host that does not
    /// answer means not accepted. A local run never asks. True when the floor holds; a leg's
    /// proof names what the change weakens, so the host shows it, and the gate judges.
    pub(crate) fn close(mut self, floor: &Floor) -> Result<bool> {
        self.proof.weakened.clone_from(&floor.weakened);
        let hosted = self.ci && self.stream.is_some();
        if hosted && !floor.holds() {
            sent(&mut self.stream, |s| ask_accept(s, &mut self.proof, floor));
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
            sent(&mut self.stream, |s| s.done(dir));
        }
        // The gate of a split proof streams nothing; it asks the host through a leg's run.
        if let (Some(dir), Some(stream), Some(_)) = (&self.bundle, &self.stream, &self.proof.leg) {
            std::fs::write(dir.join(legs::HOST_RUN), stream.run())?;
        }
        let gate = match self.proof.accepted.clone() {
            Some(accept) => Gate::Accepted(accept),
            None if hosted && self.stream.is_some() => Gate::Host,
            None if hosted => Gate::Unasked,
            None => Gate::Job,
        };
        if self.ci {
            summary(&self.proof, self.page.as_deref(), &gate)?;
        }
        if self.proof.leg.is_some() {
            if !floor.holds() {
                println!("the gate judges what the change weakens");
            }
            return Ok(true);
        }
        for line in floor.lines(&gate) {
            println!("{line}");
        }
        Ok(floor.holds_at(&gate))
    }
}

/// The line CI prints for a check that ran, so a reader can tell it did.
fn ci_line(check: &Check, result: &RunResult) -> String {
    let word = match result.outcome {
        Outcome::Passed => "passed",
        Outcome::Failed => "failed",
    };
    #[expect(clippy::cast_precision_loss, reason = "a duration shown in seconds")]
    let secs = result.duration_ms as f64 / 1000.0;
    let run = match check.run().trim().split_once('\n') {
        Some((first, _)) => format!("{first} ..."),
        None => check.run().trim().to_string(),
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
    format!("{word} {}  {secs:.1} s{tests}{coverage}  {run}", check.id)
}

/// Opens the run on the proof host, printing its page or why there is none.
fn open_on(host: host::Host, open: &Open) -> Option<host::Stream> {
    let local = matches!(open.source, Source::Local { .. });
    match host.open(open) {
        Ok(host::Opening::Opened(stream)) if local => {
            println!(
                "proof page: {} shows this run, unproven; fearlezz setup logout stops sending \
                 local runs",
                stream.url
            );
            Some(stream)
        }
        Ok(host::Opening::Opened(stream)) => {
            println!("proof page: {}", stream.url);
            Some(stream)
        }
        Ok(host::Opening::NotInstalled(install)) if local => {
            println!(
                "proof page: the Fearlezz app is not installed on this repository, so this run \
                 was not sent; install it to see local runs on the host: {install}"
            );
            None
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

/// Sends one step of the run to the host; a host that fails is said once and sent nothing
/// more, and never fails the proof.
fn sent(stream: &mut Option<host::Stream>, step: impl FnOnce(&host::Stream) -> Result<()>) {
    if let Some(s) = stream.as_ref()
        && let Err(e) = step(s)
    {
        println!("proof page not sent: {e:#}");
        *stream = None;
    }
}

/// Asks the proof host, through `stream`, whether an admin accepted exactly what the change
/// weakens at the proof's head, and keeps the accept in the proof when one did.
pub(crate) fn ask_accept(stream: &host::Stream, proof: &mut Proof, floor: &Floor) -> Result<()> {
    proof.accepted = stream
        .accepted(&floor.weakened)?
        .filter(|a| a.covers(&proof.head, &floor.weakened));
    Ok(())
}

/// Appends the job summary to the file GitHub Actions names, when it names one.
pub(crate) fn summary(proof: &Proof, page: Option<&str>, gate: &Gate) -> Result<()> {
    use std::io::Write as _;
    if let Some(path) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        let summary = action::summary(proof, page, gate)?;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(Path::new(&path))?
            .write_all(summary.as_bytes())?;
    }
    Ok(())
}

/// Why a failed run failed, with the end of its log, its failed tests and its findings.
pub(crate) fn print_failure(f: &RunResult) {
    let why = f.failure();
    println!("fail {}{}", f.check, why.line());
    if let Why::NoTests(said) = &why {
        for line in said {
            println!("  {line}");
        }
    } else if let Why::Generated(_) = &why {
        let path = f.dir.join(generated::DIFF);
        if let Ok(diff) = std::fs::read_to_string(&path) {
            let (shown, left) = text::head(&diff, 40);
            for line in shown {
                println!("  {line}");
            }
            if left > 0 {
                println!("  ({left} more lines)");
            }
            println!("  diff: {}", path.display());
        }
    }
    if let Some(fix) = why.fix_here(&f.check, run::install) {
        println!("  fix: {fix}");
    }
    // The diff and the findings say what blocks; otherwise the end of the log does.
    if !matches!(why, Why::Gate(_) | Why::Generated(_)) {
        for line in run::tail(&f.log, 12) {
            println!("  {line}");
        }
        println!("  log: {}", f.log.display());
    }
    if !matches!(why, Why::Generated(_)) {
        print_cases(f);
    }
    print_findings(f);
}

/// The items of one list the terminal shows at most: failed tests, findings.
pub(crate) const SHOWN: usize = 10;

/// Each test a run names as failed or flaky, the failed ones first and cut short, and what to
/// do about a flaky one its check does not list.
pub(crate) fn print_cases(r: &RunResult) {
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

/// The findings a run counted, worst first, with how many were left out.
pub(crate) fn print_findings(r: &RunResult) {
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
