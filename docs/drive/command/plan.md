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

A command's secrets are fetched once (`Project::secrets`, or `Project::given` for a release place, which leaves out one nobody gives), apart from the commands that use them, so a release place's publish and landed commands share them. One runner in `process.rs` (`Project::run`) takes a `Command`, those secrets, the variables Fearlezz adds (`FEARLEZZ_OUT`, `FEARLEZZ_PREVIEW_NAME`, `FEARLEZZ_VERSION`...), where its output goes (a log file, the terminal, nowhere, or a caller that starts each step, as a check does on its screen) and whether it starts inside the `[env]` shell. It builds the clean env, stops the command at its budget, and masks every form of every secret in what it wrote, also when a step cannot start. Masking, the clean env and budgets are then decided in one place for every command.

**Where a secret comes from** (decided: environment first, then SecretSpec): a secret set in Fearlezz's own environment is taken from there; otherwise SecretSpec is asked, as today. CI's own tokens (crates.io OIDC, `setup-node`, `GH_TOKEN`) become secrets with no setup.

## What fills it

| Config | Becomes |
| --- | --- |
| `[check.<id>] run, dir, env, secrets, timeout` | a check's command, as now |
| `[env] prepare, up, down, reset, ports, preview` with `[env] env` and `[env] secrets` | one command each, all sharing `env`; only the preview gets `secrets`; prepare and services start outside the shell, since they start it |
| `[release.<place>] publish, landed` with `dir`, `env` and a new `secrets` | one command each; a secret nobody gives is left out |
| `[release.<place>] dry` | a check with the place's `dir`, not its `env` or `secrets`, so a dry run never needs a token and a token kept in `env` never reaches a check |
| envtest's fetch | a command with the check's `env` |

Out of scope: Fearlezz's own tools (git, curl, ffmpeg, simctl, ast-grep) and `fearlezz run -- <command>`, which take no config.

## Visible changes

- A secret set in Fearlezz's environment is used without asking SecretSpec.
- Release places take `secrets`; the built in pieces move their tokens there (`CARGO_REGISTRY_TOKEN`, `NODE_AUTH_TOKEN`, `PUB_TOKEN`, `GH_TOKEN`, `GITHUB_TOKEN`), and the mask every `env` value patch goes. A place of a project's own that keeps a token in `env` shows it again, as every `env` value does; the skill already says a secret never goes in `env`.
- A release place's secret that neither Fearlezz's environment nor SecretSpec gives is left out: its publish and landed commands run without it, so the tool falls back to its own login (`cargo login`, `npm login`, `gh auth login`, OIDC trusted publishing that sets no token). A secret that is given is still masked. Checks and the preview stay strict. A rerun of a release that landed no longer fails without its token. One SecretSpec declares but fails to give is said (`<NAME> not given, so its tool uses its own login`, with SecretSpec's words); without SecretSpec nothing is said. The github reader asks with the place's token too, so a private repository's release lands with a token from SecretSpec.
- A check's secrets are masked in its files also when a later step cannot start.

No key is renamed or removed, so no config breaks.

## Slices

One pull request (a refactor goes in one), built after `refactor/health` merges, since both touch `run.rs`. Each slice starts from its failing test.

1. **Secret source.** `fetch_secrets` takes a secret from Fearlezz's environment first. Test: `tests/secrets.rs`, a check's secret set in the environment reaches it, masked, with no `secretspec` on PATH.
2. **The type and the runner.** `Command` in `config.rs`, `Project::run` in `process.rs`; a check's steps and envtest move onto it. No behaviour change: `tests/secrets.rs`, `tests/legs.rs` (timeout), `tests/record.rs` stay green.
3. **`[env]` commands.** prepare, services and preview move onto the runner; only the preview gets `[env] secrets`. Test: `tests/env.rs`, `up` does not see an `[env]` secret.
4. **Release.** Places take `secrets`, left out when nobody gives one; publish and landed move onto the runner; the pieces move their tokens; the mask every `env` value code goes; dry runs get no `env`. Tests: `tests/release.rs`, publish output masks a place's secret and shows its `env` as is, a publish whose secret nobody gives still runs, and a dry run sees neither the place's `env` nor its secrets.
5. **Docs.** `skills/fearlezz/SKILL.md` (secrets: where they come from; release `secrets`), `ROADMAP.md`, the module map in `AGENTS.md`.
