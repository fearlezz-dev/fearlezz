# Spike: the hosted, paid parts

A design sketch, not a decision. It answers: what stays local and free, what is hosted, how the open CLI hands proof to a service without depending on it, and what to charge.

The mocks in this folder are self contained HTML (open them in a browser; light and dark follow the system):

| Page | What it shows |
| --- | --- |
| `proof.html` | The proof link of one pull request: verdict, every check, test counts, findings, stills and films before and after |
| `approve.html` | Approving changed pictures in the browser, and how the approval flows back honestly |
| `trends.html` | Trends over months for one repository |
| `pricing.html` | A pricing draft with the recommended tiers |

## What the market does (November 2026)

| Tool | Free tier | Billed by | Entry paid | Open source |
| --- | --- | --- | --- | --- |
| Chromatic | 5,000 snapshots a month | Snapshots | $179 a month, 35k | Free on request |
| Percy | 5,000 screenshots a month | Screenshots | Unverified | Unverified |
| Applitools | Trial only | Checkpoints | $667 a month, annual | Not offered |
| Argos | 5,000 screenshots, 30 days kept | Screenshots (a video counts as 25) | $100 a month, 35k | Core is open source |
| Lost Pixel | 7,000 screenshots a month | Screenshots | $100 a month, 40k | Free; product is being sunset |
| Happo | 5,000 snapshots a month | Snapshots | $149 a month, 50k | Free on request |
| Meticulous | Unverified | Quote | Unverified | Unverified |
| Codecov | 1 private user | PR authors | $12 per user (Pro) | Public repos free |
| Currents | Trial only | Test results | $49 a month, 10k | Not stated |
| Buildkite Test Engine | 250k executions, 24 hours kept | Users and executions | $30 per user | On request |
| Trunk Flaky Tests | 5 committers | Committers and spans | Unclear | Not stated |
| Datadog Test Optimization | None | Active committers | About $20 per committer | Not stated |
| Cypress Cloud | 500 results a month, no flake detection | Test results | $67 a month, 120k a year | Free for public non commercial |
| CodeRabbit | Public repos | PR authors | $24 per developer | Public repos free |
| Greptile | 50 credits | Credits per review | $1 a credit | Qualified projects free |

Sources: the vendors' pricing pages, read on 2026-11-11 (chromatic.com/pricing, argos-ci.com/pricing, lost-pixel.com/pricing, happo.io/pricing, applitools.com/pricing, about.codecov.io/pricing, currents.dev/pricing, buildkite.com/pricing, trunk.io/pricing, cypress.io/pricing, coderabbit.ai/pricing, greptile.com/pricing). No hosted service that proves agent work (rather than reviewing it) was found with public pricing.

Lessons:

- **Per picture and per result meters are the top complaint.** Bills grow with coverage, so teams record less. Films make it worse (Argos counts a video as 25 screenshots).
- **Free tiers that pause block CI mid month.** Never stop a proof because of a quota.
- **Baselines held by the service are lock in.** Fearlezz has no baselines: it compares with the base commit, so there is nothing to hold hostage.
- **Seats per committer punish agent heavy teams.** Agents open most pull requests; people approve.
- **Short history is the paywall lever** (Buildkite keeps 24 hours free, Cypress 30 days).
- **The upload path is an attack surface.** Codecov's tampered uploader script leaked CI secrets for months in 2021. Upload from the signed binary, with a short lived OIDC token, never a script or a stored secret.
- **Vendors disappear** (Lost Pixel). Everything must stay readable without the service.

## Local and free, hosted and paid

| Stays local, open core (`crates/`) | Hosted, paid (`ee/`) |
| --- | --- |
| CLI, checks, stages, journals, results store | Proof links per pull request and commit |
| Stills and films compared with the base commit | Pictures and films kept past CI's artifacts |
| Findings, test counts, floor | Approve pictures (and, later, weakenings) in the browser, signed |
| CI status comment and check page | Trends over months |
| The publish format and the upload client | Visibility, sign in, retention, audit export |
| Verifying a signed approval offline | |

The line: anything needed to **make** proof is free and local. Anything needed to **share** proof with people who were not there, keep it, or read it across months is hosted.

## Data model

Every entity reuses the IDs the CLI already has, so a hosted row always points back at something local.

- **Org**: `id`, `slug`, `plan`, `forge` (GitHub first).
- **Repo**: `id`, `org`, `forge_id`, `owner/name`, `public` (from the forge), `link_visibility` default.
- **Proof** (one `fearlezz run --ci`): `id` = hash of (`repo`, `head_commit`, `merge_tree`, `config_hash`); `pull_request`, `head_commit`, `base_commit`, `merge_tree` (the git tree id CI proved), `config_hash`, `floor_commit`, `verdict` (`proven`, `not_proven`, `waits`), `blockers[]` (the lines of the status comment), `fearlezz_version`, `started_at`, `duration_ms`, `ci_run_url`, `uploader` (OIDC subject).
- **CheckResult**: `result_id` (the store's `RunResult.id`), `inputs_key` (the store's `key`), `proof`, `check`, `stage`, `outcome`, `exit_code`, `duration_ms`, `tests`, `findings_elsewhere`, `log_tail`, `unstable`. Same fields as `RunResult`, minus local paths.
- **Finding**: `result_id`, `file`, `line`, `end_line`, `level`, `rule`, `message`, `fix`.
- **Media**: `sha256` (content addressed, stored once), `kind` (`still`, `film`), `bytes`, `width`, `height`, `duration_ms`.
- **Look**: `result_id`, `check`, `base_commit`, `base_result`, `items[]` of (`name`, `kind`, `change`, `summary`, `before`, `after` media hashes, `boxes`).
- **Approval**: `check`, `inputs_key`, `said`, `by` (forge user id and login), `via` (`browser` or `journal`), `at`, `signature`. Keyed by `inputs_key`, exactly like the journal's `Approve` event, so it holds while the inputs stay the same.
- **JournalSummary** (opt in, for time to proof only): `journal` (the branch name, as the journal is named), `first_event_at`, `ready_at`, counts of runs per stage. Never the notes or questions.
- **Rollup** (trends, kept apart from journals with the same IDs): `repo`, `check`, `day`, `runs`, `passed`, `failed`, `unstable`, `duration_p50`, `duration_p95`, `tests`, `approvals_browser`, `approvals_journal`, `time_to_proof_p50`. Small, so kept longest.

