# Roadmap

What Fearlezz is for, where it stands and where it is going. Read this first when picking up the work, and update it in the same change that moves something.

Last updated: 2026-11-13.

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
| Checks, stages | Proof is about behaviour, never files: any change needs every check, and a pass counts only for the inputs it ran on (every file outside `no_proof`, or the check's own `inputs`, a speed hint), so an edit to prose reruns nothing; the final stage runs everything fresh. Glob lists take `!` globs back out, the last match deciding |
| Results by input hash | Shared across worktrees, pruned after 30 days |
| Journals | One per piece of work, append only |
| Command line | See, run, say, setup. A tool a check cannot find is named with how to install it; tools in `node_modules/.bin` and `.venv/bin` are found |
| Starting out | Without `fearlezz.toml`, every command leads to `fearlezz setup init`, which writes a starter from the languages at the root and `no_proof` for prose |
| Findings | Only lines the change touched can block |
| Test counts | A check's `tests` reader (JUnit reports, or a pattern in the output) counts what ran; zero tests fails it. Every language piece runs and counts its tests |
| Rules | Paths, naming, companion files, ast-grep code rules |
| Settings | One source written into each tool's own file, drift is a finding |
| Agent skill | Embedded in the binary; `fearlezz setup sync` writes it for Claude Code (`.claude/skills`) and other agents (`.agents/skills`) in every project, unless it opts out. A missing or stale copy is led to by a `next:` line and is a finding of the `settings` check |
| Pieces | Rust, TypeScript, ESLint, Playwright, Python, Go, Kotlin, Flutter, devenv, agents, safety, trufflehog, betterleaks, osv-scanner, actions; pinned with `fearlezz.lock`. Checks are named for their tools (`vitest`, `ruff`, `go-test`), so language pieces combine; Rust keeps `fmt`, `clippy`, `tests` |
| Stills and films | Compared with the same check run on the base commit, checked out in the user cache folder (outside the repository, so dev servers serve it), with a report page: counts that filter, grouping by screen, viewport or theme, five ways to compare, films on one clock with lanes of where they differ, a decision per picture and an approve bar in the developer's words. Compared per color channel; films at 720p at most, stills at their own size. Listed in `proof.jsonl` or found by `stills` and `films` globs; `flaky` pictures show but never block. The `playwright` piece takes them from `toHaveScreenshot` and videos, its `webServer` on the free port every run gets (`FEARLEZZ_PORT`) |
| Recording app windows | macOS, in the binary (ScreenCaptureKit via objc2): `fearlezz --still <window>` for the agent to look at its running app, and `record` on a check films or stills the first window that appears after its command starts, and stops a command that runs `wait` without one; the stills on request keep the newest 20. Where nothing can record it fails locally with the one time fix and is not run in CI. `flutter-look` piece: headless widget test stills, and integration test films of one entry file |
| Recording off the developer's screens | Opt in per machine with `FEARLEZZ_OFF_SCREEN=1`: the recorded window is parked in a screen corner as soon as it appears (window capture still records all of it, and a parked Flutter app keeps drawing), and focus goes back to the developer's app if the recorded one took it while it settled. AeroSpace parks it when it runs (floating on a workspace it does not show), Accessibility otherwise; where neither can, the log says why and it records where it opened. No virtual display and no private API. New windows are watched for through the window server's list (about 2 ms a listing, every 10 ms; ScreenCaptureKit, about 70 ms a listing, only captures), so the window shows where it opened for about 0.12 s by Accessibility and 0.2 to 0.45 s by AeroSpace (which picks windows up on its own schedule, over 2 s on a loaded machine; recording does not wait for it, since where the window is changes nothing in a window capture, and parking goes on for up to 10 s while it records), since macOS lets no app place another's new window. On plain macOS a strip of about 40 by 32 points stays in a bottom corner with no other display beyond it. Window positions the app saved while parked (macOS saves them on every move) are put back once the command ends, so it does not reopen in the corner; only positions still parked are touched, so the developer's own moves and a parallel recording stay, and after a killed run the next one clears the parked position. Fearlezz sends the app no input: its own test drives it. Proven on the developer's machine with a native and a Flutter app, AeroSpace on and off, in 14 runs (drive record `docs/drive/off-screen-recording.md`) |
| Approvals | A visual change waits for the developer's words. On a pull request a person with write access approves with `/fearlezz approve <words>` (a fork's naming its head), covering what the status comment showed when it was written; any written, edited or deleted approval comment makes CI settle the `fearlezz/pictures` status again from GitHub by one rule, so deleting or editing away the comment takes it back in any event order, and a re-run keeps it only for the same head, base commit and pictures (a film by which changed, since its bytes differ per recording); only the proving run writes the status comment, other pictures coming in a new one; `fearlezz say approve <check> --github` reads it into the journal by the same rule, only for a check whose pictures waited, as the person's approval rather than the agent's report. An agent signed in as the developer could still post it; the hooks only refuse the obvious ways |
| Safety | Pieces to pick from, each a check named for its tool: `safety` (gitleaks, the default every starter gets), `trufflehog` and `betterleaks` for secrets (gate on changed lines, never printed or stored), `osv-scanner` for vulnerable dependencies (flag, counted when the change touched the lockfile), `actions` (zizmor over GitHub workflows, its own levels). Read natively, without shell glue |
| Environments | Fresh checkout setup, services with their own ports per worktree |
| Distribution | GitHub Releases for Linux and macOS through release-please (merging its pull request tags and builds), `install.sh`, Homebrew tap, Nix flake and devenv input, `cargo install --git`. The tap follows new releases once its workflow (tap pull request 1) is merged |
| CI for other projects | `fearlezz-dev/fearlezz@v0`, a small composite action that installs the release of its tag (and refuses one too old) and runs `fearlezz run --ci`. On GitHub Actions the binary does what the event asks through the REST API: installs a pinned static ffmpeg when checks record pictures, proves the commit, posts the status comment with changed pictures before and after (from the `fearlezz-pictures` branch), sets the `fearlezz/pictures` status, and leaves a newer push's comment alone; a fork's run writes nothing and a maintainer approves its head by comment; a second workflow answers approvals and cleans and squashes the pictures branch atomically. Fearlezz proves itself through this action, on Linux and macOS. Tested against a fake GitHub API that enforces read only tokens; the earlier script version ran under act with the real GitHub API of a private repository, not yet on GitHub's runners or in another project |

