# Setting up Fearlezz

You are setting up Fearlezz in this repository for the developer. Fearlezz proves an agent's work with the checks the project declares in `fearlezz.toml`, on the developer's machine first and in CI at the end. The goal is a full switch: when you are done, Fearlezz runs what proves work here, and nothing runs twice.

Each step says what it is: **look** (read the repository), **ask** (the developer decides), **do** (you act), or **click** (only the developer can).

## Rules

- Look before asking. Ask only what the repository cannot answer.
- Ask about one topic at a time, and always give your recommendation with the question.
- Never invent checks the project does not already run.
- The tools a Fearlezz piece adds (for example zizmor from `actions`, gitleaks from `safety`) are Fearlezz's defaults: keep them, install them the way the project installs its tools, and fix what they find. Ask before adding any tool beyond those.
- Never weaken a check to get green. Say what fails and why.
- Never turn off or loosen a lint, a rule or a check's level without asking first. Show the developer the finding, its cause and the options (fix the code, change the config, keep it as it is), and say when it looks like a bug in Fearlezz, so they can report it.
- Never drop an opportunity Fearlezz names (a `could prove:` line, a hint of the `settings` check). Each one becomes a question to the developer (step 9).
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
- **Local only**: agents prove their work on the developer's machine, CI stays as it is. `fearlezz setup init` without `--ci`; when its `next:` line asks about CI, step 3 has already answered it.

The proof host takes proof from GitHub Actions only (through the Fearlezz GitHub App and GitHub's OIDC token), so offer it only for a repository on GitHub whose CI runs there.

`--ci` writes a GitHub Actions workflow. On another CI system, run `fearlezz setup init` and add one job to the existing pipeline that runs `fearlezz run --ci` on each pull request and push to the default branch, with the full git history fetched (the proof compares with the base branch).

## 4. Take over (ask)

Ask this only after the developer has answered step 3, as its own question: one topic per question, so never fold the setup kind and the take over into one.

List everything step 2 found, and for each say which Fearlezz check would take its place. Ask once whether to move each into Fearlezz and then remove the old one, so nothing runs twice. Recommend the full switch; name anything you would keep outside (a deploy, a release, a job that proves nothing) and why.

## 5. Init (do)

Run `fearlezz setup init` with the flag from step 3. It writes a starter `fearlezz.toml` from the languages it finds, with `could prove:` lines for step 9. Then shape each check to the commands the project really runs, from step 2:

- See what a piece sets with `fearlezz setup show <piece>` (such as `rust` or `devenv`), and a check as it resolves with `fearlezz setup show <check>`; never read Fearlezz's source for it.
- Keep the checks the pieces bring, also those whose tool the project does not use yet (see the rules). Turn one off with `off = true` only when it cannot apply here (zizmor in a repository without workflows) or the developer agrees. Add the checks the project does run that no piece brings.
- A check that needs two commands lists them as steps, `run = ["first", "second"]`, never `first && second`.
- Keep a check's `run` a plain tool command; Fearlezz reads the tool's own output, so no shell glue.
- When the project's tools come from a shell (devenv, nix develop, a container), set `[env] shell` so every check runs inside it.
- A check that needs a secret names it in `secrets`; never in `env` or a `.env` file.

Remove the systems the developer agreed to replace in step 4, in the same change.

## 6. Sync (do)

Run `fearlezz setup sync`. It writes the agent hooks, this project's copy of the Fearlezz skill and the tool settings files. Commit what it wrote.

## 7. Host (do, then click; only with the proof host)

- **Sign in (do).** Run `fearlezz setup login` yourself; do not hand it to the developer. It uses GitHub's device flow: it prints a link and a code, so give the developer both, they enter the code, and the command finishes on its own. The sign in is per user, not per checkout, so when they are already signed in it is done.
- **Install the app (click).** Read what the login says about the Fearlezz GitHub App. When it is not installed on the repository, give the developer only its install link to click, and wait.
- **Permission (do).** The proof job needs `id-token: write`. The workflow `--ci host` writes has it; add it when the job is your own.
- **Required check (ask).** Recommend making `proof` a required check in the default branch's protection, so no pull request merges without it. Change branch protection only with the developer's OK; otherwise name it as theirs to do in the hand off.

## 8. First proof (do)

The first full run can take a long time: nothing is cached yet, so every check builds and runs from scratch. Give the command a generous time budget, or run checks one by one by name (`fearlezz run <check>`), so no tool timeout kills it halfway.

Run `fearlezz run` and follow its `next:` lines until it says `all proof is in: ready to merge`. When a check fails, read its tail and log, fix the cause (a wrong command, a missing tool, a real failure in the code) and run again. Never weaken a check to pass; when a check cannot pass, tell the developer what fails and why.

## 9. Grow (ask, never skip)

This step is required. Put every opportunity Fearlezz named in front of the developer: each `could prove:` line `fearlezz setup init` printed, and each hint the `settings` check gave since. Ask about them one at a time, each as "We can now also do X. Want me to set that up?", with your recommendation. Never drop one silently; when there were none, say so in the hand off. Beyond those lines, offer only what fits:

- **Pictures**: stills of each screen of a UI, from its existing screenshot tests or new ones.
- **Films**: a recorded flow through the app.
- **Texts**: rendered output (manifests, charts, generated files) kept and compared with the base.
- **README media**: the pictures checks take, shown in the README.
- **Coverage**: changed lines no test ran, from a coverage report the tests already write.

## 10. Hand off (do)

Open a pull request from your branch and let CI prove it. Then summarise for the developer: what Fearlezz now runs, the tools its pieces added, what it replaced, what you chose not to move and why, what they still have to do (the clicks of step 7, `proof` as a required check when they did not OK it), each opportunity of step 9 with the developer's answer (or that there were none), and any lint or rule they agreed to loosen and why.
