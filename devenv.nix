# `devenv shell` gives Rust and the scripts below. `devenv test` runs the full local check.
# Script names avoid `test` and `fmt`, which lose to the shell builtin and coreutils.
{ pkgs, ... }:

{
  languages.rust = {
    enable = true;
    channel = "stable";
    components = [ "rustc" "cargo" "clippy" "rustfmt" "rust-analyzer" ];
  };

  packages = [ pkgs.git pkgs.ffmpeg pkgs.ast-grep ];

  scripts = {
    format = {
      description = "Format all Rust code";
      exec = ''cd "$DEVENV_ROOT" && cargo fmt --all'';
    };
    check = {
      description = "Run every Fearlezz check fresh, as CI does";
      exec = ''
        set -euo pipefail
        cd "$DEVENV_ROOT"
        cargo build --quiet
        # Its own journal at the final stage, so the branch's journal keeps its stage.
        export FEARLEZZ_JOURNAL="''${FEARLEZZ_JOURNAL:-check}"
        target/debug/fearlezz run --stage merge --reason "every check, fresh" --fresh
      '';
    };
  };

  enterTest = "check";
}
