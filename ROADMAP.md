# Roadmap

What Fearlezz is for, where it stands and where it is going. Read this first when picking up the work, and update it in the same change that moves something.

Last updated: 2026-11-11.

## Purpose

AI does so much now that we need a much better way to manage it, so it can actually prove its work is good and useful, not "trust me, bro".

Fearlezz is one excellent tool that helps AI coding agents audit, verify and prove that their work is right, for any kind of project, locally and in CI. Media proof was the first use, not the limit.

## What shapes every decision

- **The agent uses it too.** It is not only a report for the developer. The agent reaches for it while working ("how does this look again?", "what did I need?").
- **Three intake points.** `fearlezz` (see), `fearlezz run` and `fearlezz say`, plus `fearlezz setup`. Everything else is discovered through `next:` and `fix:` lines.
- **Layers.** Turn on, tune, combine, own check, piece in the repo, piece from someone's repo, new kind of proof. Each layer is built from the one below, so new capabilities should be new layers, not new systems.
- **Proof is what can be seen, not what is said.** Checks that ran, their results, pictures and films. What the agent says about its own work never counts as proof.
- **All proof groups matter:** honest, does what was asked, did not break anything, tests are real, built well, safe. Not every project needs all of them.
- **Local first.** Journals and results live on the developer's machine. The final stage proves everything fresh once.
- **CI trusts only what it saw.** Local proof is the agent's word, so it never counts as proof in CI. CI proves the commit itself, and "ready to merge" is never said about work that has not been proven.
- **Free and paid.** Everything local is the open core in `crates/`. Hosted, paid parts go under `ee/`.

## Built

| Capability | Notes |
| --- | --- |
| Checks, stages | A check counts only for the inputs it ran on; the final stage runs everything fresh |
| Results by input hash | Shared across worktrees, pruned after 30 days |
| Journals | One per piece of work, append only |
| Command line | See, run, say, setup. A tool a check cannot find is named with how to install it; tools in `node_modules/.bin` and `.venv/bin` are found |
| Starting out | Without `fearlezz.toml`, every command leads to `fearlezz setup init`, which writes a starter from the languages at the root and `no_proof` for prose |
| Findings | Only lines the change touched can block |
| Test counts | A check's `tests` reader (JUnit reports, or a pattern in the output) counts what ran; zero tests fails it. Every language piece runs and counts its tests |
| Rules | Paths, naming, companion files, ast-grep code rules |
| Settings | One source written into each tool's own file, drift is a finding |
| Agent skill | Embedded in the binary; `fearlezz setup sync` writes it for Claude Code (`.claude/skills`) and other agents (`.agents/skills`) in every project, unless it opts out. A missing or stale copy is led to by a `next:` line and is a finding of the `settings` check |
| Pieces | Rust, TypeScript, ESLint, Playwright, Python, Go, Kotlin, Flutter, devenv, agents, safety; pinned with `fearlezz.lock`. Checks are named for their tools (`vitest`, `ruff`, `go-test`), so language pieces combine; Rust keeps `fmt`, `clippy`, `tests` |
| Stills and films | Compared with the same check run on the base commit, with a report page: counts that filter, grouping by screen, viewport or theme, five ways to compare, films on one clock with lanes of where they differ, a decision per picture and an approve bar in the developer's words. Listed in `proof.jsonl` or found by a `stills` glob; the `playwright` piece takes them from `toHaveScreenshot` (spike, `spike/look-web`) |
| Approvals | A visual change waits for the developer's words |
| Environments | Fresh checkout setup, services with their own ports per worktree |
| Distribution | GitHub Releases for Linux and macOS through release-please (merging its pull request tags and builds), `install.sh`, Homebrew tap, Nix flake and devenv input, `cargo install --git`. The tap follows new releases once its workflow (tap pull request 1) is merged |
| CI for other projects | `fearlezz-dev/fearlezz@v0`, a composite action: installs a release, proves the commit, posts the status comment, keeps the logs. Not yet run in another project |

## Partly built

