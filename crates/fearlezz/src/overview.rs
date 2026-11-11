//! The overview page: every journal on one static page, most urgent first.

use std::fmt::Write as _;

use jiff::Timestamp;

use crate::config::Config;
use crate::journal::{Event, Journal};
use crate::page::esc;
use crate::state::{CheckState, Group, View};
use crate::{ago, state_label};

const STYLE: &str = "
:root { --you:#7a4fd6; --bad:#c8402f; --live:#1d7a4a; --stale:#9a5300; }
@media (prefers-color-scheme: dark) { :root { --you:#b49bff; --bad:#ff8a7a; --live:#4fd08a; --stale:#f5a524; } }
main { max-width: 900px; margin: 0 auto; padding: 36px 16px 80px; }
.top { display: flex; flex-wrap: wrap; align-items: center; gap: 14px 18px; margin: 0 0 12px; }
h1 { font-size: clamp(32px, 6vw, 44px); margin: 0; }
.tally { display: flex; flex-wrap: wrap; gap: 8px; }
.tally a { display: inline-flex; align-items: baseline; gap: 8px; padding: 6px 14px; border-radius: 12px;
  background: var(--mat); box-shadow: var(--shadow); color: var(--ink); text-decoration: none;
  transition: transform .2s var(--ease); }
.tally a:hover { transform: translateY(-2px); }
.tally b { font-size: 22px; }
.about { margin-left: auto; font-size: 15px; }
.about p { margin: 6px 0 0; color: var(--quiet); }
h2 { display: flex; align-items: center; gap: 10px; font-size: 21px; margin: 36px 0 12px; scroll-margin-top: 16px; }
h2 .n { font-size: 15px; color: var(--quiet); background: var(--sunk); border-radius: 999px; padding: 0 10px; }
.needs-you { color: var(--you); } .failing { color: var(--bad); } .active { color: var(--live); } .idle { color: var(--quiet); }
.card { background: var(--mat); color: var(--ink); border-radius: 18px; padding: 20px 22px; margin: 0 0 14px;
  box-shadow: var(--shadow); transition: box-shadow .3s var(--ease); }
.card.needs-you { box-shadow: 0 0 0 2px var(--you), var(--shadow); }
.card.failing { box-shadow: 0 0 0 2px var(--bad), var(--shadow); }
.head { display: flex; flex-wrap: wrap; align-items: center; gap: 8px 12px; margin: 0 0 8px; }
h3 { margin: 0; font-size: 22px; }
.age { margin-left: auto; color: var(--quiet); font-size: 15px; white-space: nowrap; }
.now { font-size: 18px; margin: 0 0 14px; }
.track { display: flex; flex-wrap: wrap; gap: 6px; list-style: none; padding: 0; margin: 0 0 14px; }
.track li { font-size: 14px; padding: 2px 12px; border-radius: 999px; border: 1.5px solid var(--line); color: var(--quiet); }
.track .done { background: var(--sunk); border-color: var(--sunk); }
.track .cur { background: var(--ink); border-color: var(--ink); color: var(--mat); font-weight: 700; }
.checks { display: flex; flex-wrap: wrap; gap: 6px; list-style: none; padding: 0; margin: 0; }
.checks li, .checks a { display: inline-flex; align-items: center; gap: 7px; font-size: 15px; padding: 3px 12px;
  border-radius: 10px; background: var(--sunk); color: var(--ink); text-decoration: none; }
.checks a { transition: transform .2s var(--ease); box-shadow: 0 0 0 1.5px var(--changed) inset; }
.checks a:hover { transform: translateY(-1px); }
.checks li.link { padding: 0; background: none; }
.dot { width: 9px; height: 9px; border-radius: 50%; background: currentColor; flex: none; }
.ok .dot { color: var(--live); } .bad .dot { color: var(--bad); } .stale .dot { color: var(--stale); }
.look .dot { color: var(--changed); }
.more { margin: 14px 0 0; font-size: 15px; }
.more p, .more ul { margin: 8px 0 0; }
.tl { list-style: none; padding: 0; }
.tl li { margin: 3px 0; }
.tl time { color: var(--quiet); display: inline-block; min-width: 90px; }
@media (max-width: 600px) { .about { margin-left: 0; flex-basis: 100%; } .card { padding: 16px; } }
";

/// The cards rise in one after another.
const SCRIPT: &str = "riseIn(document.querySelectorAll('.card'));";

const GROUPS: [Group; 4] = [Group::NeedsYou, Group::Failing, Group::Active, Group::Idle];

pub(crate) fn render(
    config: &Config,
    views: &[(View, Journal)],
    now: Timestamp,
) -> Result<String, std::fmt::Error> {
    let mut html = String::new();
    write!(
        html,
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Fearlezz overview</title><style>{}{STYLE}</style></head><body><main><header class=\"top\"><h1>Fearlezz</h1><nav class=\"tally\" aria-label=\"Groups\">",
        crate::page::STYLE
    )?;
    let in_group = |g: Group| -> Vec<&(View, Journal)> {
        views.iter().filter(|(v, _)| v.group == g).collect()
    };
    for group in GROUPS {
        let n = in_group(group).len();
        if n > 0 {
            write!(
                html,
                "<a href=\"#{}\"><b class=\"num {}\" data-count=\"{n}\">{n}</b>{}</a>",
                slug(group),
                slug(group),
                heading(group)
            )?;
        }
    }
    write!(
        html,
        "</nav><details class=\"about\"><summary>Built</summary><p>{} journals, built {}</p></details></header>",
        views.len(),
        esc(&now.to_string())
    )?;
    for group in GROUPS {
        let list = in_group(group);
        if list.is_empty() {
            continue;
        }
        write!(
            html,
            "<h2 id=\"{}\"><span class=\"{}\">{}</span><span class=\"n num\">{}</span></h2>",
            slug(group),
            slug(group),
            heading(group),
            list.len()
        )?;
        for (v, j) in list {
            journal_card(&mut html, config, v, j, now)?;
        }
    }
    write!(
        html,
        "</main><script>{}{SCRIPT}</script></body></html>",
        crate::page::SCRIPT
    )?;
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

fn slug(group: Group) -> String {
    group.label().replace(' ', "-")
}

fn journal_card(
    html: &mut String,
    config: &Config,
    v: &View,
    j: &Journal,
    now: Timestamp,
) -> std::fmt::Result {
    let last = v.last_at.map(|t| ago(now, t)).unwrap_or_default();
    write!(
        html,
        "<section class=\"card {}\"><div class=\"head\"><h3>{}</h3>",
        slug(v.group),
        esc(&v.name)
    )?;
    if !v.live {
        html.push_str("<span class=\"tag\">Not checked out</span>");
    }
    write!(
        html,
        "<span class=\"age\">{}</span></div><p class=\"now\">{}</p><ol class=\"track\" aria-label=\"Stages\">",
        esc(&last),
        esc(&v.now)
    )?;
    for stage in config.stages.all() {
        let (class, current) = match stage.cmp(&v.stage) {
            std::cmp::Ordering::Less => ("done", ""),
            std::cmp::Ordering::Equal => ("cur", " aria-current=\"step\""),
            std::cmp::Ordering::Greater => ("", ""),
        };
        write!(
            html,
            "<li class=\"{class}\"{current}>{}</li>",
            esc(config.stages.name(stage))
        )?;
    }
    html.push_str("</ol>");
    if !v.checks.is_empty() {
        html.push_str("<ul class=\"checks\" aria-label=\"Checks\">");
        for c in &v.checks {
            let (class, extra) = match &c.state {
                CheckState::Ok => ("ok", String::new()),
                CheckState::Failed | CheckState::Unstable => ("bad", String::new()),
                CheckState::Stale(why) => ("stale", format!(": {} changed", why.join(", "))),
                _ => ("stale", String::new()),
            };
            let title = format!("{} {}{extra}", c.id, state_label(&c.state));
            write!(
                html,
                "<li class=\"{class}\" title=\"{}\"><span class=\"dot\" aria-hidden=\"true\"></span>{}<span hidden>{}</span></li>",
                esc(&title),
                esc(&c.id),
                esc(state_label(&c.state))
            )?;
            if let Some(l) = &c.look {
                write!(
                    html,
                    "<li class=\"link\"><a class=\"look\" href=\"file://{}\"><span class=\"dot\" aria-hidden=\"true\"></span>Look: {}</a></li>",
                    esc(&l.report.display().to_string()),
                    esc(&l.counts())
                )?;
            }
        }
        html.push_str("</ul>");
    }
    html.push_str("<details class=\"more\"><summary>More</summary>");
    write!(html, "<p>Next: {}</p>", esc(&v.next))?;
    if let Some(reason) = &v.went_back {
        write!(html, "<p>Went back: {}</p>", esc(reason))?;
    }
    if let Some((at, text)) = &v.last_note {
        write!(html, "<p>Note, {}: {}</p>", esc(&ago(now, *at)), esc(text))?;
    }
    timeline(html, j, now)?;
    html.push_str("</details></section>");
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
    html.push_str("<ul class=\"tl\">");
    for (at, text) in lines.iter().rev().take(20) {
        write!(
            html,
            "<li><time>{}</time> {}</li>",
            esc(&ago(now, *at)),
            esc(text)
        )?;
    }
    html.push_str("</ul>");
    Ok(())
}
