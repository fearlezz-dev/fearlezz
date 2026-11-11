//! The look report: one static page per check run, changes first, with the changed area
//! outlined on the before and after pictures and a blink view to compare them in place.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::Result;

use crate::look::{Change, Item, Look};
use crate::media::{self, Box4, Comparison, Pair};

/// Width of the frames taken from films for the page.
const FRAME_W: usize = 720;

/// A light table: pictures sit on mats on a quiet surface so their colours read true.
/// Magenta means changed, both in words and in the outlines on the pictures.
const STYLE: &str = r#"
:root {
  --surface:#eef0f3; --mat:#ffffff; --ink:#14171c; --quiet:#5f6672; --rule:#d5d9df;
  --signal:#c8106f; --timing:#a45a06; --new:#1f5fd1; --focus:#1f5fd1;
  color-scheme: light dark;
}
@media (prefers-color-scheme: dark) {
  :root {
    --surface:#15181d; --mat:#0c0e11; --ink:#eceef2; --quiet:#9aa1ad; --rule:#2a2f37;
    --signal:#ff4fa6; --timing:#f5a524; --new:#7aa7ff; --focus:#7aa7ff;
  }
}
* { box-sizing: border-box; }
body { margin: 0; background: var(--surface); color: var(--ink);
  font: 17px/1.55 "Atkinson Hyperlegible", system-ui, sans-serif; }
main { max-width: 1180px; margin: 0 auto; padding: 40px 16px 96px; }
header { margin-bottom: 40px; }
h1 { font-size: 34px; line-height: 1.15; margin: 0 0 6px; letter-spacing: -0.01em; }
.verdict { font-size: 20px; margin: 0 0 18px; }
.base { color: var(--quiet); font-size: 15px; margin: 0; }
.approve { margin: 22px 0 0; padding: 14px 16px; border-left: 4px solid var(--signal);
  background: var(--mat); border-radius: 0 8px 8px 0; max-width: 760px; }
