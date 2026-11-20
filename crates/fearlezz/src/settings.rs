//! Shared settings: tool configuration carried by pieces, such as lint levels. Fearlezz writes
//! each tool's own file, so editors and plain tool runs agree with the checks, and the
//! `settings` check fails when a file drifts from what the pieces and the project's overrides
//! give. A file is in sync when writing it again would change nothing.
//!
//! `cargo-lints`, `clippy`, `claude`, `biome`, `ruff`, `tsconfig` and `ktlint` manage only
//! their part of a file and keep the rest; `dart-analysis`, `eslint`, `detekt` and
//! `golangci` own their whole file.
//!
//! `skill` and `hooks` are the settings a project has without asking for them: the agent skill
//! embedded in this binary, so every project's copy matches the Fearlezz it runs and needs no
//! network, and the agent hooks that tell an agent it skipped the proof.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result, anyhow, bail};
use fearlezz_proof::floor::Change;
use toml::{Table, Value};

use crate::findings::{Finding, Level};
use crate::hooks::Agent;
use crate::lints::{self, Lints};
use crate::names::{self, Named};

const MANAGED: &str = "Managed by Fearlezz from [settings] in fearlezz.toml; change it there.";

/// A tool Fearlezz writes settings for; its row in `TOOLS` says how.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Tool {
    CargoLints,
    DartAnalysis,
    Clippy,
    Claude,
    Biome,
    Eslint,
    Ruff,
    Ktlint,
    Detekt,
    Golangci,
    Oxlint,
    Tsconfig,
    Yamllint,
}

/// One tool: its name in `[settings.<name>]`, the files it reads its settings from in its own
/// lookup order and the one Fearlezz creates when none exists, the lint tables written into
/// it and how, and how the settings land in the file.
struct Row {
    tool: Tool,
    name: &'static str,
    files: &'static [&'static str],
    default: &'static str,
    lints: &'static [lints::Tool],
    table: fn(&Lints) -> Table,
    write: Write,
}

/// How a tool's settings land in its file.
#[derive(Clone, Copy)]
enum Write {
    /// Into each Cargo project's Cargo.toml, its workspace members inheriting.
    Cargo,
    /// Field by field into a TOML file (under `[tool.<name>]` in pyproject.toml); the rest stays.
    Toml,
    /// Merged into a JSON file; the other keys stay. With `comments` the file may hold
    /// comments; each of `owned`, a path of keys, is written whole from the settings.
    Json {
        comments: bool,
        owned: &'static [&'static [&'static str]],
    },
    /// The whole file, as its extension says: YAML unless it is TOML or JSON.
    Whole,
    /// A config the project's `eslint.config` spreads in.
    Eslint,
    /// A block of `ktlint_standard_*` switches for Kotlin files in `.editorconfig`.
    Editorconfig,
}

const fn row(
    tool: Tool,
    name: &'static str,
    (files, default): (&'static [&'static str], &'static str),
    lints: &'static [lints::Tool],
    table: fn(&Lints) -> Table,
    write: Write,
) -> Row {
    Row {
        tool,
        name,
        files,
        default,
        lints,
        table,
        write,
    }
}

const fn json(comments: bool, owned: &'static [&'static [&'static str]]) -> Write {
    Write::Json { comments, owned }
}

/// Every tool, in `Tool`'s order. Cargo's lints go into `[workspace.lints]` (or `[lints]`),
/// which members inherit with `lints.workspace = true`; `clippy` is clippy.toml, clippy's own
/// options; `claude` holds such as the commands an agent may run without asking. Fearlezz
/// decides every Biome rule ("preset": "none"), so a rule a piece dropped leaves biome.json
/// instead of keeping the level Biome gives it, and every oxlint rule and the plugins they need.
#[rustfmt::skip]
const TOOLS: [Row; 13] = {
    use lints::Tool as L;
    [
        row(Tool::CargoLints, "cargo-lints", (&["Cargo.toml"], "Cargo.toml"), &[L::Clippy, L::Rustc], cargo_lint_table, Write::Cargo),
        row(Tool::DartAnalysis, "dart-analysis", (&["analysis_options.yaml"], "analysis_options.yaml"), &[L::Dart], dart_lint_table, Write::Whole),
        row(Tool::Clippy, "clippy", (&[".clippy.toml", "clippy.toml"], "clippy.toml"), &[], no_lints, Write::Toml),
        row(Tool::Claude, "claude", (&[".claude/settings.json"], ".claude/settings.json"), &[], no_lints, json(false, &[])),
        row(Tool::Biome, "biome", (&["biome.json", "biome.jsonc", ".biome.json", ".biome.jsonc"], "biome.json"), &[L::Biome], biome_lint_table, json(true, &[&["linter", "rules"]])),
        row(Tool::Eslint, "eslint", (&[ESLINT_FILE], ESLINT_FILE), &[L::Eslint], eslint_lint_table, Write::Eslint),
        row(Tool::Ruff, "ruff", (&[".ruff.toml", "ruff.toml", "pyproject.toml"], "ruff.toml"), &[L::Ruff], ruff_lint_table, Write::Toml),
        row(Tool::Ktlint, "ktlint", (&[".editorconfig"], ".editorconfig"), &[L::Ktlint], ktlint_lint_table, Write::Editorconfig),
        row(Tool::Detekt, "detekt", (&[DETEKT_FILE], DETEKT_FILE), &[L::Detekt], detekt_lint_table, Write::Whole),
        row(Tool::Golangci, "golangci", (&[".golangci.yml", ".golangci.yaml", ".golangci.toml", ".golangci.json"], ".golangci.yml"), &[L::Golangci], golangci_lint_table, Write::Whole),
        row(Tool::Oxlint, "oxlint", (&[".oxlintrc.json"], ".oxlintrc.json"), &[L::Oxlint], oxlint_lint_table, json(true, &[&["rules"], &["plugins"], &["categories"]])),
        row(Tool::Tsconfig, "tsconfig", (&["tsconfig.json"], "tsconfig.json"), &[], no_lints, json(true, &[])),
        row(Tool::Yamllint, "yamllint", (&[".yamllint", ".yamllint.yaml", ".yamllint.yml"], ".yamllint.yaml"), &[], no_lints, Write::Whole),
    ]
};

fn no_lints(_: &Lints) -> Table {
    Table::new()
}

/// Files this binary ships, written whole: settings a project has without asking for them,
/// not a tool's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Shipped {
    /// The Fearlezz agent skill, in each agent's skills folder.
    Skill,
    /// The helpers of the `flutter-look` piece's widget tests in `look_test/`: real fonts,
    /// and `still`, which writes a still into `$FEARLEZZ_OUT`.
    FlutterLook,
    /// The proof hooks of Codex (`.codex/hooks.json`) and Cursor (`.cursor/hooks.json`);
    /// Claude Code's go into `.claude/settings.json` with the `claude` settings.
    Hooks,
}

impl Named for Shipped {
    const NAMES: &'static [(&'static str, Shipped)] = &[
        ("flutter-look", Shipped::FlutterLook),
        ("skill", Shipped::Skill),
        ("hooks", Shipped::Hooks),
    ];
}