## Upload protocol

The core needs no `ee` code: it writes an open, documented bundle and speaks three plain HTTP calls any server can implement.

1. `fearlezz run --ci --publish` (or `FEARLEZZ_PUBLISH=1` in the action) writes `proof.json` (schema `fearlezz.proof/v1`, published with the CLI) next to the check page. It holds the Proof, its CheckResults, Findings and Looks, and lists media by `sha256`. `--publish <dir>` only writes the bundle; nothing leaves the machine.
2. `POST {endpoint}/v1/proofs` with the manifest and a GitHub OIDC token in `Authorization`. No stored secret. The answer names the media hashes it lacks, each with an upload URL.
3. `PUT` each missing file to its URL.
4. `POST {endpoint}/v1/proofs/{id}/seal`. The service answers with the proof link; CI adds it to the status comment.

`endpoint` defaults to the hosted service and can point at any server (`[publish] endpoint` in `fearlezz.toml`). The calls are idempotent by proof id, so a retried CI job uploads nothing twice. A failed upload never changes the verdict: proof is what CI saw, and publishing only shares it.

## Approvals that flow back honestly

1. A signed in person with write access approves on the page, with their own words. Agents cannot sign in.
2. The service records an Approval (`check`, `inputs_key`, `said`, `by`, `via: browser`) and signs it with a per repository key whose public half is pinned in `fearlezz.toml` (`[publish] approval_key`).
3. CI runs again (the service triggers a rerun). `fearlezz run --ci` fetches approvals for the inputs keys it saw and checks each signature itself. Nothing is trusted on the service's word.
4. The status comment says who approved and quotes them. The journal gains the approval on the agent's next `fearlezz`, marked `via: browser`, so the agent never has to relay words.
5. Nothing is committed to the branch: a file would change the tree and every input key, and an approval belongs to inputs, not commits. Approvals download as JSON and verify offline.

Relayed approvals (`fearlezz say approve --said`) stay as they are and count locally. In trends they show as relayed by an agent, apart from browser approvals.

## Visibility and retention

- **Public repository**: links public by default.
- **Private repository**: links need sign in with the forge, and access mirrors read access to the repository. A maintainer can make one link **unlisted** (an unguessable URL that expires after 30 days) to share with someone outside.
- **Retention**: media 30 days on Public (the store's `KEEP_DAYS`), 1 year on Team, 3 years on Business. Proof rows and findings follow media. Rollups outlive both. After retention the link says what was proven and when, without pictures.

## Tiers (recommended)

| Plan | Price | For |
| --- | --- | --- |
| Open core | $0 | Everything local and in CI, plus the publish format |
| Public | $0 | Public repositories: links, browser approvals, 30 days of media, 90 days of trends |
| Team | $12 per approver a month | Private repositories, private and unlisted links, signed approvals, 1 year of media, 2 years of trends |
| Business | $30 per approver a month | Approve weakened proof on its own page, single sign on, audit export, 3 years, self hosting of `ee/` |

An approver is a person who approved something in the browser that month. Agents, committers, viewers, CI runs, tests and pictures are free. Storage has a generous fair use line; going over never pauses CI.

## Open decisions

1. **Billing unit.** Approvers (recommended: matches the value, agents free, no meter on coverage), active committers (familiar, but punishes agents and humans who only review), or flat per repository (simple, but scales badly for monorepos).
2. **How approvals reach CI.** CI fetches signed approvals by inputs key (recommended: no commits, verified offline), an approval file committed to the branch (fully in git, but changes every input key and needs a rerun loop), or a forge status check posted by the service (simple, but trusts the service and cannot be verified offline).
3. **What CI does with changed pictures.** `--ci` keeps an in memory journal, so no approval can reach it today (how it treats changed pictures in CI was not checked in this spike). Options: CI counts only signed approvals (recommended, honest), CI ignores pictures and leaves them to the local final stage (today's behavior, weaker), or a commit trailer with the words (in git, but anyone can type it).
4. **Journal data on the service.** None, only rollups from CI (most private, but no time to proof), an opt in summary of timestamps (recommended), or full journals (richest, but uploads the agent's notes and leaks intent).
5. **Where the publish client lives.** In `crates/` as an open feature behind `--publish` (recommended: core stays whole, anyone can self host), or a separate binary under `ee/` (cleaner boundary, but one more thing to install).
6. **Public links for private repositories.** Never (safest), unlisted with expiry by a maintainer (recommended), or public on request (simplest, easy to leak).
7. **Self hosting the paid part.** Business only under a source available license (recommended), fully open (no moat), or never (blocks regulated customers).
8. **Free for open source.** Public plan for every public repository (recommended, as Codecov and CodeRabbit do), or on request (as Chromatic and Happo do: friction for no gain).
