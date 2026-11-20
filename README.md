# Fearlezz

Proof, not "trust me". Fearlezz helps coding agents show that their work is right, on the developer's machine first and in CI at the end.

Status: early. The core loop, stills and films, environments, settings and rules work.

## How it works

- A project declares **checks** in `fearlezz.toml`: any command and the stage it belongs to. `use = ["rust"]` pulls in ready pieces (built in, from the repo, or from someone's repo) that the project tunes field by field.
- Each piece of work has a **journal**. The agent runs every check of its stage and moves through **stages**: exploring, working, look, merge.
- Results are stored by the hash of their inputs (every file outside `no_proof`, or the files a check names), so unchanged work is never run twice, across worktrees too; an edit to prose reruns nothing. The final stage runs everything fresh once.
- Checks can report **findings** at file and line (clippy, SARIF from any linter, or their own). Only findings on lines the change touched block, so old code never blocks new work.
- **Rules** cover what tests cannot: paths a change may not touch, naming, files that need a companion, and code patterns per language (through ast-grep), judged like findings.
- Pieces also carry **settings** (lint levels, analyzer rules, agent permissions). `fearlezz setup sync` writes them into each tool's own file, so editors agree, and a check catches drift.
- Checks that record **stills and films** are compared with the same check run on the base commit, on the same machine: same, only timing, or changed (where and when), with the before and after files of each picture to open.
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

It runs on Linux and macOS. On Windows, run it inside WSL, where the same command installs the Linux build.

Or pick what fits the project:

| Where | How |
| --- | --- |
| Homebrew | `brew install fearlezz-dev/tap/fearlezz` |
| Nix | `nix run github:fearlezz-dev/fearlezz -- --plan` |
| devenv | add `fearlezz.url = "github:fearlezz-dev/fearlezz"` to `inputs` in `devenv.yaml`, and `inputs.fearlezz.packages.${pkgs.stdenv.system}.default` to `packages` |
| Cargo | `cargo install --git https://github.com/fearlezz-dev/fearlezz fearlezz` |

In a project, `fearlezz setup init` writes a starter `fearlezz.toml` from the languages it finds; `fearlezz setup sync` then writes the agent skill, hooks and lint settings.

A check that needs two commands lists them as steps, never joined with `&&`:

```toml
[check.validate]
run = ["terraform init -backend=false", "terraform validate -json"]
stage = "working"
```

Each step is its own `sh -c`, run in order; the first that fails ends the check, and a reader of stdout (findings, test counts) reads only the last step's output. A failed check whose `run` joins commands with `&&` says how to split it.

Prebuilt binaries for Linux and macOS are on the [releases](https://github.com/fearlezz-dev/fearlezz/releases) page. Stills and films need `ffmpeg`, and code rules need `ast-grep`.

To work on Fearlezz itself, enter the toolchain with `devenv shell`, then run `check`.

Agents: `fearlezz setup sync` writes [the skill](skills/fearlezz/SKILL.md) into your project, matching the installed version, for Claude Code and any agent that reads `.agents/skills`.

## CI in your project

One workflow proves every pull request and push:

```yaml
# .github/workflows/proof.yml
name: proof
on:
  pull_request:
    types: [opened, synchronize, reopened, edited]
  push:
    branches: [main]
permissions:
  contents: read
  # The proof page and review need this and the Fearlezz GitHub App installed; without
  # either, CI still proves the commit but sends nothing and shows no page.
  id-token: write
jobs:
  proof:
    runs-on: ubuntu-latest
    # A newer push cancels the run of the one before it.
    concurrency:
      group: proof-${{ github.event.pull_request.number || github.ref }}
      cancel-in-progress: true
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0
          persist-credentials: false
      - uses: fearlezz-dev/fearlezz@v0
```

The checkout needs `fetch-depth: 0`: the proof compares the change with its base branch. `edited` runs the proof again when the pull request is edited, since a title starting with `fix:` makes the change a fix that needs red before green, and another base is another comparison. Do not skip description edits with a job `if`: a required job that is skipped counts as passed. The action installs the release of its own tag (`version: v0.2.0` pins one), runs `fearlezz run --ci --proof <dir>`, and keeps the proof bundle (every check's output and every picture, before and after) and the check logs as artifacts of the run. Set `devenv: true` when the checks run in a devenv shell. A workflow can also run `fearlezz run --ci` without the action: when a check records stills or films and the runner has no ffmpeg, it installs a pinned static ffmpeg into the runner's tool cache.

On [Namespace](https://namespace.so) runners the same job runs with their labels in `runs-on`. Cache Nix with `namespacelabs/nscloud-cache-action` only on Linux runners: with `cache: nix` it fails on macOS runners, whose system root is read only.

```yaml
jobs:
  proof:
    runs-on: # your Namespace Linux runner label
    steps:
      - uses: namespacelabs/nscloud-cache-action@v1
        with:
          cache: nix
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0
          persist-credentials: false
      - uses: fearlezz-dev/fearlezz@v0
        with:
          devenv: true
```

Free CI only proves. Unless asked to open an issue (below), it writes nothing to GitHub: the job summary shows whether the commit is proven, each blocker with why and what to do, every check with its tests and time, the end of each failing output, and the changed pictures. Changed pictures never fail the proof and never wait; a person reviews them on the pull request, through GitHub's own review. Require the proof check on the base branch to hold merging to it.

A check too slow, costly or reliant on real infrastructure to run for each change (an end to end run against a real cluster, an eval against a paid model) gets `schedule = true`. No change needs it, and a second workflow proves the default branch with it on a schedule. Its one write to GitHub, asked for with `issue-on-failure`, opens an issue that links the failed run, or comments on the one still open:

```yaml
# .github/workflows/scheduled.yml
name: scheduled proof
on:
  schedule:
    - cron: "17 3 * * *"
  workflow_dispatch:
permissions:
  contents: read
  issues: write
jobs:
  proof:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0
          persist-credentials: false
      - uses: fearlezz-dev/fearlezz@v0
        with:
          scheduled: true
          issue-on-failure: true
        env:
          # When scheduled checks name secrets: SecretSpec reads them from 1Password with it.
          OP_SERVICE_ACCOUNT_TOKEN: ${{ secrets.OP_SERVICE_ACCOUNT_TOKEN }}
```

Putting an existing check on a schedule weakens the proof, so a person with admin rights merges that change.

A change that weakens the proof the base branch requires (a check turned off, another command, a lint lowered; see the floor below) fails the proof everywhere: locally, on any CI, and on every pull request, from the repository or a fork. When it must change, a person with admin rights reads why and merges past the required check.

`fearlezz run --ci --proof <dir>` writes everything a run showed as a folder: `proof.json`, every check's output and every picture with its comparison. Only the proof host draws a page from it. A repository that installs the Fearlezz GitHub App and grants its proof job `id-token: write` has CI send the bundle to the proof host (`[host] url` in `fearlezz.toml`, default `https://fearlezz.seamlezz.com`; its protocol is in `ee/README.md`), and the log and job summary link to the page. Both are needed: without the app or the permission, CI still proves the commit but sends nothing and shows no page. A job on GitHub Actions without `id-token: write` says so in its log and job summary, and `fearlezz setup login` says whether the app is installed on the repository.

The proof host is where hosted review happens. Its page of a work is live: it fills in as CI runs, with every changed picture compared with the base. People with write access say per picture that it looks right, or leave a note on its before, its after or both; an admin accepts what a change weakens there, which runs the proof job again. The host writes where review stands to the pull request as the `Fearlezz review` check run and one comment. An agent signs in with `fearlezz setup login` and reads the open notes, with their pictures, through `fearlezz --notes`. Once signed in, a local `fearlezz run` shows on the page too, marked unproven: it is the agent's word, so it never moves the review. How it works is in [`ee/README.md`](ee/README.md).

### Several platforms, several jobs

A check that must pass on more than one machine names its platforms, and Fearlezz splits the proof over jobs: one leg per platform, and per `job` a check names on it.

```toml
[platform.linux]
runs-on = "ubuntu-latest"
default = true                  # where a check without `on` runs

[platform.windows]
runs-on = "windows-latest"
os = "windows"                  # needs that OS; locally it is left to CI

[platform.py310]
runs-on = "ubuntu-latest"
matrix = { python = "3.10" }    # read by the leg's setup steps as matrix.python

[check.integration]
run = "..."
stage = "merge"
on = ["linux", "windows"]
job = "integration"             # its own leg on each platform: linux/integration, ...
timeout = "30m"                 # stopped and failed past it
```

Call the reusable workflow, and require its `gate` job:

```yaml
jobs:
  proof:
    uses: fearlezz-dev/fearlezz/.github/workflows/prove.yml@v0
    with:
      devenv: true
```

A leg that needs its own setup steps uses the action in the project's own three jobs:

```yaml
permissions:
  contents: read
  actions: read     # a pull request reuses passes from its base commit's proof
  id-token: write
jobs:
  plan:
    runs-on: ubuntu-latest
    outputs:
      legs: ${{ steps.plan.outputs.legs }}
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0
          persist-credentials: false
      - id: plan
        uses: fearlezz-dev/fearlezz@v0
        with:
          plan: true
  leg:
    needs: plan
    if: needs.plan.outputs.legs != '[]'
    name: leg ${{ matrix.leg }}
    strategy:
      fail-fast: false
      matrix:
        include: ${{ fromJSON(needs.plan.outputs.legs) }}
    runs-on: ${{ matrix.runs-on }}
    timeout-minutes: ${{ matrix.timeout-minutes }}
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0
          persist-credentials: false
      - if: matrix.python
        uses: actions/setup-python@v5
        with:
          python-version: ${{ matrix.python }}
      - uses: fearlezz-dev/fearlezz@v0
        with:
          leg: ${{ toJSON(matrix) }}
  gate:
    needs: [plan, leg]
    if: always()
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0
          persist-credentials: false
      - uses: fearlezz-dev/fearlezz@v0
        with:
          gate: true
```

The plan (`fearlezz run --ci --plan`) prints the legs for `matrix.include`. Each leg (`--leg`) runs its own checks and keeps its bundle. The gate (`--gate`) combines them into one proof with one job summary: a leg that failed, was cancelled or never ran leaves the commit not proven, and the gate alone judges what the change weakens. Dropping a platform from a check's `on`, or giving a platform another runner, weakens the proof.

A pull request reuses a check's pass from the proof its base commit's push kept, when the check read the same inputs on the same platform, so a change runs only the checks its inputs reach; the summary marks those `reused`. A push proves everything fresh, so a check whose `inputs` leave out a file it reads is caught on the base branch. Locally a check runs once, unless every platform it is on names another `os`.

## License

The code in `crates/` is available under either the [MIT](LICENSE-MIT) or the [Apache 2.0](LICENSE-APACHE) license, at your option.
