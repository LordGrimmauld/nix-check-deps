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
    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
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
      treefmt-nix,
      ...
    }:
    let
      build-nix-check-deps-pkg =
        pkgs:
        pkgs.rustPlatform.buildRustPackage {
          name = "nix-check-deps";
          version = "1.0.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
        };

      outputs = flake-utils.lib.eachDefaultSystem (
        system:
        let
          overlays = [ (import rust-overlay) ];
          pkgs = import nixpkgs {
            inherit system overlays;
          };
          rustToolchain = pkgs.pkgsBuildHost.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
          treefmtEval = treefmt-nix.lib.evalModule pkgs ./treefmt.nix;
        in
        rec {
          packages.nix-check-deps = build-nix-check-deps-pkg pkgs;

          devShell = pkgs.mkShell {
            buildInputs = [
              rustToolchain
              pkgs.jq
            ];
          };

          formatter = treefmtEval.config.build.wrapper;

          defaultPackage = self.packages.${system}.nix-check-deps;

          checks = {
            formatting = treefmtEval.config.build.check self;
          } // packages;
        }
      );
    in
    outputs
    // {

      githubActions = nix-github-actions.lib.mkGithubMatrix {
        checks = nixpkgs.lib.getAttrs [ "x86_64-linux" ] outputs.checks;
      };

      overlays.default = final: prev: { nix-check-deps = build-nix-check-deps-pkg prev; };

      nixosModules.default = {
        nixpkgs.overlays = [ self.overlays.default ];
      };
    };
}
