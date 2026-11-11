{
  description = "Proof for work done by AI agents";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      forAll = f: nixpkgs.lib.genAttrs systems (s: f nixpkgs.legacyPackages.${s});
    in
    {
      packages = forAll (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "fearlezz";
          version = (builtins.fromTOML (builtins.readFile ./crates/fearlezz/Cargo.toml)).package.version;
          src = self;
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [ "-p" "fearlezz" ];
          # The tests drive real git repositories, ffmpeg and services; proof.yml runs them.
          doCheck = false;
          meta.mainProgram = "fearlezz";
        };
      });
    };
}
