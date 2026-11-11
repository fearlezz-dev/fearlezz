# Fearlezz

Proof, not "trust me". Fearlezz helps coding agents show that their work is right, on the developer's machine first and in CI at the end.

Status: early. The core loop, stills and films, environments, settings and rules work.

## How it works

- A project declares **checks** in `fearlezz.toml`: any command, the files it reads, and the stage it belongs to. `use = ["rust"]` pulls in ready pieces (built in, from the repo, or from someone's repo) that the project tunes field by field.
- Each piece of work has a **journal**. The agent runs the checks the change touched and moves through **stages**: exploring, working, look, merge.
- Results are stored by the hash of their inputs, so unchanged work is never run twice, across worktrees too. The final stage runs everything fresh once.
- Checks can report **findings** at file and line (clippy, SARIF from any linter, or their own). Only findings on lines the change touched block, so old code never blocks new work.
- **Rules** cover what tests cannot: paths a change may not touch, naming, files that need a companion, and code patterns per language (through ast-grep), judged like findings.
- Pieces also carry **settings** (lint levels, analyzer rules, agent permissions). `fearlezz setup sync` writes them into each tool's own file, so editors agree, and a check catches drift.
- Checks that record **stills and films** are compared with the same check run on the base commit, on the same machine: same, only timing, or changed (where and when), in one report.
- An **environment** makes a fresh checkout usable and runs the services checks need, with their own ports per worktree. The final stage starts them from reset data once; idle ones stop on their own.

```console
$ fearlezz run
fail tests (exit 1)
  ...
$ fearlezz run
2 checks: 2 passed, 0 failed, 1 reused, 4.2 s
next: when ready: fearlezz run --stage look
$ fearlezz --all
needs you  feat/onboarding  look  Waiting for you: which empty state copy?
active     fix/crash        working  in working: 2 of 2 checks ok
```

## Use it

```sh
curl -fsSL https://raw.githubusercontent.com/fearlezz-dev/fearlezz/main/install.sh | sh
```

Or pick what fits the project:

| Where | How |
| --- | --- |
| Homebrew | `brew install fearlezz-dev/tap/fearlezz` |
| Nix | `nix run github:fearlezz-dev/fearlezz -- --plan` |
| devenv | add `fearlezz.url = "github:fearlezz-dev/fearlezz"` to `inputs` in `devenv.yaml`, and `inputs.fearlezz.packages.${pkgs.stdenv.system}.default` to `packages` |
| Cargo | `cargo install --git https://github.com/fearlezz-dev/fearlezz fearlezz` |

Prebuilt binaries for Linux and macOS are on the [releases](https://github.com/fearlezz-dev/fearlezz/releases) page. Stills and films need `ffmpeg`, and code rules need `ast-grep`.

To work on Fearlezz itself, enter the toolchain with `devenv shell`, then run `check`.

Agents: `fearlezz setup sync` writes [the skill](skills/fearlezz/SKILL.md) into your project, matching the installed version, for Claude Code and any agent that reads `.agents/skills`.

## CI in your project

```yaml
on:
  pull_request:
  push:
    branches: [main]
permissions:
  contents: read
  pull-requests: write
jobs:
  proof:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0
      - uses: fearlezz-dev/fearlezz@v0
```

The checkout needs `fetch-depth: 0`: the proof compares the change with its base branch. The action installs the latest release (`version: v0.2.0` pins one), runs `fearlezz run --ci`, keeps one status comment on the pull request (`comment: false` turns it off) and uploads the check logs. Set `devenv: true` when the checks run in a devenv shell.

## License

The code in `crates/` is available under either the [MIT](LICENSE-MIT) or the [Apache 2.0](LICENSE-APACHE) license, at your option.
