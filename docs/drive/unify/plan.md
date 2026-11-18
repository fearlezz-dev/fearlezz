# Unify Fearlezz

The goal, in the developer's words: "go over all of the code ... I have a suspicion that there are like eight different ways to somewhat express the same thing, or eight different subsystems. We should unify stuff, simplify things, and massively reduce the complexity. We still want to have all of the features, but expressed in much more similar ways." Plus a code health pass.

This plan is the Phase A result: every module on `main` at `1b1c6fe` (crates and `ee/`) and the open feature pull requests (#32, #35, #36, #37, #38, #39, #40, #42, #45) were read in full. Phase B carries it out after the feature pull requests merge.

## The answer

The suspicion holds. Fearlezz has about **twelve** ideas that are each written two to five ways. Most come from features built in parallel, each adding its own small version of a concept the core already had.

The fix is **seven shared models** in the core, two in `ee/`, and a plumbing layer under them:

| # | Model | Replaces | Rough lines removed |
| --- | --- | --- | --- |
| 1 | **Plumbing**: one process builder, one lease, one set of file helpers | 5 clean env builders, 3 curl spawns, 3 git runners, 6 sha256 spellings, 5 lock styles | 350 |
| 2 | **Why**: one typed reason a check did not pass | `not_found`, `missing_secret`, `no_preview`, `proof_errors[0]`, `generated`, `print_failure` and `failure_item` | 250 |
| 3 | **Readers**: one way to read what a tool wrote | `findings::Reader`, `counts::Reader`, `coverage::Reader`, picture and text globs, built in checks writing jsonl to read it back | 450 |
| 4 | **Base**: one session against the base commit | 3 base checkouts per command, 4 copies of "the result for these inputs", look and red and preview each on their own | 250 |
| 5 | **Change**: one typed weakening, rendered once | 4 return shapes, 4 renderers, 3 list diffs | 200 |
| 6 | **Needs**: everything a check needs before it runs | services, preview, envtest, iOS simulator, Linux display, secrets (2 kinds), heavy queue slot | 350 |
| 7 | **Check**: one typed check, placed and timed | `Raw*` mirrors, `set_aside`, `left_to_ci`, `light`, release dry checks injected by hand, platforms beside screens | 350 |
| 8 | **One run loop**: selection, runner, one report | `run_checks` (264 lines in `main.rs`), `ci_gate`, `release_run`, the by hand command, three not run paths | 450 |
| 9 | **Tools and TOML**: one tool registry, one level, one TOML tree helper | 2 `Tool` enums, 2 `Level` enums, 2 flatten and diff walkers, 5 "no X; X are" lookups | 150 |
| 10 | **Screens**: one model of where an app runs and how it is filmed | `Need::Ios` and `Target::Ios` and `Place::Own`, 3 stop semantics, 3 rectangle encodings | 150 |
| 11 | **Host viewer** (`ee/host`): one identity, one policy | 2 person gates, 5 visibility functions, 8 run row shapes, 3 R2 key builders | 400 |
| 12 | **Page shows what the host decided** (`ee/app`) | review wording, picture standing and listing words decided again in TypeScript, 3 diff algorithms | 200 |

Together about **3,500 lines** of about 25,000 (core, host and page source after the feature pull requests), with every feature and every config key kept. Estimates come from the readers' per pair counts and are rough; each pull request reports its real number.

## What every pull request keeps

- **Every feature and every config key.** No config break is planned. Where two keys now say one thing (`device` and `needs = ["ios"]`), both stay and map to one model.
- **Every line a person or agent reads**, word for word, unless a test pins the new wording in the same pull request. The integration tests in `crates/fearlezz/tests/` pin most of it; they must pass unchanged except where a pull request names the line it changes.
- **`proof.json` and protocol version 2.** Fields stay; new typed fields are only added. The host reads bundles from older CLIs.
- **The store and journals** on developers' machines. A store format change reads the old format, or the version bump that invalidates results is named in the pull request.
- **Free and paid.** Nothing in `crates/` depends on `ee/`.

## The models

Each section: what is duplicated now (with where), the one model, what goes, and what to watch.

### 1. Plumbing

**Now.**
- A clean environment for a command is built five ways: checks (`run.rs:163-195`), envtest (`run.rs:381-403`), services (`env.rs:150-167`), #35's secret fetch, #36's `clean_command`. `env.rs` never applies `[env] shell`.
- Spawning with output and a time limit: `host.rs:417-535` (curl twice), `action.rs:475-504` (ffmpeg download), `media.rs:246-288, 453-468, 1138-1156`, `record/place.rs:151-197`, git in `git.rs:37`, `git.rs:459`, `presets.rs:743`.
- Kill and wait until a deadline: six loops (`record.rs:903-1125`, `place.rs:173-186`, `macos.rs:79-83, 477-483`). #40 adds `timeout` beside `record.seconds` in the same `spawn`, and skips the timeout for record checks.
- File locks: services `env.rs:171-193`, base holds `git.rs:236-299, 437`, prune `git.rs:341`, #45 queue slots, #38 simulator pool by raw `libc::flock` under `~/Library/Caches` instead of `git::cache_dir`.
- Small helpers: sha256 hex six times, recursive walkers three times (`counts.rs:161`, `look.rs:163`, #36 `release.rs:716`), path normalizers three times (`config.rs:946`, `presets.rs:723`, `settings.rs:818`), jsonl readers with three error policies (`run.rs:491`, `journal.rs:72`, `store.rs:273`), `config_dir` and `cache_dir` (`git.rs:398-434`), two daily prune markers (`store.rs:298`, `git.rs:341`), three write strategies (`presets.rs:312`, `presets.rs:479`, `settings.rs:384`).

**Model.**
- `process.rs`: `Cmd::clean(dir)` with `.vars(names)`, `.shell(prefix)`, `.within(budget)`, `.output()` or `.stream(log)`, kill by process group at the budget. Every spawn goes through it: checks, services, preview, envtest, secrets, release, curl, git, ffmpeg, simctl, Xvfb.
- `lease.rs`: one `Lease` over std `File::lock`: exclusive, shared, or one of N slots (polled, with a holder record), under one cache root. Services, base holds, prune, the heavy queue and the simulator pool use it.
- `files.rs` (core): `sha256_hex`, `walk`, `normalize`, `read_jsonl`, `write_atomic`, `daily(marker)`.

**Goes.** About 350 lines. #40's `timeout` and `record.seconds` become one budget.

**Watch.** The clean env is a security boundary (no inherited tokens). One builder makes it one place to get right; its tests move with it. Secret fetch must still inherit what SecretSpec needs, as an explicit list.

### 2. Why

**Now.** Why a check did not pass is spread over: exit code 127 read back by `RunResult::not_found` (`store.rs:64`), `missing_secret` (#35), `no_preview` (#42), a coverage error pushed to `proof_errors[0]` with exit 0 (#32), `generated` (#37), gate findings, zero tests, failed tests, flaky tests. Each is turned into words twice: terminal `print_failure` (`main.rs:725-754`) and proof `failure_item` (`main.rs:1082-1122`), and `red::Unshown` beside `proof::Verdict` (`red.rs:204`, `main.rs:1427`). `store::Outcome` and `fearlezz_proof::Ran` are the same two variants (`proof.rs:28`).

**Model.** In `fearlezz-proof`: `Outcome = Passed | Failed(Why) | NotRun(Why)` with `Why` a closed enum (`Exit(code)`, `Signal`, `MissingTool(name)`, `MissingSecret(name)`, `NoPreview(reason)`, `NoTests`, `FailedTests`, `FlakyTests`, `GateFindings`, `NoCoverage`, `StaleGenerated`, `CannotRecord`, `NotOnThisMachine(platform)`, `Scheduled`). One `Why::item() -> Item { what, why, fix }`. The terminal prints the item; the summary and host show the same item.

**Goes.** About 250 lines; the four "never ran" channels become one.

**Watch.** `proof.json` keeps `failure` as it is (the item) and adds `why` as a typed field. The store keeps reading old results (an old result without `why` derives it as today).

### 3. Readers

**Now.** Three `Reader` types with three TOML shorthands and argument bundles: `findings::Reader` (`findings.rs`, seven hand written severity and `anywhere` matches, `findings.rs:331-661`), `counts::Reader` (`counts.rs`), #32's `coverage::Reader` (copies counts' glob code and "no report" text). Pictures are a fourth (`MediaGlob`, `proof.jsonl` parsed as untyped JSON at `look.rs:183` and `run.rs:491`), #37's texts a fifth. Where output is read from (stdout, `$FEARLEZZ_OUT`, a report glob) is decided in four places (`findings.rs:89-99`, `counts.rs:96-143`). Report paths become repository paths four ways (`findings.rs:107, 302, 524, 679`). The `Finding` literal is written about 12 times. Built in checks (`rules`, `commits`, `settings`) write `findings.jsonl` and the jsonl reader reads it back (`run.rs:453-488`). JUnit XML is parsed by hand (`counts.rs:215-303`).

**Model.** One `Reader { source: Source, format: Format }` with `Source = Stdout | Out(file) | Glob(pattern)` and one `Format` table (cargo, sarif, eslint, jsonl, junit, pattern, lcov, jacoco, go cover, pictures, texts, ...). Each format yields typed evidence (`Finding`, `TestCase`, `CoverageLine`, `Picture`, `Text`) through one path resolver and `Finding::new`. A check's `findings`, `tests`, `coverage`, `stills`, `films`, `texts` keys each build a `Reader`; config is unchanged. Built in checks return findings in process. `generates` (#37) is a reader of the check's own folder before and after.

**Goes.** About 450 lines.

**Watch.** Readers decide levels and what blocks. Every reader test stays; the severity tables become data next to each format so a reviewer reads one table, not seven matches.

### 4. Base

**Now.** `look::Base::at` is the one place base checkouts are made, but each user opens its own: look once per command (`main.rs:1464`), red twice per check (`red.rs:81`, `red.rs:109`), floor and red reread the base config separately (`floor.rs:37`, `red.rs:135`). "The result for these inputs" is looked up four ways with different rules: `run_checks` honors `unstable` and the final stage (`main.rs:1221`), look and red do not (`look.rs:227`, `red.rs:153`), #35 adds `latest_that_ran`, #45 adds a fifth with another outcome rule. #42 deploys a preview of the base inside `look.rs`; #40 reuses passes from the base CI bundle beside the local store. `Base` lives in `look.rs` though look is only one user.

**Model.** `base.rs`: one `BaseSession` per command, opened on first use, holding the base checkout, its config and its environment. `look`, `red`, `texts`, `preview` and the floor ask it. One `Store::reusable(inputs, Policy)` answers "is there a result for these inputs" for every caller, with the policy explicit (pass only, final stage, unstable). `AgainstBase` keeps every comparison of a run (look, red) in one record, as now. #40's base bundle becomes a second source behind the same `reusable`.

**Goes.** About 250 lines, and one base checkout per command instead of up to three per check.

**Watch.** Red before green lays test files over the base; it needs its own overlay checkout. The session hands out a plain view and an overlay view; it must not share the overlay with look.

### 5. Change

**Now.** "How this part weakened" has four shapes: `Vec<String>` (`config.rs:191` pictures, `config.rs:221` record), `(path, how)` (`settings.rs:227`, `lints.rs:190`, `rules.rs:179`), `Widening` (`config.rs:849`, re-rendered in `floor.rs:222`), and `Part` (`floor.rs:129`). The `+x/-x` list diff is written three times (`floor.rs:190, 277`, `config.rs:839`); "it changes from / is added / is removed" twice (`settings.rs:242`, `floor.rs:263`). A weakening is rendered in four places (`floor.rs:73-98`, `action.rs:156-287`) and again by the page. Every new feature adds its own lines: #32 coverage, #35 secrets, #37 generates, #38 screen, #39 schedule, #40 platforms.

**Model.** In `fearlezz-proof`: `Change = Removed(what) | Added(what) | Narrowed { what, from, to } | Widened { what, from, to } | Moved { what, from, to }`, one `names_diff(base, head)`, one renderer. Each part (check fields, env, settings, lints, rules, pictures, record) returns `Vec<Change>`; `floor` collects them into `Weakening`.

**Goes.** About 200 lines; a new feature adds one match arm, not a renderer.

**Watch.** Weakening text is pinned by `ci.rs` and shown on the host; keep it exact.

### 6. Needs

**Now.** Things a check needs before its command runs, each with its own lifecycle code:

| Need | Where now | Lifecycle |
| --- | --- | --- |
| services | `needs = ["services"]`, `env.rs:264` `ensure_up`, idle stop | per worktree, idle stop |
| envtest | `needs = ["envtest"]`, `run.rs:356` | per go.mod version, cached |
| preview (#42) | `needs = ["preview"]`, `env.rs` +232 | per tree, never stopped |
| iOS simulator (#38) | `needs = ["ios"]` and `device`, `simulator.rs` | pooled lease, `Drop` |
| Linux display (#38) | `needs = ["linux"]` and `display`, `display.rs` | per run, `Drop` |
| check secrets (#35) | `secrets = [...]`, `run.rs:314-364` | per run, masked |
| env secrets (#42) | `[env] secrets`, `NoPreview::Secret` | per deploy |
| heavy slot (#45) | `FEARLEZZ_QUEUE`, `light = true` | lease of one of N |

Env reaches the command two ways (`Where.vars` and direct `cmd.env` in #38's `spawn`). Failures are typed three ways (`NoPreview`, `Unfetched`, exit 127).

**Model.** `Need` is the one list: `Services | Preview | Envtest | Screen(Screen) | Secret(name) | Slot`. Each need is a `Resource` with `acquire(&Checkout) -> Granted { vars, lines }` and a lifetime (`Run`, `Tree`, `Worktree` with idle stop, `Pool`). The runner acquires a check's needs in one fixed order (secrets, slot, services, preview, screen) through one function, so lock order is written once. Granted vars are the one channel into the command. A failed acquire is a `Why`. Secrets are one fetch and one mask for both check and env secrets; masking applies to every run's files.

**Goes.** About 350 lines; preview stops being a copy of the services block in `run_checks` and `look.rs`.

**Watch.**
- #45 holds the services lease while waiting for a slot (a second run in the worktree then waits silently). One acquire order (slot before services) fixes it by construction.
- #45's `light` is not honored for picture comparison. In the model, comparison takes a `Slot` only when the check does.
- `light` stays as a key; built in checks are light by being built in, so the second bit goes.

### 7. Check

**Now.** `RawCheck` and `Check` mirror each other field for field (`config.rs:104-138`, `386-415`) and are copied by hand in the 216 line `from_table`. Derived values are stored beside their source (`run` and `steps`, `test_files` and `test_globs`, `services` rebuilt from `Env`). Which checks run now depends on five mechanisms: `stage`, `off` (dropped from the TOML), `schedule` (#39 moves checks to `set_aside` and swaps lists in `on_schedule`, so `checks` means different things per run and `every_check` exists), release dry checks (#36 writes them into the raw TOML), and platform (#40 `on` and `job`, and #38 `left_to_ci` derived from the screen). `each` fans a check out by folder; #40 fans out by platform and job.

**Model.**
- `Check` deserializes directly into typed fields where serde can (newtypes for globs, stage names checked after), with derived values computed, not stored.
- `Run = Steps(Vec<String>) | Builtin(Builtin) | Release(Place)`.
- `When = Stage(Stage) | Scheduled | Release`. One list of checks; a `Selection` (below) picks.
- `Placement { folder, platforms, job }`: `each` fills `folder`, `on` fills `platforms`, and needs add to them (`Screen::Ios` and a window recording mean macOS, `Screen::Linux` means Linux). `left_to_ci` becomes "this machine is not one of its platforms".
- One expansion pass over the merged table: pieces, `off`, `each`, release places, built ins.

**Goes.** About 350 lines; `set_aside`, `on_schedule`, `every_check`, `left_to_ci`, the hand copies.

**Watch.** Check ids (`tests/crates`, `release/npm`) are keys in the store and on the host; they must not change. `deny_unknown_fields` errors must stay as helpful.

### 8. One run loop

**Now.** `run_checks` in `main.rs:1126-1384` mixes reuse, recording availability, prepare, services, execute, journal, look, report, red, floor and next lines. #40's `ci_gate` rewrites `Report::finish`; #36's `release_run` and #45's by hand command run commands their own way. A check is "not run here" by three paths (record unavailable, #38 left to CI, #40 not in this leg). The same facts are rendered for the terminal in `main.rs`, the job summary in `action.rs`, `proof.json`, host events and hooks; a run's local view (`state.rs` `CheckState`) and its proof (`proof::Check`) are two models of one result. `Report.ci`, `CiOut` and `Journal::in_memory` are three ways to say "CI" (`main.rs:793, 322, 1138`). The host open is built twice (`main.rs:861` and `1034`).

**Model.**
- `Selection = Stage(stage) | Ids(..) | Leg(leg) | Scheduled | Release | Hand(command)`: one function turns it and the config into a `Plan` (what runs, what is not run here and why).
- `Runner` (in `run.rs`) walks the plan: reuse, acquire needs, execute, read evidence, compare with base, append to the journal. It emits events to sinks.
- The report always builds a `proof::Proof`, local runs too. Sinks render it: terminal lines, job summary, host stream, bundle. The gate of #40 is the same report over legs' bundles.
- `Mode = Local(journal) | Ci(workflow)` replaces the three CI flags.

**Goes.** About 450 lines; `main.rs` becomes the CLI only, as `AGENTS.md` says.

**Watch.** This touches the most behavior. It goes last, after models 2, 6 and 7 are in, and is split into two pull requests (plan and runner, then report and sinks).

### 9. Tools and TOML

**Now.** Two `Tool` enums share nine names (`settings.rs:28`, `lints.rs:55`, joined by `LINT_FILES`), two `Level` enums (`lints.rs:21`, `fearlezz-proof` findings), two dart severity maps (`settings.rs:641`, `1216`). Flatten to dotted keys and diff is written twice (`settings.rs:505`, `presets.rs:338`); "no X; X are ..." lookups five times; hand kept name tables for embedded files four times (`presets.rs:23`, `settings.rs:68, 1095, 1120`). `Lock`, `LockFile`, `LockEntry` and `Origin` and `Source` are two shapes each (`presets.rs:278-387`). Golangci writers skip the merge every other writer does (`settings.rs:331`).

**Model.** One `Tool` registry row per tool (name, settings file and format, lint table, findings format). `Level` lives once in `fearlezz-proof`, `Off` as `Option<Level>`. `toml_tree` helpers (flatten, diff, find by path) used by presets, settings, floor and `Config::part`. One `lookup(kind, name, table)` error.

**Goes.** About 150 lines.

### 10. Screens

**Now.** `record.rs` is 2,072 lines. #38 adds the iOS simulator and a Linux display and states the screen kind three times (`Need::Ios`, `Target::Ios`, `Place::Own`), reads `crop_top` twice, and gives three `Filming` implementations three stop semantics. The `Screen` trait mixes window manager work (park, front, put back) with recording, so two of three screens stub it out. Native windows are listed two ways on macOS (`macos.rs:142`, `macos.rs:223`). Rectangles have three encodings (`Box4`, `CGRect`, region fractions).

**Model.** `Screen = Window | Simulator { device } | Display { size }` is the one fact (a `Need`, model 6). Each screen gives a `Recorder` (still, film, stop by one `Budget`). Parking stays a macOS window concern beside the window recorder, not in the trait. One `Rect` in `fearlezz-proof`.

**Goes.** About 150 lines, and `record.rs` splits by screen.

**Watch.** Recording is proven only on the developer's machine. Keep the drive record's manual checks (`docs/drive/off-screen-recording.md`) and run them once after this pull request.

### 11. Host viewer (`ee/host`)

**Now.** Who is asking is decided by two `person` gates (`say.rs:48`, `cli.rs:160`) over four identity structs; whether they may see is decided by five functions (`pages.rs:23-106`, `room.rs:174`, `cli.rs:222`) and two permission caches with their own keys (`say.rs:179`, `pages.rs:84`); "open then require" is written five times. Eight row structs project runs. Local runs are told apart by a `local:` ref prefix in some places and the source kind in others. The R2 key is built in three places; files are served twice (`files.rs:92`, `cli.rs:353`).

**Model.** `Viewer = Anonymous | Share(work) | Person { login, via: Session | Cli }` resolved once per request; one `can(viewer, Action, repo) -> Result<(), Refusal>` with one cached permission lookup. One `RunRow`, one `blob_key`, one file response, local by source kind only.

**Goes.** About 400 lines.

**Watch.** This is the security boundary of the host. Every existing host test (`ee/host-tests`) stays; add one per action and viewer kind before changing it.

### 12. The page shows what the host decided (`ee/app`)

**Now.** The page decides again in TypeScript what `fearlezz-proof` already decides: review wording (`review.ts:58-81`, `App.tsx:150-215`, already different: "Proven at ..., with nothing to review" against "Proved ..."), picture standing (`review.ts:33-49`), the listing's words (`repository.ts:41-60`, no Rust side). Some TS types mirror Rust by hand. #37 adds a third line diff (`diff.ts`, capped at 4M cells, so it can disagree with `similar` in the CLI).

**Model.** The host sends the derived `Review` (state, title, each picture's standing) with each change, and the listing's word with each row; the page shows them. Every TS type is generated by ts-rs. The CLI writes each text's diff with `similar` into the bundle; the page renders it.

**Goes.** About 200 TS lines.

## Health

Besides the models, the readers found these. Each goes into the pull request of the model it touches, or the health pull request.

- **Long functions**: `run_checks` (264 lines, model 8), `Config::from_table` (216, model 7), `EnvState::preview` (#42, 105, model 6), #38's `spawn` (101, models 1 and 6).
- **Hand written parsers**: JUnit XML (`counts.rs:215-303`), YAML in `sops.rs:184-264`, percent decoding (`findings.rs:696`). JUnit moves to `quick-xml` (model 3). The sops YAML reader reads only metadata and stays unless a small, maintained YAML crate fits; the pull request decides and says why.
- **Test helpers**: two fake HTTP servers (`tests/common/github.rs:50`, `tests/common/host.rs:143`); `Repo::fz_input` skips the Actions env scrub `fz_in` applies (`tests/common/mod.rs:147`); the scrub list lacks `GITHUB_JOB`, `GITHUB_REF`, `RUNNER_NAME` and the OIDC request variables that production reads, so a test run inside Actions can see them.
- **Small parallel helpers**: plural words in five places, "shown at most" caps of 10 and 20 for the same lists, short SHA in Rust and eleven `.slice(0, 7)` in TS.
- **About 85 lint flags** listed in `ROADMAP.md`: each model's pull request clears the ones in code it touches.

## Defects found while reading

Out of scope for unification. Listed so the coordinator can decide; none is fixed by this plan without a decision.

On `main`:
- **A removed line that starts with `-- `** (an SQL or Lua comment) reads as a diff header in `Changed::from_diff` (`findings.rs:170-183`), and an added `++ ` line likewise: the changed lines of that file are lost. Confirmed by reading the code.
- A `changed` rule on a deleted path becomes an unplaced gate with the wrong text, and `skip` is ignored for `changed` rules (`rules.rs:226-231`). Reported by a reader, not yet run.
- Cargo, dart and yamllint readers drop lines they cannot parse silently (`findings.rs:321, 416, 633`). Reported by a reader.

In open pull requests (worth a look before merging):
- **#36 against #35**: release pieces put publish tokens in `env` (`presets/crates.toml:11`, `npm.toml`, `github.toml`, `pub.toml`), which #35's skill text forbids ("Never put a secret in `env`"), and the release path masks nothing.
- **#40**: the new `timeout` is skipped for checks with `record`.
- **#45**: `light` is not honored for picture comparison; the services lease is held while waiting for a slot; the holder record shows a by hand command's full text to every agent on the machine; `FEARLEZZ_QUEUE=abc` turns the queue off silently.
- **#32**: coverage passes silently when `git ls-files` fails; unplaced JaCoCo lines skip the project filter.
- **#42**: the log is sliced at an offset taken before masking (unverified).

## Order

Each row is one pull request, small enough to review, keeping every feature, with a Fearlezz proof and a fresh review. Rows in the same wave touch different files and can run in parallel worktrees. Every pull request updates the module map in `AGENTS.md` and `ROADMAP.md` when it moves something.

| Wave | Pull request | Model | Depends on | Risk |
| --- | --- | --- | --- | --- |
| 1 | `refactor: one process builder, one lease and shared file helpers` | 1 | | low |
| 1 | `refactor(ee): one viewer and one access policy on the host` | 11 | | medium (security) |
| 1 | `refactor(ee): the page shows the review the host decided` | 12 | | low |
| 2 | `refactor: one typed reason a check did not pass` | 2 | 1 | medium |
| 2 | `refactor: one reader model for findings, tests, coverage, pictures and texts` | 3 | 1 | medium |
| 2 | `refactor: weakenings as typed changes, rendered once` | 5 | | low |
| 3 | `refactor: one base session and one reuse policy` | 4 | 1, 2 | medium |
| 3 | `refactor: one tool registry, one level and TOML tree helpers` | 9 | 5 | low |
| 3 | `refactor: needs as resources with one acquire order` | 6 | 1, 2 | high (locks, secrets) |
| 4 | `refactor: a typed check with placement and timing` | 7 | 6 | medium |
| 4 | `refactor: screens and recorders` | 10 | 1, 6 | medium (manual proof) |
| 5 | `refactor: selection and one runner` | 8a | 2, 6, 7 | high |
| 5 | `refactor: one report, rendered by sinks` | 8b | 8a | medium |
| 6 | `chore: health pass` | Health | all | low |

Each pull request starts from the tests that pin its behavior (`crates/fearlezz/tests/<feature>.rs`); where a model changes behavior on purpose (none planned), a failing test comes first.

## Decisions made

- **No config break.** Every key stays; where two keys say one thing, both map to one model. Titles stay `refactor:` (no release); a pull request that turns out to change behavior a user sees is retitled `fix:` or `feat:`.
- **Wait for the feature pull requests.** Phase B starts after the coordinator says all nine merged, so each model absorbs every parallel version at once, instead of unifying twice.
- **The core keeps its shape.** Models live in the crates that own them today: decisions and words in `fearlezz-proof` (shared with the host), effects in `crates/fearlezz`. No new crates.
- **Order by what makes the most code redundant first**, plumbing at the bottom, the run loop last because it depends on the others.
- **Defects are not fixed silently.** They are listed above for the coordinator; a confirmed one goes into the pull request of its model only with a failing test first and a line in its description.

## Coverage of the read

| Area | Files | Read |
| --- | --- | --- |
| Config, presets, settings, lints, floor, pieces | `config.rs`, `presets.rs`, `settings.rs`, `lints.rs`, `floor.rs`, `presets/*.toml` (41) | all |
| Readers and rules | `findings.rs`, `counts.rs`, `rules.rs`, `commits.rs`, `sops.rs`, `init.rs` | all |
| Engine | `run.rs`, `state.rs`, `store.rs`, `journal.rs`, `git.rs`, `env.rs`, `red.rs`, `look.rs` | all |
| CLI and outputs | `main.rs`, `feedback.rs`, `hooks.rs`, `host.rs`, `action.rs`, `proof.rs`, `tests/common` | all |
| Recording and media | `record.rs`, `record/*`, `media.rs` | all |
| Proof model and page | `crates/fearlezz-proof/src`, `ee/app/src` | all |
| Host | `ee/host/src` | all |
| Open pull requests | #32, #35, #36, #37, #38, #39, #40, #42, #45 | whole diffs |
| Not read line by line | integration test bodies beyond `tests/common`, `ee/host-tests`, CSS | named where a model depends on them |

## Phase B: what changed after the feature pull requests merged

Every feature pull request merged by `c79424f`, several with fixes after this plan read them, and #46 (the host's D1 queries) is new. The area readers checked each claim above again against `main` at `c79424f`. Line numbers above are from the older reading. What changes:

**Models and waves.** The twelve models and six waves stand. Changes inside them:

- **1. Plumbing** (done in its pull request): one `process::Project` builds every project command (checks, envtest, services, `prepare`, the preview, SecretSpec, releases), `process::read` runs curl, git, ffprobe, ffmpeg and the window tools, one `lease::Lease` locks services, base checkouts, the queue's slots and the simulator pool, `files` holds the hash, the user folders, the walker and whole writes. Kept apart on purpose: the store's prune marker (it takes the run's clock, so tests set it) and the bases' marker (file time); `settings::clean_path` and `presets::normalize` (one keeps `..`, the other refuses leaving the repository); the three jsonl readers (each error policy fits its file). The kill at a deadline loops are now at least nine; they move with one `Budget` into **10. Screens**, and #40's `timeout` skipped for checks with `record` (still true, `run.rs` routes them past `budgeted`) is fixed in **7. Check**.
- **2. Why**: #37 brought its own typed reason (`generated::Failed` with `why` and `fix`); a coverage failure still travels as `proof_errors[0]` with the exit code kept. A run now has three failure channels beside the exit code; all become `Why`.
- **3. Readers**: texts reuse `look::MediaGlob`, so there are four reader shapes, not five. The `$FEARLEZZ_OUT/` strip and glob build is copied three times (`counts.rs`, `coverage.rs`, `look.rs`) and the "no report matching" text twice. Every tracked file is listed three times per run (`run.rs` twice, `generated.rs`).
- **4. Base**: `run_checks` opens one base checkout per command; red still opens two per check. The fourth reuse rule is `legs::Reuse::pass` (the base CI bundle), and the queue polls the run loop's own rule, so it adds none.
- **6. Needs**: #45 takes the services lease before waiting for a slot, on purpose (a second run in the worktree then waits on the services, not silently on a slot). The one acquire order keeps theirs: secrets, services, slot, preview, screen. `light` is honored for picture comparison now. Still open: a by hand command's full text is shown to every waiter.
- **8. One run loop**: `run_checks` grew to 418 lines; whether a run is at the final stage is tested about six times in it, so `Plan` carries it once. `queue::by_hand` is the by hand path.
- **11. Host viewer**: five identity structs (adding `room::Viewer`, `share::Link`, `api::Writer`), six visibility functions, three permission caches, three `not_found` functions, the sign in hint three times, and the run to repository join four times. #46 changed query shapes only, no row struct.
- **12. Page**: `proof.ts` is already generated by ts-rs; the hand kept types left are the viewer, `Said`, `Share`, the listing's `Works` and `Standing`, `Place` and the live messages. The CLI does not put text diffs into the bundle; this pull request adds them as a new field, and the page keeps its own diff only for bundles from older CLIs. The picture standing the page recomputes is already sent by Rust.

**Defects, refined.**
- `Changed::from_diff`: only an added line starting with `++ ` loses lines (the hunks after it in that file); a removed `-- ` line only clears the old name, which nothing reads after the header. Fixed in **3. Readers** with a failing test first.
- #32: a failing `git ls-files` now fails coverage (fixed before merge). JaCoCo sections that name no single repository file are dropped silently when other sections place; **3. Readers** says so in the check's log.
- #42: the deploy's log is sliced at an offset taken before masking: confirmed, narrow (only when SecretSpec's own words hold a secret). Fixed in **6. Needs**.
- #36 against #35: release pieces put publish tokens in `env` (`presets/crates.toml:10`, `npm.toml:10`, `github.toml:16`, `pub.toml:10`) and the publish output goes unmasked to the terminal. Fixed in **6. Needs** (one kind of secret).

## Phase B, round 2: one pull request, and a fresh read

The developer asked for the rest of the work as **one pull request** (`refactor/unify`), and for a fresh read of the whole core to find what this plan missed. #48 (plumbing) and #49 (host viewer) merged on their own; the Why, Change, Readers and Page branches are merged into `refactor/unify`. A line count that grows is acceptable when each decision moves to one place, but each step should remove what it replaces, and a step that grows the core says why.

The fresh read (at `6fbf8fd`) ranks these, and changes the order: the run loop first, since every other change threads through the 418 line `run_checks`.

1. **The run produces `Vec<proof::Check>`; everything else derives from it.** "Not run here" is built five ways (`main.rs:1584-1620`, `1734-1749`, `1756-1773`, `legs.rs:456` `missing`, `legs.rs:477` `left_out`), each printing its own lines. Seven counters (`main.rs:1707-1717`) derive from the entries. `Report` is optional, so `if let Some(r) = report.as_mut()` appears nine times. `ci_gate` repeats `Report::finish` (summary append `main.rs:493-500` and `1342-1349`; accept ask `518-544` and `1307-1317`). The `Open` event and its `Opening` match are written twice (`1196-1225`, `1377-1410`). Model: always build the proof, local runs too; every exit of the loop body returns one entry (passed, failed with its item, not run with its item); `proof::Check` gives its terminal lines once; one `close` used by `run_checks` and `ci_gate`.
2. **One `prove(check, site, reuse)` for the change and the base.** `look::run_on_base` (`look.rs:323-397`) copies the loop body (`main.rs:1793-1851`): queue turn, prepare, services, preview, `Where`, `execute`, touch or stop. "Is there a result for these inputs" is answered five ways (`main.rs:1721-1729`, `latest_that_ran` in `look::look`, `red.rs` `on_base`, the queue callback, `legs::Reuse::pass`); one `Reuse` policy answers it. Needs are acquired inside `prove` in one fixed order; no resource trait.
3. **Why is stored where it is known.** `execute` knows why at the moment it fails; its helpers return `Result<Ended>` with `Ended = Exit(i32) | Signal | Never(Why)`, `RunResult` gets `why: Option<Why>` (serde default, so old results read as before), and `missing_secret`, `no_preview`, `generated`, the first `proof_errors` entry used as a reason, exit 127 parsing and the tuple returns go. `Why` lives in `fearlezz-proof` so the host gets it typed; the proof keeps its `failure` item.
4. **Config: one expansion pass, `Check: Default`, nothing derived stored.** `builtin_check` lists all 30 fields (`config.rs:970-1017`); `with_context(|| format!("check {id}"))` is written 17 times (`683-816`); `run` and `steps`, `test_files` and `test_globs`, the services and preview fingerprints are stored beside their source; `RawEnv::resolve` copies 12 fields; `checks` and `set_aside` swap in `on_schedule` (one list and `due(when)` instead); `off` is dropped at table level in three places leaving dead serde placeholders (`config.rs:534-540`, `release.rs:105-110`, `rules.rs:129`); magic stage names `"look"` and `"working"` (`config.rs:936, 942, 990`).
5. **One base handle per command**, lazy, holding the base commit, its config and its checkout; floor (`floor.rs:39-43`, reached three times per final stage run), look (`look.rs:122-124`) and red (`red.rs:136-140`, three checkouts per check) borrow it.
6. **`Next` as a type and the run's mode stated once.** Control flow compares the `next:` sentence (`main.rs:657`, `1635`, `1935`); `run_checks` overrides `View.next` in a closure; final stage, `in_memory`, `ids.is_empty()` and `out.is_some()` are recombined about twelve times.
7. **Names and tools as data.** Seven hand written "no X; X are ..." lookups; `settings::Tool` has four parallel matches (`table`, `render`, `lookup`, `LINT_FILES`); one row per tool. `Skill`, `Hooks` and `FlutterLook` are files written whole, not tools.
8. **The screen stated once.** `needs` parsed once into `Needs { services, envtest, preview, screen }`; `Place` follows from the screen. Do not split `record.rs` for its own sake.

Where the earlier plan was too abstract: a derived `Why` added code (3 replaces it); one `Reader { Source, Format }` for findings, counts and coverage buys little beyond the shared glob and path placing; needs as resources with lifetimes, and a `Selection`/`Plan`/`Runner`/sinks framework, are heavier than 1 and 2; the TOML tree helper is low value.

Dead: `Proof::of_look` (`fearlezz-proof/src/lib.rs:310`, no caller); `Event::Retired` (`journal.rs:45`, never constructed, but old journals may hold it); the three `off` placeholders; `run.rs:400-402` ("check has no command to run", unreachable).

Still to fold in from the defects above: release pieces' tokens in `env` and unmasked publish output; the preview log sliced before masking; #40's `timeout` skipped for checks with `record`; a by hand command's full text shown to every waiter; the tests' Actions env scrub (`tests/common/mod.rs`); a `changed` rule on a deleted path; two worktrees recording the same iOS simulator at once ("A recording is already in progress").
