# Fearlezz

Fearlezz helps coding agents prove their work. It is a Rust CLI in `crates/fearlezz`, with the proof bundle model in `crates/fearlezz-proof`, shared with the proof host.

## Working here

- Read `ROADMAP.md` first: the purpose, what is built, what is not, and what comes next. Update it in the same change that moves something.
- devenv provides the toolchain. When `DEVENV_ROOT` is set it is already loaded; otherwise enter it with `devenv shell`. `check` builds Fearlezz and proves the commit with `fearlezz run --ci`, the same command CI runs. It is not the proof of a piece of work; only the journal is.
- Fearlezz proves its own changes: follow `skills/fearlezz/SKILL.md` with the checks in `fearlezz.toml`. Build first (`cargo build`), then use `target/debug/fearlezz`.
- Behavior is specified by the integration tests in `crates/fearlezz/tests/`, one file per feature, which drive the real binary in real git repositories. Add a failing test to the feature's file before changing behavior.
- Before opening a pull request, run a fresh review of the whole change and fix what it confirms. A green proof shows the change builds and passes; it does not judge design, security or scope.
- Spike code never merges. Rebuild what is worth keeping through the loop, with its own tests.

## Model

- `crates/fearlezz-proof`: the proof bundle, `proof.json`, as serde types (checks, tests, findings, looks, weakenings, red before green) with what blocks, what to read and red before green's one wording; `notes.rs` holds a person's notes on a picture (side, region, moment, words, open or resolved) and what the CLI reads of them (`Feedback`); `floor.rs` holds what a change weakens about the proof its base branch requires (`Weakening`, one per part with every way it weakened and the part as TOML before and after); `review.rs` is review on the host (who may say what, a verdict per picture kept by its content, an admin's accept, where review of a head stands, and the check run's summary); `events.rs` is protocol version 2 (what CI and a signed in CLI stream to the host, and what a live work page and repository page receive). Only serde (ts-rs behind the `ts` feature generates the page's TypeScript types), so the host builds it to wasm; the core and `ee/host` both depend on it.
- `config.rs`: `fearlezz.toml`, stages and checks (those of each change, and the scheduled ones a scheduled run proves instead); a check's `inputs` are what it reads (the reuse key, by default every file outside `no_proof`), a speed hint that never decides whether it is needed; `each` makes a check one per project folder (`<id>/<folder>`, run there through `dir`; a Cargo project is a manifest no workspace above names a member), and the same project folders are where Rust lint tables are written.
- `init.rs`: `fearlezz setup init`, the starter `fearlezz.toml` from the languages at a repository's root (Rust also from Cargo projects in folders): a TypeScript project's linters and formatters (Biome unless package.json names others), frameworks and test runner (Vitest, Node or Bun), Svelte in place of tsc, a Dart package apart from a Flutter app, envtest for a Go operator.
- `git.rs`: the facts read from git (root, common dir, changed files, tree id, worktrees).
- `process.rs`: running other programs: `Project`, the one builder of a project's commands (`sh -c` inside the `[env]` shell, in a folder, with a clean env of the base variables and the ones a check names, the project's tool folders on PATH), and `read`, which runs a program to its end or a time budget and turns a failure into what it said.
- `files.rs`: small file facts every module shares: SHA-256 hex, the per user config and cache folders, walking a folder, reading a file that may be missing, and writing a file whole.
- `lease.rs`: a lock on a file held until dropped, only or shared: the services of a worktree, base checkouts, the heavy queue's slots and the simulator pool.
- `store.rs`: results keyed by inputs hash, shared by every worktree, and what each passing run showed against a base commit (its look and red before green).
- `journal.rs`: the append-only events of one piece of work.
- `legs.rs`: a proof split over several CI jobs: `[platform.<name>]`, a check's `on`, `job` and `timeout`, the legs a plan prints, reusing a pass the base commit's CI proof shows for the same inputs, and the gate combining the legs' bundles.
- `state.rs`: everything shown, derived from journal, store and worktree. Nothing derived is stored.
- `presets.rs`: `use`, pulling pieces of config in by name, path or pinned repository and merging them; built in pieces live in `crates/fearlezz/presets/`.
- `run.rs`: running one check in a clean env with only the secrets it names (from SecretSpec, masked in its files), or a built in check in process; for a check that `needs = ["envtest"]`, the Kubernetes binaries of its go.mod's version first.
- `queue.rs`: the machine's heavy queue (`FEARLEZZ_QUEUE=<slots>`, off by default and on CI): slot lock files in the user's cache folder that a check's command, a run on the base commit, a picture comparison and `fearlezz run -- <command>` take first, what a waiter says it waits behind, and reusing a result another worktree produced meanwhile.
- `settings.rs`: shared tool settings from pieces, written into each tool's own file, and the agent skill every project gets; drift is a finding of the built in `settings` check. The skill's source is `skills/fearlezz/SKILL.md`, embedded in the binary; after changing it run `fearlezz setup sync`.
- `lints.rs`: `[lints.<tool>]` tables, the level (gate, flag, hint, off) a finding of each rule gets, by rule name.
- `rules.rs`: `[rule.<name>]` conventions (paths, naming, companion files, encrypted files, ast-grep code rules) that form the built in `rules` check.
- `commits.rs`: `[commits]`, the built in `commits` check: Conventional Commits for the pull request title in CI, or every commit since the base locally; the messages are part of its inputs.
- `sops.rs`: what `.sops.yaml` says must be encrypted and whether a file is, from the metadata sops writes; it never decrypts. The `encrypted = "sops"` rule kind reads it.
- `floor.rs`: the base branch's config as the floor of a change's proof: what the change weakens about its checks, as typed weakenings, one per part with every way it weakened and the part as it was and is now, rendered from the merged config table `Config` keeps. Proof is about behaviour, so it never judges files.
- `findings.rs`: findings a check reports at file and line (cargo JSON, SARIF, jsonl), the lines a change touched, and the verdict.
- `counts.rs`: how many tests a check ran (JUnit reports, or a pattern in its output) and which failed or passed only on a rerun; zero fails the check.
- `coverage.rs`: a test check's coverage report (lcov, JaCoCo XML, Go coverprofile) read into findings of the rule `uncovered`, one per run of changed lines no test ran.
- `env.rs`: the environment: preparing a fresh checkout (copy-on-write clones of ignored folders), starting, reusing, resetting and stopping the services checks need, and deploying the preview once per tree.
- `media.rs`: comparing two stills or films (stable states, time warping, blurred pixel diff).
- `record.rs`: recording an app on a screen: a window on macOS (ScreenCaptureKit through objc2), for `fearlezz --still` and a check's `record` table; `record/simulator.rs` leases a booted iOS simulator of Fearlezz's own and records it with `simctl io` (`needs = ["ios"]`); `record/display.rs` starts Xvfb and reads the window over X into ffmpeg (`needs = ["linux"]`); a screen the machine can never have leaves the check to CI; `record/place.rs` parks it off the developer's screens (`FEARLEZZ_OFF_SCREEN=1`), by AeroSpace when it runs, Accessibility otherwise, gives focus back, and puts back the window positions the app saved while parked.
- `look.rs`: comparing a check's pictures and texts with the same check run on the base commit, and the lines naming each changed one's before and after files (a text with the start of its diff).
- `text.rs`: the one line diff, shared by texts and generated code.
- `generated.rs`: `generates`, the files a check's command writes into the repository, which must come out the same as the tree had them.
- `why.rs`: why a check failed (`Why`), read once from its run, in the order that names the cause first (a secret, the preview or a tool that kept its command from running, then a blocking finding, stale generated files, no tests, a report it did not leave, its exit), and said both as the terminal's `fail` line with its `fix:` and as the proof's item.
- `proof.rs`: a check's entry in the proof from its run, and writing the bundle `run --ci --proof` writes (the model is `fearlezz-proof`), which feeds the job summary and the upload. Only the proof host draws a page from it.
- `feedback.rs`: what a person says on the proof host, as the agent reads it: `setup login` and `setup logout` (GitHub's device flow through the host, the read only token kept in the user's config folder with mode 0600), the host's lines in `fearlezz`, `fearlezz --notes` saving each open note's pictures with its region drawn (`media::noted`), and the open notes that hold the stop hook.
- `host.rs`: streaming a run to the proof host (`[host] url`, default `https://fearlezz.seamlezz.com`) through curl, from GitHub Actions with an OIDC token or, as an unproven local run, from a CLI signed in with `setup login`: open, each check's start and end with its files, done with the bundle; a host error never fails the proof or the run. The host itself lives in `ee/host` (`ee/README.md`), its pages in `ee/app`.
- `red.rs`: red before green, a check run on the base commit with the change's test files laid over it.
- `release.rs`: releases, the places in `[release.<place>]`: each one's dry run as the check `release/<place>`, and `fearlezz run --release`, which publishes where the version (from the place's own file) has not landed and waits until each place shows it. It asks registries through `host.rs`'s curl calls.
- `hooks.rs`: agent hooks (Claude Code, Codex, Cursor): what `setup sync` writes into each agent's settings, and the answers of `fearlezz setup hook` that hold an agent to the proof while it works.
- `action.rs`: `run --ci` on GitHub Actions: `Workflow`, the one reader of GitHub's variables and event file (the repository, run page and pull request the proof names), the job summary in Markdown, and the pinned ffmpeg. The core writes nothing to GitHub.
- `main.rs`: the CLI.

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
- Every command prints a `next:` line as its last line, except output meant for programs (`--check`, `setup compare --json`, `setup hook`, `run --ci --plan`) and errors, whose message says what to do.
