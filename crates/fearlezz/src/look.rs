//! The look at a change: every still, film and text a check takes, compared with the same
//! check run on the base commit (where the branch left the base branch), on this machine.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::base::{Base, Checkout};
use crate::config::Check;
use crate::env::EnvCtx;
use crate::media::{self, Verdict};
use crate::prove::{Reuse, Site, prove};
use crate::store::{Outcome, RunResult, Store};

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

/// The kind of a film in a look.
const FILM: &str = "film";

/// Makes the animated WebP copy of each film `look` took (`Item::webp`), beside its after in the
/// look folder. A film it cannot copy gets one line saying why and none; that never fails the
/// proof, as the copy only shows what the proof already holds.
pub(crate) fn webp_copies(look: &mut Look) {
    for item in &mut look.items {
        let (FILM, Some(after)) = (item.kind.as_str(), &item.after) else {
            continue;
        };
        let rel = Path::new(after)
            .with_extension("anim.webp")
            .to_string_lossy()
            .into_owned();
        let said = match media::webp_copy(&look.dir.join(after), &look.dir.join(&rel)) {
            Ok(media::WebpCopy::Made) => {
                item.webp = Some(rel);
                continue;
            }
            Ok(media::WebpCopy::TooLong(seconds)) => format!(
                "it runs {seconds:.0} s, longer than the {:.0} s a WebP copy may",
                media::WEBP_LONGEST
            ),
            Ok(media::WebpCopy::NoFfmpeg) => {
                println!(
                    "look {}: no WebP copies of its films: there is no ffmpeg to make them",
                    look.check
                );
                return;
            }
            Err(e) => format!("{e:#}"),
        };
        println!(
            "look {}: {} gets no WebP copy: {said}",
            look.check, item.name
        );
    }
}

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
    let lines = |n: usize| fearlezz_proof::plural(n, "line");
    let summary = match (added, removed) {
        (0, r) => format!("{} removed", lines(r)),
        (a, 0) => format!("{} added", lines(a)),
        (a, r) => format!("{} added, {r} removed", lines(a)),
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

/// A still, film or text a run lists, by the name its look gives it.
pub(crate) struct MediaProof {
    pub name: String,
    pub kind: String,
    file: String,
}

/// The stills, films and texts `result` lists, in its order.
pub(crate) fn media_proofs(result: &RunResult) -> Vec<MediaProof> {
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
/// change removed all show as gone. Runs the check on the base only when no result that got
/// going exists for its inputs there.
pub(crate) fn look(
    store: &Store,
    check: &Check,
    result: &RunResult,
    (base, site): (&Base, &mut Checkout),
    stage: &str,
    env: &EnvCtx<'_>,
) -> Result<LookOutcome> {
    let inputs = site.wt.inputs(check)?;
    if inputs.key == result.key {
        return Ok(if has_media(result) {
            LookOutcome::SameInputs
        } else {
            LookOutcome::NoMedia
        });
    }
    let on_base = Site::base(base, &mut *site, stage);
    let (before, _) = prove(store, env, check, &inputs, on_base, &Reuse::Ran)?;
    if !has_media(result) && (before.outcome == Outcome::Failed || !has_media(&before)) {
        return Ok(LookOutcome::NoMedia);
    }
    if before.outcome == Outcome::Failed {
        let reason = format!(
            "the check fails on the base commit (log: {})",
            before.log.display()
        );
        return without_before(check, result, &site.wt.git.common, &reason).map(LookOutcome::Done);
    }
    let dir = look_dir(&site.wt.git.common, check, &before.id, &result.id);
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
    common.join("fearlezz").join("looks").join(format!(
        "{}-{}",
        check.id,
        pair.get(..10).unwrap_or(&pair)
    ))
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
            webp: None,
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
                webp: None,
            },
            None,
        ));
    }

    let mut items = compare_all(check, &pending)?;
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

/// Compares every item that has a before, a few at a time.
fn compare_all(check: &Check, pending: &[(Item, Option<(PathBuf, PathBuf)>)]) -> Result<Vec<Item>> {
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
                .map(|(item, pair)| s.spawn(move || compare_one(item, pair.as_ref())))
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
    Ok(items)
}

/// One item compared with its before: a text by its lines, a still or film by its states. An
/// item without a pair stays as it is.
fn compare_one(item: &Item, pair: Option<&(PathBuf, PathBuf)>) -> Item {
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
        let compared = media::decode_both(b, a).and_then(|(fb, fa)| media::compare(&fb, &fa));
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
}
