{
  description = "bzzt";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable-small";
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
      nix-github-actions,
      treefmt-nix,
      ...
    }:
    let
      inherit (nixpkgs) lib;
      cargo-toml = (lib.importTOML ./Cargo.toml).package;
      inherit (cargo-toml) name;
      forEachSystem =
        f:
        builtins.listToAttrs (
          map
            (system: {
              name = system;
              value = f {
                inherit system;
                pkgs = nixpkgs.legacyPackages.${system};
              };
            })
            [
              "x86_64-linux"
              "aarch64-linux"
              "aarch64-darwin"
            ]
        );

      package =
        {
          lib,
          rustPlatform,
        }:
        rustPlatform.buildRustPackage {
          pname = name;
          inherit (cargo-toml) version;
          src = lib.cleanSource ./.;
          cargoLock.lockFile = ./Cargo.lock;

          strictDeps = true;
          __structuredAttrs = true;

          meta = {
            inherit (cargo-toml) description;
            mainProgram = name;
            license = lib.getLicenseFromSpdxId cargo-toml.license;
            maintainers = with lib.maintainers; [ grimmauld ];
          };
        };

      treefmtEval = (lib.flip treefmt-nix.lib.evalModule) ./treefmt.nix;
    in
    {
      packages = forEachSystem (
        { pkgs, system }:
        {
          ${name} = pkgs.callPackage package { };
          default = self.packages.${system}.${name};
        }
      );

      devShells = forEachSystem (
        { pkgs, system }:
        {
          default = pkgs.mkShell {
            inputsFrom = [ self.packages.${system}.default ];
            packages = [
              pkgs.clippy
              pkgs.rust-analyzer
              pkgs.rustfmt
            ];
          };
        }
      );

      formatter = forEachSystem ({ pkgs, ... }: (treefmtEval pkgs).config.build.wrapper);

      checks = forEachSystem (
        { pkgs, system }:
        {
          formatting = (treefmtEval pkgs).config.build.check self;
        }
        // self.packages.${system}
      );
      githubActions = nix-github-actions.lib.mkGithubMatrix {
        checks = { inherit (self.checks) x86_64-linux; };
      };

      overlays.default = final: prev: {
        ${name} = final.callPackage package { };
      };

      nixosModules.default = {
        nixpkgs.overlays = [ self.overlays.default ];
      };
    };
}
