//! The look at a change: every still and film a check records, compared with the same check
//! run on the base commit (where the branch left the base branch), on this machine.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::config::{Check, Config};
use crate::env::EnvCtx;
use crate::git::Git;
use crate::media::{self, Comparison, Verdict};
use crate::run;
use crate::state::Worktree;
use crate::store::{Outcome, RunResult, Store};

/// The journal name base runs are recorded under.
pub(crate) const BASE_JOURNAL: &str = "(base)";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Change {
    Changed,
    Timing,
    New,
    Gone,
    Unreadable,
    Same,
}

impl Change {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Change::Changed => "changed",
            Change::Timing => "timing",
            Change::New => "new",
            Change::Gone => "gone",
            Change::Unreadable => "unreadable",
            Change::Same => "same",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Item {
    pub name: String,
    pub kind: String,
    pub change: Change,
    pub summary: String,
    /// Copies of the media inside the look folder, relative to it.
    pub before: Option<String>,
    pub after: Option<String>,
    pub comparison: Option<Comparison>,
    /// Named by the check's `flaky`: compared and shown, but never blocks or waits for
    /// approval.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flaky: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Look {
    pub check: String,
    pub base_commit: String,
    pub base_result: String,
    /// Why there is no before (no base branch, the check fails on the base, the look
    /// failed). Then every picture counts as new and needs approval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_before: Option<String>,
    /// Something about the comparison a person should know, such as a base run that passed
    /// without taking any pictures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub items: Vec<Item>,
    /// The report page on this machine; empty in a proof bundle.
    #[serde(default, skip_serializing_if = "no_path")]
    pub report: PathBuf,
}

fn no_path(path: &Path) -> bool {
    path.as_os_str().is_empty()
}

impl Look {
    /// For example `1 changed, 2 same, 1 new, 1 flaky`. Flaky pictures count apart.
    pub(crate) fn counts(&self) -> String {
        let mut changes: Vec<Change> = self
            .items
            .iter()
            .filter(|i| !i.flaky)
            .map(|i| i.change)
            .collect();
        changes.sort();
        let mut parts: Vec<String> = Vec::new();
        for c in changes.iter().copied() {
            let n = changes.iter().filter(|x| **x == c).count();
            let part = format!("{n} {}", c.label());
            if !parts.contains(&part) {
                parts.push(part);
            }
        }
        let flaky = self.items.iter().filter(|i| i.flaky).count();
        if flaky > 0 {
            parts.push(format!("{flaky} flaky"));
        }
        parts.join(", ")
    }

    /// Items a person should look at: anything but same, and never a flaky one.
    pub(crate) fn to_look_at(&self) -> usize {
        self.items
            .iter()
            .filter(|i| i.change != Change::Same && !i.flaky)
            .count()
    }

    /// The look saved in the folder `dir`, if any.
    pub(crate) fn at(dir: &Path) -> Option<Look> {
        let text = fs::read_to_string(dir.join("look.json")).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// The folder it is saved in.
    pub(crate) fn dir(&self) -> &Path {
        self.report.parent().unwrap_or(&self.report)
    }
}

pub(crate) enum LookOutcome {
    /// The check recorded no stills or films.
    NoMedia,
    /// The check's inputs are the same as on the base, so its pictures are too.
    SameInputs,
    /// The pictures compared with the base, or all new when there is no before.
    Done(Look),
}

/// The base commit, checked out in the per user cache (see [`Git::bases_dir`]).
pub(crate) struct Base {
    pub commit: String,
    pub wt: Worktree,
    prepared: bool,
    /// Keeps other commands from removing the worktree while this one uses it.
    _hold: fs::File,
}

impl Base {
    /// The base commit in a checkout of its own, told apart from others by `name`. Every
    /// run on a base commit goes through here, so where base checkouts live is decided once.
    pub(crate) fn at(git: &Git, config: &Config, commit: String, name: &str) -> Result<Base> {
        let (base, hold) = git.base_worktree(&commit, name)?;
        let wt = Worktree::read(base, config)?;
        Ok(Base {
            commit,
            wt,
            prepared: false,
            _hold: hold,
        })
    }

