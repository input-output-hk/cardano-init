{ pkgs, lib, project, withDevTools }:

project.shellFor {
  name = "plutarch-${project.args.compiler-nix-name}";

  withHoogle = false;
  exactDeps = false;

  tools = {
    cabal = "latest";
  } // lib.optionalAttrs withDevTools {
    haskell-language-server = "latest";
    hlint = "latest";
    fourmolu = "latest";
    cabal-fmt = "latest";
  };

  # cabal fetches the pinned Plutarch source-repository-package with git.
  buildInputs = [ pkgs.git ];
}