.approve p { margin: 0 0 8px; }
.cmd { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
code { font: 14px/1.4 ui-monospace, "SF Mono", Menlo, monospace; }
.cmd code { padding: 6px 10px; border: 1px solid var(--rule); border-radius: 6px; background: var(--surface);
  overflow-x: auto; max-width: 100%; }
button { font: inherit; font-size: 15px; color: var(--ink); background: var(--mat);
  border: 1.5px solid var(--rule); border-radius: 6px; padding: 5px 12px; cursor: pointer; }
button:hover { border-color: var(--quiet); }
button:focus-visible, summary:focus-visible { outline: 3px solid var(--focus); outline-offset: 2px; }
article { padding: 32px 0; border-top: 1px solid var(--rule); }
.title { display: flex; gap: 14px; align-items: baseline; flex-wrap: wrap; }
h2 { font-size: 24px; margin: 0; overflow-wrap: anywhere; }
.word { font-weight: 700; font-size: 18px; }
.changed, .unreadable { color: var(--signal); } .timing { color: var(--timing); }
.new, .gone { color: var(--new); } .same { color: var(--quiet); }
.what { margin: 4px 0 20px; font-size: 18px; }
h3 { font-size: 17px; margin: 24px 0 10px; }
.tools { display: flex; gap: 10px; align-items: center; margin: 0 0 10px; color: var(--quiet); font-size: 15px; }
.pair { display: flex; flex-wrap: wrap; gap: 24px; align-items: flex-start; }
.pair figure { margin: 0; max-width: calc(50% - 12px); }
.pair figcaption { font-size: 15px; color: var(--quiet); margin: 0 0 6px; }
.mat { background: var(--mat); padding: 10px; border-radius: 8px; display: inline-block; max-width: 100%; }
.shot { position: relative; display: inline-block; max-width: 100%; line-height: 0; }
.shot img, .shot video { max-width: 100%; max-height: 640px; display: block; }
.mark { position: absolute; outline: 3px solid var(--signal); outline-offset: 2px; border-radius: 3px; pointer-events: none; }
.pair.blink { display: grid; grid-template-columns: max-content; }
.pair.blink figure { grid-area: 1 / 1; max-width: 100%; }
.pair.blink figure.before { visibility: hidden; }
.pair.blink.show-before figure.before { visibility: visible; }
.pair.blink.show-before figure.after { visibility: hidden; }
details { margin-top: 18px; }
summary { cursor: pointer; color: var(--quiet); font-size: 16px; }
.strip { display: flex; gap: 12px; overflow-x: auto; padding: 12px 0 4px; }
.strip figure { margin: 0; flex: none; display: grid; grid-template-columns: 1fr 1fr; gap: 4px;
  padding: 6px; background: var(--mat); border-radius: 8px; outline: 2px solid transparent; }
.strip figure.hit { outline-color: var(--signal); }
.strip img { width: 96px; display: block; }
.strip figcaption { grid-column: 1 / 3; font-size: 13px; color: var(--quiet); text-align: center;
  font-variant-numeric: tabular-nums; }
.same-list { color: var(--quiet); padding-left: 20px; }
@media (max-width: 700px) { .pair figure { max-width: 100%; } h1 { font-size: 28px; } }
"#;

/// Hold to show the before in place of the after; copy the approve command.
const SCRIPT: &str = "
document.querySelectorAll('[data-blink]').forEach(function (btn) {
  var pair = document.getElementById(btn.dataset.blink);
  btn.addEventListener('click', function () {
    var on = pair.classList.toggle('blink');
    btn.setAttribute('aria-pressed', on);
    btn.textContent = on ? 'Side by side' : 'Blink';
    btn.parentNode.querySelector('.hold').hidden = !on;
  });
});
document.querySelectorAll('.hold').forEach(function (hold) {
  var pair = hold.closest('.pair-wrap').querySelector('.pair');
  var show = function (v) { pair.classList.toggle('show-before', v); };
  hold.addEventListener('pointerdown', function () { show(true); });
  ['pointerup', 'pointerleave', 'blur'].forEach(function (e) { hold.addEventListener(e, function () { show(false); }); });
  hold.addEventListener('keydown', function (e) { if (e.key === ' ' || e.key === 'Enter') { e.preventDefault(); show(true); } });
  hold.addEventListener('keyup', function () { show(false); });
});
document.querySelectorAll('[data-copy]').forEach(function (btn) {
  btn.addEventListener('click', function () {
    navigator.clipboard.writeText(btn.dataset.copy).then(function () {
      btn.textContent = 'Copied';
      setTimeout(function () { btn.textContent = 'Copy'; }, 1500);
    });
  });
});
";

/// Writes `index.html` into `dir`, taking the frames it shows from the films.
pub(crate) fn write(dir: &Path, look: &mut Look) -> Result<()> {
    let mut html = String::new();
    let check = esc(&look.check);
    write!(
        html,
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Look: {check}</title><link rel=\"preconnect\" href=\"https://fonts.googleapis.com\"><link rel=\"stylesheet\" href=\"https://fonts.googleapis.com/css2?family=Atkinson+Hyperlegible:wght@400;700&amp;display=swap\"><style>{STYLE}</style></head><body><main><header><h1>Look: {check}</h1><p class=\"verdict\">{}</p><p class=\"base\">{}</p>",
        esc(&verdict_sentence(look)),
        esc(&match &look.no_before {
            Some(why) => format!("There is no before: {why}. Every picture counts as new."),
            None => format!(
                "Compared with the base commit {}, recorded on this machine.",
                look.base_commit.get(..10).unwrap_or(&look.base_commit)
            ),
        }),
    )?;
    if look.to_look_at() > 0 {
        let command = format!("fearlezz say approve {} --said \"looks good\"", look.check);
        write!(
            html,
            "<div class=\"approve\"><p>When this looks right, tell your agent. Or run:</p><div class=\"cmd\"><code>{}</code><button type=\"button\" data-copy=\"{}\">Copy</button></div></div>",
            esc(&command),
            esc(&command)
        )?;
    }
    html.push_str("</header>");
    let (same, rest): (Vec<&Item>, Vec<&Item>) =
        look.items.iter().partition(|i| i.change == Change::Same);
    for (n, item) in rest.iter().enumerate() {
        item_section(&mut html, dir, n, item)?;
    }
    if !same.is_empty() {
        write!(
            html,
            "<article><details><summary>{} unchanged</summary><ul class=\"same-list\">",
            same.len()
        )?;
        for item in &same {
            write!(html, "<li>{}</li>", esc(&item.name))?;
        }
        html.push_str("</ul></details></article>");
    }
    write!(html, "</main><script>{SCRIPT}</script></body></html>")?;
    std::fs::write(dir.join("index.html"), html)?;
    look.report = dir.join("index.html");
    Ok(())
}

/// For example "1 picture changed, 1 is new, 1 is unchanged."
fn verdict_sentence(look: &Look) -> String {
    let count = |c: Change| look.items.iter().filter(|i| i.change == c).count();
    let mut parts = Vec::new();
    for (c, one, many) in [
        (Change::Changed, "1 picture changed", "pictures changed"),
        (
            Change::Timing,
            "1 changed only in timing",
            "changed only in timing",
        ),
        (Change::New, "1 is new", "are new"),
        (Change::Gone, "1 is gone", "are gone"),
        (
            Change::Unreadable,
            "1 could not be read",
            "could not be read",
        ),
        (Change::Same, "1 is unchanged", "are unchanged"),
    ] {
        match count(c) {
            0 => {}
            1 => parts.push(one.to_string()),
            n => parts.push(format!("{n} {many}")),
        }
    }
    format!("{}.", parts.join(", "))
}

fn item_section(html: &mut String, dir: &Path, n: usize, item: &Item) -> Result<()> {
    let label = item.change.label();
    write!(
        html,
        "<article><div class=\"title\"><h2>{}</h2><span class=\"word {label}\">{}</span></div><p class=\"what\">{}</p>",
        esc(&item.name),
        esc(word(item.change)),
        esc(&if item.flaky {
            format!("Flaky: shown, never blocks. {}", plain_summary(item))
        } else {
            plain_summary(item)
        })
    )?;
    match &item.comparison {
        Some(c) if c.verdict == media::Verdict::Changed => {
            if c.still || c.size_before != c.size_after {
                let area = c.moments.first().map(|m| vec![m.area]).unwrap_or_default();
                pair_html(
                    html,
                    &format!("p{n}-0"),
                    (item.before.as_deref(), c.size_before),
                    (item.after.as_deref(), c.size_after),
                    &area,
                )?;
            } else {
                // One film whose frames cannot be cut keeps its summary; the page still shows
                // every other item.
                let mut moments = String::new();
                match film_moments(&mut moments, dir, n, item, c) {
                    Ok(()) => html.push_str(&moments),
                    Err(e) => write!(
                        html,
                        "<p class=\"same\">The frames of this film could not be cut: {}</p>",
                        esc(&format!("{e:#}"))
                    )?,
                }
            }
        }
        _ => {}
    }
    let is_still = matches!(&item.comparison, Some(c) if c.still) || item.kind == "still";
    if is_still && item.comparison.is_none() {
        // New or gone stills: show the one there is.
        pair_html(
            html,
            &format!("p{n}-s"),
            (item.before.as_deref(), (0, 0)),
            (item.after.as_deref(), (0, 0)),
            &[],
        )?;
    } else if !is_still {
        let size = item.comparison.as_ref().map_or((0, 0), |c| c.size_after);
        write!(
            html,
            "<details{}><summary>Play both</summary>",
            if item.change == Change::Changed {
                ""
            } else {
                " open"
            }
        )?;
        pair_html(
            html,
            &format!("p{n}-play"),
            (item.before.as_deref(), size),
            (item.after.as_deref(), size),
            &[],
        )?;
        html.push_str("</details>");
    }
    html.push_str("</article>");
    Ok(())
}

fn word(change: Change) -> &'static str {
    match change {
        Change::Changed => "Changed",
        Change::Timing => "Timing only",
        Change::New => "New",
        Change::Gone => "Gone",
        Change::Unreadable => "Could not read",
        Change::Same => "Unchanged",
    }
}

