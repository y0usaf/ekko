{
  description = "ekko terminal multiplexer";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    # The shared WASM kernel (crate `cordis`). A Cargo path dep
    # path=../../cordis-rs/crates/cordis is NOT covered by this repo's source
    # filter, so cordis-rs is a flake input and its crates/cordis source is
    # materialised into the tree via a symlink (see `cordisSymlink`).
    cordis-rs.url = "github:y0usaf/cordis-rs";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      cordis-rs,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        lib = pkgs.lib;
        sourceFiles = lib.fileset.unions [
          ./crates
          ./examples
          ./guests
          ./Cargo.toml
          ./Cargo.lock
          (lib.fileset.fileFilter (file: lib.hasPrefix "LICENSE" file.name) ./.)
        ];
        src = lib.fileset.toSource {
          root = ./.;
          fileset = sourceFiles;
        };
        # Point the Cargo path dep `path=../../cordis-rs/...` (up two
        # levels, i.e. a `cordis-rs` sibling of the source root) at the flake
        # input so the cordis crate resolves inside the sandbox. Runs in
        # preConfigure (cwd = source root), not postUnpack (cwd = top build
        # dir where `../` would escape the tree).
        cordisSymlink = ''
          ln -s ${cordis-rs} "$PWD/cordis-rs"
        '';

        mkEkko =
          args:
          pkgs.rustPlatform.buildRustPackage (
            {
              pname = "ekko";
              version = "0.1.0";
              inherit src;
              cargoLock.lockFile = ./Cargo.lock;
              preConfigure = cordisSymlink;
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
                name = "features-wasm";
                flags = [
                  "-p"
                  "ekko"
                  "-p"
                  "ekko-ext"
                  "--no-default-features"
                  "--features"
                  "wasm"
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
      let
        # Build the which-key WASM extension guest (targets wasm32, standalone
        # crate under `guests/which-key`). Produces `which-key.wasm` for the
        # finix flake to place at `~/.config/ekko/extensions/which-key.wasm`.
        whichKeyWasm = pkgs.rustPlatform.buildRustPackage {
          pname = "which-key-wasm";
          version = "0.1.0";
          src = lib.fileset.toSource {
            root = ./guests/which-key;
            fileset = lib.fileset.unions [
              ./guests/which-key/Cargo.toml
              ./guests/which-key/Cargo.lock
              ./guests/which-key/src
            ];
          };
          cargoLock.lockFile = ./guests/which-key/Cargo.lock;
          # `rustPlatform`'s rustc already ships the wasm32-unknown-unknown std
          # (confirmed: the guest compiled; only the host-target test phase was
          # the failure). Provide `lld` for the wasm linker and drop the default
          # host-target check (it can't link wasm-only externs).
          nativeBuildInputs = [ pkgs.lld ];
          doCheck = false;
          doInstallCheck = false;
          # Ensure cargo targets the wasm ABI (not the default host build).
          cargoBuildFlags = [ "--target" "wasm32-unknown-unknown" ];
          CARGO_BUILD_TARGET = "wasm32-unknown-unknown";
          buildPhase = ''
            runHook preBuild
            cargo build --release --target wasm32-unknown-unknown --lib
            runHook postBuild
          '';
          installPhase = ''
            runHook preInstall
            mkdir -p $out
            cp target/wasm32-unknown-unknown/release/which_key_wasm.wasm $out/which-key.wasm
            runHook postInstall
          '';
        };
      in
      {
        packages.default = mkEkko {
          meta = {
            description = "ekko terminal multiplexer";
            license = pkgs.lib.licenses.mit;
            mainProgram = "ekko";
          };
        };
        packages.which-key = whichKeyWasm;

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
                cp -rL ${src} source
                chmod -R u+w source
                cd source
                # cargo fmt formats only this workspace's members; it does not
                # need the cordis path dep resolved, so no symlink here.
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
