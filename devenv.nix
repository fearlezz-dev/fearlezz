# `devenv shell` gives Rust and the scripts below. `devenv test` runs the full local check.
# Script names avoid `test` and `fmt`, which lose to the shell builtin and coreutils.
{ pkgs, ... }:

let
  # The proof host uses worker 0.9, which needs worker-build 0.9; nixpkgs still has 0.8.7.
  # Drop this once nixpkgs has 0.9.
  worker-build = pkgs.worker-build.overrideAttrs (final: old: {
    version = "0.9.0";
    src = pkgs.fetchFromGitHub {
      owner = "cloudflare";
      repo = "workers-rs";
      tag = "v${final.version}";
      hash = "sha256-zmeQp7db2rwfECp9p5NthKvzsZFnQPFO+eueIhFsIwE=";
      fetchSubmodules = true;
    };
    cargoDeps = pkgs.rustPlatform.fetchCargoVendor {
      inherit (final) src;
      hash = "sha256-EbRwSrxRys2EcDfB88apmH5Rm0NUWLKtmGPS77fAhpI=";
    };
  });
in
{
  languages.rust = {
    enable = true;
    channel = "stable";
    components = [ "rustc" "cargo" "clippy" "rustfmt" "rust-analyzer" ];
    # The proof host in ee/ is a Cloudflare Worker built to wasm.
    targets = [ "wasm32-unknown-unknown" ];
  };

  packages = [ pkgs.git pkgs.ffmpeg pkgs.ast-grep pkgs.actionlint pkgs.shellcheck pkgs.jq pkgs.yq-go pkgs.gitleaks pkgs.trufflehog pkgs.betterleaks pkgs.osv-scanner pkgs.zizmor pkgs.wrangler worker-build ];

  scripts = {
    format = {
      description = "Format all Rust code";
      exec = ''cd "$DEVENV_ROOT" && cargo fmt --all'';
    };
    check = {
      description = "Prove this commit with every check, fresh, as CI does";
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
