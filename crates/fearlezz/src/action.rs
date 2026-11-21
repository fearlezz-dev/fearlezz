//! GitHub Actions: what `fearlezz run --ci` reads of the workflow it runs in (the repository,
//! the run's page and the pull request its event names), the job summary it writes on the
//! run's page, and the pinned ffmpeg it installs when nothing else has one. It writes nothing
//! to GitHub: a person reviews the pull request there as usual, and the proof host (`host.rs`)
//! is the only place with a page.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

use crate::look::{Change, Item as Picture};
use crate::proof::{Proof, Ran};

/// The items of one list the job summary shows at most, so it stays readable: pictures,
/// failed tests.
const SHOWN: usize = 20;

/// What GitHub Actions tells every step of a run, read once from its environment and event
/// file: the repository, the run's page and the event. Off GitHub every part is empty.
#[derive(Clone)]
pub(crate) struct Workflow {
    /// Whether Fearlezz runs on GitHub Actions.
    pub actions: bool,
    /// `owner/name`.
    pub repository: Option<String>,
    /// GitHub's REST API, `GITHUB_API_URL` (another host on GitHub Enterprise Server).
    pub api_url: Option<String>,
    /// The run's page.
    pub run_url: Option<String>,
    /// The job's id in its workflow.
    pub job: Option<String>,
    /// Which job this is exactly: its workflow run, the run's attempt and the runner that took
    /// it, which tell the legs of a matrix apart.
    pub ci_job: Option<CiJob>,
    /// The ref the run is for, such as `refs/heads/main` or `refs/pull/24/merge`.
    git_ref: Option<String>,
    /// The event that started the workflow, such as `push` or `pull_request`.
    event_name: Option<String>,
    /// The event, Null without one or when its file cannot be read.
    event: Value,
}

/// One job of one attempt of a workflow run, as GitHub names it in every job.
#[derive(Clone)]
pub(crate) struct CiJob {
    pub run_id: u64,
    pub run_attempt: u32,
    pub runner_name: String,
}

impl CiJob {
    /// The job `GITHUB_RUN_ID`, `GITHUB_RUN_ATTEMPT` and `RUNNER_NAME` name, when all are set.
    fn read(var: &dyn Fn(&str) -> Option<String>) -> Option<CiJob> {
        Some(CiJob {
            run_id: var("GITHUB_RUN_ID")?.parse().ok()?,
            run_attempt: var("GITHUB_RUN_ATTEMPT")?.parse().ok()?,
            runner_name: var("RUNNER_NAME")?,
        })
    }
}

