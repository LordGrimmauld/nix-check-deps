{ ... }:
{
  projectRootFile = "flake.nix";
  programs.nixfmt-rfc-style.enable = true;
  programs.rustfmt.enable = true;
  programs.toml-sort.enable = true;
}
