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

1. Work on a branch; its journal starts by itself.
2. When the change fixes a bug or adds behavior, add a test that shows it, so a check proves it.
3. Change code. `fearlezz --plan` shows the checks per stage and where each stands; any change needs every check.
4. `fearlezz run` runs the checks due in the current stage. A check whose inputs did not change reuses its earlier result, so run it often.
5. When a check fails, read the tail it prints (and the log path), fix the cause, run again.
6. `fearlezz run --stage <next>` when the stage's proof is in. Going back is fine: `fearlezz run --stage working --no-run --reason "<why>"`.
7. The last stage runs every check fresh once. `next: all proof is in: ready to merge` means done; it needs every check it ran fresh and green, changed pictures approved, and everything committed. Until then the run exits non-zero.

## What counts as proof

- Only `fearlezz run` on your journal is proof. Raw commands (`cargo test`, `bun test`, a project's `check` script) are quick feedback; running them proves nothing.
- `fearlezz run --ci` proves a commit for CI. They say nothing about the work and keep no journal. On the base branch itself it runs every check. On GitHub it writes the check page (the job summary) and posts the short status on the pull request: proven or not, each blocker with why and what to do, and changed pictures, which never read as proven until a person approves them. Only CI writes these; the developer reads them, not your output.
- The base branch's `fearlezz.toml` is the floor. At the final stage and in CI, a change that turns a check off, changes its command or env list, moves it to a later stage, stops counting its tests or findings or reads them another way, takes or records fewer pictures or calls more of them flaky, lowers or unsets a lint, turns off, lowers, narrows or changes a rule, changes the `[settings]` of a tool that judges code or anything in `[env]` but its inputs, or widens `no_proof` or `flaky` (a new glob, a `!` glob dropped, or a new order that lets a glob win over a `!` glob) fails with `weakens ...`. Proof is about behaviour (checks pass, red before green, test counts, pictures), so no file is ever judged unproven. Never weaken proof to get green; when it must change, ask the developer, since only a person with admin rights merges past it.
- Do not open a pull request before `fearlezz` says ready to merge. When you report proof, report that line from the journal.
- Hooks hold you to this. A pull request needs ready, unless it is a draft (`--draft`); merging past the required checks (`gh pr merge --admin`) is for the developer; stopping with unproven work is blocked once, unless you asked the developer with `fearlezz say ask`. When a hook refuses, do what its message says; never work around it.

## Rules

- A run counts only for the inputs it ran on. Any edit to them makes the check stale again.
- `unstable` means the same inputs gave both outcomes: the check is flaky or reads something it does not declare. Fix that; do not rerun until green.
- When only the developer can decide, `fearlezz say ask "<question>"` and stop. Their journal moves to the top of `fearlezz --all`.
- Leave `fearlezz say note "<one line>"` at turning points so whoever catches up later knows why.

## Stills and films

- A check that records the UI lists each picture in `proof.jsonl` with `"kind": "still"` or `"kind": "film"` (any format ffmpeg reads; animated WebP, GIF, PNG, MP4). An optional `"id"` names it; otherwise the file name does.
- A tool that writes its pictures into a folder needs no `proof.jsonl`: `stills = "stills/**/*.png"` (or `films = "films/**/*.webm"`) on the check makes every matching file in `$FEARLEZZ_OUT` a still (or film), named by its path below the fixed folders without the extension (`stills/mobile/home.png` is `mobile/home`).
- For a web app, `use = ["playwright"]`: each `toHaveScreenshot` call is a still, and videos are films. Its piece shows the lines `playwright.config` needs: Playwright's own `webServer` starts the app on `$FEARLEZZ_PORT`, a free port Fearlezz gives every run, so worktrees never collide. Add the still harness on the base branch first, so the base run takes the same pictures.
- Freeze what changes on its own, or every run shows it as changed: `page.clock.setFixedTime(...)` for clocks, `timezoneId` and `locale` in `use`, a seeded or fixed value for randomness, and `[data-volatile] { visibility: hidden }` in the `stylePath` for the rest. `toHaveScreenshot` already stops animations. When a picture still differs between identical runs, `flaky = ["<id or glob>"]` on the check shows it but never blocks or waits for approval; it weakens the proof, so ask the developer first.
- After such a check passes, `fearlezz run` runs the same check on the base commit (where the branch left the base branch) in a checkout in the user cache folder, once per base, and compares every picture: same, timing, changed, new, gone. It prints `look <check>: ...` and a report path. `note: the base run passed without any pictures` means the base could not take them; read its log unless the recording is new in this change.
- Changed pictures are not failures. Read the report, and when the change is not the intended one, fix it.
- Merge waits until the developer approves changed pictures. Show them the report: they mark each picture and copy either the approve command in their own words or feedback for you. Only after they agree, run `fearlezz say approve <check> --said "<their words, quoted>"`. Never approve on your own judgment. The approval holds while the check's inputs stay the same; any further change needs a new one.
- On a pull request, CI shows the changed pictures and the developer approves them there with `/fearlezz approve <their words>`. That comment is theirs to write, never yours (a hook refuses it). Once they did, with the work committed and pushed, `fearlezz say approve <check> --github` records their approval in the journal; if they delete the comment, the approval is gone.
- `fearlezz setup compare BEFORE AFTER` compares any two stills or films directly, for quick tries while exploring.
- While building a UI, look at it often: `fearlezz --still <window>` (macOS) takes a still of a running app's window, named by pid, bundle id, app name or part of its title, and prints the PNG's path. Read the picture and critique it. It is for you, not proof.
- For a Flutter app, `use = ["flutter-look"]`: widget tests in `look_test/` call `still(tester, "home")` on a widget wrapped in `lookable(...)` (helpers written by `fearlezz setup sync`), headless and fast; `flutter-film` films the macOS app while `integration_test/app_test.dart` drives it, at the final stage.

## Recording an app's window

A check with `record` films or stills the window of the app its command starts (macOS, with Screen Recording for the app that started the agent), so a check needs no script around a recorder:

```toml
[check.app-look]
run = "swift run LookDemo"        # or xcodebuild test, flutter test integration_test/app_test.dart -d macos
stage = "look"
record = { still = "home", film = "home", seconds = 3 }
# window = "com.example.app"     the app when the command does not start it itself (open, launchd)
# wait = 300                     most seconds without a window to record (the default)
# crop_top = 28                  leave out the title bar (off by default; some apps have none)
```

Only a window that appears after the command starts is recorded, never one already open, so quit the app before a run that names `window`. The still is taken once the window is up; the film runs until the command ends, or for `seconds`, after which Fearlezz stops the command and the check counts as passed. A run that goes `wait` seconds without a window (none appeared, or every window it opened closed while the command kept running) is stopped and fails. A run records its first window only; a later window keeps the command running but is not recorded. `flutter test integration_test -d macos` starts the app once per file, so film one entry file: `flutter-film` runs `integration_test/app_test.dart`. Where nothing can record, the check does not run: locally it fails with the one time fix; in CI it is listed as not run and does not block, since looks of app windows are proven on a developer's machine.

## Seeing the work

- `fearlezz`: this journal in 12 lines or fewer.
- `fearlezz --all`: every journal, most urgent first.
- `fearlezz --since 2h`: what happened since a moment.
- `fearlezz --check <check>`: the latest result, with its proof folder.
- `fearlezz --page`: one HTML page with every journal.

## Writing a check

A check is any command that exits 0 on success. Fearlezz gives it `FEARLEZZ_OUT`, a folder for proof files, `FEARLEZZ_PORT`, a free port for a server it starts itself, and reads `$FEARLEZZ_OUT/proof.jsonl` (one JSON object per proof, with at least `kind` and `file`). The command sees only base env vars (PATH, HOME, TMPDIR, TERM, LANG, LC_ALL, USER, SHELL, and rustup's RUSTUP_TOOLCHAIN, RUSTUP_HOME, CARGO_HOME) plus those listed under `env`. Tools a project installs in `node_modules/.bin` or `.venv/bin` come first on PATH; a command that is not found fails the check with how to install it, and never makes it unstable. A passing result is reused while its inputs stay the same: by default every file outside `no_proof`, plus its command and env. `inputs` is an opt in speed hint: a check that sets it is keyed on those files only, so everything it reads must match them, or reuse will be wrong. It never decides whether a check is needed; every check is. In `inputs`, `test_files`, `flaky` and `no_proof` a glob with a leading `!` takes back what earlier globs matched, and a later glob can match it again: the last glob that matches decides, as in `.gitignore`.

```toml
[check.tests]
run = "cargo test --quiet"
stage = "working"          # exploring, working, look, merge, or the project's own [stages] order
env = ["DATABASE_URL"]
# inputs = ["src/**", "Cargo.toml", "Cargo.lock"]   only to rerun less often
```

Files that need no proof, such as prose, are declared once at the top of `fearlezz.toml`; an edit to them alone reruns no check. CI reads this list from the base branch, so add to it in a change of its own first:

```toml
no_proof = ["*.md", "docs/**"]
```

## Starting out

In a repository without `fearlezz.toml`, `fearlezz setup init` writes a starter: the built in pieces of the languages at its root, and `no_proof` for prose. Commit it on the base branch first, then run `fearlezz setup sync`.

## Pieces

`use` pulls in pieces of config with the same shape as `fearlezz.toml`. The project file wins field by field, so write only what changes; `off = true` drops a check a piece brings in.

```toml
use = [
  "rust",                                  # built in: fmt, clippy, tests (also: flutter, flutter-look, typescript, eslint, playwright, python, kotlin, go, devenv, agents, actions: zizmor over GitHub workflows, safety: secret scanning with gitleaks where a leaked key on a changed line is a gate, trufflehog and betterleaks as other secret scanners, osv-scanner: vulnerable dependencies as flags)
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
findings = "cargo"     # or "dart" (dart analyze --format=machine), "eslint" (eslint --format json), "sarif" (stdout or $FEARLEZZ_OUT/findings.sarif), or "jsonl"
# findings = { read = "sarif", on = "everywhere" }   to count old findings too
# findings = { read = "sarif", level = "gate", fix = "..." }   for a tool whose results carry no level or fix
```

`jsonl` reads `$FEARLEZZ_OUT/findings.jsonl`, one object per line: `file`, `line`, `level` (gate, flag, hint), `message`, and optionally `end_line`, `rule`, `fix`, `why`, `anywhere`.

## Test counts

A check that runs tests says where its runner reports how many ran, with `tests`. A run of zero tests fails the check, as does a run with no count, so a filter that matches nothing or a runner that found no tests cannot pass. CI shows each count. Every language piece sets it, so a project only writes `tests` when it changes its test runner.

```toml
tests = "junit"                                   # the JUnit report in $FEARLEZZ_OUT/junit.xml
tests = { junit = "**/build/test-results/*.xml" } # JUnit reports anywhere this run wrote them
tests = { count = 'test result: \w+\. (\d+) passed; (\d+) failed' }  # numbers a pattern captures in the output, or one per match
```

Only reports written by the run count, so an old report never passes for a new run.

## Red before green

`test_files` names which files are a check's tests; the language pieces set them (Rust only for `tests` folders, since unit tests inside `src` cannot be split from the code). When the change touches some of them, the final stage and CI run the check again on the base commit with the change's test files laid over it:

- `red before green <check>`: it fails there and passes here. A check with a JUnit reader names the tests that failed there; a test that does not build there counts as failing, and says so.
- `not red before green <check>`: it passes there too, naming the change's tests that passed on both.
- `the base already fails`: it fails there without the change's test files too, so failing there shows nothing.

The change is a fix when its pull request title starts with `fix:`; in CI the title decides, so a `feat:` title with an interim `fix:` commit is no fix. Locally, where no title is known, any of its commits since the base starting with `fix:` makes it one; uncommitted work with no commits of its own is not one yet. Then tests that pass on the base commit do not show the fix, and a check that cannot run there decides nothing: the final stage's run fails, `fearlezz` does not say ready, and the commit is not proven. Change a test so it fails without the fix. The verdict holds for one base commit: after a rebase, `fearlezz` asks for a run before it says ready again, and the same goes for the look.

```toml
[check.tests]
test_files = ["tests/**"]
```

```
passed go-test  2.8 s  2 tests  go test -v ./...
red before green go-test: sum_test.go fails on the base commit and passes here
  log on the base: .../output.log
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
needs = ["services"]
```