## Partly built

| Capability | Missing |
| --- | --- |
| Overview of the work | A static page and `fearlezz --all` exist; nothing live for runs of hours or days |
| Local and CI together | "Ready to merge" and the final exit code are honest. `fearlezz run --ci` proves a commit with no journal, runs every check, and holds a change to the base branch's config as its floor (what it weakens about the checks, never which files they read); `main` requires the check. At the final stage `fearlezz` itself holds the work to the floor, so it never says ready while the change weakens the proof. CI posts one status comment on the pull request, replaced each run (proven or not, each blocker with why and what to do, naming the head commit and the merge it proved), and writes a check page with every check, its test count and the end of each failing output. Agent hooks (Claude Code, Codex, Cursor) refuse a pull request before ready, a merge past the required checks and an agent writing `/fearlezz approve`, block stopping once, and say a check's command run by hand is not proof. The Codex and Cursor hooks follow their docs but have not run in those agents yet, and they replace a project's own hooks for the same events in those files |
| Tests are real | Zero tests fails. Red before green runs the whole check on the base commit with the change's test files, in CI and at the final stage; a fix (its pull request title starts with `fix:`, or locally, without a title, any of its commits) blocks when it passes there or cannot run there. JUnit checks name the tests; a base that already fails shows nothing; every language piece sets `test_files` (Rust only `tests` folders). The verdict is kept with the run per base commit, beside its look, so `fearlezz` and the hooks hold the work to it too, and a rebase asks for a run again. No mutation or coverage proof |
| Extension layers | Pieces work; a new kind of proof and a library layer do not exist yet |

## Not built

| Capability | State |
| --- | --- |
| Showcase | Stitch proofs into polished README films and release notes; prototyped once |
| Code scanning | Secrets, dependencies and workflows are scanned (Safety, above); the code itself is not |
| Hosted parts and proof links | `ee/` is empty; links may be hidden or need sign in |
| Trends over months | Decided to live apart from journals, with the same IDs |
| MCP layer | Decided to come later, as a thin layer on the CLI |

## Wishes

- **Releases as a layer.** Help with product releases, so publishing to many places at once is easy: GitHub, Homebrew, Nix, package registries and release notes. It should fit the existing system as layers, with pieces per destination and proof that each place really got the release.
- **Name the failing test.** A failed test check shows its last lines; naming the failures directly would be better.
- **Faster film comparison.** A 94 second 1080p film took about 236 seconds. Timing from packets, a second read of state frames only, cheaper frame checks and comparing films at 720p at most cut the CPU time to less than half; on a 60 second 1080p film most of what is left is ffmpeg decoding it twice (the second read only picks state frames). Keeping state frames in memory would save that read at a cost in memory.

## Next

1. The hosted parts in `ee/`: proof links and pages a team can open without the developer's machine.

## Known problems

- **Sync never removes a rule a piece dropped.** Biome's `linter.rules` keeps stale keys and the `settings` check calls it in sync.
- **Monorepos.** `setup init` and the language pieces look only at the repository root.
- **Films stop working until replayd restarts.** On the developer's machine, after a few dozen films (27 in one series, during a load spike), every film fails with "failure to process first sample buffer" while stills still work, with the 0.2.0 build as well; killing `replayd` (launchd restarts it) brings films back. Its cause is unknown. Every film is noted in the machine's film log (`films.jsonl` in Fearlezz's cache folder: repository, check, how it went, the `replayd` pid and the load), so how often it happens can be counted; failures in a row under one `replayd` pid are this problem.
- **Recording app windows is proven only on the developer's machine.** CI runs the proof on macOS too, but hosted runners cannot grant Screen Recording.
- **Linux only code is not proven locally.** The local proof runs on macOS, so code behind `cfg(not(target_os = "macos"))` is first linted in CI.
- **CI pictures never ran on GitHub's runners.** The binary's GitHub flow is tested against a fake GitHub API, and the earlier script version ran under act with real API calls to a private scratch repository; no other project uses them yet.
- **Red before green costs are measured for Rust and Go only.** What running the base commit costs for TypeScript, Python, Kotlin and Flutter projects is not known.
- **A flaky test.** One test in `core` has failed twice on CI and not locally; its name is still unknown. Failed checks now show 12 lines, so the next failure should name it.
- **Any rebase needs a new comparison run.** A run's look and red before green verdict are kept per base commit (`AgainstBase`), though each depends only on the base run's inputs, so a rebase over a change outside a check's inputs (prose, for one) still makes the next run check out the base and compare again. Keying them by the base inputs, hashed from git blobs without a checkout, would remove that; its cost is not measured.
- **The default reuse key reads the whole tree.** A check without `inputs` hashes every file outside `no_proof` on each command (once per file, however many checks); the cost in a large repository is not measured. Git's blob ids for unchanged files would avoid reading them.
- **About 85 lint flags** in the code, mostly possible panics from indexing, numeric casts in the recorder and long functions (`run_checks`). They do not block.

## Goldens

There are none, by decision: a look compares against the same check run on the base commit, so nothing goes stale. Tests generate their pictures and films when they run, and check command output with text matches.