impl Shipped {
    /// Every file it may write; the table picks which.
    fn files(self) -> &'static [&'static str] {
        match self {
            Shipped::Skill => &[CLAUDE_SKILL, AGENTS_SKILL],
            Shipped::FlutterLook => &[FLUTTER_LOOK[0].0, FLUTTER_LOOK[1].0],
            Shipped::Hooks => &[CODEX_HOOKS, CURSOR_HOOKS],
        }
    }

    /// The files as they should be.
    fn render(self, root: &Path, table: &Table) -> Result<Vec<(String, String)>> {
        Ok(match self {
            Shipped::FlutterLook => {
                if let Some(key) = table.keys().next() {
                    bail!("[settings.flutter-look]: no field \"{key}\"; it has none");
                }
                FLUTTER_LOOK
                    .iter()
                    .map(|(path, text)| ((*path).to_string(), dart_file(text)))
                    .collect()
            }
            Shipped::Skill => skill_files(table)?
                .into_iter()
                .map(|path| (path.to_string(), skill_text()))
                .collect(),
            Shipped::Hooks => {
                let agents = hook_agents(table)?.unwrap_or_else(|| agents_in_use(root));
                agents
                    .into_iter()
                    .filter_map(|agent| hooks_file(agent).map(|file| (agent, file)))
                    .map(|(agent, file)| {
                        let hooks: Table = serde_json::from_value(agent.settings())?;
                        json_file(root, file, &hooks, false, &[])
                    })
                    .collect::<Result<_>>()?
            }
        })
    }
}

/// The settings of every tool, after merging pieces and overrides, and the lint tables
/// whose rules they turn on or off.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Settings {
    tools: BTreeMap<Tool, Table>,
    shipped: BTreeMap<Shipped, Table>,
    lints: Lints,
    /// The tools a check reads the findings of, whether or not they have settings.
    read: BTreeSet<Tool>,
    /// The folders of the repository's Cargo projects (`.` for the root), which the Rust lint
    /// tables are written into.
    pub cargo: Vec<String>,
    /// A check runs Biome, through a piece or a command of the project's own.
    pub biome_runs: bool,
    /// What the config proves already, against which the check names what the project could
    /// prove but does not yet.
    pub grow: crate::grow::Have,
    /// The proof host's URL, which a README showing the proof's pictures names.
    pub host: String,
}

impl Settings {
    /// `read` are the lint tools a check reads the findings of: only their tables are written
    /// into the tool's file, so a framework piece's rules for one linter leave no file behind
    /// in a project that runs another.
    pub(crate) fn parse(
        table: Table,
        lints: Lints,
        read: &BTreeSet<lints::Tool>,
    ) -> Result<Settings> {
        let (mut tools, mut shipped) = (BTreeMap::new(), BTreeMap::new());
        for (name, value) in table {
            let tool = TOOLS
                .iter()
                .find(|row| row.name == name)
                .map(|row| row.tool);
            let file = Shipped::named(&name).ok();
            if tool.is_none() && file.is_none() {
                let names: Vec<&str> = TOOLS.iter().map(|row| row.name).collect();
                let files: Vec<&str> = Shipped::NAMES.iter().map(|(n, _)| *n).collect();
                bail!(
                    "[settings.{name}]: no such tool; tools are {}, {}",
                    names.join(", "),
                    files.join(", ")
                );
            }
            let Value::Table(t) = value else {
                bail!("[settings.{name}] must be a table");
            };
            if let Some(tool) = tool {
                tools.insert(tool, t);
            } else if let Some(file) = file {
                shipped.insert(file, t);
            }
        }
        // Every project gets the skill unless it lists no agent.
        if skill_files(shipped.entry(Shipped::Skill).or_default())?.is_empty() {
            shipped.remove(&Shipped::Skill);
        }
        // And the hooks, for the agents it names or else the ones it uses.
        let hooks = hook_agents(shipped.entry(Shipped::Hooks).or_default())?;
        if hooks.as_ref().is_some_and(Vec::is_empty) {
            shipped.remove(&Shipped::Hooks);
        }
        if hooks.is_none_or(|agents| agents.contains(&Agent::Claude)) {
            tools.entry(Tool::Claude).or_default();
        }
        // A lint table a check reads is written into its tool's file.
        for row in &TOOLS {
            if row
                .lints
                .iter()
                .any(|lint| lints.of(*lint).is_some() && read.contains(lint))
            {
                tools.entry(row.tool).or_default();
            }
        }
        let read = TOOLS
            .iter()
            .filter(|row| row.lints.iter().any(|lint| read.contains(lint)))
            .map(|row| row.tool)
            .collect();
        Ok(Settings {
            tools,
            shipped,
            lints,
            read,
            // Config::load finds them in the repository, the checks that run Biome, what the
            // checks prove and the proof host.
            cargo: Vec::new(),
            biome_runs: false,
            grow: crate::grow::Have::default(),
            host: String::new(),
        })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.tools.is_empty() && self.shipped.is_empty()
    }

    /// A tool's settings with its lint tables written in; the settings win.
    fn table(&self, tool: Tool) -> Table {
        let mut out = if tool == Tool::Claude {
            self.claude_hooks()
        } else {
            (tool.row().table)(&self.lints)
        };
        if let Some(own) = self.tools.get(&tool) {
            crate::presets::merge(&mut out, own.clone());
        }
        out
    }

    /// Claude Code's proof hooks, when the project gives Claude Code hooks.
    fn claude_hooks(&self) -> Table {
        let wanted = self.shipped.get(&Shipped::Hooks).is_some_and(|table| {
            hook_agents(table)
                .ok()
                .flatten()
                .is_none_or(|agents| agents.contains(&Agent::Claude))
        });
        if wanted {
            serde_json::from_value(Agent::Claude.settings()).unwrap_or_default()
        } else {
            Table::new()
        }
    }

    /// The files these settings may touch, as input globs.
    pub(crate) fn globs(&self) -> Vec<String> {
        let mut globs = self.files();
        if self.tools.contains_key(&Tool::CargoLints) {
            globs.push("**/Cargo.toml".into());
        }
        // The files that can make an opportunity timely, so a change to one runs it again.
        globs.extend(self.grow.globs(&self.growing()));
        globs
    }

    /// Every file a tool may read its settings from, by exact path. They are hashed from
    /// disk, so a managed file git ignores still counts.
    pub(crate) fn files(&self) -> Vec<String> {
        let tools = self.tools.keys().flat_map(|t| t.row().files);
        let shipped = self.shipped.keys().flat_map(|s| s.files());
        tools.chain(shipped).map(|p| (*p).to_string()).collect()
    }

    /// How `head` tunes the tools that judge code differently: any setting added, removed or
    /// changed, since Fearlezz cannot tell which way a tool's option loosens it. Each as
    /// `settings.<tool>.<key>` and how. Only a tool the base already has settings for or a
    /// check reading the findings of counts: a tool the base neither has settings for nor
    /// reads the findings of is new to the floor, whose files it never held, so its settings
    /// are not held either. Agent settings, the skill and hooks are left out.
    pub(crate) fn weakenings(&self, head: &Settings) -> Vec<(String, Change)> {
        let empty = Table::new();
        let mut found = Vec::new();
        let tools: BTreeSet<Tool> = self
            .tools
            .keys()
            .chain(&self.read)
            .copied()
            .filter(|t| *t != Tool::Claude)
            .collect();
        for tool in tools {
            let base = leaves(self.tools.get(&tool).unwrap_or(&empty));
            let now = leaves(head.tools.get(&tool).unwrap_or(&empty));
            let keys: BTreeSet<&String> = base.keys().chain(now.keys()).collect();
            for key in keys {
                if let Some(how) = Change::set(base.get(key).cloned(), now.get(key).cloned()) {
                    found.push((format!("settings.{}.{key}", tool.row().name), how));
                }
            }
        }
        found
    }

