# Fearlezz

Fearlezz helps coding agents prove their work. It is a Rust CLI in `crates/fearlezz`.

## Working here

- Read `ROADMAP.md` first: the purpose, what is built, what is not, and what comes next. Update it in the same change that moves something.
- Enter the toolchain with `devenv shell`. `check` builds Fearlezz and proves the commit with `fearlezz run --ci`, the same command CI runs. It is not the proof of a piece of work; only the journal is.
- Fearlezz proves its own changes: follow `skills/fearlezz/SKILL.md` with the checks in `fearlezz.toml`. Build first (`cargo build`), then use `target/debug/fearlezz`.
- Behavior is specified by the integration tests in `crates/fearlezz/tests/`, one file per feature, which drive the real binary in real git repositories. Add a failing test to the feature's file before changing behavior.
- Before opening a pull request, run a fresh review of the whole change and fix what it confirms. A green proof shows the change builds and passes; it does not judge design, security or scope.
- Spike code never merges. Rebuild what is worth keeping through the loop, with its own tests.

## Model

- `config.rs`: `fearlezz.toml`, stages and checks; a check's `inputs` are what it reads (the reuse key, by default every file outside `no_proof`), a speed hint that never decides whether it is needed.
- `init.rs`: `fearlezz setup init`, the starter `fearlezz.toml` from the languages at a repository's root.
- `git.rs`: the facts read from git (root, common dir, changed files, tree id, worktrees).
- `store.rs`: results keyed by inputs hash, shared by every worktree, and what each passing run showed against a base commit (its look and red before green).
- `journal.rs`: the append-only events of one piece of work.
- `state.rs`: everything shown, derived from journal, store and worktree. Nothing derived is stored.
- `presets.rs`: `use`, pulling pieces of config in by name, path or pinned repository and merging them; built in pieces live in `crates/fearlezz/presets/`.
- `run.rs`: running one check in a clean env, or a built in check in process.
- `settings.rs`: shared tool settings from pieces, written into each tool's own file, and the agent skill every project gets; drift is a finding of the built in `settings` check. The skill's source is `skills/fearlezz/SKILL.md`, embedded in the binary; after changing it run `fearlezz setup sync`.
- `lints.rs`: `[lints.<tool>]` tables, the level (gate, flag, hint, off) a finding of each rule gets, by rule name.
- `rules.rs`: `[rule.<name>]` conventions (paths, naming, companion files, ast-grep code rules) that form the built in `rules` check.
- `floor.rs`: the base branch's config as the floor of a change's proof: what the change weakens about its checks. Proof is about behaviour, so it never judges files.
- `findings.rs`: findings a check reports at file and line (cargo JSON, SARIF, jsonl), the lines a change touched, and the verdict.
- `counts.rs`: how many tests a check ran (JUnit reports, or a pattern in its output); zero fails the check.
- `env.rs`: the environment: preparing a fresh checkout (copy-on-write clones of ignored folders), and starting, reusing, resetting and stopping the services checks need.
- `media.rs`: comparing two stills or films (stable states, time warping, blurred pixel diff).
- `record.rs`: recording an app's window on macOS (ScreenCaptureKit through objc2): `fearlezz --still` and a check's `record` table; `record/place.rs` parks it off the developer's screens (`FEARLEZZ_OFF_SCREEN=1`), by AeroSpace when it runs, Accessibility otherwise, gives focus back, and puts back the window positions the app saved while parked.
- `look.rs`: comparing a check's pictures with the same check run on the base commit; `report.rs`: the look report page.
- `page.rs`: what the static pages share (colours, controls, motion); they fetch nothing.
- `red.rs`: red before green, a check run on the base commit with the change's test files laid over it.
- `hooks.rs`: agent hooks (Claude Code, Codex, Cursor): what `setup sync` writes into each agent's settings, and the answers of `fearlezz setup hook` that hold an agent to the proof while it works.
- `github.rs`: GitHub: the one client (REST through curl, GraphQL for the atomic ref update), the status comment and check page with their Markdown, and the one rule for which approval counts, shared by CI and `say approve --github`.
- `action.rs`: `run --ci` on GitHub Actions: what the event asks (prove, approve, withdraw, clean, squash), forks and stale runs, the pictures branch through the git data API, and the pinned ffmpeg.
- `main.rs`: the CLI; `overview.rs`: the overview page.

## Releases

- release-please opens the release PR from commits on `main`; merging it tags `vX.Y.Z` and builds the binaries (`.github/workflows/release.yml`).
- Only conventional commits count: a change that should release starts with `fix:` or `feat:` (`feat!:` for a breaking one). Plain sentence commits are fine but cause no release; with a squash merge the pull request title is the commit.

## Free and paid

- Everything that runs locally (CLI, checks, journals, diffs, reports) belongs to the open core in `crates/`. Hosted, paid parts go under `ee/`. Keep the core free of any dependency on `ee/`.

## Command line

- When adding a capability, make it a flag on `fearlezz` or `fearlezz run`, or a `say` or `setup` subcommand, and teach it through `next:` or `fix:` lines. Never add a fourth intake point.
- A piece never needs shell glue: its check's `run` is a plain tool command, and Fearlezz reads the tool's own output.

## Style

- No dashes as punctuation in prose, comments or commit messages.
- Every command prints a `next:` line as its last line, except output meant for programs (`--check`, `setup compare --json`, `setup hook`) and errors, whose message says what to do.