    /// Reads the worktree again after its files changed.
    pub(crate) fn reread(&mut self, config: &Config) -> Result<()> {
        self.wt = Worktree::read(self.wt.git.clone(), config)?;
        Ok(())
    }
}

/// `stills = "<glob>"` or `films = "<glob>"`: every file the check wrote under
/// `$FEARLEZZ_OUT` matching the glob is a still (or film), for tools that write pictures into a
/// folder and no `proof.jsonl`. Each is named by its path below the glob's fixed folders,
/// without the extension: `stills/desktop/home.png` matched by `stills/**/*.png` is
/// `desktop/home`.
pub(crate) struct MediaGlob {
    kind: &'static str,
    /// As written.
    glob: String,
    matcher: globset::GlobMatcher,
    /// The folders before the first segment with a wildcard, with a trailing slash.
    fixed: String,
}

impl MediaGlob {
    pub(crate) fn parse(kind: &'static str, written: &str) -> Result<MediaGlob> {
        let glob = written.strip_prefix("$FEARLEZZ_OUT/").unwrap_or(written);
        let matcher = globset::GlobBuilder::new(glob)
            .literal_separator(true)
            .build()
            .map_err(|e| anyhow::anyhow!("{kind}s \"{glob}\": {e}"))?
            .compile_matcher();
        // The file name segment is never a folder, so only the ones before it count.
        let folders = glob.rsplit_once('/').map_or("", |(dirs, _)| dirs);
        let mut fixed = String::new();
        for segment in folders
            .split('/')
            .filter(|s| !s.is_empty())
            .take_while(|s| !s.contains(['*', '?', '[', '{']))
        {
            fixed.push_str(segment);
            fixed.push('/');
        }
        Ok(MediaGlob {
            kind,
            glob: written.to_string(),
            matcher,
            fixed,
        })
    }

    /// The glob as written.
    pub(crate) fn glob(&self) -> &str {
        &self.glob
    }