/// The comparison in plain words.
fn plain_summary(item: &Item) -> String {
    let Some(c) = &item.comparison else {
        return match item.change {
            Change::New => "Not in the base. Check that it looks right.".into(),
            Change::Gone => "Was in the base, not anymore.".into(),
            _ => item.summary.clone(),
        };
    };
    match c.verdict {
        media::Verdict::Same => "Looks the same.".into(),
        media::Verdict::Timing => format!(
            "Same pictures, different timing: {:.2} s long, was {:.2} s.",
            c.total_after, c.total_before
        ),
        media::Verdict::Changed if c.size_before != c.size_after => format!(
            "The size changed from {}×{} to {}×{}.",
            c.size_before.0, c.size_before.1, c.size_after.0, c.size_after.1
        ),
        media::Verdict::Changed if c.still => "The outlined area changed.".into(),
        media::Verdict::Changed => {
            let at: Vec<String> = c
                .moments
                .iter()
                .map(|m| format!("{:.2} s", m.after.0))
                .collect();
            format!("Changes at {}.", at.join(", "))
        }
    }
}

/// For each changed moment of a film: the matched before and after frames, outlined.
fn film_moments(
    html: &mut String,
    dir: &Path,
    n: usize,
    item: &Item,
    c: &Comparison,
) -> Result<()> {
    let (Some(before), Some(after)) = (&item.before, &item.after) else {
        return Ok(());
    };
    // The most changed pair of each run of changed pairs stands for its moment.
    let mut picks: Vec<&Pair> = Vec::new();
    let mut best: Option<&Pair> = None;
    for p in &c.pairs {
        if p.px > 0 {
            if best.is_none_or(|b| p.px > b.px) {
                best = Some(p);
            }
        } else {
            picks.extend(best.take());
        }
    }
    picks.extend(best);
    // Pairs around each change, for the strip.
    let near: Vec<&Pair> = c
        .pairs
        .iter()
        .enumerate()
        .filter(|(k, _)| {
            (k.saturating_sub(1)..(k + 2).min(c.pairs.len())).any(|x| c.pairs[x].px > 0)
        })
        .map(|(_, p)| p)
        .collect();
    let frames_dir = dir.join("frames");
    let want_b: Vec<usize> = near.iter().map(|p| p.before_frame).collect();
    let want_a: Vec<usize> = near.iter().map(|p| p.after_frame).collect();
    let fb = media::extract(
        &dir.join(before),
        &want_b,
        FRAME_W,
        &frames_dir,
        &format!("{n}-b"),
    )?;
    let fa = media::extract(
        &dir.join(after),
        &want_a,
        FRAME_W,
        &frames_dir,
        &format!("{n}-a"),
    )?;
    let rel = |list: &[(usize, std::path::PathBuf)], frame: usize| -> Option<String> {
        list.iter()
            .find(|(f, _)| *f == frame)
            .and_then(|(_, p)| Some(format!("frames/{}", p.file_name()?.to_string_lossy())))
    };
    for (k, p) in picks.iter().enumerate() {
        let title = match c.moments.get(k) {
            Some(m) if (m.after.0 - m.before.0).abs() >= 0.01 => {
                format!("At {:.2} s (was {:.2} s)", m.after.0, m.before.0)
            }
            Some(m) => format!("At {:.2} s", m.after.0),
            None => format!("Change {}", k + 1),
        };
        if picks.len() > 1 {
            write!(html, "<h3>{}</h3>", esc(&title))?;
        }
        pair_html(
            html,
            &format!("p{n}-{k}"),
            (rel(&fb, p.before_frame).as_deref(), c.size_before),
            (rel(&fa, p.after_frame).as_deref(), c.size_after),
            &p.boxes,
        )?;
    }
    if !near.is_empty() {
        html.push_str("<details><summary>Frames around the changes</summary><div class=\"strip\">");
        for p in &near {
            let hit = if p.px > 0 { " class=\"hit\"" } else { "" };
            write!(
                html,
                "<figure{hit}><img src=\"{}\" alt=\"Before at {:.2} s\"><img src=\"{}\" alt=\"After at {:.2} s\"><figcaption>{:.2} s</figcaption></figure>",
                esc(&rel(&fb, p.before_frame).unwrap_or_default()),
                p.before_at,
                esc(&rel(&fa, p.after_frame).unwrap_or_default()),
                p.after_at,
                p.after_at
            )?;
        }
        html.push_str("</div></details>");
    }
    Ok(())
}

