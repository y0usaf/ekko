{
  description = "ekko terminal multiplexer";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
        mkEkko = args: pkgs.rustPlatform.buildRustPackage ({
          pname = "ekko";
          version = "0.1.0";
          src = self;
          cargoLock.lockFile = ./Cargo.lock;
        } // args);
      in
      {
        packages.default = mkEkko {
          meta = {
            description = "ekko terminal multiplexer";
            license = pkgs.lib.licenses.mit;
            mainProgram = "ekko";
          };
        };

        checks = {
          bare-harness = mkEkko {
            cargoBuildFlags = [ "--no-default-features" ];
            doCheck = false;
          };
          tests = mkEkko {
            cargoTestFlags = [ "--workspace" ];
          };
        };

        devShells.default = pkgs.mkShell {
          inputsFrom = [ self.packages.${system}.default ];
        };
      });
}
