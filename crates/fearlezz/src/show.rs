//! `fearlezz setup show`: what a built in piece sets as shipped, what a check is once the
//! pieces are merged, or the whole merged config, each value named by the piece or file it
//! came from. Lint tables are counted per level unless all of them are asked for, since one
//! piece can set hundreds.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use anyhow::Result;
use toml::{Table, Value};

use crate::config::{Builtin, Config};
use crate::presets::{BUILTIN, Origins};

/// The built in piece `name` as it ships, its comments kept, or None when no piece has that
/// name. Without `lints`, each lint table is one line of counts.
pub(crate) fn piece(name: &str, lints: bool) -> Result<Option<String>> {
    let Some((_, text)) = BUILTIN.iter().find(|(n, _)| *n == name) else {
        return Ok(None);
    };
    let mut out = format!("# the piece {name}, as shipped\n");
    if lints {
        out.push_str(text);
        return Ok(Some(out));
    }
    let table: Table = toml::from_str(text)?;
    let lint_tables = table.get("lints").and_then(Value::as_table);
    // Lines inside a lint table are left out; the comments and blank lines right before the
    // next header belong to it and are kept.
    let mut skipping = false;
    let mut held = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            out.extend(held.drain(..).map(|l: &str| format!("{l}\n")));
            let tool = trimmed
                .strip_prefix("[lints.")
                .and_then(|t| t.strip_suffix(']'));
            let rules = tool.and_then(|t| Some((t, lint_tables?.get(t)?.as_table()?)));
            skipping = rules.is_some();
            if let Some((tool, rules)) = rules {
                writeln!(out, "[lints.{tool}] # {}", counted(rules, None, &[]))?;
            } else {
                writeln!(out, "{line}")?;
            }
        } else if !skipping {
            writeln!(out, "{line}")?;
        } else if trimmed.is_empty() || trimmed.starts_with('#') {
            held.push(line);
        } else {
            held.clear();
        }
    }
    Ok(Some(out))
}

/// The merged config: every table as the pieces and `fearlezz.toml` set it, then the checks
/// Fearlezz forms itself.
pub(crate) fn config(config: &Config, lints: bool) -> Result<String> {
    let mut out =
        String::from("# the merged config: each value says the piece or file it came from\n");
    out.push_str(&annotated(config.table().clone(), config, lints)?);
    let built_in: Vec<&str> = config
        .checks
        .iter()
        .filter(|c| c.builtin.is_some())
        .map(|c| c.id.as_str())
        .collect();
    if !built_in.is_empty() {
        writeln!(
            out,
            "\n# built in checks, formed by Fearlezz from the tables above: {}",
            built_in.join(", ")
        )?;
    }
    Ok(out)
}

/// The check `id` resolved: its table once pieces merged, `each` fanned out and what is off
/// left out, or for a built in check what forms it. None when the config has no such check.
pub(crate) fn check(config: &Config, id: &str, lints: bool) -> Result<Option<String>> {
    let Some(check) = config.checks.iter().find(|c| c.id == id) else {
        return Ok(None);
    };
    let mut out = format!("# the check {id}, resolved\n");
    let Some(builtin) = &check.builtin else {
        let Some(table) = config.table().get("check").and_then(|c| c.get(id)).cloned() else {
            return Ok(None);
        };
        let mut whole = Table::new();
        whole.insert(
            "check".into(),
            Value::Table(Table::from_iter([(id.to_string(), table.clone())])),
        );
        out.push_str(&annotated(whole, config, lints)?);
        if table.get("inputs").is_none() {
            writeln!(out, "# inputs, by default: {:?}", check.inputs.patterns())?;
        }
        return Ok(Some(out));
    };
    let (what, part) = match builtin {
        Builtin::Settings(_) => (
            "keeps each tool's own file in line with [settings] and [lints]",
            None,
        ),
        Builtin::Rules(_) => ("checks the conventions of the [rule] tables", Some("rule")),
        Builtin::Commits(_) => ("checks commit messages as [commits] says", Some("commits")),
    };
    writeln!(out, "# built in: Fearlezz {what}")?;
    if let Some(part) = part.and_then(|p| Some((p, config.table().get(p)?.clone()))) {
        out.push_str(&annotated(
            Table::from_iter([(part.0.to_string(), part.1)]),
            config,
            lints,
        )?);
    }
    Ok(Some(out))
}

