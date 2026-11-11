//! The overview page: every journal on one static page, most urgent first.

use std::fmt::Write as _;

use jiff::Timestamp;

use crate::config::Config;
use crate::journal::{Event, Journal};
use crate::state::{CheckState, Group, View};
use crate::{ago, state_label};

const STYLE: &str = "
:root { --bg:#f6f7f9; --card:#fff; --fg:#1b2230; --muted:#5d6675; --line:#dfe3ea;
  --you:#7a4fd6; --bad:#c8402f; --ok:#2e8b57; --stale:#b7791f; color-scheme: light dark; }
@media (prefers-color-scheme: dark) { :root { --bg:#12161d; --card:#1a1f28; --fg:#e8ebf0;
  --muted:#9aa3b2; --line:#2b323e; --you:#b49bff; --bad:#ff8a7a; --ok:#6fcf97; --stale:#e0b25a; } }
* { box-sizing: border-box; }
body { margin:0; background:var(--bg); color:var(--fg); font:16px/1.55 system-ui, sans-serif; }
main { max-width: 860px; margin: 0 auto; padding: 24px 16px 64px; }
h1 { font-size: 26px; margin: 0 0 4px; }
h2 { font-size: 18px; margin: 32px 0 10px; color: var(--muted); }
.built { color: var(--muted); font-size: 14px; }
section { background: var(--card); border: 1px solid var(--line); border-left: 4px solid var(--line);
  border-radius: 10px; padding: 16px 18px; margin-bottom: 12px; }
section.needs-you { border-left-color: var(--you); }
section.failing { border-left-color: var(--bad); }
section.active { border-left-color: var(--ok); }
h3 { margin: 0; font-size: 19px; }
.meta { color: var(--muted); font-size: 14px; margin: 2px 0 10px; }
.now { font-size: 17px; margin: 0 0 12px; }
.stages { display: flex; gap: 6px; flex-wrap: wrap; margin-bottom: 12px; }
.stages span { font-size: 13px; padding: 2px 10px; border-radius: 99px; border: 1px solid var(--line); }
.stages .done { color: var(--muted); }
.stages .cur { border: 2px solid var(--fg); font-weight: 600; }
ul { margin: 6px 0 10px; padding-left: 20px; }
li { margin: 2px 0; }
.ok { color: var(--ok); } .bad { color: var(--bad); } .stale { color: var(--stale); }
details { margin-top: 8px; font-size: 14px; }
summary { cursor: pointer; color: var(--muted); }
.tl { list-style: none; padding: 0; }
.tl time { color: var(--muted); display: inline-block; min-width: 90px; }
";

pub(crate) fn render(
    config: &Config,
    views: &[(View, Journal)],
    now: Timestamp,
) -> Result<String, std::fmt::Error> {
    let mut html = String::new();
    write!(
        html,
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Fearlezz overview</title><style>{STYLE}</style></head><body><main><h1>Fearlezz</h1><p class=\"built\">{} journals, built {}</p>",
        views.len(),
        esc(&now.to_string())
    )?;
    for group in [Group::NeedsYou, Group::Failing, Group::Active, Group::Idle] {
        let in_group: Vec<&(View, Journal)> =
            views.iter().filter(|(v, _)| v.group == group).collect();
        if in_group.is_empty() {
            continue;
        }
        write!(html, "<h2>{} ({})</h2>", heading(group), in_group.len())?;
        for (v, j) in in_group {
            journal_card(&mut html, config, v, j, now)?;
        }
    }
    html.push_str("</main></body></html>");
    Ok(html)
}

fn heading(group: Group) -> &'static str {
    match group {
        Group::NeedsYou => "Needs you",
        Group::Failing => "Failing",
        Group::Active => "Active",
        Group::Idle => "Idle",
    }
}

fn journal_card(
    html: &mut String,
    config: &Config,
    v: &View,
    j: &Journal,
    now: Timestamp,
) -> std::fmt::Result {
    let class = v.group.label().replace(' ', "-");
    let last = v.last_at.map(|t| ago(now, t)).unwrap_or_default();
    write!(
        html,
        "<section class=\"{class}\"><h3>{}</h3><p class=\"meta\">last activity {}{}</p><p class=\"now\">{}</p><div class=\"stages\">",
        esc(&v.name),
        esc(&last),
        if v.live { "" } else { ", not checked out" },
        esc(&v.now)
    )?;
    for stage in config.stages.all() {
        let class = match stage.cmp(&v.stage) {
            std::cmp::Ordering::Less => "done",
            std::cmp::Ordering::Equal => "cur",
            std::cmp::Ordering::Greater => "",
        };
        write!(
            html,
            "<span class=\"{class}\">{}</span>",
            esc(config.stages.name(stage))
        )?;
    }
    html.push_str("</div>");
    if let Some(reason) = &v.went_back {
        write!(html, "<p>Went back: {}</p>", esc(reason))?;
    }
    if !v.checks.is_empty() {
        html.push_str("<strong>Checks</strong><ul>");
        for c in &v.checks {
            let (class, extra) = match &c.state {
                CheckState::Ok => ("ok", String::new()),
                CheckState::Failed | CheckState::Unstable => ("bad", String::new()),
                CheckState::Stale(why) => ("stale", format!(": {} changed", why.join(", "))),
                _ => ("stale", String::new()),
            };
            let look = c.look.as_ref().map_or(String::new(), |l| {
                format!(
                    ", <a href=\"file://{}\">look: {}</a>",
                    esc(&l.report.display().to_string()),
                    esc(&l.counts())
                )
            });
            write!(
                html,
                "<li>{} <span class=\"{class}\">{}</span>{}{look}</li>",
                esc(&c.id),
                state_label(&c.state),
                esc(&extra)
            )?;
        }
        html.push_str("</ul>");
    }
    if let Some((at, text)) = &v.last_note {
        write!(html, "<p>Note, {}: {}</p>", esc(&ago(now, *at)), esc(text))?;
    }
    write!(html, "<p class=\"meta\">Next: {}</p>", esc(&v.next))?;
    timeline(html, j, now)?;
    html.push_str("</section>");
    Ok(())
}

/// The latest events, newest first, with runs folded into one line per stretch.
fn timeline(html: &mut String, j: &Journal, now: Timestamp) -> std::fmt::Result {
    let mut lines: Vec<(Timestamp, String)> = Vec::new();
    let mut runs = (0usize, 0usize);
    let mut run_at = None;
    let flush = |lines: &mut Vec<(Timestamp, String)>,
                 runs: &mut (usize, usize),
                 at: &mut Option<Timestamp>| {
        if let Some(t) = at.take() {
            lines.push((t, format!("{} runs, {} red", runs.0, runs.1)));
            *runs = (0, 0);
        }
    };
    for e in &j.entries {
        let text = match &e.event {
            Event::Ran { outcome, .. } => {
                runs.0 += 1;
                runs.1 += usize::from(*outcome == crate::store::Outcome::Failed);
                run_at = Some(e.at);
                continue;
            }
            Event::Retired => continue,
            Event::Stage { to, reason } => match reason {
                Some(r) => format!("stage {to}: {r}"),
                None => format!("stage {to}"),
            },
            Event::Wait { question } => format!("asked you: {question}"),
            Event::Approve {
                check,
                said,
                by: Some(by),
                ..
            } => format!("approved {check} on GitHub by @{by}: \"{said}\""),
            Event::Approve { check, said, .. } => format!("approved {check}: \"{said}\""),
            Event::Note { text } => format!("note: {text}"),
        };
        flush(&mut lines, &mut runs, &mut run_at);
        lines.push((e.at, text));
    }
    flush(&mut lines, &mut runs, &mut run_at);
    if lines.is_empty() {
        return Ok(());
    }
    html.push_str("<details><summary>Timeline</summary><ul class=\"tl\">");
    for (at, text) in lines.iter().rev().take(20) {
        write!(
            html,
            "<li><time>{}</time> {}</li>",
            esc(&ago(now, *at)),
            esc(text)
        )?;
    }
    html.push_str("</ul></details>");
    Ok(())
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