    /// A stable text of the settings, part of the settings check's inputs. It holds the skill
    /// text, which can change between builds of one version.
    pub(crate) fn fingerprint(&self) -> String {
        format!(
            "{:?}{:?}{:?}{:?}{}{SKILL}",
            self.tools, self.shipped, self.lints, self.grow, self.host
        )
    }

    /// Whether an agent would miss the skill, or read another version's. Only the skill files
    /// are read, so this is cheap enough for every command.
    pub(crate) fn skill_out_of_date(&self, root: &Path) -> Result<bool> {
        let Some(table) = self.shipped.get(&Shipped::Skill) else {
            return Ok(false);
        };
        let text = skill_text();
        for path in skill_files(table)? {
            if read(root, path)?.as_deref() != Some(text.as_str()) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Every managed file as it should be, given the files as they are now. Only files whose
    /// content would change are returned.
    pub(crate) fn render(&self, root: &Path) -> Result<Vec<(String, String)>> {
        let mut out = Vec::new();
        for tool in self.tools.keys() {
            // Nothing to write the Rust lints into; the settings check says so.
            if *tool == Tool::CargoLints && self.cargo_lints_without_manifest() {
                continue;
            }
            out.extend(self.write(root, *tool)?);
        }
        for (file, table) in &self.shipped {
            out.extend(file.render(root, table)?);
        }
        let mut drifted = Vec::new();
        for (path, content) in out {
            if read(root, &path)?.as_deref() != Some(content.as_str()) {
                drifted.push((path, content));
            }
        }
        Ok(drifted)
    }

    /// The files of one tool as they should be.
    fn write(&self, root: &Path, tool: Tool) -> Result<Vec<(String, String)>> {
        let row = tool.row();
        let table = self.table(tool);
        let path = tool.resolve(root)?;
        Ok(match row.write {
            Write::Cargo => {
                let mut files = Vec::new();
                for dir in &self.cargo {
                    files.extend(cargo_lints(root, dir, &table)?);
                }
                files
            }
            Write::Toml if path == "pyproject.toml" => {
                let own = Table::from_iter([(row.name.to_string(), Value::Table(table))]);
                let tool = Table::from_iter([("tool".to_string(), Value::Table(own))]);
                vec![toml_file(root, &path, &tool)?]
            }
            Write::Toml => vec![toml_file(root, &path, &table)?],
            Write::Json { comments, owned } => {
                // A project's own Biome rules stay until a piece gives Biome its rules.
                let given = table.get("linter").and_then(|l| l.get("rules")).is_some();
                let owned = if tool == Tool::Biome && !given {
                    &[]
                } else {
                    owned
                };
                vec![json_file(root, &path, &table, comments, owned)?]
            }
            Write::Whole => {
                let ext = Path::new(&path)
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                let body = match ext.as_str() {
                    "toml" => format!("# {MANAGED}\n{}", toml::to_string(&table)?),
                    "json" => {
                        let mut body = String::new();
                        let layout = Layout {
                            indent: "  ",
                            width: line_width(root)?,
                        };
                        pretty_json(&serde_json::to_value(&table)?, &layout, 0, 0, &mut body);
                        body + "\n"
                    }
                    _ => format!("# {MANAGED}\n{}", yaml(&table, 0)?),
                };
                vec![(path, body)]
            }
            Write::Eslint => vec![(path, eslint_file(&table)?)],
            Write::Editorconfig => vec![editorconfig(root, &path, &table)?],
        })
    }

    /// Writes every file that drifted; returns their paths. Each file is written whole to a
    /// temporary file first, so a failed write leaves the old one as it was.
    pub(crate) fn sync(&self, root: &Path) -> Result<Vec<String>> {
        let mut written = Vec::new();
        for (path, content) in self.render(root)? {
            // Write through a link, so a shared file stays shared.
            let full = std::fs::canonicalize(root.join(&path)).unwrap_or_else(|_| root.join(&path));
            crate::files::write_atomic(&full, content)?;
            written.push(path);
        }
        Ok(written)
    }

    /// A gate finding for each file that drifted.
    pub(crate) fn drift(&self, root: &Path) -> Result<Vec<Finding>> {
        let finding = |file: String, message: String, fix: &str| Finding {
            file,
            line: 1,
            end_line: None,
            level: Level::Gate,
            rule: "settings".into(),
            message,
            fix: Some(fix.into()),
            why: Some("shared settings keep every project's tools alike".into()),
            anywhere: true,
        };
        let mut found: Vec<Finding> = self
            .render(root)?
            .into_iter()
            .map(|(path, _)| {
                let from = if SKILL_FILES.iter().any(|(_, file)| *file == path) {
                    "the skill this Fearlezz version ships"
                } else {
                    "the settings in fearlezz.toml"
                };
                // A finding names a file that exists, so a missing one is named in the message.
                if root.join(&path).exists() {
                    finding(path, format!("differs from {from}"), "fearlezz setup sync")
                } else {
                    let message = format!("{path} is missing; {from} write it");
                    finding(String::new(), message, "fearlezz setup sync")
                }
            })
            .collect();
        if self.cargo_lints_without_manifest() {
            found.push(finding(
                String::new(),
                "no Cargo.toml to write the [lints.clippy] and [lints.rustc] rules into".into(),
                "add the Cargo.toml, or drop those tables (and the rust piece) from this project",
            ));
        }
        // Biome and oxlint run a framework's rules only when its piece gives them, so a project
        // set up before the framework pieces existed would lose them without a word.
        // Only a linter a check runs counts (its file is written then).
        let mut lacking: Vec<&str> = Vec::new();
        for lint in [lints::Tool::Biome, lints::Tool::Oxlint] {
            if !self.tools.keys().any(|t| t.row().lints.contains(&lint)) {
                continue;
            }
            let given = self.lints.of(lint);
            for piece in crate::init::frameworks(root) {
                let rules = crate::presets::lint_rules(piece, lint.name());
                if !rules.is_empty()
                    && !rules
                        .iter()
                        .any(|rule| given.is_some_and(|g| g.contains_key(rule)))
                    && !lacking.contains(&piece)
                {
                    lacking.push(piece);
                }
            }
        }
        for piece in lacking {
            found.push(finding(
                "package.json".into(),
                format!(
                    "package.json depends on {piece}, but no rule of the {piece} piece has a level"
                ),
                &format!("add \"{piece}\" to use in fearlezz.toml"),
            ));
        }
        found.extend(self.idle_biome(root)?);
        Ok(found)
    }

    /// A hint for each kind of proof the project could add that the change, touching
    /// `changed` among the repository's `files`, makes timely.
    pub(crate) fn grow(&self, root: &Path, files: &[String], changed: &[&str]) -> Vec<Finding> {
        crate::grow::hints(&self.opportunities(root, files), changed, &self.lints)
    }

    /// What the project in `root`, with `files`, could prove but does not yet, leaving out
    /// the kinds `[lints.fearlezz]` turns off.
    pub(crate) fn opportunities(
        &self,
        root: &Path,
        files: &[String],
    ) -> Vec<crate::grow::Opportunity> {
        let kinds = self.growing();
        crate::grow::find(root, files, &self.grow, &self.host)
            .into_iter()
            .filter(|o| kinds.contains(&o.kind))
            .collect()
    }

    /// The kinds of proof the project could add that the config does not prove yet and
    /// `[lints.fearlezz]` leaves on.
    fn growing(&self) -> Vec<crate::grow::Kind> {
        let levels = self.lints.of(lints::Tool::Fearlezz);
        self.grow
            .could()
            .into_iter()
            .filter(|k| {
                levels
                    .and_then(|l| l.get(k.rule()))
                    .is_none_or(|l| l.is_on())
            })
            .collect()
    }

    /// A biome.json that no piece configures and no check runs: Biome left the typescript
    /// piece, so a project set up before keeps the file and would stop linting with Biome
    /// without a word. One Fearlezz wrote (it decides every rule, `"preset": "none"`) is a
    /// gate; one the project wrote itself is a hint, since an editor may read it.
    pub(crate) fn idle_biome(&self, root: &Path) -> Result<Option<Finding>> {
        if self.tools.contains_key(&Tool::Biome) || self.biome_runs {
            return Ok(None);
        }
        let biome = Tool::Biome.resolve(root)?;
        let Some(text) = read(root, &biome)? else {
            return Ok(None);
        };
        let written = serde_json::from_str::<serde_json::Value>(&strip_jsonc(&text))
            .is_ok_and(|doc| doc.pointer("/linter/rules/preset") == Some(&"none".into()));
        Ok(Some(Finding {
            file: biome.clone(),
            line: 1,
            end_line: None,
            level: if written { Level::Gate } else { Level::Hint },
            rule: "settings".into(),
            message: format!(
                "{biome} is here, but no piece gives Biome its rules and no check runs Biome"
            ),
            fix: Some(
                "add \"biome\" to use in fearlezz.toml (Biome's checks and rules come with the \
                 biome piece since 0.5), or remove the file when another tool lints"
                    .into(),
            ),
            why: Some("shared settings keep every project's tools alike".into()),
            anywhere: true,
        }))
    }

    /// Rust lint settings in a project without a `Cargo.toml` to hold them.
    fn cargo_lints_without_manifest(&self) -> bool {
        self.tools.contains_key(&Tool::CargoLints) && self.cargo.is_empty()
    }
}

/// Every value of a settings table by its dotted key, tables walked into and the rest as
/// written.
fn leaves(table: &Table) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (key, value) in table {
        if let Value::Table(t) = value {
            for (k, v) in leaves(t) {
                out.insert(format!("{key}.{k}"), v);
            }
        } else {
            out.insert(key.clone(), value.to_string());
        }
    }
    out
}

impl Tool {
    #[expect(
        clippy::indexing_slicing,
        reason = "TOOLS holds every tool at its place; each_row_sits_at_its_tool keeps it so"
    )]
    fn row(self) -> &'static Row {
        &TOOLS[self as usize]
    }

    /// The file the tool would read now, so the settings land where the project keeps them
    /// instead of in a new file that shadows them.
    fn resolve(self, root: &Path) -> Result<String> {
        let row = self.row();
        for path in row.files {
            let Some(text) = read(root, path)? else {
                continue;
            };
            // Ruff reads pyproject.toml only when it has a [tool.ruff] table.
            let own_section = || {
                text.parse::<Table>()
                    .ok()
                    .and_then(|t| t.get("tool")?.get(row.name).cloned())
                    .is_some()
            };
            if *path != "pyproject.toml" || own_section() {
                return Ok((*path).to_string());
            }
        }
        Ok(row.default.to_string())
    }
}

