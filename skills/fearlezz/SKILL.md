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
3. Change code. `fearlezz --plan` shows the checks per stage and where each stands; any change needs every check not on a schedule.
4. `fearlezz run` runs the checks due in the current stage. A check whose inputs did not change reuses its earlier result, so run it often.
5. When a check fails, read the tail it prints (and the log path), fix the cause, run again.
6. `fearlezz run --stage <next>` when the stage's proof is in. Going back is fine: `fearlezz run --stage working --no-run --reason "<why>"`.
7. The last stage runs every check fresh once. `next: all proof is in: ready to merge` means done; it needs every check it ran fresh and green and everything committed. Changed pictures never hold it: the last stage says how many changed, and a person reviews them on the pull request. Until then the run exits non-zero.

## What counts as proof

- Only `fearlezz run` on your journal is proof. Raw commands (`cargo test`, `bun test`, a project's `check` script) are quick feedback; running them proves nothing.
- `fearlezz run --ci` proves a commit for CI. They say nothing about the work and keep no journal. On the base branch itself it runs every check not on a schedule (`--scheduled` runs only those). On GitHub Actions it writes the job summary: proven or not, each blocker with why and what to do, each weakening as a diff of the config, every check, and the changed pictures for a person to review on the pull request, with a link to the proof page when the repository sends the proof to the proof host. It writes nothing to the pull request; a person reviews it there as usual. Only CI writes these; the developer reads them, not your output.
- A check whose `on` names only platforms with another `os` than this machine's is `left to CI`: its leg in CI proves it, and it never holds the work here. Dropping a platform from a check's `on` weakens the proof.
- The base branch's `fearlezz.toml` is the floor. At the final stage and in CI, a change that turns a check off, changes its command or env list, moves it to a later stage, stops counting its tests or findings or reads them another way, takes or records fewer pictures or texts or calls more of them flaky, calls more tests flaky, checks fewer generated files, lowers or unsets a lint, turns off, lowers, narrows or changes a rule, changes the `[settings]` of a tool that judges code or anything in `[env]` but its inputs, or widens `no_proof`, `flaky` or `flaky_tests` (a new glob, a `!` glob dropped, or a new order that lets a glob win over a `!` glob) fails with `weakens ...`, on a pull request too; only a person with admin rights can merge past it, after reading why. With the proof host, it fails the CI job just the same until an admin accepts exactly that weakening of the head on the proof page, which runs the job again, and then it passes. Proof is about behaviour (checks pass, red before green, test counts, pictures), so no file is ever judged unproven. Never weaken proof to get green; when it must change, ask the developer.
- Do not open a pull request before `fearlezz` says ready to merge. When you report proof, report that line from the journal.
- Hooks hold you to this. A pull request needs ready, unless it is a draft (`--draft`); merging past the required checks (`gh pr merge --admin`) is for the developer; stopping with unproven work is blocked once, unless you asked the developer with `fearlezz say ask`. When a hook refuses, do what its message says; never work around it.

## Rules

- A run counts only for the inputs it ran on. Any edit to them makes the check stale again.
- `unstable` means the same inputs gave both outcomes: the check is flaky or reads something it does not declare. Fix that; do not rerun until green.
- When only the developer can decide, `fearlezz say ask "<question>"` and stop. Their journal moves to the top of `fearlezz --all`.
- Leave `fearlezz say note "<one line>"` at turning points so whoever catches up later knows why.

## Stills and films

- A check that records the UI lists each picture in `proof.jsonl` with `"kind": "still"` or `"kind": "film"` (any format ffmpeg reads; animated WebP, GIF, PNG, MP4). An optional `"id"` names it; otherwise the file name does.
- A tool that writes its pictures into a folder needs no `proof.jsonl`: `stills = "stills/**/*.png"` (or `films = "films/**/*.webm"`) on the check makes every matching file in `$FEARLEZZ_OUT` a still (or film), named by its path below the fixed folders, a still without its extension and a film with it (`stills/mobile/home.png` is `mobile/home`, `films/mobile/home.webm` is `mobile/home.webm`). Two pictures of one name fail the check.
- For a web app, `use = ["playwright"]`: each `toHaveScreenshot` call is a still, and videos are films. Its piece shows the lines `playwright.config` needs: Playwright's own `webServer` starts the app on `$FEARLEZZ_PORT`, a free port Fearlezz gives every run, so worktrees never collide. Add the still harness on the base branch first, so the base run takes the same pictures.
- A film is timed from its start, so a run that loads slower or waits longer on an answer shows as `timing`. Say what happened in it and it is timed from that instead: `{"kind": "mark", "film": "<its file>", "name": "pressed save", "at": 1.2}` lines in `proof.jsonl` (seconds into the film; `"hidden": true` keeps a mark off the page's timeline) and `{"kind": "span", "film": "<its file>", "name": "loading", "start": 0, "end": 0.8, "timing": "free"}` for a stretch that looks and takes different each run, such as loading, a fade or a spinner: only the state it ends on is compared, its timing is not, and it is no anchor, so films may have more or fewer (`"same"`, the default, must keep its timing and is an anchor). A said moment may read up to a tenth of a second late, never early, so write `start` at the recorder's first frame. A tool that knows only the wall clock writes `"wall"` (`"wall_start"`, `"wall_end"`) in milliseconds and one `{"kind": "start", "film": "<its file>", "wall": <ms at its first frame>}`; Fearlezz's own recorder writes the start line itself. `film` is the film's file below `$FEARLEZZ_OUT`, as its proof line or `films` glob names it. After each mark the timing must match the base's; a film whose marks differ (one names a step the other does not) is changed; a film without marks is timed from its start.
- Freeze what changes on its own, or every run shows it as changed: `page.clock.setFixedTime(...)` for clocks, `timezoneId` and `locale` in `use`, a seeded or fixed value for randomness, and `[data-volatile] { visibility: hidden }` in the `stylePath` for the rest. `toHaveScreenshot` already stops animations. When a picture still differs between identical runs, `flaky = ["<id or glob>"]` on the check shows it apart and leaves it out of the count of changed pictures; it weakens the proof, so ask the developer first.
- After such a check passes, `fearlezz run` runs the same check on the base commit (where the branch left the base branch) in a checkout in the user cache folder, once per base, and compares every picture: same, timing, changed, new, gone. It prints `look <check>: ...` and, for each picture that is not the same, its comparison and the paths of its before and after files. `fearlezz` lists them too. `note: the base run passed without any pictures` means the base could not take them; read its log unless the recording is new in this change.
- Changed pictures are not failures and never hold the work. Read the before and after files, and when the change is not the intended one, fix it. A person reviews them on the pull request; when you are unsure a change is wanted, ask the developer with `fearlezz say ask`.
- When the repository sends its proof to the proof host, a person reviews the pictures on its page: per picture "Looks right", or notes that mark a region of the before, the after or both (at a moment of a film) and say what should change; a picture with an open note needs change. To read them, the developer signs the CLI in once with `fearlezz setup login` (a token kept outside the repository, which never gives a verdict). Once signed in, every `fearlezz run` also shows on the host as an unproven local run, so a person can leave notes before a pull request exists; `fearlezz setup logout` stops that. Then `fearlezz` shows each picture's verdict and its open notes, and `fearlezz --notes` prints every open note with its words and saves its pictures with the region drawn; read those PNGs, change the code so each note is settled, and prove it again. You cannot resolve a note or give a verdict; a person does that on the page. While notes are open, stopping is held once.
- `fearlezz setup compare BEFORE AFTER` compares any two stills or films directly, for quick tries while exploring.
- To show the proof's pictures in a README, a docs site or a website, `fearlezz setup showcase` prints the markup for each still and film of the work's last run, from its stable address on the proof host's files origin (the newest passing proof of the default branch; the repository needs the Fearlezz app). A light and a dark variant, named apart by one `light` or `dark` folder or file name, become one `<picture>` that follows the reader's theme.

## Texts and generated code

- Rendered text a person should review, such as manifests (`kustomize build`, `flate`) or a plan, is a text: `texts = "rendered/**/*.yaml"` on a check (or `"kind": "text"` in `proof.jsonl`) makes each matching file in `$FEARLEZZ_OUT` a text, compared line by line with the same check on the base commit like a picture. `fearlezz run` prints each changed text's summary, its before and after files and the start of its diff; it never holds the work, and a person reviews it on the pull request.
- Code a generator writes into the repository is proven up to date with `generates = ["gen/**"]` on the check that runs the generator (globs from the folder the check runs in), never with `git diff` glue. The files it names must come out of the command the same as the tree had them; a difference fails the check with the diff, and the regenerated files stay, so commit them.
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

A check that needs the iOS simulator or a Linux display says so in `needs`, with or without `record`. Fearlezz owns that screen, needs no Screen Recording, and runs it on CI's runners too; `record` then films or stills it:

```toml
[check.journeys]
run = "flutter test integration_test/journeys_test.dart -d \"$FEARLEZZ_DEVICE\""
stage = "merge"
needs = ["ios"]                  # device = "iPad Air 11-inch (M4)"; by default the newest plain iPhone
record = { film = "journeys" }

[check.linux-tests]
run = "flutter test integration_test -d linux"
stage = "merge"
needs = ["linux"]                # display = "1280x720" (the default); no xvfb-run
```

`ios` leases a booted simulator of Fearlezz's own (kept booted between runs, the apps a run installed removed and the keychain reset before and after) and hands its id in `FEARLEZZ_DEVICE`; a recording films the whole device screen from the first app that launches on it. `linux` starts a virtual display (Xvfb, no window manager) and hands it in `DISPLAY`; a recording films the first window that appears on it. Without `on`, the check runs on the platforms whose `os` its screen needs (`linux`, or `macos` for `ios`), else on the default one. A machine that can never have the screen (`linux` on a Mac, `ios` off macOS) leaves the check to CI: `left to CI <check>: needs Linux`, and it does not hold the work. A leg on the wrong OS, or a missing tool (no Xvfb, no iOS runtime), fails the check with the fix.

Never set `FEARLEZZ_OFF_SCREEN` yourself: parking recorded windows off screen is the developer's choice for their machine. Drive the app from its own test; Fearlezz sends a recorded app no input.

## The machine's heavy queue

On a developer's machine that sets `FEARLEZZ_QUEUE=<slots>` (the developer's choice for their machine; never set it yourself), heavy work of every worktree and repository waits for one of that many slots instead of all running at once: each check's command, a run on the base commit, picture comparisons. A waiting step prints `wait <check>: the heavy queue is full (...); behind <what> in <worktree> (pid, time)`. Let it wait: run `fearlezz run` in the background and wait for it to end. Never stop the holder. A waiting check that another worktree proves meanwhile for the same inputs reuses that result. CI never queues.

- A heavy tool you run by hand for quick feedback (`cargo test`, `flutter test`, a build) goes through `fearlezz run -- <command>`: it waits its turn, then runs with the terminal and exits with the command's code. Without the queue it just runs. It proves nothing.
- A check quick enough never to wait (a formatter's check) says `light = true`, and none of its steps waits; Fearlezz's built in checks (`settings`, `rules` and the like) never wait.

## Seeing the work

- `fearlezz`: this journal in 12 lines or fewer.
- `fearlezz --all`: every journal, most urgent first.
- `fearlezz --since 2h`: what happened since a moment.
- `fearlezz --check <check>`: the latest result, with its proof folder.
- `update: ...` in `fearlezz` or at a session start: a newer Fearlezz is out, or the proof host no longer takes this one. Update it the way this machine installed it and tell the developer; the pin in `fearlezz.toml` moves only when the config needs a newer Fearlezz.

## Writing a check

A check is any command that exits 0 on success. Fearlezz gives it `FEARLEZZ_OUT`, a folder for proof files, `FEARLEZZ_PORT`, a free port for a server it starts itself, and reads `$FEARLEZZ_OUT/proof.jsonl` (one JSON object per proof, with at least `kind` and `file`). The command sees only base env vars (PATH, HOME, TMPDIR, TERM, LANG, LC_ALL, USER, SHELL, and rustup's RUSTUP_TOOLCHAIN, RUSTUP_HOME, CARGO_HOME) plus those listed under `env`. Tools a project installs in `node_modules/.bin` or `.venv/bin` come first on PATH; a command that is not found fails the check with how to install it, and never makes it unstable. A passing result is reused while its inputs stay the same: by default every file outside `no_proof`, plus its command and env. `inputs` is an opt in speed hint: a check that sets it is keyed on those files only, so everything it reads must match them, or reuse will be wrong. It never decides whether a check is needed; every check is. In `inputs`, `test_files`, `flaky`, `flaky_tests` and `no_proof` a glob with a leading `!` takes back what earlier globs matched, and a later glob can match it again: the last glob that matches decides, as in `.gitignore`.

```toml
[check.tests]
run = "cargo test --quiet"
stage = "working"          # exploring, working, look, merge, or the project's own [stages] order
env = ["DATABASE_URL"]
# inputs = ["src/**", "Cargo.toml", "Cargo.lock"]   only to rerun less often
```

A check that needs two commands lists them as steps, each a plain command, never joined with `&&` or a pipe: `run = ["terraform init -backend=false", "terraform validate -json"]` runs them in order, the first that fails ends the check, and a reader of stdout reads only the last one's.

A check runs at the repository root. `dir = "shell"` runs it in that folder; `each = "Cargo.toml"` makes it one check per project folder holding that file (outside `no_proof`, and not inside another such folder; for Cargo.toml, any manifest no workspace above names a member, as Cargo decides): the root's keeps the check's name, one in a folder is `<check>/<folder>`, and `[check."clippy/shell"]` tunes that one alone. A folder's check reads every file until its own table names `inputs`, since the check's own `inputs` name the root project's files. The rust piece's checks have `each = "Cargo.toml"`, so a repository with several separate Cargo projects gets fmt, clippy and tests for each, and `setup sync` writes the lint tables into each project's Cargo.toml. A project folder that goes takes its checks with it; moving a check to another `dir` weakens the proof.

A check that needs a secret names it: `secrets = ["EVAL_API_KEY"]`. Fearlezz takes each name from its own environment when it is set there (CI's own tokens, such as `GH_TOKEN`, need no setup), and asks SecretSpec for the others (in the `[env] shell` and the check's folder, so the project's `secretspec.toml` and provider apply); it hands only those values to the check, and the provider's own token never reaches it. Never put a secret in `env` or a `.env` file. A value never shows in the check's log or files, where it reads `***` (also as JSON escapes it and line by line; pictures and values under 4 characters are left as they are). A secret may not be named like a variable every check gets (`PATH`, `FEARLEZZ_*`). A secret SecretSpec cannot give fails the check with the fix, without running it; in CI, give the job the provider's token (for 1Password, `OP_SERVICE_ACCOUNT_TOKEN`). Changing a check's `secrets` weakens the proof, as changing its `env` does.

A check too slow, costly or reliant on real infrastructure to run for each change gets `schedule = true`: no change needs it, `fearlezz --plan` lists it apart, and a scheduled workflow proves the default branch with it through `fearlezz run --ci --scheduled` (the action's `scheduled: true`, with `issue-on-failure: true` to open an issue when it fails). `fearlezz run <check>` still runs it by name while you work on it. Putting an existing check on a schedule weakens the proof, so ask the developer first.

Files that need no proof, such as prose, are declared once at the top of `fearlezz.toml`; an edit to them alone reruns no check. CI reads this list from the base branch, so add to it in a change of its own first:

```toml
no_proof = ["*.md", "docs/**"]
```

## Starting out

To set Fearlezz up in a repository, run `fearlezz setup guide` and follow it: it learns the project, asks the developer only what the repository cannot answer, and moves what proves work today into Fearlezz.

In a repository without `fearlezz.toml`, `fearlezz setup init` writes a starter: the built in pieces of the languages at its root, `no_proof` for prose, and `fearlezz = "<version>"`, the lowest Fearlezz the config needs. Run `fearlezz setup sync`, and land both on the base branch through their own pull request before other work builds on them. On GitHub, when no workflow proves with Fearlezz yet, ask the developer whether CI should send its proof to the proof host, then `fearlezz setup init --ci host` (or `--ci plain`, which also turns the host off with `[host] off = true`) writes `.github/workflows/proof.yml`, also beside a `fearlezz.toml` that is there; run `fearlezz setup sync` after. An older Fearlezz refuses the config and says which it needs; raise the pin when the config starts using something a newer Fearlezz added.

## Grow the proof

Proof that people can see should grow with the project. When setting up, or when a change makes a new kind of proof possible (a first UI screen, a new rendered output, a README that describes the UI), offer it to the developer, with `fearlezz say ask` or directly, and add it when they agree:

- A UI no check takes pictures of: `use = ["playwright"]` for a web app, `record` for a desktop window, `flutter-look` and `flutter-film` for a Flutter app, films for flows.
- Rendered output (kustomize, helm, terraform) no check keeps: write it into `$FEARLEZZ_OUT` and name it with `texts`.
- Checks that take stills and films while README.md shows none: `fearlezz setup showcase` prints the markup that shows each from its stable address, `https://fearlezz.seamlezz.net/showcase/<owner>/<repo>/<check>/<picture>` (Fearlezz's README, "Showcase media").

`fearlezz setup init` lists what the project could prove, one `could prove:` line each. While working, the `settings` check gives one as a hint when the change touches files of that kind; it never blocks, whatever level `[lints.fearlezz]` gives it. `[lints.fearlezz]` turns one off for the repository: `pictures`, `texts` or `readme-media` = `"off"`.

## Pieces

`use` pulls in pieces of config with the same shape as `fearlezz.toml`. The project file wins field by field, so write only what changes; `off = true` drops a check a piece brings in.

```toml
use = [
  "rust",                                  # built in: fmt, clippy, tests (also: dart, flutter, flutter-look, typescript: tsc, svelte: svelte-check in its place, biome, eslint, oxlint, prettier: a TypeScript project's linting and formatting, react, next, qwik, solid, vue: the rules of that framework, vitest, bun, node-test: a TypeScript project's tests, playwright, python, kotlin, go, go-race: the tests under the race detector at merge, envtest: Kubernetes binaries for an operator's tests, lua, devenv, agents, commits: Conventional Commits, actions: zizmor over GitHub workflows, safety: secret scanning with gitleaks where a leaked key on a changed line is a gate, trufflehog and betterleaks as other secret scanners, osv-scanner: vulnerable dependencies as flags)
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

[rule.sops]
level = "gate"
encrypted = "sops"                       # every file a .sops.yaml path_regex names is encrypted, every value sops encrypts ciphertext

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

Pieces decide every lint rule once under `[lints.<tool>]` (tools: clippy, rustc, dart, biome, eslint, ruff, ktlint, detekt, golangci, coverage, fearlezz) as `gate`, `flag`, `hint` or `off`; the project overrides any rule. A check names the tables its findings use, and each finding gets its rule's level; an off rule is never reported. `fearlezz setup sync` turns the rules on or off in the tool's own file: Cargo.toml, analysis_options.yaml, biome.json, eslint.fearlezz.mjs (spread it into eslint.config.* after the plugin configs), ruff.toml, a marked block in .editorconfig (ktlint), config/detekt/detekt.yml, .golangci.yml. When the project already keeps a tool's config in another file that tool reads first (biome.jsonc, .ruff.toml, pyproject.toml with `[tool.ruff]`, .golangci.yaml), sync writes into that file instead of adding one that shadows it.

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
findings = "cargo"     # or "dart" (dart analyze --format=machine), "eslint" (eslint --format json), "terraform" (terraform or tofu validate -json), "yamllint" (yamllint -f parsable), "sarif" (stdout or $FEARLEZZ_OUT/findings.sarif), or "jsonl"
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
tests = { count = '...', failed = '(?m)^---- (.+?) stdout ----$' }    # and each failed test's name, its first capture
```

Only reports written by the run count, so an old report never passes for a new run.

A failed check names each test that failed, from its JUnit reports or its `failed` pattern, in `fearlezz run` (`failed: <name>`), the job summary and on the proof host; a test named as failed fails the check even when the runner exits 0. A runner that reruns a failed test (Playwright with `includeRetries` on its junit reporter, cargo nextest, Maven) writes each failed attempt as `<flakyFailure>` or `<flakyError>`. A test that passed only on a rerun fails the check like a failure (`flaky: <name> (failed, then passed on a rerun)`): find why it is flaky and fix it. Only with the developer's OK, `flaky_tests = ["<name or glob>"]` on the check (a glob's `*` stops at `/`) lets it pass on a rerun, shown apart; it weakens the proof.

## Coverage

A test check that measures coverage names its report with `coverage`. Each run of lines this change added or changed that no test ran is a finding of the rule `uncovered`. Old untested code never counts, test files are never measured, coverage never makes a failing test run pass, and a run that writes no report fails. The go, bun and flutter pieces measure it; the rust, vitest, python and kotlin pieces show the line that turns it on.

How strict it is follows the project's coverage against a target, 80% unless the check sets one, since more tests add little above it:

- project at or above the target: untested lines are hints
- project under it, the change tests at least the target's share of its lines: flags
- project under it, the change tests less: gates

Never write a test only to reach a number: test what the code should do, or delete code nothing needs.

```toml
coverage = "lcov"                                           # $FEARLEZZ_OUT/lcov.info (cargo llvm-cov, Vitest, Bun, Flutter, pytest-cov)
coverage = "go"                                             # $FEARLEZZ_OUT/cover.out (go test -coverprofile)
coverage = { jacoco = "**/build/reports/kover/report.xml" }  # JaCoCo XML anywhere this run wrote it (Kover)
coverage = { read = "lcov", target = 70 }                   # another target
coverage = false                                            # turn off what a piece measures

[lints.coverage]
uncovered = "flag"   # one level, whatever the coverage
```

## Red before green

`test_files` names which files are a check's tests; the language pieces set them (Rust only for `tests` folders, since unit tests inside `src` cannot be split from the code). When the change touches some of them, the final stage and CI run the check again on the base commit with the change's test files laid over it:

- `red before green <check>`: it fails there and passes here. A check with a JUnit reader names the tests that failed there; a test that does not build there counts as failing, and says so.
- `not red before green <check>`: it passes there too, naming the change's tests that passed on both.
- `the base already fails`: it fails there without the change's test files too, so failing there shows nothing.

The change is a fix when its pull request title starts with `fix:`; the title decides, so a `feat:` title with an interim `fix:` commit is no fix. Locally the title is that of the branch's open pull request, which Fearlezz asks `gh` for when `origin` is on GitHub (kept a minute). Without one (no `gh`, no sign in, offline, or no pull request yet), any of its commits since the base starting with `fix:` makes it one; uncommitted work with no commits of its own is not one yet. The run names what decided, as `red before green: the pull request title is "fix: ..."`, or, without a title, the commit and why there is no title (`(no pull request title: gh is not installed)`); `red before green not needed: ...` likewise. Then tests that pass on the base commit do not show the fix, and a check that cannot run there decides nothing: the final stage's run fails, `fearlezz` does not say ready, and the commit is not proven. Change a test so it fails without the fix. The verdict holds for one base commit: after a rebase, `fearlezz` asks for a run before it says ready again, and the same goes for the look.

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

A preview deploy is a service that lives remotely. `[env] preview` deploys the checkout and prints its URL; a check with `needs = ["preview"]` gets the last URL it printed as `FEARLEZZ_PREVIEW_URL`. It runs at the checkout root in the `[env] shell` with a clean environment, like a check: the base variables, the `[env] env` names, the `[env] secrets` (fetched and masked as a check's are) and `FEARLEZZ_PREVIEW_NAME` (the branch made safe for a URL, `pr-<number>` for a pull request in CI, `commit-<sha>` on a detached HEAD, `base-<tree>` for the base a look compares with). Print a URL tied to this deploy (a version URL), not only an alias another deploy can move; the last URL printed after the deploy starts counts, so a banner the shell prints first never does. A checkout deploys each tree once; a later check or run in it reuses that URL. A preview that fails or prints no URL fails each check that needs it, without running it, with the fix. In CI the job summary and the proof name the URL. Changing `preview` or `secrets` weakens the proof, as any `[env]` part does.

```toml
[env]
preview = "wrangler versions upload --preview-alias $FEARLEZZ_PREVIEW_NAME"
secrets = ["CLOUDFLARE_API_TOKEN"]

[check.site-look]
run = "playwright test"
stage = "look"
needs = ["preview"]
```

## Releases

`[release.<place>]` names a place a release goes to. Its `dry` run becomes the check `release/<place>` at the last stage, so a release pull request proves the bumped version will publish. `fearlezz run --release` (CI, after the release is tagged) publishes to every place that does not have its version yet and waits until each shows it; places that have it are skipped, so running it again finishes a half done release. The version is the one in the place's own file (`Cargo.toml`, `package.json`, `pubspec.yaml`), which release-please bumps.

```toml
use = ["crates", "github"]     # built in: crates, npm, pub (dry run, publish, landed), github (files on the release)

[release.github]
version = "Cargo.toml"         # the file the version comes from
files = ["dist/*.tar.gz"]      # built before the release; landed when each is there with the same digest
# tag = "v{version}"           the default

[release.shop]                 # a place of your own
dry = "shop check"
publish = "shop push"          # sees FEARLEZZ_VERSION, FEARLEZZ_TAG, env and secrets; the files are its arguments ("$@")
landed = "shop has \"$FEARLEZZ_VERSION\""   # exits 0 once the place has it
version = "package.json"
env = ["SHOP_REGION"]          # seen by publish and landed, shown as it is; never by the dry run
secrets = ["SHOP_TOKEN"]       # a token CI gives the job, for publish and landed only; shows as *** in what they print;
                               # without it the tool logs in its own way (cargo login, OIDC trusted publishing)
# wait = 600                   seconds it may take to show the version
```
