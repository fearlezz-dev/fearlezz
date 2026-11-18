# One command shape

Every command a project's config names takes one shape and runs one way. Today seven commands take five shapes: only a check and the preview get secrets, only a check has a budget, release tokens travel unmasked in `env` (patched by masking every `env` value of a release place), and release dry runs get the place's folder but not its variables. `record.yaml` holds the inventory with its `file:line` evidence.

## The model

```rust
/// A command the project's config names: what it runs, where, and what it may see.
pub(crate) struct Command {
    pub run: Run,                    // one command, or steps in order
    pub dir: Option<String>,         // relative to the root
    pub env: Vec<String>,            // passed through from Fearlezz's own, shown as they are
    pub secrets: Vec<String>,        // masked in everything kept, printed or uploaded
    pub timeout: Option<Budget>,     // only a check sets one from config today
}
```

One runner in `process.rs` (`Project::run`) takes a `Command`, the variables Fearlezz adds (`FEARLEZZ_OUT`, `FEARLEZZ_PREVIEW_NAME`, `FEARLEZZ_VERSION`...), where its output goes (a log file, the terminal, or nowhere) and whether it starts inside the `[env]` shell. It fetches the secrets once, builds the clean env, stops the command at its budget, and masks every form of every secret in what it wrote. Masking, the clean env and budgets are then decided in one place for every command.

**Where a secret comes from** (decided: environment first, then SecretSpec): a secret set in Fearlezz's own environment is taken from there; otherwise SecretSpec is asked, as today. CI's own tokens (crates.io OIDC, `setup-node`, `GH_TOKEN`) become secrets with no setup.

## What fills it

| Config | Becomes |
| --- | --- |
| `[check.<id>] run, dir, env, secrets, timeout` | a check's command, as now |
| `[env] prepare, up, down, reset, ports, preview` with `[env] env` and `[env] secrets` | one command each, all sharing `env` and `secrets`; prepare and services start outside the shell, since they start it |
| `[release.<place>] publish, landed` with `dir`, `env` and a new `secrets` | one command each |
| `[release.<place>] dry` | a check with the place's `dir` and `env`, not its `secrets`, so a dry run never needs a token |
| envtest's fetch | a command with the check's `env` |

Out of scope: Fearlezz's own tools (git, curl, ffmpeg, simctl, ast-grep) and `fearlezz run -- <command>`, which take no config.

## Visible changes

- A secret set in Fearlezz's environment is used without asking SecretSpec.
- Services and prepare get `[env] secrets`, masked in their logs. `[env] secrets` was the preview's only.
- Release places take `secrets`; the built in pieces move their tokens there (`CARGO_REGISTRY_TOKEN`, `NODE_AUTH_TOKEN`, `PUB_TOKEN`, `GH_TOKEN`, `GITHUB_TOKEN`), and the mask every `env` value patch goes. A place of a project's own that keeps a token in `env` shows it again, as every `env` value does; the skill already says a secret never goes in `env`.
- A release dry run gets the place's `env`.

No key is renamed or removed, so no config breaks.

## Slices

One pull request (a refactor goes in one), built after `refactor/health` merges, since both touch `run.rs`. Each slice starts from its failing test.

1. **Secret source.** `fetch_secrets` takes a secret from Fearlezz's environment first. Test: `tests/secrets.rs`, a check's secret set in the environment reaches it, masked, with no `secretspec` on PATH.
2. **The type and the runner.** `Command` in `config.rs`, `Project::run` in `process.rs`; a check's steps and envtest move onto it. No behaviour change: `tests/secrets.rs`, `tests/legs.rs` (timeout), `tests/record.rs` stay green.
3. **`[env]` commands.** prepare, services and preview move onto the runner; services get `[env] secrets`. Test: `tests/env.rs`, `up` sees a secret and its log shows `***`.
4. **Release.** Places take `secrets`; publish and landed move onto the runner; the pieces move their tokens; the mask every `env` value code goes; dry runs get `env`. Tests: `tests/release.rs`, publish output masks a place's secret and shows its `env` as is, and a dry run sees the place's `env`.
5. **Docs.** `skills/fearlezz/SKILL.md` (secrets: where they come from; release `secrets`), `ROADMAP.md`, the module map in `AGENTS.md`.