/// Before and after side by side, with a blink view when both exist.
fn pair_html(
    html: &mut String,
    id: &str,
    before: (Option<&str>, (usize, usize)),
    after: (Option<&str>, (usize, usize)),
    marks: &[Box4],
) -> std::fmt::Result {
    let both = before.0.is_some() && after.0.is_some();
    html.push_str("<div class=\"pair-wrap\">");
    if both {
        write!(
            html,
            "<div class=\"tools\"><button type=\"button\" data-blink=\"{id}\" aria-pressed=\"false\">Blink</button><button type=\"button\" class=\"hold\" hidden>Hold to see before</button></div>"
        )?;
    }
    write!(html, "<div class=\"pair\" id=\"{id}\">")?;
    for (class, label, (src, (w, h))) in [("before", "Before", before), ("after", "After", after)] {
        write!(
            html,
            "<figure class=\"{class}\"><figcaption>{label}</figcaption>"
        )?;
        match src {
            None => html.push_str("<p class=\"same\">Not there</p>"),
            Some(src) => {
                html.push_str("<div class=\"mat\"><div class=\"shot\">");
                html.push_str(&media_tag(src, label));
                if w > 0 && h > 0 {
                    for b in marks {
                        let pct = |v: u32, of: usize| f64::from(v) * 100.0 / of as f64;
                        write!(
                            html,
                            "<div class=\"mark\" style=\"left:{:.2}%;top:{:.2}%;width:{:.2}%;height:{:.2}%\"></div>",
                            pct(b[0], w),
                            pct(b[1], h),
                            pct(b[2] - b[0], w),
                            pct(b[3] - b[1], h)
                        )?;
                    }
                }
                html.push_str("</div></div>");
            }
        }
        html.push_str("</figure>");
    }
    html.push_str("</div></div>");
    Ok(())
}

fn media_tag(src: &str, label: &str) -> String {
    let lower = src.to_ascii_lowercase();
    if [".mp4", ".webm", ".mov", ".m4v"]
        .iter()
        .any(|e| lower.ends_with(e))
    {
        format!(
            "<video src=\"{}\" autoplay loop muted playsinline controls aria-label=\"{label}\"></video>",
            esc(src)
        )
    } else {
        format!("<img src=\"{}\" alt=\"{label}\">", esc(src))
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
