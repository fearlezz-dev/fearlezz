//! GitHub Actions: what `fearlezz run --ci` reads of the workflow it runs in (the repository,
//! the run's page and the pull request its event names), the job summary it writes on the
//! run's page, and the pinned ffmpeg it installs when checks take pictures. It writes nothing
//! to GitHub: a person reviews the pull request there as usual, and the proof host (`host.rs`)
//! is the only place with a page.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::look::{Change, Item as Picture};
use crate::proof::{Proof, Ran};

/// Pictures listed in the job summary at most, so it stays readable.
const SHOWN: usize = 20;

/// What GitHub Actions tells every step of a run, read once from its environment and event
/// file: the repository, the run's page and the event. Off GitHub every part is empty.
#[derive(Clone)]
pub(crate) struct Workflow {
    /// Whether Fearlezz runs on GitHub Actions.
    pub actions: bool,
    /// `owner/name`.
    pub repository: Option<String>,
    /// The run's page.
    pub run_url: Option<String>,
    /// The job's id in its workflow.
    pub job: Option<String>,
    /// Which job this is exactly: its workflow run, the run's attempt and the runner that took
    /// it, which tell the legs of a matrix apart.
    pub ci_job: Option<CiJob>,
    /// The ref the run is for, such as `refs/heads/main` or `refs/pull/24/merge`.
    git_ref: Option<String>,
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
            actions: var("GITHUB_ACTIONS").as_deref() == Some("true"),
            repository,
            run_url,
            job: var("GITHUB_JOB"),
            ci_job: CiJob::read(&var),
            git_ref: var("GITHUB_REF"),
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
/// and what to do, what a reader should know, the changed pictures for a person to review on
/// the pull request, every check, and the end of each failing output. `page` is the proof
/// host's page of the work, when the host opened the run.
pub(crate) fn summary(proof: &Proof, page: Option<&str>) -> Result<String, std::fmt::Error> {
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
    if let Some(url) = page {
        writeln!(
            text,
            "[The proof page]({url}) shows this run live on the proof host: each check as it \
             runs, and how it came out.\n"
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
    match (&proof.accepted, page) {
        (Some(accept), _) if !proof.weakened.is_empty() => {
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
        (None, Some(url)) if !proof.weakened.is_empty() => writeln!(
            text,
            "An admin can accept what it weakens on [the proof page]({url}), which runs this \
             job again.\n"
        )?,
        _ => {}
    }
    for note in proof.notes() {
        writeln!(text, "{note}\n")?;
    }
    pictures(proof, &mut text)?;
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
            cell(&check.id),
            check.outcome.word(),
            check.seconds,
            code(&run)
        )?;
    }
    if ran == 0 {
        writeln!(text, "\nNo check reads what this commit changed.")?;
    }
    for check in proof.checks.iter().filter(|c| c.outcome == Ran::Failed) {
        let Some(log) = &check.log else { continue };
        writeln!(
            text,
            "\n<details><summary>The end of what {} printed</summary>\n\n```",
            cell(&check.id)
        )?;
        for line in crate::run::tail(log, 40) {
            writeln!(text, "{}", line.replace("```", "'''"))?;
        }
        writeln!(text, "```\n\n</details>")?;
    }
    Ok(text)
}

/// The pictures that differ from the base, each check with its counts and then each picture,
/// for a person to review on the pull request. They never keep the commit from being proven.
fn pictures(proof: &Proof, text: &mut String) -> std::fmt::Result {
    let looks: Vec<_> = proof.looks.iter().filter(|l| l.to_look_at() > 0).collect();
    if looks.is_empty() {
        return Ok(());
    }
    writeln!(
        text,
        "#### Changed pictures\n\nA person reviews them on the pull request. `fearlezz run --ci \
         --proof <dir>` writes every picture, before and after, into a folder; the Fearlezz \
         action keeps it as an artifact of the run (`fearlezz-proof-<job>`).\n"
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
        }
        writeln!(text)?;
    }
    if hidden > 0 {
        writeln!(text, "And {hidden} more.\n")?;
    }
    Ok(())
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

/// Installs the pinned ffmpeg into the runner's tool cache, unless ffmpeg is on PATH already,
/// and returns the folder to put in front of PATH.
pub(crate) fn install_ffmpeg() -> Result<Option<PathBuf>> {
    let found = Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if found {
        println!("ffmpeg for the pictures: the one on PATH");
        return Ok(None);
    }
    let arch = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x64",
        ("linux", "aarch64") => "arm64",
        (os, arch) => {
            println!(
                "::warning::no static ffmpeg for {os} on {arch}; install ffmpeg before Fearlezz"
            );
            return Ok(None);
        }
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
    println!(
        "ffmpeg for the pictures: 7.0.2, installed into {}",
        dir.display()
    );
    Ok(Some(dir))
}

/// Downloads a gzipped program, checks its sha256 and unpacks it to `target`.
fn fetch(url: &str, sum: &str, target: &Path) -> Result<()> {
    use std::io::Read as _;
    let out = Command::new("curl")
        .args(["-fsSL", "--retry", "3", url])
        .output()
        .context("downloading ffmpeg needs curl")?;
    if !out.status.success() {
        bail!(
            "downloading {url} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let got = hex::encode(Sha256::digest(&out.stdout));
    if got != sum {
        bail!("{url} has sha256 {got}, not the pinned {sum}");
    }
    let mut program = Vec::new();
    flate2::read::GzDecoder::new(out.stdout.as_slice())
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