| Capability | Missing |
| --- | --- |
| Overview of the work | A static page and `fearlezz --all` exist; nothing live for runs of hours or days |
| Local and CI together | "Ready to merge" and the final exit code are honest. `fearlezz run --ci` proves a commit with no journal, runs every check on the base branch, and holds a change to the base branch's config as its floor; `main` requires the check. At the final stage `fearlezz` itself holds the work to the floor, so it never says ready while a changed file is unproven. CI posts one status comment on the pull request, replaced each run (proven or not, each blocker with why and what to do, naming the head commit and the merge it proved), and writes a check page with every check, its test count and the end of each failing output. Agent hooks (Claude Code, Codex, Cursor) refuse a pull request before ready and a merge past the required checks, block stopping once, and say a check's command run by hand is not proof. The rest of the proof gate is under Next |
| Tests are real | Zero tests fails. Red before green is a spike (`test_files`, CI only, whole check on the base commit, a `fix:` title blocks when it passes there); no piece sets `test_files` yet, no mutation or coverage proof |
| Extension layers | Pieces work; a new kind of proof and a library layer do not exist yet |

## Not built

| Capability | State |
| --- | --- |
| Recording apps on macOS | A spike recorder (`tools/macos-recorder`) takes a still or film of one window, covered or offscreen, in about a second plus the film length. It needs Screen Recording for the app that started the agent, and hosted CI runners cannot be granted it. Flutter stills need no recorder: widget tests render them headlessly (`docs/spikes/look-native`). No built in piece yet |
| Showcase | Stitch proofs into polished README films and release notes; prototyped once |
| Safety proof | The `safety` piece scans for secrets (gitleaks, gate on changed lines). No dependency, workflow or code scanning yet |
| Hosted parts and proof links | `ee/` is empty; links may be hidden or need sign in |
| Trends over months | Decided to live apart from journals, with the same IDs |
| MCP layer | Decided to come later, as a thin layer on the CLI |

## Wishes

- **Releases as a layer.** Help with product releases, so publishing to many places at once is easy: GitHub, Homebrew, Nix, package registries and release notes. It should fit the existing system as layers, with pieces per destination and proof that each place really got the release.
- **Name the failing test.** A failed test check shows its last lines; naming the failures directly would be better.
- **Faster film comparison.** A 94 second 1080p film took about 236 seconds. Timing from packets, a second read of state frames only, and cheaper frame checks cut the CPU time about in half; a film whose every frame changes still costs about 4 seconds per second of film, mostly the blur used to cut it into states.

## Next

1. Finish the proof gate, so unproven work cannot look proven (design: `docs/drive/ci-proof-loop/record.yaml`):
   - Red before green: settle the spike on `spike/red-before-green` (its draft pull request lists the open decisions), then set `test_files` in the language pieces.
   - The Codex and Cursor hooks follow their docs but have not run in those agents yet. Hooks replace a project's own hooks for the same events in those files.
2. Adopt Fearlezz in a first real project outside this repository, which then tests the gate for real.
3. Pick up the list above one piece at a time, each in its own thread.

## Known problems

- **The comparison is blind to color.** Pictures are compared in gray, so a blue accent turned green (or red turned green) is called same, and the report outlines only noise. Repro: the ignored test `a_change_of_color_alone_is_a_change`.
- **The base checkout lives under `.git`.** Tools that refuse or skip `.git` paths break there: the Vite dev server answers 403 to every file (its `server.fs.deny` has `**/.git/**`), so a look against a dev server has no before.
- **Sync never removes a rule a piece dropped.** Biome's `linter.rules` keeps stale keys and the `settings` check calls it in sync.
- **Monorepos.** `setup init` and the language pieces look only at the repository root.
- **macOS is not covered by CI.** It is only proven on the developer's machine.
- **A flaky test.** One test in `core` has failed twice on CI and not locally; its name is still unknown. Failed checks now show 12 lines, so the next failure should name it.
- **60 lint flags** in existing code, mostly possible panics from indexing. They do not block.

## Goldens

There are none, by decision: a look compares against the same check run on the base commit, so nothing goes stale. Tests generate their pictures and films when they run, and check command output with text matches.
