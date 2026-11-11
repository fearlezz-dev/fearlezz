---
name: fearlezz
description: Use when working in a repository that has a fearlezz.toml, to plan, run and show the proof a change needs before calling it done.
---

# Fearlezz

Fearlezz keeps a journal per piece of work and proves the work with the checks the project declares in `fearlezz.toml`. There are three things to run:

- `fearlezz` shows where the work stands.
- `fearlezz run` proves it.
- `fearlezz say` records what only you know.

Every command ends with a `next:` line (JSON output and errors aside); follow it. It names the other commands when they matter.

## The loop

1. `fearlezz say start --kind bug|feature|refactor|other --goal "<the goal in the developer's words>"` once, on the work branch.
2. Add what the change must prove: `fearlezz say claim "<behavior>" --check <check>`. Claims only add requirements; never remove one to get green.
   `fearlezz say` writes the goal and claims into `.fearlezz/work/<branch>.toml`. Commit it with the change: CI reads it, needs it for any change a check reads, and proves each claim again by running its check. It is public with the code, so write the goal and claims for any reader.
3. Change code. `fearlezz --plan` shows which checks the change needs, per stage.
4. `fearlezz run` runs the checks due in the current stage. Unchanged inputs reuse earlier results, so run it often.
5. When a check fails, read the tail it prints (and the log path), fix the cause, run again.
6. `fearlezz run --stage <next>` when the stage's proof is in. Going back is fine: `fearlezz run --stage working --no-run --reason "<why>"`.
7. The last stage runs every check fresh once. `next: all proof is in: ready to merge` means done; it needs a goal, at least one claim, every claim proven and everything committed. Until then the run exits non-zero.

## What counts as proof