    /// The proofs for the files under `out` this glob matches, sorted by path.
    pub(crate) fn proofs(&self, out: &Path) -> Vec<serde_json::Value> {
        let mut files = Vec::new();
        walk(out, out, &mut files);
        files.sort();
        files
            .into_iter()
            .filter(|rel| self.matcher.is_match(rel))
            .map(|rel| {
                let below = rel.strip_prefix(&self.fixed).unwrap_or(&rel);
                let id = below.rsplit_once('.').map_or(below, |(stem, _)| stem);
                serde_json::json!({ "kind": self.kind, "file": rel, "id": id })
            })
            .collect()
    }
}

/// Every file below `dir`, as a path relative to `base` with forward slashes.
fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(base, &path, out);
        } else if let Ok(rel) = path.strip_prefix(base) {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

struct MediaProof {
    name: String,
    kind: String,
    file: String,
}

fn media_proofs(result: &RunResult) -> Vec<MediaProof> {
    result
        .proofs
        .iter()
        .filter_map(|p| {
            let kind = p["kind"].as_str()?;
            if kind != "still" && kind != "film" {
                return None;
            }
            let file = p["file"].as_str()?.to_string();
            let name = p["id"].as_str().map(String::from).unwrap_or(file.clone());
            Some(MediaProof {
                name,
                kind: kind.to_string(),
                file,
            })
        })
        .collect()
}

pub(crate) fn has_media(result: &RunResult) -> bool {
    !media_proofs(result).is_empty()
}

/// Compares the stills and films of `result`, a run of a check that takes pictures, with the
/// same check run on the base commit. A run without any is compared too, so pictures the
/// change removed all show as gone. Runs the check on the base only when no result exists for
/// its inputs there.
pub(crate) fn look(
    store: &Store,
    check: &Check,
    result: &RunResult,
    base: &mut Base,
    stage: &str,
    env: &EnvCtx<'_>,
) -> Result<LookOutcome> {
    let inputs = base.wt.inputs(check)?;
    if inputs.key == result.key {
        return Ok(if has_media(result) {
            LookOutcome::SameInputs
        } else {
            LookOutcome::NoMedia
        });
    }
    let before = match store.for_key(&inputs.key).pop() {
        Some(r) => r,
        None => run_on_base(store, check, &inputs, base, stage, env)?,
    };
    if !has_media(result) && (before.outcome == Outcome::Failed || !has_media(&before)) {
        return Ok(LookOutcome::NoMedia);
    }
    if before.outcome == Outcome::Failed {
        let reason = format!(
            "the check fails on the base commit (log: {})",
            before.log.display()
        );
        return without_before(check, result, &base.wt.git.common, &reason).map(LookOutcome::Done);
    }
    let dir = look_dir(&base.wt.git.common, check, &before.id, &result.id);
    let saved: Option<Look> = fs::read_to_string(dir.join("look.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok());
    // A base run that passes without any picture usually means it could not reach what it
    // records, or the recording is new in this change.
    let note = (!has_media(&before)).then(|| {
        format!(
            "the base run passed without any pictures; when the recording is not new in this \
             change, read its log: {}",
            before.log.display()
        )
    });
    let look = if let Some(look) = saved {
        look
    } else {
        build(check, Some(&before), result, &base.commit, None, note, &dir)?
    };
    Ok(LookOutcome::Done(look))
}

/// The look when there is no before: every picture is new and needs approval, so a missing
/// base never lets changed pictures through unseen.
pub(crate) fn without_before(
    check: &Check,
    result: &RunResult,
    common: &Path,
    reason: &str,
) -> Result<Look> {
    let dir = look_dir(common, check, "", &result.id);
    build(
        check,
        None,
        result,
        "",
        Some(reason.to_string()),
        None,
        &dir,
    )
}

/// One folder per pair of runs and the pictures called flaky, with a short name people can
/// open.
fn look_dir(common: &Path, check: &Check, before: &str, after: &str) -> PathBuf {
    let flaky = check.pictures.flaky.patterns().join("\0");
    let pair = hex::encode(Sha256::digest(format!("{before}\0{after}\0{flaky}")));
    common
        .join("fearlezz")
        .join("looks")
        .join(format!("{}-{}", check.id, &pair[..10]))
}

/// Runs a check on the base checkout: prepared like any checkout, with its own services
/// for this one run.
pub(crate) fn run_on_base(
    store: &Store,
    check: &Check,
    inputs: &crate::store::Inputs,
    base: &mut Base,
    stage: &str,
    env: &EnvCtx<'_>,
) -> Result<RunResult> {
    let root = base.wt.git.root.clone();
    if !base.prepared {
        if let Some(line) = env.state.prepare(env.cfg, &env.main, &root)? {
            println!("base {line}");
        }
        base.prepared = true;
    }
    let mut vars = std::collections::BTreeMap::new();
    if check.needs_services() {
        let (services, line) = env
            .state
            .ensure_up(env.cfg, &root, BASE_JOURNAL, false, env.now)?;
        if let Some(line) = line {
            println!("base {line}");
        }
        vars = services.ports;
    }
    let at = run::Where {
        root: &root,
        tree: &base.wt.tree,
        journal: BASE_JOURNAL,
        stage,
        vars: &vars,
        changed: &base.wt.lines,
    };
    let result = run::execute(store, check, inputs, &at);
    let stopped = if check.needs_services() {
        env.state.stop(env.cfg, &root)
    } else {
        Ok(())
    };
    let result = result?;
    stopped?;
    Ok(result)
}

fn build(
    check: &Check,
    before: Option<&RunResult>,
    after: &RunResult,
    base_commit: &str,
    no_before: Option<String>,
    note: Option<String>,
    dir: &Path,
) -> Result<Look> {
    let media_dir = dir.join("media");
    fs::create_dir_all(&media_dir)?;
    let befores = before.map(media_proofs).unwrap_or_default();
    let afters = media_proofs(after);
    // A short hash keeps `a/b.png` and `a_b.png` apart.
    let copy = |side: &str, run: &RunResult, p: &MediaProof| -> Result<String, String> {
        let tag = hex::encode(&Sha256::digest(p.file.as_bytes())[..4]);
        let name = p.file.rsplit('/').next().unwrap_or(&p.file);
        let rel = format!("media/{side}-{tag}-{name}");
        fs::copy(run.dir.join(&p.file), dir.join(&rel))
            .map(|_| rel)
            .map_err(|e| format!("{} is missing from the proof folder ({e})", p.file))
    };

    let mut pending: Vec<(Item, Option<(PathBuf, PathBuf)>)> = Vec::new();
    for a in &afters {
        let b = before.and_then(|run| Some((run, befores.iter().find(|b| b.name == a.name)?)));
        let mut item = Item {
            name: a.name.clone(),
            kind: a.kind.clone(),
            change: Change::New,
            summary: "new".into(),
            before: None,
            after: None,
            comparison: None,
            flaky: check.pictures.is_flaky(&a.name),
        };
        let copies = (
            b.map(|(run, b)| copy("before", run, b)).transpose(),
            copy("after", after, a),
        );
        match copies {
            (Ok(bc), Ok(ac)) => {
                item.before = bc;
                item.after = Some(ac);
                let pair = b.map(|(run, b)| (run.dir.join(&b.file), after.dir.join(&a.file)));
                pending.push((item, pair));
            }
            (Err(e), _) | (_, Err(e)) => {
                item.change = Change::Unreadable;
                item.summary = e;
                pending.push((item, None));
            }
        }
    }
    for b in befores
        .iter()
        .filter(|b| !afters.iter().any(|a| a.name == b.name))
    {
        let copied = before.map(|run| copy("before", run, b));
        pending.push((
            Item {
                name: b.name.clone(),
                kind: b.kind.clone(),
                change: Change::Gone,
                summary: match &copied {
                    Some(Err(e)) => format!("gone; {e}"),
                    _ => "gone".into(),
                },
                before: copied.and_then(Result::ok),
                after: None,
                comparison: None,
                flaky: check.pictures.is_flaky(&b.name),
            },
            None,
        ));
    }

    // Compare a few items at a time; each decodes two films in memory.
    let width = std::thread::available_parallelism()
        .map_or(2, std::num::NonZero::get)
        .clamp(1, 4);
    let mut items = Vec::new();
    for chunk in pending.chunks(width) {
        let done: Result<Vec<Item>> = std::thread::scope(|s| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|(item, pair)| {
                    s.spawn(move || {
                        let mut item = item.clone();
                        if let Some((b, a)) = pair {
                            let compared = media::decode_both(b, a)
                                .and_then(|(fb, fa)| media::compare(&fb, &fa));
                            match compared {
                                Ok(c) => {
                                    item.change = match c.verdict {
                                        Verdict::Same => Change::Same,
                                        Verdict::Timing => Change::Timing,
                                        Verdict::Changed => Change::Changed,
                                    };
                                    item.summary = c.summary();
                                    item.comparison = Some(c);
                                }
                                Err(e) => {
                                    item.change = Change::Unreadable;
                                    item.summary = format!("{e:#}");
                                }
                            }
                        }
                        item
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join().map_err(|panic| {
                        anyhow::anyhow!(
                            "comparing a still or film crashed: {}",
                            crate::panic_text(&*panic)
                        )
                    })
                })
                .collect()
        });
        items.extend(done?);
    }
    items.sort_by(|a, b| a.change.cmp(&b.change).then(a.name.cmp(&b.name)));

    let mut look = Look {
        check: check.id.clone(),
        base_commit: base_commit.to_string(),
        base_result: before.map(|b| b.id.clone()).unwrap_or_default(),
        no_before,
        note,
        items,
        report: dir.join("index.html"),
    };
    crate::report::write(dir, &mut look)?;
    // Whole or not at all: a reader never sees a look written in part.
    let part = dir.join("look.json.part");
    fs::write(&part, serde_json::to_string_pretty(&look)?)?;
    fs::rename(&part, dir.join("look.json"))?;
    Ok(look)
}