/// A file's text, or None when there is none. Any other read error is an error, so a file
/// that cannot be read is never written over as if it were empty.
fn read(root: &Path, path: &str) -> Result<Option<String>> {
    crate::files::read_if_exists(&root.join(path)).with_context(|| format!("reading {path}"))
}

/// Clippy and rustc rules as Cargo.toml lints. A rule that is on warns: Fearlezz gives the
/// finding its level, and `deny` would break plain builds on old code and stop clippy from
/// checking crates that depend on a failing one.
fn cargo_lint_table(lints: &Lints) -> Table {
    let mut out = Table::new();
    for (tool, group) in [
        (lints::Tool::Clippy, "clippy"),
        (lints::Tool::Rustc, "rust"),
    ] {
        let Some(rules) = lints.of(tool) else {
            continue;
        };
        let table: Table = rules
            .iter()
            .map(|(rule, level)| {
                let cargo = if level.is_on() { "warn" } else { "allow" };
                (rule.clone(), Value::String(cargo.into()))
            })
            .collect();
        out.insert(group.into(), Value::Table(table));
    }
    out
}

/// The analyzer's language switches, which are lint table rows but live under
/// `analyzer.language`.
const DART_LANGUAGE: [&str; 3] = ["strict-casts", "strict-inference", "strict-raw-types"];

/// Dart rules as analysis_options: each rule on or off, and the severity the editor shows.
fn dart_lint_table(lints: &Lints) -> Table {
    let mut out = Table::new();
    let Some(rules) = lints.of(lints::Tool::Dart) else {
        return out;
    };
    let (mut on, mut errors, mut language) = (Table::new(), Table::new(), Table::new());
    for (rule, level) in rules {
        if DART_LANGUAGE.contains(&rule.as_str()) {
            language.insert(rule.clone(), Value::Boolean(level.is_on()));
            continue;
        }
        on.insert(rule.clone(), Value::Boolean(level.is_on()));
        if let Some(severity) = severity(*level, "warning", "info") {
            errors.insert(rule.clone(), Value::String(severity.into()));
        }
    }
    let mut analyzer = Table::new();
    if !errors.is_empty() {
        analyzer.insert("errors".into(), Value::Table(errors));
    }
    if !language.is_empty() {
        analyzer.insert("language".into(), Value::Table(language));
    }
    if !analyzer.is_empty() {
        out.insert("analyzer".into(), Value::Table(analyzer));
    }
    let mut linter = Table::new();
    linter.insert("rules".into(), Value::Table(on));
    out.insert("linter".into(), Value::Table(linter));
    out
}

