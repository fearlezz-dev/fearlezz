//! The look at a change: every still, film and text a check takes, compared with the same
//! check run on the base commit (where the branch left the base branch), on this machine.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::config::{Check, Config, Env};
use crate::env::EnvCtx;
use crate::git::Git;
use crate::media::{self, Verdict};
use crate::run;
use crate::state::Worktree;
use crate::store::{Outcome, RunResult, Store};

/// The journal name base runs are recorded under.
pub(crate) const BASE_JOURNAL: &str = "(base)";

pub(crate) use fearlezz_proof::look::{Change, DiffLine, Item, Look};

/// The look saved in the folder `dir`, if any. Its pictures are relative to that folder,
/// wherever it was when the look was written.
pub(crate) fn saved(dir: &Path) -> Option<Look> {
    let text = fs::read_to_string(dir.join("look.json")).ok()?;
    let look: Look = serde_json::from_str(&text).ok()?;
    Some(Look {
        dir: dir.to_path_buf(),
        ..look
    })
}

/// What a person reviews of `look`: each picture or text that is not the same as on the base,
/// with the summary of its comparison and the before and after files to open, one per line,
/// and for a text the start of its diff.
pub(crate) fn lines(look: &Look) -> Vec<String> {
    let mut lines = Vec::new();
    for item in look.items.iter().filter(|i| i.change != Change::Same) {
        let flaky = if item.flaky { " (flaky)" } else { "" };
        lines.push(format!("  {}{flaky}: {}", item.name, item.summary));
        for (side, rel) in [("before", &item.before), ("after", &item.after)] {
            if let Some(rel) = rel {
                lines.push(format!("    {side}: {}", look.dir.join(rel).display()));
            }
        }
        if item.kind == TEXT {
            let diff = text_diff(look, item);
            let (shown, left) = crate::text::head(&diff, DIFF_LINES);
            lines.extend(shown.iter().map(|l| format!("    {l}")));
            if left > 0 {
                lines.push(format!(
                    "    ({left} more lines; diff the before and after files)"
                ));
            }
        }
    }
    lines
}

/// The kind of a text in a look.
pub(crate) const TEXT: &str = "text";

/// How many lines of a text's diff are shown where it is named.
pub(crate) const DIFF_LINES: usize = 40;

/// The unified diff of a text `item` of `look`, from its before (nothing when new) to its
/// after (nothing when gone).
pub(crate) fn text_diff(look: &Look, item: &Item) -> String {
    let side = |rel: &Option<String>| {
        rel.as_ref()
            .and_then(|rel| fs::read(look.dir.join(rel)).ok())
            .map(|b| crate::text::readable(&b))
            .unwrap_or_default()
    };
    crate::text::unified(&item.name, &side(&item.before), &side(&item.after))
}

/// How many kept lines stay on each side of a change in a text's diff the bundle carries.
const CONTEXT: usize = 3;

/// How two texts compare, from their files: the same bytes, or changed with the lines it adds
/// and removes in a summary and its lines against the before.
fn compare_texts(before: &Path, after: &Path) -> Result<(Change, String, Option<Vec<DiffLine>>)> {
    let (b, a) = (fs::read(before)?, fs::read(after)?);
    if b == a {
        return Ok((Change::Same, "same".into(), None));
    }
    let (b, a) = (crate::text::readable(&b), crate::text::readable(&a));
    let (added, removed) = crate::text::counts(&b, &a);
    let lines = |n: usize| if n == 1 { "line" } else { "lines" };
    let summary = match (added, removed) {
        (0, r) => format!("{r} {} removed", lines(r)),
        (a, 0) => format!("{a} {} added", lines(a)),
        (a, r) => format!("{a} {} added, {r} removed", lines(a)),
    };
    Ok((
        Change::Changed,
        summary,
        Some(crate::text::shown(&b, &a, CONTEXT)),
    ))
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
    /// The base commit's own `[env]`: its code ran in that environment, so its checkout is
    /// prepared, its services run and its checks run inside its shell by it, never by the
    /// change's.
    env: Env,
    prepared: bool,
    /// Keeps other commands from removing the worktree while this one uses it.
    _hold: crate::lease::Lease,
}

impl Base {
    /// The base commit in a checkout of its own, told apart from others by `name`. Every
    /// run on a base commit goes through here, so where base checkouts live is decided once.
    pub(crate) fn at(git: &Git, config: &Config, commit: String, name: &str) -> Result<Base> {
        let (base, hold) = git.base_worktree(&commit, name)?;
        let env = if base.root.join(crate::config::FILE).exists() {
            Config::load(&base.root, &git.common)?.env
        } else {
            Env::default()
        };
        let wt = Worktree::read_in(base, config, &env)?;
        Ok(Base {
            commit,
            wt,
            env,
            prepared: false,
            _hold: hold,
        })
    }

    /// Reads the worktree again after its files changed.
    pub(crate) fn reread(&mut self, config: &Config) -> Result<()> {
        self.wt = Worktree::read_in(self.wt.git.clone(), config, &self.env)?;
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
    glob: crate::read::Glob,
}

impl MediaGlob {
    pub(crate) fn parse(kind: &'static str, written: &str) -> Result<MediaGlob> {
        Ok(MediaGlob {
            kind,
            glob: crate::read::Glob::parse(written, &format!("{kind}s"))?,
        })
    }

    /// The glob as written.
    pub(crate) fn glob(&self) -> &str {
        self.glob.written()
    }

