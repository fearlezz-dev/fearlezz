# Setting up Fearlezz

You are setting up Fearlezz in this repository for the developer. Fearlezz proves an agent's work with the checks the project declares in `fearlezz.toml`, on the developer's machine first and in CI at the end. The goal is a full switch: when you are done, Fearlezz runs what proves work here, and nothing runs twice.

Each step says what it is: **look** (read the repository), **ask** (the developer decides), **do** (you act), or **click** (only the developer can).

## Rules

- Look before asking. Ask only what the repository cannot answer.
- Ask about one topic at a time, and always give your recommendation with the question.
- Never invent checks the project does not already run. Ask before adding a tool it does not use yet.
- Never weaken a check to get green. Say what fails and why.
- Work on a branch, never on the default branch.

## 1. Learn the project (look)

Find out what it builds (a library, a service, a web or mobile app, infrastructure, several of these), how it is tested and released, and how its tools get installed: a package manager's lockfile, a version manager file, a container image, a Nix flake or devenv, a Makefile that downloads them. Read the README, the contributing guide and the build files.

## 2. Find what proves work today (look)

Every place that runs a command to judge a change:

- CI jobs, on any CI system: GitHub Actions, GitLab CI, CircleCI, Buildkite, Jenkins, Azure Pipelines.
- Git hooks: husky, lefthook, pre-commit, simple-git-hooks, a `.githooks` folder.
- Task runners: make, just, Taskfile, package.json scripts, a `scripts` folder.
- Test, lint, format and type check setups, and their configs.
- Screenshot, visual or snapshot tests, and recorded demos.
- Coverage uploads (Codecov, Coveralls) and their thresholds.
- Checklists in pull request templates.

Note the exact commands each one runs and where (the runner and its operating system show in the CI config; do not ask about them).

## 3. Setup kind (ask)

Ask which of these the developer wants, and recommend one:

- **With the proof host**: CI proves each pull request and sends the proof to a page where people review the pictures and leave notes. `fearlezz setup init --ci host`.
- **CI only**: CI proves each pull request, nothing is sent anywhere. `fearlezz setup init --ci plain`.
- **Local only**: agents prove their work on the developer's machine, CI stays as it is. `fearlezz setup init` without `--ci`.

`--ci` writes a GitHub Actions workflow. On another CI system, run `fearlezz setup init` and add one job to the existing pipeline that runs `fearlezz run --ci` on each pull request and push to the default branch, with the full git history fetched (the proof compares with the base branch).

## 4. Take over (ask)

List everything step 2 found, and for each say which Fearlezz check would take its place. Ask once whether to move each into Fearlezz and then remove the old one, so nothing runs twice. Recommend the full switch; name anything you would keep outside (a deploy, a release, a job that proves nothing) and why.

## 5. Init (do)

Run `fearlezz setup init` with the flag from step 3. It writes a starter `fearlezz.toml` from the languages it finds, with `could prove:` lines for step 9. Then shape each check to the commands the project really runs, from step 2:

- Turn a piece's check off with `off = true` when the project does not run it, and add the checks it does run that no piece brings.
- A check that needs two commands lists them as steps, `run = ["first", "second"]`, never `first && second`.
- Keep a check's `run` a plain tool command; Fearlezz reads the tool's own output, so no shell glue.
- When the project's tools come from a shell (devenv, nix develop, a container), set `[env] shell` so every check runs inside it.
- A check that needs a secret names it in `secrets`; never in `env` or a `.env` file.

Remove the systems the developer agreed to replace in step 4, in the same change.

## 6. Sync (do)

Run `fearlezz setup sync`. It writes the agent hooks, this project's copy of the Fearlezz skill and the tool settings files. Commit what it wrote.

## 7. Host (click, only with the proof host)

The developer does these; give them the links and wait:

- Install the Fearlezz GitHub App on the repository (the link `fearlezz setup init --ci host` printed).
- Run `fearlezz setup login`; it signs the CLI in and says whether the app is installed.
- The proof job needs the permission `id-token: write`. The workflow `--ci host` writes has it; add it when the job is your own.

## 8. First proof (do)

Run `fearlezz run` and follow its `next:` lines until it says `all proof is in: ready to merge`. When a check fails, read its tail and log, fix the cause (a wrong command, a missing tool, a real failure in the code) and run again. Never weaken a check to pass; when a check cannot pass, tell the developer what fails and why.

## 9. Grow (ask)

Offer what this project could prove next, one at a time, each as "We can now also do X. Want me to set that up?" Start from the `could prove:` lines `fearlezz setup init` printed (the `settings` check gives the same ones later). Pick only what fits:

- **Pictures**: stills of each screen of a UI, from its existing screenshot tests or new ones.
- **Films**: a recorded flow through the app.
- **Texts**: rendered output (manifests, charts, generated files) kept and compared with the base.
- **README media**: the pictures checks take, shown in the README.
- **Coverage**: changed lines no test ran, from a coverage report the tests already write.

## 10. Hand off (do)

Open a pull request from your branch and let CI prove it. Then summarise for the developer: what Fearlezz now runs, what it replaced, what you chose not to move and why, what they still have to do (the clicks of step 7, required checks in branch protection), and what they could grow next.