/// Cargo.toml with the lints table set, and each workspace member inheriting it.
fn cargo_lints(repo: &Path, dir: &str, lints: &Table) -> Result<Vec<(String, String)>> {
    // Paths below are from the project's folder; the files returned are from the repository.
    let root = &repo.join(dir);
    let from_repo = |path: String| {
        if dir == "." {
            path
        } else {
            format!("{dir}/{path}")
        }
    };
    let text = std::fs::read_to_string(root.join("Cargo.toml")).with_context(|| {
        format!(
            "[settings.cargo-lints]: reading {}",
            from_repo("Cargo.toml".into())
        )
    })?;
    let mut doc: toml_edit::DocumentMut = text.parse().context("Cargo.toml")?;
    let rendered: toml_edit::DocumentMut = toml::to_string(lints)?.parse()?;
    let mut table = rendered.as_table().clone();
    table.set_implicit(true);
    // A lint with a priority reads `pedantic = { level = "warn", priority = -1 }`, as Cargo's
    // own documentation writes it.
    for (_, group) in table.iter_mut() {
        let Some(old) = group.as_table() else {
            continue;
        };
        let mut lints: Vec<(String, toml_edit::Item)> = old
            .iter()
            .map(|(name, lint)| {
                let lint = match lint.as_table() {
                    Some(t) => toml_edit::Item::Value(toml_edit::Value::InlineTable(
                        t.clone().into_inline_table(),
                    )),
                    None => lint.clone(),
                };
                (name.to_string(), lint)
            })
            .collect();
        lints.sort_by(|a, b| a.0.cmp(&b.0));
        let mut fresh = toml_edit::Table::new();
        for (name, lint) in lints {
            fresh.insert(&name, lint);
        }
        *group = toml_edit::Item::Table(fresh);
    }
    if let Some((_, first)) = table.iter_mut().next()
        && let Some(t) = first.as_table_mut()
    {
        t.decor_mut().set_prefix(format!("\n# {MANAGED}\n"));
    }
    let workspace = doc.get("workspace").is_some_and(toml_edit::Item::is_table);
    let inherit = || {
        let mut inherit = toml_edit::Table::new();
        inherit["workspace"] = toml_edit::value(true);
        inherit.decor_mut().set_prefix("\n");
        toml_edit::Item::Table(inherit)
    };
    let mut out = Vec::new();
    if workspace {
        doc["workspace"]["lints"] = toml_edit::Item::Table(table);
        let list = |key: &str| -> Vec<String> {
            doc["workspace"]
                .get(key)
                .and_then(|m| m.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default()
        };
        let (members, exclude) = (list("members"), list("exclude"));
        // A root package is a member too.
        if doc.get("package").is_some_and(toml_edit::Item::is_table) {
            doc["lints"] = inherit();
        }
        for dir in member_dirs(root, &members, &exclude)? {
            let path = format!("{dir}/Cargo.toml");
            let text = read(root, &path)?.unwrap_or_default();
            let mut member: toml_edit::DocumentMut = text.parse().with_context(|| path.clone())?;
            member["lints"] = inherit();
            out.push((path, member.to_string()));
        }
    } else {
        doc["lints"] = toml_edit::Item::Table(table);
    }
    out.insert(0, ("Cargo.toml".to_string(), doc.to_string()));
    Ok(out
        .into_iter()
        .map(|(path, text)| (from_repo(path), text))
        .collect())
}

/// Whether the Cargo.toml in folder `workspace` (`.` the root) is a workspace that names the
/// project in folder `project` (below it) a member.
pub(crate) fn workspace_has(root: &Path, workspace: &str, project: &str) -> bool {
    let at = if workspace == "." {
        root.to_path_buf()
    } else {
        root.join(workspace)
    };
    let Ok(text) = std::fs::read_to_string(at.join("Cargo.toml")) else {
        return false;
    };
    let Ok(doc) = text.parse::<Table>() else {
        return false;
    };
    let Some(ws) = doc.get("workspace").and_then(Value::as_table) else {
        return false;
    };
    let list = |key: &str| -> Vec<String> {
        ws.get(key)
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };
    let below = format!("{workspace}/");
    let inside = if workspace == "." {
        project
    } else {
        project.strip_prefix(&below).unwrap_or(project)
    };
    member_dirs(&at, &list("members"), &list("exclude"))
        .is_ok_and(|dirs| dirs.iter().any(|d| d == inside))
}

/// The folders of a workspace's members below the root, as Cargo finds them: each pattern
/// expanded segment by segment (with `*`, `?`, `[..]` and `**`), only folders with a
/// Cargo.toml, and nothing inside an `exclude` path.
fn member_dirs(root: &Path, members: &[String], exclude: &[String]) -> Result<Vec<String>> {
    let exclude: Vec<String> = exclude.iter().map(|e| clean_path(e)).collect();
    let excluded = |dir: &str| {
        exclude
            .iter()
            .any(|e| dir == e || dir.starts_with(&format!("{e}/")))
    };
    let mut dirs = BTreeSet::new();
    for pattern in members {
        for dir in expand(root, &clean_path(pattern))? {
            if !dir.is_empty() && !excluded(&dir) && root.join(&dir).join("Cargo.toml").is_file() {
                dirs.insert(dir);
            }
        }
    }
    Ok(dirs.into_iter().collect())
}

/// A relative path without `.` segments or a trailing slash.
fn clean_path(path: &str) -> String {
    path.split('/')
        .filter(|s| !s.is_empty() && *s != ".")
        .collect::<Vec<_>>()
        .join("/")
}

/// The folders below `root` a glob pattern names.
fn expand(root: &Path, pattern: &str) -> Result<Vec<String>> {
    let join = |dir: &str, name: &str| {
        if dir.is_empty() {
            name.to_string()
        } else {
            format!("{dir}/{name}")
        }
    };
    let subdirs = |dir: &str| -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect()
    };
    let mut found = vec![String::new()];
    for segment in pattern.split('/').filter(|s| !s.is_empty()) {
        let mut next = Vec::new();
        if segment == "**" {
            // This folder and every folder below it, except hidden ones.
            let mut stack = found;
            while let Some(dir) = stack.pop() {
                for name in subdirs(&dir) {
                    if !name.starts_with('.') {
                        stack.push(join(&dir, &name));
                    }
                }
                next.push(dir);
            }
        } else if segment.contains(['*', '?', '[']) {
            let glob = globset::Glob::new(segment)
                .with_context(|| format!("workspace member pattern {pattern}"))?
                .compile_matcher();
            for dir in &found {
                for name in subdirs(dir) {
                    if glob.is_match(&name) {
                        next.push(join(dir, &name));
                    }
                }
            }
        } else {
            next = found.iter().map(|dir| join(dir, segment)).collect();
        }
        found = next;
    }
    Ok(found)
}

/// A TOML file with the settings set field by field; everything else in it stays.
fn toml_file(root: &Path, path: &str, table: &Table) -> Result<(String, String)> {
    let text = read(root, path)?.unwrap_or_default();
    let mut doc: toml_edit::DocumentMut = text.parse().with_context(|| path.to_string())?;
    let rendered: toml_edit::DocumentMut = toml::to_string(table)?.parse()?;
    merge_toml(doc.as_table_mut(), rendered.as_table());
    Ok((path.to_string(), doc.to_string()))
}

fn merge_toml(into: &mut toml_edit::Table, from: &toml_edit::Table) {
    for (key, item) in from {
        match (
            into.get_mut(key).and_then(|i| i.as_table_mut()),
            item.as_table(),
        ) {
            (Some(a), Some(b)) => merge_toml(a, b),
            _ => {
                into.insert(key, item.clone());
            }
        }
    }
}