/// `table`, a part of the merged config from its root, as TOML with each value followed by
/// where it came from, and without `lints` each lint table as one line of counts.
fn annotated(mut table: Table, config: &Config, lints: bool) -> Result<String> {
    let lint_tables = if lints { None } else { table.remove("lints") };
    let mut doc: toml_edit::DocumentMut = toml::to_string(&table)?.parse()?;
    name_origins(doc.as_table_mut(), &mut Vec::new(), config);
    let mut out = doc.to_string();
    for (tool, rules) in lint_tables
        .as_ref()
        .and_then(Value::as_table)
        .into_iter()
        .flatten()
    {
        let Some(rules) = rules.as_table() else {
            continue;
        };
        let path = ["lints".to_string(), tool.clone()];
        writeln!(
            out,
            "\n[lints.{tool}] # {}",
            counted(rules, Some(&config.origins), &path)
        )?;
    }
    Ok(out)
}

/// One line of counts for a lint table: how many rules at each level, and with `origins`
/// how many each piece or file set.
fn counted(rules: &Table, origins: Option<&Origins>, path: &[String]) -> String {
    let mut levels: BTreeMap<usize, (&str, usize)> = BTreeMap::new();
    let mut from: Vec<(String, usize)> = Vec::new();
    for (rule, level) in rules {
        let level = level.as_str().unwrap_or("?");
        let order = ["gate", "flag", "hint", "off"]
            .iter()
            .position(|l| *l == level)
            .unwrap_or(usize::MAX);
        levels.entry(order).or_insert((level, 0)).1 += 1;
        let mut at = path.to_vec();
        at.push(rule.clone());
        if let Some(label) = origins.and_then(|o| o.get(&at)) {
            match from.iter_mut().find(|(l, _)| l == label) {
                Some((_, n)) => *n += 1,
                None => from.push((label.clone(), 1)),
            }
        }
    }
    let levels: Vec<String> = levels
        .values()
        .map(|(level, n)| format!("{n} {level}"))
        .collect();
    let from = match from.as_slice() {
        [] => String::new(),
        [(label, _)] => format!("; from {label}"),
        many => {
            let many: Vec<String> = many.iter().map(|(l, n)| format!("{l} {n}")).collect();
            format!("; from {}", many.join(", "))
        }
    };
    format!(
        "{} rules: {}{from}; --lints prints each",
        rules.len(),
        levels.join(", ")
    )
}

/// Follows each value in `table`, under the keys `path`, with a comment naming its origin.
fn name_origins(table: &mut toml_edit::Table, path: &mut Vec<String>, config: &Config) {
    for (key, item) in table.iter_mut() {
        path.push(key.get().to_string());
        match item {
            toml_edit::Item::Table(inner) => name_origins(inner, path, config),
            toml_edit::Item::ArrayOfTables(array) => {
                for inner in array.iter_mut() {
                    name_origins(inner, path, config);
                }
            }
            toml_edit::Item::Value(value) => {
                if let Some(label) = origin(config, path) {
                    value.decor_mut().set_suffix(format!(" # {label}"));
                }
            }
            toml_edit::Item::None => {}
        }
        path.pop();
    }
}

/// Where the value at `path` came from: every origin under it, for a value that is an
/// inline table or a list of tables. A check `each` made for a folder takes its values from
/// the check it was made from, unless the project tunes that folder's own table.
fn origin(config: &Config, path: &[String]) -> Option<String> {
    let under = |path: &[String]| -> Option<String> {
        let mut labels: Vec<&str> = config
            .origins
            .iter()
            .filter(|(at, _)| at.starts_with(path))
            .map(|(_, label)| label.as_str())
            .collect();
        labels.sort_unstable();
        labels.dedup();
        (!labels.is_empty()).then(|| labels.join(", "))
    };
    if let Some(found) = under(path) {
        return Some(found);
    }
    let [check, id, rest @ ..] = path else {
        return None;
    };
    let (base, folder) = id.split_once('/')?;
    if check != "check" || !config.fanned.get(base)?.iter().any(|f| f == folder) {
        return None;
    }
    if rest.first().is_some_and(|k| k == "dir") {
        return Some("each".into());
    }
    let mut from = vec![check.clone(), base.to_string()];
    from.extend_from_slice(rest);
    under(&from)
}
