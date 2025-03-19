{
  description = "bzzt";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable-small";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        packages.nix-check-deps = pkgs.rustPlatform.buildRustPackage {
          name = "nix-check-deps";
          version = "1.0.0";

          src = ./.;
          cargoLock = {
            lockFile = ./Cargo.lock;
          };
          
          # skips rebuilding the whole thing with debug info
          doCheck = false;
        };

        defaultPackage = self.packages.${system}.nix-check-deps;

        devShell = pkgs.mkShell {
          RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";

          inputsFrom = builtins.attrValues self.packages.${system};
          buildInputs = [
            pkgs.cargo-outdated
            pkgs.rustfmt
            pkgs.clippy
            pkgs.flamegraph
          ];
        };
      }
    );
}