    /// The proofs for the files under `out` this glob matches, sorted by path. The run's own
    /// folder is walked whole: a tool may keep pictures in a hidden folder of it.
    pub(crate) fn proofs(&self, out: &Path) -> Vec<serde_json::Value> {
        self.glob
            .matches(out, true)
            .into_iter()
            .map(|rel| {
                let id = self.glob.id(&rel);
                serde_json::json!({ "kind": self.kind, "file": rel, "id": id })
            })
            .collect()
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
            if kind != "still" && kind != "film" && kind != TEXT {
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
    let before = match store.latest_that_ran(&inputs.key) {
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
    let saved = saved(&dir);
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

/// The look when there is no before: every picture is new and shows as such, so a missing
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
    let pair = crate::files::sha256_hex(format!("{before}\0{after}\0{flaky}"));
    common
        .join("fearlezz")
        .join("looks")
        .join(format!("{}-{}", check.id, &pair[..10]))
}

/// Runs a check on the base checkout: prepared like any checkout, with its own services
/// for this one run and inside its shell, all by the base commit's own `[env]`.
pub(crate) fn run_on_base(
    store: &Store,
    check: &Check,
    inputs: &crate::store::Inputs,
    base: &mut Base,
    stage: &str,
    env: &EnvCtx<'_>,
) -> Result<RunResult> {
    // Heavy work waits its turn on this machine; another worktree may run the base meanwhile.
    let label = format!("{} on the base commit", check.id);
    let _slot = if check.light {
        None
    } else {
        match crate::queue::turn(&label, || store.for_key(&inputs.key).pop())? {
            crate::queue::Turn::Go(slot) => slot,
            crate::queue::Turn::Done(found) => return Ok(found),
        }
    };
    let root = base.wt.git.root.clone();
    if !base.prepared {
        if let Some(line) = env.state.prepare(&base.env, &env.main, &root)? {
            println!("base {line}");
        }
        base.prepared = true;
    }
    let mut vars = std::collections::BTreeMap::new();
    if check.needs_services() {
        let (services, line) =
            env.state
                .ensure_up(&base.env, &root, BASE_JOURNAL, false, env.now)?;
        if let Some(line) = line {
            println!("base {line}");
        }
        vars = services.ports;
    }
    // The base is deployed by its own `[env]` as the change is, so pictures of its preview
    // compare with pictures of the change's.
    let mut undeployed = None;
    if check.needs_preview() {
        // Named by its tree: a red before green checkout of the same commit holds the change's
        // tests, so it is another deploy. Base checkouts are shared by worktrees, so the deploy
        // holds the checkout.
        let name = format!("base-{}", base.wt.tree.get(..7).unwrap_or(&base.wt.tree));
        let _held = env.state.hold(&root)?;
        match env.state.preview(&base.env, &root, &base.wt.tree, &name)? {
            Ok(deployed) => {
                if let Some(line) = deployed.line {
                    println!("base {line}");
                }
                vars.insert("FEARLEZZ_PREVIEW_URL".to_string(), deployed.url);
            }
            Err(u) => undeployed = Some(u),
        }
    }
    let at = run::Where {
        root: &root,
        shell: base.wt.shell.as_deref(),
        tree: &base.wt.tree,
        journal: BASE_JOURNAL,
        stage,
        vars: &vars,
        changed: &base.wt.lines,
        no_preview: undeployed.as_ref(),
    };
    let result = run::execute(store, check, inputs, &at);
    let stopped = if check.needs_services() {
        env.state.stop(&base.env, &root)
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
        let mut tag = crate::files::sha256_hex(&p.file);
        tag.truncate(8);
        let name = p.file.rsplit('/').next().unwrap_or(&p.file);
        // A text is kept as `.txt` whatever its own extension, the one type of text the proof
        // host serves, as plain text a browser never runs.
        let is_txt = Path::new(name)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("txt"));
        let txt = if p.kind == TEXT && !is_txt {
            ".txt"
        } else {
            ""
        };
        let rel = format!("media/{side}-{tag}-{name}{txt}");
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
            diff: None,
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
                diff: None,
            },
            None,
        ));
    }

    // Decoding and comparing is heavy work, so it waits its turn on this machine, unless the
    // check says it is light.
    let _slot = if !check.light && pending.iter().any(|(_, pair)| pair.is_some()) {
        crate::queue::slot(&format!("look {}", check.id))?
    } else {
        None
    };
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
                        if let (Some((b, a)), true) = (pair, item.kind == TEXT) {
                            match compare_texts(b, a) {
                                Ok((change, summary, diff)) => {
                                    item.change = change;
                                    item.summary = summary;
                                    item.diff = diff;
                                }
                                Err(e) => {
                                    item.change = Change::Unreadable;
                                    item.summary = format!("{e:#}");
                                }
                            }
                        } else if let Some((b, a)) = pair {
                            let compared = media::decode_both(b, a)
                                .and_then(|(fb, fa)| media::compare(&fb, &fa));
                            match compared {
                                Ok(c) => {
                                    item.change = match c.verdict {
                                        Verdict::Same => Change::Same,
                                        Verdict::Timing => Change::Timing,
                                        Verdict::Changed => Change::Changed,
                                    };
                                    item.summary = media::summary(&c);
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

    let look = Look {
        check: check.id.clone(),
        base_commit: base_commit.to_string(),
        base_result: before.map(|b| b.id.clone()).unwrap_or_default(),
        no_before,
        note,
        items,
        dir: dir.to_path_buf(),
    };
    // Whole or not at all: a reader never sees a look written in part.
    let part = dir.join("look.json.part");
    fs::write(&part, serde_json::to_string_pretty(&look)?)?;
    fs::rename(&part, dir.join("look.json"))?;
    Ok(look)
}
