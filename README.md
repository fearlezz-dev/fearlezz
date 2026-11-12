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

In a project, `fearlezz setup init` writes a starter `fearlezz.toml` from the languages it finds; `fearlezz setup sync` then writes the agent skill, hooks and lint settings.

Prebuilt binaries for Linux and macOS are on the [releases](https://github.com/fearlezz-dev/fearlezz/releases) page. Stills and films need `ffmpeg`, and code rules need `ast-grep`.

To work on Fearlezz itself, enter the toolchain with `devenv shell`, then run `check`.

Agents: `fearlezz setup sync` writes [the skill](skills/fearlezz/SKILL.md) into your project, matching the installed version, for Claude Code and any agent that reads `.agents/skills`.

## CI in your project

Two workflows. The first proves every pull request and push:

```yaml
# .github/workflows/proof.yml
name: proof
on:
  pull_request:
    types: [opened, synchronize, reopened, edited]
  push:
    branches: [main]
permissions:
  contents: write
  pull-requests: write
  statuses: write
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

The checkout needs `fetch-depth: 0`: the proof compares the change with its base branch. `edited` runs the proof again when the pull request is edited, since a title starting with `fix:` makes the change a fix that needs red before green, and another base is another comparison. Do not skip description edits with a job `if`: a required job that is skipped counts as passed. The action installs the release of its own tag (`version: v0.2.0` pins one), runs `fearlezz run --ci` and uploads the check logs. Set `devenv: true` when the checks run in a devenv shell. On GitHub Actions with a token (`GITHUB_TOKEN`), `fearlezz run --ci` does what the event asks through GitHub's API, so a workflow can also run it without the action: it keeps one status comment on the pull request, and when a check records stills or films and the runner has no ffmpeg, it installs a pinned static ffmpeg into the runner's tool cache. The token is the workflow's or a GitHub App's; Fearlezz never takes a person's comment or status for its own. A run that finishes after a newer push leaves that push's comment and pictures alone.

On another CI, `fearlezz run --ci --comment status.md` writes the same status comment to a file, for that CI to post on its merge request.

Changed pictures show on the pull request, before and after, from the `fearlezz-pictures` branch (one folder per pull request; in a private repository only people who can read it see them). Changed pictures do not fail the proof: they set the `fearlezz/pictures` status to pending, and the comment says they need a person's approval. Whether they have it is the status alone: an approval sets the status and a reply, never the comment.

To make merging wait for a person, require the `fearlezz/pictures` status next to the proof check, with **GitHub Actions as its expected source** (a ruleset's required status checks can name one). Without that source, anyone who can write to the repository could set the status by hand.

The second workflow answers approvals and keeps the pictures branch small:

```yaml
# .github/workflows/pictures.yml
name: pictures
on:
  issue_comment:
    types: [created, edited, deleted]
  pull_request:
    types: [closed]
  schedule:
    - cron: "0 4 * * 1"
permissions:
  contents: write
  pull-requests: write
  statuses: write
jobs:
  pictures:
    if: >-
      github.event_name != 'issue_comment'
      || startsWith(github.event.comment.body, '/fearlezz approve')
      || startsWith(github.event.changes.body.from, '/fearlezz approve')
    runs-on: ubuntu-latest
    steps:
      - uses: fearlezz-dev/fearlezz@v0
```

A person with write access approves the pictures by commenting `/fearlezz approve <what they think of them>`. The approval covers what the status comment showed when it was written: the checks it names, at the pull request's current head. A comment written before the proof of the current head showed its pictures does not count, so a push between looking and approving needs a new approval. Whenever such a comment is written, edited or deleted, CI reads the pull request again and applies one rule: `fearlezz/pictures` is green, linked to the newest comment that approves, or pending when none does; it reads everything again after setting the status, so events that arrive late or out of order settle the same way. It replies quoting the words. Only the proving run writes the status comment; pictures other than those it showed come in a new comment, so when it was written is when they were shown. **Deleting the approval comment, or editing it so it no longer approves, takes the approval back.** A new run keeps the approval only for the same head, the same base commit and the same pictures (stills by their bytes; films by which changed, since two recordings of one film never match byte for byte), and waits again otherwise. When the status comment is gone or broken, what an approval covered can no longer be told, so a green status it set goes back to pending. Closing a pull request removes its pictures, and the schedule removes those of pull requests closed without it; both squash the branch into one commit in a single atomic update, so a push in between is never lost.

On the developer's machine, `fearlezz say approve <check> --github` records the approval in the journal. It reads the pull request with the developer's token (`GITHUB_TOKEN`, or the GitHub CLI's `gh auth token`) and counts an approval by the same rule CI uses: the status comment, written by CI rather than a person's account, shows the pictures of `<check>` waiting at the commit checked out, and a `/fearlezz approve` comment written after it showed them still exists and was written by a person with write access. A check whose pictures did not wait is refused, since nobody looked at them.

### Pull requests from forks

A fork's run gets a read only token, so it writes nothing to the pull request: its status is on the check page and its changed pictures in the run's `fearlezz-pictures` artifact. A person with write access looks at them and comments `/fearlezz approve <head> <words>`, naming the head commit of the run they looked at (its first seven characters are enough); the pictures workflow runs with the repository's own token and sets `fearlezz/pictures` green on that head only while it is the fork's current head, so a push after looking needs a new approval. A fork's approval covers every check of that head. So a fork's pull request always needs that comment before it can merge, even without changed pictures.

### What the approval guarantees

Fearlezz cannot tell a person from an agent that uses the same GitHub account: an agent signed in as the developer could post `/fearlezz approve` itself. The agent hooks refuse the obvious ways (a `gh`, `curl` or similar command carrying it), but they are a speed bump, not the guarantee. An agent with its own account and no write access (it pushes to a fork) can never approve. Review changes to `.github/workflows` too, since a pull request runs its own version of them.

## License

The code in `crates/` is available under either the [MIT](LICENSE-MIT) or the [Apache 2.0](LICENSE-APACHE) license, at your option.
