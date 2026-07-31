{
  description = "ekko terminal multiplexer";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        lib = pkgs.lib;
        sourceFiles = lib.fileset.unions [
          ./crates
          ./examples
          ./Cargo.toml
          ./Cargo.lock
          (lib.fileset.fileFilter (file: lib.hasPrefix "LICENSE" file.name) ./.)
        ];
        src = lib.fileset.toSource {
          root = ./.;
          fileset = sourceFiles;
        };
        mkEkko =
          args:
          pkgs.rustPlatform.buildRustPackage (
            {
              pname = "ekko";
              version = "0.1.0";
              inherit src;
              cargoLock.lockFile = ./Cargo.lock;
            }
            // args
          );
        featureChecks = builtins.listToAttrs (
          map
            ({ name, flags }: {
              inherit name;
              value = mkEkko {
                cargoBuildFlags = flags;
                cargoTestFlags = flags;
              };
            })
            [
              {
                name = "features-none";
                flags = [
                  "-p"
                  "ekko"
                  "--no-default-features"
                ];
              }
              {
                name = "features-builtins";
                flags = [
                  "-p"
                  "ekko"
                  "--no-default-features"
                  "--features"
                  "builtins"
                ];
              }
              {
                name = "features-lua";
                flags = [
                  "-p"
                  "ekko"
                  "--no-default-features"
                  "--features"
                  "lua"
                ];
              }
              {
                name = "features-default";
                flags = [
                  "-p"
                  "ekko"
                ];
              }
            ]
        );
      in
      {
        packages.default = mkEkko {
          meta = {
            description = "ekko terminal multiplexer";
            license = pkgs.lib.licenses.mit;
            mainProgram = "ekko";
          };
        };

        checks = featureChecks // {
          bare-harness = mkEkko {
            cargoBuildFlags = [
              "-p"
              "ekko"
              "--no-default-features"
            ];
            cargoTestFlags = [
              "-p"
              "ekko"
              "--no-default-features"
            ];
          };
          clippy = mkEkko {
            nativeBuildInputs = [
              pkgs.cargo
              pkgs.clippy
            ];
            checkPhase = "cargo clippy --workspace --all-targets --all-features -- -D warnings";
          };
          fmt =
            pkgs.runCommand "ekko-fmt"
              {
                nativeBuildInputs = [
                  pkgs.cargo
                  pkgs.rustfmt
                ];
              }
              ''
                cp -r ${src} source
                chmod -R u+w source
                cd source
                cargo fmt --all -- --check
                touch $out
              '';
        };

        formatter = pkgs.nixfmt-tree;

        devShells.default = pkgs.mkShell {
          inputsFrom = [ self.packages.${system}.default ];
          packages = [
            pkgs.clippy
            pkgs.rustfmt
            pkgs.rust-analyzer
          ];
        };
      }
    );
}