/// A JSON file with the settings merged in; other keys stay. With `comments`, the file may
/// hold comments and trailing commas (tsconfig.json does); they do not survive the rewrite.
/// Each of `owned`, a path of keys, is written whole from the settings, so a key there that
/// they no longer name goes.
fn json_file(
    root: &Path,
    path: &str,
    table: &Table,
    comments: bool,
    owned: &[&[&str]],
) -> Result<(String, String)> {
    let text = read(root, path)?;
    let mut doc: serde_json::Value = match &text {
        Some(text) if comments => {
            serde_json::from_str(&strip_jsonc(text)).with_context(|| path.to_string())?
        }
        Some(text) => serde_json::from_str(text).with_context(|| path.to_string())?,
        None => serde_json::json!({}),
    };
    for keys in owned {
        if let Some((last, parents)) = keys.split_last() {
            let parent = parents.iter().try_fold(&mut doc, |d, k| d.get_mut(*k));
            // Emptied in place, so the file keeps its key order and the merge fills it.
            if let Some(owned) = parent.and_then(|d| d.get_mut(*last)) {
                *owned = serde_json::json!({});
            }
        }
    }
    merge_json(&mut doc, serde_json::to_value(table)?);
    // Keep the file's indentation, so the project's formatter agrees with the rewrite.
    let indent = text.as_deref().map_or("  ", json_indent);
    let mut out = String::new();
    let layout = Layout {
        indent,
        width: line_width(root)?,
    };
    pretty_json(&doc, &layout, 0, 0, &mut out);
    out.push('\n');
    Ok((path.to_string(), out))
}

/// How the project's formatter lays out JSON.
struct Layout<'a> {
    indent: &'a str,
    /// The longest line, in columns, a tab counting as two.
    width: usize,
}

/// The line width the project's JSON formatter uses: Biome's `json.formatter.lineWidth` or
/// `formatter.lineWidth`, else Prettier's `printWidth`, else their shared default of 80.
fn line_width(root: &Path) -> Result<usize> {
    let biome = Tool::Biome.resolve(root)?;
    let prettier = [".prettierrc", ".prettierrc.json"];
    for path in std::iter::once(biome.as_str()).chain(prettier) {
        let Some(text) = read(root, path)? else {
            continue;
        };
        // A broken or YAML file is the formatter's to report; the default stands.
        let Ok(doc) = serde_json::from_str::<serde_json::Value>(&strip_jsonc(&text)) else {
            continue;
        };
        let width = [
            &doc["json"]["formatter"]["lineWidth"],
            &doc["formatter"]["lineWidth"],
            &doc["printWidth"],
        ]
        .into_iter()
        .find_map(serde_json::Value::as_u64);
        if let Some(width) = width.and_then(|w| usize::try_from(w).ok()) {
            return Ok(width);
        }
    }
    Ok(80)
}

/// JSON laid out the way Biome and Prettier do: objects one key per line, and an array of
/// plain values on one line when the whole line fits in the width (checked against Biome
/// 2.6.5). `tail` is how many characters follow the value on its line (1 for a comma).
fn pretty_json(
    value: &serde_json::Value,
    layout: &Layout<'_>,
    depth: usize,
    tail: usize,
    out: &mut String,
) {
    use serde_json::Value as J;
    let indent = layout.indent;
    let pad = |d: usize| indent.repeat(d);
    match value {
        J::Object(map) if !map.is_empty() => {
            out.push_str("{\n");
            for (i, (key, v)) in map.iter().enumerate() {
                out.push_str(&pad(depth + 1));
                out.push_str(&J::String(key.clone()).to_string());
                out.push_str(": ");
                let comma = usize::from(i + 1 < map.len());
                pretty_json(v, layout, depth + 1, comma, out);
                out.push_str(if comma == 1 { ",\n" } else { "\n" });
            }
            out.push_str(&pad(depth));
            out.push('}');
        }
        J::Array(items) if !items.is_empty() => {
            let line = out.rsplit('\n').next().unwrap_or_default();
            let used = line.replace('\t', "  ").chars().count();
            let flat = format!(
                "[{}]",
                items
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            let plain = items.iter().all(|v| !v.is_object() && !v.is_array());
            if plain && used + flat.chars().count() + tail <= layout.width {
                out.push_str(&flat);
                return;
            }
            out.push_str("[\n");
            for (i, v) in items.iter().enumerate() {
                out.push_str(&pad(depth + 1));
                let comma = usize::from(i + 1 < items.len());
                pretty_json(v, layout, depth + 1, comma, out);
                out.push_str(if comma == 1 { ",\n" } else { "\n" });
            }
            out.push_str(&pad(depth));
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// The indentation of the first indented line: a tab or some spaces.
fn json_indent(text: &str) -> &str {
    text.lines()
        .filter_map(|l| l.strip_suffix(l.trim_start_matches([' ', '\t'])))
        .find(|i| !i.is_empty())
        .map_or("  ", |i| if i.starts_with('\t') { "\t" } else { i })
}

/// JSON with comments and trailing commas made plain JSON.
fn strip_jsonc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    // Where the last comma outside a string went, until something other than space follows.
    let mut comma: Option<usize> = None;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match c {
                '\\' => out.extend(chars.next()),
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match (c, chars.peek()) {
            ('/', Some('/')) => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut last = ' ';
                for n in chars.by_ref() {
                    if last == '*' && n == '/' {
                        break;
                    }
                    last = n;
                }
            }
            _ if c.is_whitespace() => out.push(c),
            _ => {
                if let Some(at) = comma.take()
                    && matches!(c, '}' | ']')
                {
                    out.remove(at);
                }
                if c == ',' {
                    comma = Some(out.len());
                }
                in_string = c == '"';
                out.push(c);
            }
        }
    }
    out
}

/// The helpers of the `flutter-look` piece, shipped in the binary like the skill.
const FLUTTER_LOOK: [(&str, &str); 2] = [
    (
        "look_test/fearlezz_look.dart",
        include_str!("../presets/flutter-look/fearlezz_look.dart"),
    ),
    (
        "look_test/flutter_test_config.dart",
        include_str!("../presets/flutter-look/flutter_test_config.dart"),
    ),
];

/// A Dart file Fearlezz owns whole, with a first line that says so.
fn dart_file(text: &str) -> String {
    format!(
        "// Written by fearlezz setup sync for the flutter-look piece; do not edit.
{text}"
    )
}

/// The skill this binary ships. It is embedded, so it matches the version and needs no network.
const SKILL: &str = include_str!("../../../skills/fearlezz/SKILL.md");
/// The setup guide `fearlezz setup guide` prints, embedded like the skill so it matches the
/// installed version.
pub(crate) const GUIDE: &str = include_str!("../../../skills/fearlezz/GUIDE.md");
const CLAUDE_SKILL: &str = ".claude/skills/fearlezz/SKILL.md";
const AGENTS_SKILL: &str = ".agents/skills/fearlezz/SKILL.md";
/// Each agent name of `[settings.skill] agents` and the file its agents read. Claude Code reads
/// only its own folder; `.agents/skills` is the one Codex, Cursor and others share.
const SKILL_FILES: [(&str, &str); 2] = [("claude", CLAUDE_SKILL), ("agents", AGENTS_SKILL)];

/// The skill with a note after its frontmatter, which an agent needs to find first in the file.
fn skill_text() -> String {
    let note = "<!-- Written by `fearlezz setup sync` from the Fearlezz binary; edits are overwritten. -->";
    SKILL.replacen("\n---\n", &format!("\n---\n\n{note}\n"), 1)
}

/// The skill files `[settings.skill]` asks for: every agent's when it names none.
fn skill_files(table: &Table) -> Result<Vec<&'static str>> {
    if let Some(key) = table.keys().find(|key| *key != "agents") {
        bail!("[settings.skill]: no field \"{key}\"; the one field is agents");
    }
    let Some(agents) = table.get("agents") else {
        return Ok(SKILL_FILES.iter().map(|(_, file)| *file).collect());
    };
    let Some(agents) = agents.as_array() else {
        bail!("[settings.skill] agents must be a list");
    };
    agents
        .iter()
        .map(|agent| {
            names::find(&SKILL_FILES, agent.as_str().unwrap_or_default())
                .map_err(|names| anyhow!("[settings.skill]: no agent {agent}; agents are {names}"))
        })
        .collect()
}

const CODEX_HOOKS: &str = ".codex/hooks.json";
const CURSOR_HOOKS: &str = ".cursor/hooks.json";

/// The file of an agent's hooks, unless they live in another tool's file.
fn hooks_file(agent: Agent) -> Option<&'static str> {
    match agent {
        Agent::Claude => None,
        Agent::Codex => Some(CODEX_HOOKS),
        Agent::Cursor => Some(CURSOR_HOOKS),
    }
}

