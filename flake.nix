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
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "ekko";
          version = "0.1.0";

          src = self;
          cargoLock.lockFile = ./Cargo.lock;

          meta = {
            description = "ekko terminal multiplexer";
            license = pkgs.lib.licenses.mit;
            mainProgram = "ekko";
          };
        };

        devShells.default = pkgs.mkShell {
          inputsFrom = [ self.packages.${system}.default ];
        };
      });
}
