# Set up Fearlezz

You are a coding agent, asked to set up Fearlezz in a repository. Fearlezz helps coding agents prove their work: the project declares its checks in `fearlezz.toml`, and Fearlezz runs them on the developer's machine while the agent works and in CI at the end. It shows what passed, what failed and why, and the pictures a change made.

## 1. Install it the way the project installs its tools

Look at how the project already gets its tools, and add Fearlezz the same way:

| The project uses | Add Fearlezz with |
| --- | --- |
| Nix flake | the input `github:fearlezz-dev/fearlezz`, and its `packages.<system>.default` in the dev shell |
| devenv | `fearlezz.url = "github:fearlezz-dev/fearlezz"` under `inputs` in `devenv.yaml`, and `inputs.fearlezz.packages.${pkgs.stdenv.system}.default` in `packages` |
| A Brewfile | `brew "fearlezz-dev/tap/fearlezz"` in the Brewfile (`brew install fearlezz-dev/tap/fearlezz` to install it now) |
| Cargo | `cargo install --git https://github.com/fearlezz-dev/fearlezz fearlezz` |
| None of these, or a container image | the release binary: `curl -fsSL https://raw.githubusercontent.com/fearlezz-dev/fearlezz/main/install.sh \| sh`, or a binary from the [releases](https://github.com/fearlezz-dev/fearlezz/releases) page |

It runs on Linux and macOS; on Windows, inside WSL. Ask the developer only when the project shows no way of installing tools.

## 2. Check the install

```sh
which -a fearlezz
fearlezz --version
```

When `which -a` lists more than one, the first one runs, and agents and hooks run it too. If the first is older than the one you installed, remove it or move the new one ahead of it on PATH, then check again.

## 3. Follow the guide

```sh
fearlezz setup guide
```

It prints the full setup guide for the installed version. Follow it from step 1 to the end, every step: its rules (ask before loosening any lint, keep the tools Fearlezz's pieces add) hold throughout, and step 9 puts every opportunity Fearlezz names to the developer as a question.