/// The agents `[settings.hooks] agents` names, or `None` when it names none, which means the
/// agents the project uses.
fn hook_agents(table: &Table) -> Result<Option<Vec<Agent>>> {
    if let Some(key) = table.keys().find(|key| *key != "agents") {
        bail!("[settings.hooks]: no field \"{key}\"; the one field is agents");
    }
    let Some(agents) = table.get("agents") else {
        return Ok(None);
    };
    let Some(agents) = agents.as_array() else {
        bail!("[settings.hooks] agents must be a list");
    };
    agents
        .iter()
        .map(|agent| {
            Agent::named(agent.as_str().unwrap_or_default())
                .map_err(|names| anyhow!("[settings.hooks]: no agent {agent}; agents are {names}"))
        })
        .collect::<Result<Vec<_>>>()
        .map(Some)
}

/// Claude Code, whose folder the skill makes anyway, and every other agent whose folder the
/// project has.
fn agents_in_use(root: &Path) -> Vec<Agent> {
    let mut agents = vec![Agent::Claude];
    agents.extend(
        [(".codex", Agent::Codex), (".cursor", Agent::Cursor)]
            .into_iter()
            .filter(|(dir, _)| root.join(dir).is_dir())
            .map(|(_, agent)| agent),
    );
    agents
}

const ESLINT_FILE: &str = "eslint.fearlezz.mjs";
const DETEKT_FILE: &str = "config/detekt/detekt.yml";

/// A level as a tool spells a gate (error), a flag and a hint; `None` for a rule that is off.
fn severity<'a>(level: lints::Level, flag: &'a str, hint: &'a str) -> Option<&'a str> {
    level.finding().map(|level| match level {
        Level::Gate => "error",
        Level::Flag => flag,
        Level::Hint => hint,
    })
}

/// A level as a tool with `off` spells it.
fn switch(level: lints::Level, hint: &str) -> String {
    severity(level, "warn", hint).unwrap_or("off").into()
}

/// oxlint's categories, all off in the file Fearlezz writes, so only rules with a level run.
const OXLINT_CATEGORIES: [&str; 7] = [
    "correctness",
    "suspicious",
    "pedantic",
    "perf",
    "style",
    "restriction",
    "nursery",
];

/// oxlint rules by `plugin/rule`, the plugins of the rules that are on (each one's rules run
/// only when it is listed), and every category off. oxlint has no info level, so a hint warns.
fn oxlint_lint_table(lints: &Lints) -> Table {
    let Some(rules) = lints.of(lints::Tool::Oxlint) else {
        return Table::new();
    };
    let plugins: BTreeSet<&str> = rules
        .iter()
        .filter(|(_, level)| level.is_on())
        .filter_map(|(name, _)| name.split_once('/').map(|(plugin, _)| plugin))
        .collect();
    let table: Table = rules
        .iter()
        .map(|(rule, level)| (rule.clone(), Value::String(switch(*level, "warn"))))
        .collect();
    let categories: Table = OXLINT_CATEGORIES
        .iter()
        .map(|c| ((*c).to_string(), Value::String("off".into())))
        .collect();
    Table::from_iter([
        (
            "plugins".to_string(),
            Value::Array(
                plugins
                    .into_iter()
                    .map(|p| Value::String(p.into()))
                    .collect(),
            ),
        ),
        ("categories".to_string(), Value::Table(categories)),
        ("rules".to_string(), Value::Table(table)),
    ])
}

/// Biome rules under their group; only the listed rules run.
fn biome_lint_table(lints: &Lints) -> Table {
    let Some(rules) = lints.of(lints::Tool::Biome) else {
        return Table::new();
    };
    let mut groups = Table::new();
    groups.insert("preset".into(), Value::String("none".into()));
    for (name, level) in rules {
        // Lints::parse makes sure every name has its group.
        let Some((group, rule)) = name.split_once('/') else {
            continue;
        };
        let Value::Table(g) = groups
            .entry(group.to_string())
            .or_insert_with(|| Value::Table(Table::new()))
        else {
            continue;
        };
        g.insert(rule.into(), Value::String(switch(*level, "info")));
    }
    let mut linter = Table::new();
    linter.insert("rules".into(), Value::Table(groups));
    Table::from_iter([("linter".to_string(), Value::Table(linter))])
}

/// ESLint rules; ESLint has no info level, so a hint warns.
fn eslint_lint_table(lints: &Lints) -> Table {
    let Some(rules) = lints.of(lints::Tool::Eslint) else {
        return Table::new();
    };
    let table: Table = rules
        .iter()
        .map(|(rule, level)| (rule.clone(), Value::String(switch(*level, "warn"))))
        .collect();
    Table::from_iter([("rules".to_string(), Value::Table(table))])
}

/// The generated ESLint config: a list the project's `eslint.config.*` spreads in, after the
/// configs that register the plugins.
fn eslint_file(table: &Table) -> Result<String> {
    let body = serde_json::to_string_pretty(&serde_json::to_value(table)?)?;
    Ok(format!(
        "// {MANAGED}\n// Spread it into eslint.config.*: `import fearlezz from \"./{ESLINT_FILE}\"`, then\n// `...fearlezz` after the configs that register the plugins.\nexport default [{body}];\n"
    ))
}

/// Ruff runs exactly the rules that are on.
fn ruff_lint_table(lints: &Lints) -> Table {
    let Some(rules) = lints.of(lints::Tool::Ruff) else {
        return Table::new();
    };
    let select: Vec<Value> = rules
        .iter()
        .filter(|(_, l)| l.is_on())
        .map(|(code, _)| Value::String(code.clone()))
        .collect();
    let lint = Table::from_iter([("select".to_string(), Value::Array(select))]);
    Table::from_iter([("lint".to_string(), Value::Table(lint))])
}

