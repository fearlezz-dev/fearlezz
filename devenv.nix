# `devenv shell` gives Rust and the scripts below. `devenv test` runs the full local check.
# Script names avoid `test` and `fmt`, which lose to the shell builtin and coreutils.
{ pkgs, ... }:

{
  languages.rust = {
    enable = true;
    channel = "stable";
    components = [ "rustc" "cargo" "clippy" "rustfmt" "rust-analyzer" ];
  };

  packages = [ pkgs.git pkgs.ffmpeg pkgs.ast-grep pkgs.actionlint pkgs.shellcheck pkgs.jq pkgs.yq-go pkgs.gitleaks ];

  scripts = {
    format = {
      description = "Format all Rust code";
      exec = ''cd "$DEVENV_ROOT" && cargo fmt --all'';
    };
    check = {
      description = "Prove this commit with every check the change needs, fresh, as CI does";
      exec = ''
        set -euo pipefail
        cd "$DEVENV_ROOT"
        cargo build --quiet
        # Proves the commit, not the work: no journal, and never "ready to merge".
        target/debug/fearlezz run --ci "$@"
      '';
    };
  };

  enterTest = "check";
}
