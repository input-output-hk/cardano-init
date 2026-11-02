{ pkgs, lib, project, withDevTools }:

project.shellFor {
  name = "plinth-${project.args.compiler-nix-name}";

  withHoogle = false;
  exactDeps = false;

  tools = {
    cabal = "latest";
  } // lib.optionalAttrs withDevTools {
    haskell-language-server = "latest";
    hlint = "latest";
    fourmolu = "latest";
    stylish-haskell = "latest";
    cabal-fmt = "latest";
  };
}
