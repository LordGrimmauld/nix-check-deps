{
  description = "bzzt";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable-small";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    nix-github-actions = {
      url = "github:nix-community/nix-github-actions";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      rust-overlay,
      nix-github-actions,
      ...
    }:
    let
      outputs = flake-utils.lib.eachDefaultSystem (
        system:
        let
          overlays = [ (import rust-overlay) ];
          pkgs = import nixpkgs {
            inherit system overlays;
          };
          rustToolchain = pkgs.pkgsBuildHost.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
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

          devShell = pkgs.mkShell {
            buildInputs = [
              rustToolchain
            ];
          };

          defaultPackage = self.packages.${system}.nix-check-deps;

        }
      );
    in
    outputs
    // {

      githubActions = nix-github-actions.lib.mkGithubMatrix {
        checks = outputs.x86_64.packages;
      };
    };
}