impl Workflow {
    /// The workflow GitHub's variables describe.
    pub(crate) fn read() -> Workflow {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let event = var("GITHUB_EVENT_PATH")
            .and_then(|path| fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or(Value::Null);
        let repository = var("GITHUB_REPOSITORY");
        let run_url = match (var("GITHUB_SERVER_URL"), &repository, var("GITHUB_RUN_ID")) {
            (Some(server), Some(repo), Some(run)) => {
                Some(format!("{server}/{repo}/actions/runs/{run}"))
            }
            _ => None,
        };
        Workflow {
            actions: on_actions(),
            repository,
            api_url: var("GITHUB_API_URL"),
            run_url,
            job: var("GITHUB_JOB"),
            ci_job: CiJob::read(&var),
            git_ref: var("GITHUB_REF"),
            event_name: var("GITHUB_EVENT_NAME"),
            event,
        }
    }

    fn text(&self, pointer: &str) -> Option<&str> {
        self.event.pointer(pointer).and_then(Value::as_str)
    }

    /// The pull request's title, when the event is one.
    pub(crate) fn title(&self) -> Option<&str> {
        self.text("/pull_request/title")
    }

    /// Whether the event is a pull request from a fork, whose jobs GitHub never gives an OIDC
    /// token, whatever the workflow asks.
    pub(crate) fn fork(&self) -> bool {
        self.event.pointer("/pull_request/head/repo/fork") == Some(&Value::Bool(true))
    }

    /// The pull request's head commit, when the event is one.
    pub(crate) fn head(&self) -> Option<&str> {
        self.text("/pull_request/head/sha")
    }

    /// The branch the work is on: a pull request's head branch, or the pushed branch. None
    /// for a run on a tag or another ref that is no branch.
    pub(crate) fn branch(&self) -> Option<&str> {
        self.text("/pull_request/head/ref")
            .or_else(|| self.git_ref.as_deref()?.strip_prefix("refs/heads/"))
    }

    /// The default branch, when a push to it started the run. Its pull request already showed
    /// and approved what changed, so the default branch is trusted as it is. Other events that
    /// carry its ref (`pull_request_target`, `workflow_run`) often check out a fork's code, so
    /// only a push counts.
    pub(crate) fn pushed_default(&self) -> Option<&str> {
        if self.event_name.as_deref() != Some("push") {
            return None;
        }
        let default = self.text("/repository/default_branch")?;
        let pushed = self.git_ref.as_deref()?.strip_prefix("refs/heads/")?;
        (pushed == default).then_some(default)
    }

    /// The pull request a proof is for, as the proof names it, when the event is one.
    pub(crate) fn pull(&self) -> Option<crate::proof::Pull> {
        Some(crate::proof::Pull {
            number: self.event.pointer("/pull_request/number")?.as_u64()?,
            title: self.title()?.to_string(),
            url: self.text("/pull_request/html_url").map(str::to_string),
        })
    }
}

/// The job summary of a proof: whether the commit is proven, what keeps it from being proven
/// and what to do, what a reader should know, the changed pictures and texts for a person to review on
/// the pull request, every check, and the end of each failing output. `page` is the proof
/// host's page of the work, when the host opened the run, `unsent` why a job that could have
/// had that page sent nothing, and `gate` where what the change weakens got past, or can.
pub(crate) fn summary(
    proof: &Proof,
    page: Option<&str>,
    unsent: Option<&str>,
    gate: &crate::floor::Gate,
) -> Result<String, std::fmt::Error> {
    let mut text = String::new();
    let blockers = proof.blockers();
    let ran = proof.ran_here().count();
    let passed = proof
        .ran_here()
        .filter(|c| c.outcome == Ran::Passed)
        .count();
    let word = if blockers.is_empty() {
        "Proven"
    } else {
        "Not proven"
    };
    writeln!(
        text,
        "### Fearlezz\n\n**{word}**: commit {}, {passed} of {ran} checks passed\n",
        proof.commit
    )?;
    if let Some(url) = &proof.preview {
        writeln!(text, "Preview: {url}\n")?;
    }
    if let Some(url) = page {
        writeln!(
            text,
            "[The proof page]({url}) shows this run live on the proof host: each check as it \
             runs, and how it came out.\n"
        )?;
    }
    if let Some(why) = unsent {
        writeln!(
            text,
            "The proof page was not sent: {}.\n",
            why.replace("id-token: write", "`id-token: write`")
        )?;
    }
    if !blockers.is_empty() {
        writeln!(text, "| what | why | what to do |\n| --- | --- | --- |")?;
        for item in &blockers {
            writeln!(
                text,
                "| {} | {} | {} |",
                cell(&item.what),
                cell(&item.why),
                cell(&item.fix)
            )?;
        }
        writeln!(text)?;
    }
    weakened(proof, page, gate, &mut text)?;
    diffs(proof, &mut text)?;
    for note in proof.notes() {
        writeln!(text, "{note}\n")?;
    }
    looks(proof, &mut text)?;
    writeln!(text, "| check | result | tests | time | command |")?;
    writeln!(text, "| --- | --- | --- | --- | --- |")?;
    for check in proof.ran_here() {
        let tests = check.tests.map(|n| n.to_string()).unwrap_or_default();
        let run = match check.run.split_once('\n') {
            Some((first, _)) => format!("{first} ..."),
            None => check.run.clone(),
        };
        writeln!(
            text,
            "| {} | {} | {tests} | {:.1} s | `{}` |",
            cell(&check.name()),
            check.word(),
            check.seconds,
            code(&run)
        )?;
    }
    if ran == 0 {
        writeln!(text, "\nNo check reads what this commit changed.")?;
    }
    tests(proof, &mut text)?;
    failing_logs(proof, &mut text)?;
    Ok(text)
}

/// What the change weakens: who accepted it, or where an admin can.
fn weakened(
    proof: &Proof,
    page: Option<&str>,
    gate: &crate::floor::Gate,
    text: &mut String,
) -> std::fmt::Result {
    use crate::floor::Gate;
    match (&proof.accepted, page) {
        _ if proof.weakened.is_empty() => {}
        (Some(accept), _) => {
            writeln!(
                text,
                "What the change weakens was accepted by {} on the proof page:\n",
                accept.by
            )?;
            for w in &proof.weakened {
                writeln!(text, "- {}", cell(&w.to_string()))?;
            }
            writeln!(text)?;
        }
        (None, Some(_)) if matches!(gate, Gate::Unasked) => writeln!(
            text,
            "The proof host could not be asked, so what it weakens fails as without a host.\n"
        )?,
        (None, Some(url)) => writeln!(
            text,
            "An admin can accept what it weakens on [the proof page]({url}), which runs this \
             job again.\n"
        )?,
        (None, None) => {}
    }
    Ok(())
}

/// The end of what each failing check printed.
fn failing_logs(proof: &Proof, text: &mut String) -> std::fmt::Result {
    for check in proof.checks.iter().filter(|c| c.outcome == Ran::Failed) {
        let Some(log) = &check.log else { continue };
        writeln!(
            text,
            "\n<details><summary>The end of what {} printed</summary>\n\n```",
            cell(&check.name())
        )?;
        for line in crate::run::tail(log, 40) {
            writeln!(text, "{}", line.replace("```", "'''"))?;
        }
        writeln!(text, "```\n\n</details>")?;
    }
    Ok(())
}

/// Each test a check names as failed or flaky, the failed ones first and cut short.
fn tests(proof: &Proof, text: &mut String) -> std::fmt::Result {
    for check in proof.ran_here() {
        let failed: Vec<&fearlezz_proof::Case> = check.cases.iter().filter(|c| c.failed).collect();
        let flaky = check.cases.iter().filter(|c| c.flaky && !c.failed);
        let mut lines: Vec<String> = failed
            .iter()
            .take(SHOWN)
            .map(|c| {
                if c.flaky {
                    format!("`{}` passed only on a rerun", code(&c.name))
                } else {
                    format!("`{}` failed", code(&c.name))
                }
            })
            .collect();
        if failed.len() > SHOWN {
            lines.push(format!("{} more failed", failed.len() - SHOWN));
        }
        lines.extend(flaky.map(|c| {
            format!(
                "`{}` passed only on a rerun (listed in flaky_tests)",
                code(&c.name)
            )
        }));
        if !lines.is_empty() {
            writeln!(text, "\n**{}**: {}", cell(&check.id), lines.join(", "))?;
        }
        // Why the first few failed, in their own words.
        for c in failed.iter().take(3) {
            let Some(output) = &c.output else { continue };
            writeln!(
                text,
                "\n<details><summary>What <code>{}</code> printed</summary>\n\n```\n{}\n```\n\n</details>",
                cell(&c.name),
                output.replace("```", "'''")
            )?;
        }
    }
    Ok(())
}

/// Each weakened part that names itself as it was and is now, as one diff of the config.
fn diffs(proof: &Proof, text: &mut String) -> std::fmt::Result {
    for w in &proof.weakened {
        if w.was.is_none() && w.now.is_none() {
            continue;
        }
        let was: Vec<&str> = w.was.as_deref().unwrap_or("").lines().collect();
        let now: Vec<&str> = w.now.as_deref().unwrap_or("").lines().collect();
        let lines = crate::text::lines(&was, &now);
        let fence = fence(&lines);
        writeln!(text, "`{}`: {}\n", w.part.name(), cell(&w.why()))?;
        writeln!(text, "{fence}diff")?;
        for line in &lines {
            writeln!(text, "{line}")?;
        }
        writeln!(text, "{fence}\n")?;
    }
    Ok(())
}

/// The pictures and texts that differ from the base, each check with its counts and then each
/// one, a text with its diff, for a person to review on the pull request. They never keep the
/// commit from being proven.
fn looks(proof: &Proof, text: &mut String) -> std::fmt::Result {
    let looks: Vec<_> = proof.looks.iter().filter(|l| l.to_look_at() > 0).collect();
    if looks.is_empty() {
        return Ok(());
    }
    writeln!(
        text,
        "#### Changed looks\n\nA person reviews these pictures and texts on the pull request. \
         `fearlezz run --ci --proof <dir>` writes every one, before and after, into a folder; \
         the Fearlezz action keeps it as an artifact of the run (`fearlezz-proof-<job>`).\n"
    )?;
    let mut left = SHOWN;
    let mut hidden = 0;
    for look in looks {
        let why = look
            .no_before
            .as_ref()
            .map(|w| format!(" (no before: {})", cell(w)))
            .unwrap_or_default();
        writeln!(
            text,
            "**`{}`**: {}{why}\n",
            code(&look.check),
            look.counts()
        )?;
        let items: Vec<&Picture> = look
            .items
            .iter()
            .filter(|i| i.change != Change::Same)
            .collect();
        let take = items.len().min(left);
        hidden += items.len() - take;
        left -= take;
        for item in items.iter().take(take) {
            let flaky = if item.flaky { " (flaky)" } else { "" };
            writeln!(
                text,
                "- **{}** {}{flaky}: {}",
                cell(&item.name),
                item.change.label(),
                cell(&item.summary)
            )?;
            if item.kind == crate::look::TEXT {
                text_diff(look, item, text)?;
            }
        }
        writeln!(text)?;
    }
    if hidden > 0 {
        writeln!(text, "And {hidden} more.\n")?;
    }
    Ok(())
}

/// The start of a text's diff, folded under its line.
fn text_diff(look: &crate::look::Look, item: &Picture, text: &mut String) -> std::fmt::Result {
    let diff = crate::look::text_diff(look, item);
    let (shown, left) = crate::text::head(&diff, crate::look::DIFF_LINES);
    let fence = fence(&shown);
    writeln!(
        text,
        "\n  <details><summary>diff</summary>\n\n  {fence}diff"
    )?;
    for line in &shown {
        writeln!(text, "  {line}")?;
    }
    writeln!(text, "  {fence}")?;
    if left > 0 {
        writeln!(text, "\n  And {left} more lines.")?;
    }
    writeln!(text, "\n  </details>\n")
}

/// A code fence longer than any run of backticks in `lines`, so it stays closed.
fn fence(lines: &[impl AsRef<str>]) -> String {
    let ticks = lines
        .iter()
        .filter_map(|l| l.as_ref().split(|c| c != '`').map(str::len).max())
        .max()
        .unwrap_or(0);
    "`".repeat(ticks.max(2) + 1)
}

/// Text for a table cell.
fn cell(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('|', "\\|")
        .replace('\n', " ")
}

/// Text for a code span in a table cell, where HTML entities would show as typed.
fn code(s: &str) -> String {
    s.replace('`', "'").replace('|', "\\|").replace('\n', " ")
}

/// The ffmpeg Fearlezz installs on a runner when a check needs it: John Van Sickle's static
/// Linux builds of ffmpeg 7.0.2, as the ffmpeg-static project republishes them under its
/// release tag b6.1.1, with the sha256 of each gzipped file.
const FFMPEG_RELEASE: &str = "https://github.com/eugeneware/ffmpeg-static/releases/download/b6.1.1";
const FFMPEG_SUMS: [(&str, &str); 4] = [
    (
        "ffmpeg-linux-x64",
        "bfe8a8fc511530457b528c48d77b5737527b504a3797a9bc4866aeca69c2dffa",
    ),
    (
        "ffprobe-linux-x64",
        "25d9b6ccb05e3d9de9e04e31e2506d8dd7f9f0418981965ac6df12e8d3afd067",
    ),
    (
        "ffmpeg-linux-arm64",
        "754a678672298bc68156adff58aa7385a592c2b30b1d0ae8750c45c915c4bac0",
    ),
    (
        "ffprobe-linux-arm64",
        "2ab6aba60ee84412dff9188720703376cb4e7aaf7e0b5e43aa8249f2acae5bf8",
    ),
];

/// Whether Fearlezz runs in a GitHub Actions job.
pub(crate) fn on_actions() -> bool {
    std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true")
}

/// Installs the pinned ffmpeg into the runner's tool cache and returns its folder, or None on
/// a platform it has no build for. [`crate::ffmpeg`] asks for it on Actions when neither the
/// project's shell nor PATH has ffmpeg.
pub(crate) fn pinned_ffmpeg() -> Result<Option<PathBuf>> {
    let arch = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x64",
        ("linux", "aarch64") => "arm64",
        _ => return Ok(None),
    };
    let cache = std::env::var_os("RUNNER_TOOL_CACHE")
        .or_else(|| std::env::var_os("RUNNER_TEMP"))
        .map_or_else(std::env::temp_dir, PathBuf::from);
    let dir = cache.join("fearlezz-ffmpeg").join("b6.1.1");
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    for tool in ["ffmpeg", "ffprobe"] {
        let target = dir.join(tool);
        if !target.exists() {
            let name = format!("{tool}-linux-{arch}");
            let sum = FFMPEG_SUMS
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, s)| *s)
                .unwrap_or_default();
            fetch(&format!("{FFMPEG_RELEASE}/{name}.gz"), sum, &target)?;
        }
    }
    Ok(Some(dir))
}

/// Downloads a gzipped program, checks its sha256 and unpacks it to `target`.
fn fetch(url: &str, sum: &str, target: &Path) -> Result<()> {
    use std::io::Read as _;
    let out = crate::process::read(
        Command::new("curl").args(["-fsSL", "--retry", "3", url]),
        None,
    )
    .context("downloading ffmpeg needs curl")?;
    let got = crate::files::sha256_hex(&out);
    if got != sum {
        bail!("{url} has sha256 {got}, not the pinned {sum}");
    }
    let mut program = Vec::new();
    flate2::read::GzDecoder::new(out.as_slice())
        .read_to_end(&mut program)
        .with_context(|| format!("unpacking {url}"))?;
    let partial = target.with_extension("partial");
    fs::write(&partial, program)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&partial, fs::Permissions::from_mode(0o755))?;
    }
    fs::rename(&partial, target)?;
    Ok(())
}