- Only `fearlezz run` on your journal is proof. Raw commands (`cargo test`, `bun test`, a project's `check` script) are quick feedback; running them proves nothing.
- `fearlezz run --ci` and the scripts around it prove a commit for CI. They say nothing about the work and keep no journal. On the base branch itself it runs every check.
- The base branch's `fearlezz.toml` is the floor. At the final stage and in CI, a change that turns a check off, narrows its inputs, changes its command or env list, moves it to a later stage, stops counting its tests or findings, lowers or unsets a lint, turns off, lowers, narrows or changes a rule, changes the `[settings]` of a tool that judges code or anything in `[env]` (dropping an `[env]` input too), or adds to `no_proof` fails with `weakens ...`, and a changed file no check reads and `no_proof` does not declare fails with `unproven ...`. Never weaken proof to get green; when it must change, ask the developer, since only a person with admin rights merges past it.
- Do not open a pull request before `fearlezz` says ready to merge. When you report proof, report that line from the journal.
- Hooks hold you to this. A push needs the goal and claims; a pull request needs ready, unless it is a draft (`--draft`); stopping with unproven work is blocked once, unless you asked the developer with `fearlezz say ask`. When a hook refuses, do what its message says; never work around it.

## Rules

- A bug claim needs its check red before the fix and green after. Run the check before fixing.
- A claim counts only for the tree it ran on. Any edit after the run makes it unproven again.
- `unstable` means the same inputs gave both outcomes: the check is flaky or reads something it does not declare. Fix that; do not rerun until green.
- When only the developer can decide, `fearlezz say ask "<question>"` and stop. Their journal moves to the top of `fearlezz --all`.
- Leave `fearlezz say note "<one line>"` at turning points so whoever catches up later knows why.

## Stills and films

- A check that records the UI lists each picture in `proof.jsonl` with `"kind": "still"` or `"kind": "film"` (any format ffmpeg reads; animated WebP, GIF, PNG, MP4). An optional `"id"` names it; otherwise the file name does.
- After such a check passes, `fearlezz run` runs the same check on the base commit (where the branch left the base branch) in a hidden worktree, once per base, and compares every picture: same, timing, changed, new, gone. It prints `look <check>: ...` and a report path.
- Changed pictures are not failures. Read the report, and when the change is not the intended one, fix it.
- Merge waits until the developer approves changed pictures. Show them the report. Only after they agree, run `fearlezz say approve <check> --said "<their words, quoted>"`. Never approve on your own judgment. The approval holds while the check's inputs stay the same; any further change needs a new one.
- `fearlezz setup compare BEFORE AFTER` compares any two stills or films directly, for quick tries while exploring.

## Seeing the work

- `fearlezz`: this journal in 12 lines or fewer.
- `fearlezz --all`: every journal, most urgent first.
- `fearlezz --since 2h`: what happened since a moment.
- `fearlezz --check <check>`: the latest result, with its proof folder.
- `fearlezz --page`: one HTML page with every journal.

## Writing a check

A check is any command that exits 0 on success. Fearlezz gives it `FEARLEZZ_OUT`, a folder for proof files, and reads `$FEARLEZZ_OUT/proof.jsonl` (one JSON object per proof, with at least `kind` and `file`). The command sees only base env vars (PATH, HOME, TMPDIR, TERM, LANG, LC_ALL, USER, SHELL) plus those listed under `env`. Everything it reads must match `inputs`, or reuse will be wrong.

```toml
[check.tests]
run = "cargo test --quiet"
stage = "working"          # exploring, working, look, merge, or the project's own [stages] order
inputs = ["src/**", "Cargo.toml", "Cargo.lock"]
env = ["DATABASE_URL"]
```

Files that need no proof, such as prose, are declared once at the top of `fearlezz.toml`. CI reads this list from the base branch, so add to it in a change of its own first:

```toml
no_proof = ["*.md", "docs/**"]
```

## Pieces

`use` pulls in pieces of config with the same shape as `fearlezz.toml`. The project file wins field by field, so write only what changes; `off = true` drops a check a piece brings in.

```toml
use = [
  "rust",                                  # built in: fmt, clippy, tests (also: flutter, typescript, eslint, python, kotlin, go, devenv, agents)
  "./proof/screens.toml",                  # a file in this repo
  "github:acme/presets//web.toml@v1.2",    # someone's repo, pinned to a tag or commit
]

[check.tests]
stage = "look"

[check.clippy]
off = true
```

Repository pieces are pinned in `fearlezz.lock` (commit it). Commands on a developer machine add new pieces to it; on CI (`CI` set) a lock that does not match is an error. When a tag moves, every command stops until `fearlezz setup update` accepts it, wherever the piece is named. `fearlezz setup update [piece]` moves the repository pieces `fearlezz.toml` names to their newest release tag of the same major version (prereleases and other tag families are skipped), says when a higher major is out (change the tag by hand to take it) and lists every setting that changes; review that list before committing.

## Rules

`[rule.<name>]` writes a project convention that tests cannot express. All rules together form the `rules` check, judged like other findings: only lines the change touched count, at the rule's `level` (gate, flag, hint, off). Each rule has `message`, and optionally `fix`, `why`, `files` and `skip` (globs), and exactly one of:

```toml
[rule.engine-frozen]
level = "gate"
changed = ["engine/**"]                  # the change may not touch these paths (deletes count)

[rule.screen-names]
level = "flag"
files = "lib/screens/**/*.dart"
name = "snake_case"                      # or kebab-case, PascalCase, camelCase, or a regex

[rule.widget-has-test]
level = "hint"
files = "lib/widgets/*.dart"
needs = "test/widgets/{stem}_test.dart"  # also {name} and {dir}

[rule.silent-fallback]
level = "gate"
message = "A failure becomes a default value."
[rule.silent-fallback.rust]              # one table per language, matched by ast-grep
any = ["$R.unwrap_or_default()", "$R.ok();"]   # or pattern = "...", or rule = {...} with utils
[[rule.silent-fallback.test]]
rust = { bad = "let x = r().unwrap_or_default();", good = "let x = r()?;" }
```

Code rules need `ast-grep` on the path (`pkgs.ast-grep` in devenv.nix). A rule test whose bad example no longer matches, or whose good one does, fails the check. A piece can ship rules; the project sets `off = true` or another `level`.

## Settings

Pieces carry tool settings (lint levels, analyzer rules, agent permissions) under `[settings.<tool>]`; the project overrides any field. `fearlezz setup sync` writes them into each tool's own file (`[workspace.lints]` in Cargo.toml, `analysis_options.yaml`, `.claude/settings.json`), and the `settings` check fails when a file drifts. Change settings in `fearlezz.toml`, never in the generated part of the tool's file; after changing them run `fearlezz setup sync`.

```toml
[settings.cargo-lints.clippy]
unwrap_used = "warn"       # tools: cargo-lints, dart-analysis, claude
```

`fearlezz setup sync` also writes the proof hooks: into `.claude/settings.json` for Claude Code, and into `.codex/hooks.json` and `.cursor/hooks.json` when the project has those folders. `[settings.hooks]` narrows it: `agents = ["claude", "cursor"]` writes those, `agents = []` none.

`fearlezz setup sync` also writes this skill, from the Fearlezz binary, into `.claude/skills/fearlezz/` and `.agents/skills/fearlezz/`, so it always matches the installed version. Commit both. `[settings.skill]` narrows it: `agents = ["claude"]` writes one copy, `agents = []` none.

## Lint levels

Pieces decide every lint rule once under `[lints.<tool>]` (tools: clippy, rustc, dart, biome, eslint, ruff, ktlint, detekt, golangci) as `gate`, `flag`, `hint` or `off`; the project overrides any rule. A check names the tables its findings use, and each finding gets its rule's level; an off rule is never reported. `fearlezz setup sync` turns the rules on or off in the tool's own file: Cargo.toml, analysis_options.yaml, biome.json, eslint.fearlezz.mjs (spread it into eslint.config.* after the plugin configs), ruff.toml, a marked block in .editorconfig (ktlint), config/detekt/detekt.yml, .golangci.yml. When the project already keeps a tool's config in another file that tool reads first (biome.jsonc, .ruff.toml, pyproject.toml with `[tool.ruff]`, .golangci.yaml), sync writes into that file instead of adding one that shadows it.

```toml
[check.clippy]
findings = { read = "cargo", lints = ["clippy", "rustc"] }

[lints.clippy]
wildcard_imports = "hint"
```

## Findings

A check with `findings` reports problems at file and line. Only findings on lines this change added or changed count (old code never blocks new work); compile errors count anywhere. `gate` means this must never happen and fails the check; `flag` means there is little or no reason for it, so fix it unless you have one; `hint` means there can be a good reason, so look at it again and keep it only when you can say why. `fearlezz run` and `fearlezz` list them with the fix; fix every gate before moving on.

```toml
[check.clippy]
run = "cargo clippy --all-targets --quiet --message-format=json"
stage = "working"
inputs = ["**/*.rs"]
findings = "cargo"     # or "dart" (dart analyze --format=machine), "eslint" (eslint --format json), "sarif" (stdout or $FEARLEZZ_OUT/findings.sarif), or "jsonl"
# findings = { read = "sarif", on = "everywhere" }   to count old findings too
```

`jsonl` reads `$FEARLEZZ_OUT/findings.jsonl`, one object per line: `file`, `line`, `level` (gate, flag, hint), `message`, and optionally `end_line`, `rule`, `fix`, `why`, `anywhere`.

## Test counts

A check that runs tests says where its runner reports how many ran, with `tests`. A run of zero tests fails the check, as does a run with no count, so a filter that matches nothing or a runner that found no tests cannot pass. CI shows each count. The rust piece sets `tests = "cargo"`.

```toml
[check.tests]
run = "cargo test --quiet"
stage = "working"
tests = "cargo"       # the "test result" lines in the output; or "junit" ($FEARLEZZ_OUT/junit.xml, skipped cases left out)
```

## Environments

`[env]` makes a fresh checkout usable, gives checks their toolchain, and runs services. With `shell` set (the devenv piece sets `devenv shell --`), every check runs inside it and the `[env] inputs` files are inputs of every check. Fearlezz prepares a checkout once before its first run (and the base checkout before a look), starts services only for checks with `needs = ["services"]`, and passes their ports as env vars. Do not start or stop services yourself; `fearlezz setup down` stops this checkout's services by hand.

```toml
use = ["devenv"]           # checks run inside devenv shell; prepare, up, down and ports from devenv

[env]
clone = ["node_modules"]   # copied copy-on-write from the main checkout
clone_when = ["bun.lock"]  # only while these match the main checkout
reset = "rm -rf .devenv/state/postgres"   # wipes data before the fresh run at the final stage

[check.e2e]
run = "bun run e2e"        # sees WEB_HTTP_PORT and the other ports
stage = "look"
inputs = ["src/**"]
needs = ["services"]
```
