{
  description = "cardano-init — scaffolds Cardano protocol projects";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = {
    self,
    nixpkgs,
  }: let
    systems = ["x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin"];
    forAllSystems = f:
      nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

    version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
  in {
    packages = forAllSystems (pkgs: rec {
      cardano-init = pkgs.rustPlatform.buildRustPackage {
        pname = "cardano-init";
        inherit version;

        src = pkgs.lib.cleanSource self;

        cargoLock.lockFile = ./Cargo.lock;

        meta = with pkgs.lib; {
          description = "Scaffolds Cardano protocol projects";
          homepage = "https://github.com/input-output-hk/cardano-init";
          license = licenses.asl20;
          mainProgram = "cardano-init";
        };
      };

      default = cardano-init;
    });

    devShells = forAllSystems (pkgs: {
      default = pkgs.mkShell {
        packages = with pkgs; [
          rustc
          cargo
          clippy
          rustfmt
          rust-analyzer
        ];

        RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
      };
    });
  };
}