/// ktlint's `ktlint_standard_<rule>` switches.
fn ktlint_lint_table(lints: &Lints) -> Table {
    let Some(rules) = lints.of(lints::Tool::Ktlint) else {
        return Table::new();
    };
    rules
        .iter()
        .map(|(id, level)| {
            let (set, rule) = id.split_once(':').unwrap_or(("standard", id));
            let state = if level.is_on() { "enabled" } else { "disabled" };
            (format!("ktlint_{set}_{rule}"), Value::String(state.into()))
        })
        .collect()
}

const EDITORCONFIG_BEGIN: &str = "# Fearlezz: begin";
const EDITORCONFIG_END: &str = "# Fearlezz: end";

/// `.editorconfig` with a block for Kotlin files between markers; the rest stays.
fn editorconfig(root: &Path, path: &str, table: &Table) -> Result<(String, String)> {
    let text = read(root, path)?.unwrap_or_default();
    let mut block = format!("{EDITORCONFIG_BEGIN}. {MANAGED}\n[*.{{kt,kts}}]\n");
    for (key, value) in table {
        let value = match value {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        writeln!(block, "{key} = {value}")?;
    }
    block.push_str(EDITORCONFIG_END);
    block.push('\n');
    let out = match (text.find(EDITORCONFIG_BEGIN), text.find(EDITORCONFIG_END)) {
        (Some(a), Some(b)) if a < b => {
            let before = text
                .split_once(EDITORCONFIG_BEGIN)
                .map_or("", |(before, _)| before);
            let end = text.split_once(EDITORCONFIG_END).map_or("", |(_, end)| end);
            let after = end.split_once('\n').map_or("", |(_, after)| after);
            format!("{before}{block}{after}")
        }
        (None, None) if text.is_empty() => format!("root = true\n\n{block}"),
        (None, None) => format!("{}\n{block}", text.trim_end()),
        // Writing a second block would pair the old begin with the new end later, and drop
        // every line between them.
        _ => bail!(
            "{path}: the lines \"{EDITORCONFIG_BEGIN}\" and \"{EDITORCONFIG_END}\" do not \
             pair up; fix them or remove the broken block, then run fearlezz setup sync"
        ),
    };
    Ok((path.to_string(), out))
}

/// detekt rules under their rule set, each active or not.
fn detekt_lint_table(lints: &Lints) -> Table {
    let Some(rules) = lints.of(lints::Tool::Detekt) else {
        return Table::new();
    };
    let mut sets = Table::new();
    for (name, level) in rules {
        let Some((set, rule)) = name.split_once('/') else {
            continue;
        };
        let Value::Table(s) = sets
            .entry(set.to_string())
            .or_insert_with(|| Value::Table(Table::new()))
        else {
            continue;
        };
        let active = Table::from_iter([("active".to_string(), Value::Boolean(level.is_on()))]);
        s.insert(rule.into(), Value::Table(active));
    }
    sets
}

/// golangci-lint v2: no linter runs unless listed.
fn golangci_lint_table(lints: &Lints) -> Table {
    let Some(rules) = lints.of(lints::Tool::Golangci) else {
        return Table::new();
    };
    let enable: Vec<Value> = rules
        .iter()
        .filter(|(_, l)| l.is_on())
        .map(|(name, _)| Value::String(name.clone()))
        .collect();
    let linters = Table::from_iter([
        ("default".to_string(), Value::String("none".into())),
        ("enable".to_string(), Value::Array(enable)),
    ]);
    Table::from_iter([
        ("version".to_string(), Value::String("2".into())),
        ("linters".to_string(), Value::Table(linters)),
    ])
}

fn merge_json(into: &mut serde_json::Value, from: serde_json::Value) {
    match (into, from) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            for (k, v) in b {
                merge_json(a.entry(k).or_insert(serde_json::Value::Null), v);
            }
        }
        (slot, v) => *slot = v,
    }
}

/// A TOML table as block YAML. Strings are double quoted, which YAML reads like JSON.
fn yaml(table: &Table, depth: usize) -> Result<String, std::fmt::Error> {
    let mut out = String::new();
    let pad = "  ".repeat(depth);
    for (key, value) in table {
        let key = yaml_key(key);
        match value {
            Value::Table(t) if t.is_empty() => {
                writeln!(out, "{pad}{key}: {{}}")?;
            }
            Value::Table(t) => {
                writeln!(out, "{pad}{key}:")?;
                out.push_str(&yaml(t, depth + 1)?);
            }
            Value::Array(items) if items.is_empty() => {
                writeln!(out, "{pad}{key}: []")?;
            }
            Value::Array(items) => {
                writeln!(out, "{pad}{key}:")?;
                for item in items {
                    writeln!(out, "{pad}  - {}", yaml_scalar(item))?;
                }
            }
            scalar => {
                writeln!(out, "{pad}{key}: {}", yaml_scalar(scalar))?;
            }
        }
    }
    Ok(out)
}

fn yaml_key(key: &str) -> String {
    let plain = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'));
    if plain {
        key.to_string()
    } else {
        serde_json::to_string(key).unwrap_or_default()
    }
}

fn yaml_scalar(value: &Value) -> String {
    match value {
        Value::String(s) => serde_json::to_string(s).unwrap_or_default(),
        Value::Integer(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Boolean(b) => b.to_string(),
        Value::Datetime(d) => serde_json::to_string(&d.to_string()).unwrap_or_default(),
        // Nested values inside a list are written as inline JSON, which YAML accepts.
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dart_analysis_options_render_as_yaml() {
        let table: Table = toml::from_str(
            r#"
include = "package:lints/recommended.yaml"
[analyzer]
exclude = ["build/**", "lib/**.g.dart"]
[analyzer.errors]
todo = "ignore"
[linter.rules]
prefer_const_constructors = true
avoid_print = false
"#,
        )
        .unwrap();
        assert_eq!(
            yaml(&table, 0).unwrap(),
            "analyzer:\n  errors:\n    todo: \"ignore\"\n  exclude:\n    - \"build/**\"\n    - \"lib/**.g.dart\"\ninclude: \"package:lints/recommended.yaml\"\nlinter:\n  rules:\n    avoid_print: false\n    prefer_const_constructors: true\n"
        );
    }

    #[test]
    fn jsonc_loses_comments_and_trailing_commas_but_never_string_content() {
        let text = "{\n  // a comment\n  \"a\": \"C:\\\\\", /* x */\n  \"b\": \"1, ]\",\n  \"c\": [1, 2,],\n}";
        let plain: serde_json::Value = serde_json::from_str(&strip_jsonc(text)).unwrap();
        assert_eq!(
            plain,
            serde_json::json!({"a": "C:\\", "b": "1, ]", "c": [1, 2]})
        );
    }

    #[test]
    fn each_row_sits_at_its_tool() {
        for (at, row) in TOOLS.iter().enumerate() {
            assert_eq!(row.tool as usize, at, "{}", row.name);
        }
    }

    #[test]
    fn an_unknown_tool_says_which_exist() {
        let table: Table = toml::from_str("[clipy]\npedantic = \"warn\"\n").unwrap();
        let e = Settings::parse(table, Lints::default(), &BTreeSet::new())
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("tools are cargo-lints, dart-analysis, clippy, claude, biome, eslint, ruff, ktlint, detekt, golangci, oxlint, tsconfig, yamllint, flutter-look, skill, hooks"),
            "{e}"
        );
    }
}
