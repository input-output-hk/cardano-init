{ inputs, system }:

let
  inherit (pkgs) lib;

  pkgs = import ./pkgs.nix { inherit inputs system; };

  project = import ./project.nix { inherit inputs pkgs lib; };

  mkShell = withDevTools: import ./shell.nix { inherit pkgs lib project withDevTools; };

in

{
  # GHC, the prebuilt Haskell dependencies, cabal, and editor/lint tooling.
  devShells.default = mkShell true;

  # Toolchain only: no language server or linters, which build from source.
  # Used by CI (`nix develop .#minimal`).
  devShells.minimal = mkShell false;
}
