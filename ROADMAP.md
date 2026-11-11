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
- **Proof comes from three sources:** what the edits touch, the kind of change, and the claims. The agent may only add to it.
- **All proof groups matter:** honest, does what was asked, did not break anything, tests are real, built well, safe. Not every project needs all of them.
- **Local first.** Journals and results live on the developer's machine. The final stage proves everything fresh once.
- **Free and paid.** Everything local is the open core in `crates/`. Hosted, paid parts go under `ee/`.

## Built

| Capability | Notes |
| --- | --- |
| Claims, checks, stages | Red then green for bugs; a claim needs a green run on the current tree |
| Results by input hash | Shared across worktrees, pruned after 30 days |
| Journals | One per piece of work, append only |
| Command line | See, run, say, setup |
| Findings | Only lines the change touched can block |
| Rules | Paths, naming, companion files, ast-grep code rules |
| Settings | One source written into each tool's own file, drift is a finding |
| Pieces | Rust, TypeScript, ESLint, Python, Go, Kotlin, Flutter, devenv, agents; pinned with `fearlezz.lock` |
| Stills and films | Compared with the same check run on the base commit, with a report page |
| Approvals | A visual change waits for the developer's words |
| Environments | Fresh checkout setup, services with their own ports per worktree |
| Distribution | GitHub Releases, `install.sh`, Nix flake and devenv input, `cargo install --git` |

## Partly built

| Capability | Missing |
| --- | --- |
| Overview of the work | A static page and `fearlezz --all` exist; nothing live for runs of hours or days |
| Local and CI together | CI runs Fearlezz; there is no waiting pull request check that gates merging on local proof |
| Tests are real | Red then green only; no mutation or coverage proof |
| Extension layers | Pieces work; a new kind of proof and a library layer do not exist yet |

## Not built

| Capability | State |
| --- | --- |
| Recording apps on macOS | Spike proved a covered window can be recorded with ScreenCaptureKit; no recorder yet |
| Showcase | Stitch proofs into polished README films and release notes; prototyped once |
| Safety proof | No secret scanning or security piece |
| Hosted parts and proof links | `ee/` is empty; links may be hidden or need sign in |
| Trends over months | Decided to live apart from journals, with the same IDs |
| MCP layer | Decided to come later, as a thin layer on the CLI |

## Wishes

- **Releases as a layer.** Help with product releases, so publishing to many places at once is easy: GitHub, Homebrew, Nix, package registries and release notes. It should fit the existing system as layers, with pieces per destination and proof that each place really got the release.
- **Ship the agent skill with the CLI.** `skills/fearlezz/SKILL.md` lives in this repository only, so a new project does not get it.
- **Name the failing test.** A failed test check shows its last lines; naming the failures directly would be better.
- **Faster film comparison.** A 94 second 1080p film takes about 236 seconds.

## Next

1. Finish the first release: `v0.1.0`, the Homebrew tap, and install it on the developer's machine.
2. Adopt Fearlezz in a first real project outside this repository.
3. Decide what a run on `main` should prove (see below).
4. Pick up the list above one piece at a time, each in its own thread.

## Known problems

- **CI has not run the full proof on the GitHub runner.** Fearlezz only runs checks whose inputs a change touched. A run on `main` has no change, and a pull request that touches only packaging files reads no check input, so both report "nothing to run" and pass. The first pull request that touches code gives the first real run.
- **macOS is not covered by CI.** It is only proven on the developer's machine.
- **A flaky test.** One test in `core` has failed twice on CI and not locally; its name is still unknown. Failed checks now show 12 lines, so the next failure should name it.
- **60 lint flags** in existing code, mostly possible panics from indexing. They do not block.

## Goldens

There are none, by decision: a look compares against the same check run on the base commit, so nothing goes stale. Tests generate their pictures and films when they run, and check command output with text matches.
