{
  description = "Ekko v2 terminal multiplexer";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
  inputs.terminal-browser = {
    url = "github:zenbu-labs/terminal-browser/cce10b6131d15bf46a3e4b8dc827e0544ff7fc65";
    flake = false;
  };
  outputs = { self, nixpkgs, terminal-browser }:
    let
      systems = [ "x86_64-linux" ];
      forEachSystem = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in {
      packages = forEachSystem (pkgs: {
        browser-source = pkgs.stdenvNoCC.mkDerivation {
          name = "ekko-terminal-browser-source";
          src = terminal-browser;
          patches = [ ./patches/terminal-browser-session-transport.patch ];
          dontConfigure = true;
          dontBuild = true;
          installPhase = "cp -R . $out";
          dontFixup = true;
        };
        performance = pkgs.writeShellScriptBin "ekko-performance" ''
          exec ${pkgs.python3}/bin/python ${./scripts/performance.py} ${self.packages.${pkgs.system}.default}/bin/ekko "$@"
        '';
        workspace = pkgs.writeShellScriptBin "ekko-workspace" ''
          export EKKO_WORKSPACE_MODE=shell-browser
          exec ${self.packages.${pkgs.system}.benchmark}/bin/ekko-benchmark "$@"
        '';
        benchmark = pkgs.writeShellApplication {
          name = "ekko-benchmark";
          runtimeInputs = [ pkgs.nix pkgs.coreutils ];
          text = ''
            export EKKO_BINARY=${self.packages.${pkgs.system}.default}/bin/ekko
            export EKKO_BROWSER_SOURCE=${self.packages.${pkgs.system}.browser-source}
            export EKKO_KITTY=${pkgs.kitty}/bin/kitty
            export EKKO_FONTCONFIG=${pkgs.makeFontsConf { fontDirectories = [ pkgs.dejavu_fonts ]; }}
            exec ${pkgs.bash}/bin/bash ${./scripts/benchmark.sh} "$@"
          '';
        };
        kitty-oracle = import ./nix/kitty-oracle.nix { inherit pkgs; };
        default = pkgs.stdenv.mkDerivation {
          pname = "ekko";
          version = "0.1.0";
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [
              ./ekko.asd
              (pkgs.lib.fileset.fileFilter (file: file.hasExt "lisp" || file.hasExt "c") ./src)
              (pkgs.lib.fileset.fileFilter (file: file.hasExt "lisp" || file.hasExt "py") ./tests)
              ./scripts/build.sh ./scripts/build.lisp ./scripts/build-demo.lisp
              ./scripts/test.sh ./scripts/test.lisp ./scripts/smoke.sh
            ];
          };
          nativeBuildInputs = [ pkgs.sbcl ];
          buildInputs = [ pkgs.zlib ];
          dontConfigure = true;
          # A saved SBCL core is appended to the ELF runtime. Stripping loses it.
          dontStrip = true;
          dontPatchELF = true;
          doCheck = true;
          buildPhase = ''
            export XDG_CACHE_HOME=$TMPDIR/ekko-build-cache
            export EKKO_SOURCE_DIR=$PWD
            export EKKO_OUTPUT=$PWD/ekko
            export EKKO_PLATFORM_LIBRARY=$out/lib/libekko-platform.so
            mkdir -p $out/lib
            cc -O2 -Wall -Wextra -Werror -fPIC -shared src/platform.c -o $out/lib/libekko-platform.so -lutil -lz
            sh scripts/build.sh
            EKKO_BUILD_SYSTEM=ekko/core EKKO_OUTPUT=$PWD/ekko-bare sh scripts/build.sh
            EKKO_OUTPUT=$PWD/ekko-graphics-demo sbcl --no-userinit --no-sysinit \
              --non-interactive --load scripts/build-demo.lisp
          '';
          checkPhase = ''
            sh scripts/test.sh
            sh scripts/smoke.sh "$PWD/ekko"
          '';
          installPhase = ''
            install -Dm755 ekko $out/bin/ekko
            install -Dm755 ekko-bare $out/bin/ekko-bare
            install -Dm755 ekko-graphics-demo $out/bin/ekko-graphics-demo
          '';
        };
      });
      apps = forEachSystem (pkgs: {
        performance = {
          type = "app";
          meta.description = "Measure deterministic PTY workloads and emit JSON";
          program = "${self.packages.${pkgs.system}.performance}/bin/ekko-performance";
        };
        workspace = {
          type = "app";
          meta.description = "Open an interactive shell beside terminal-browser in Ekko panes";
          program = "${self.packages.${pkgs.system}.workspace}/bin/ekko-workspace";
        };
        benchmark = {
          type = "app";
          meta.description = "Open terminal-browser and local terminal-slack in two Ekko panes";
          program = "${self.packages.${pkgs.system}.benchmark}/bin/ekko-benchmark";
        };
        demo-graphics = {
          type = "app";
          meta.description = "Write the synthetic two-pane graphics fixture to a file";
          program = "${self.packages.${pkgs.system}.default}/bin/ekko-graphics-demo";
        };
        test-kitty = {
          type = "app";
          meta.description = "Isolated real Kitty red-pixel precursor (not P0 acceptance)";
          program = "${self.packages.${pkgs.system}.kitty-oracle}/bin/ekko-kitty-red-pixel";
        };
        default = { type = "app"; meta.description = "Ekko terminal multiplexer CLI"; program = "${self.packages.${pkgs.system}.default}/bin/ekko"; };
      });
      checks = forEachSystem (pkgs: {
        daily = pkgs.runCommand "ekko-daily" { nativeBuildInputs = [ pkgs.python3 ]; } ''
          python ${./tests/daily.py} ${self.packages.${pkgs.system}.default}/bin/ekko > $out
          python ${./tests/daily.py} ${self.packages.${pkgs.system}.default}/bin/ekko-bare bare >> $out
        '';
        runtime = pkgs.runCommand "ekko-runtime" { nativeBuildInputs = [ pkgs.python3 ]; } ''
          python ${./tests/runtime.py} ${self.packages.${pkgs.system}.default}/bin/ekko > $out
        '';
        fake-host = pkgs.runCommand "ekko-fake-host" {
          nativeBuildInputs = [ pkgs.python3 ];
        } ''
          export HOME=$(mktemp -d)
          cd $(mktemp -d)
          ${self.packages.${pkgs.system}.default}/bin/ekko-graphics-demo scene.bin
          python ${./tests/fake-host.py} --self-test
          mkdir -p $out
          python ${./tests/fake-host.py} < scene.bin > $out/report.json
          cp scene.bin $out/scene.bin
          EKKO_GRAPHICS_FIXTURE=checkerboard ${self.packages.${pkgs.system}.default}/bin/ekko-graphics-demo checkerboard.bin
          python ${./tests/fake-host.py} --fixture checkerboard < checkerboard.bin > $out/checkerboard.json
          cp checkerboard.bin $out/checkerboard.bin
          EKKO_GRAPHICS_FIXTURE=native ${self.packages.${pkgs.system}.default}/bin/ekko-graphics-demo native.bin
          python ${./tests/fake-host.py} --fixture native < native.bin > $out/native.json
          cp native.bin $out/native.bin
          status=0
          ${self.packages.${pkgs.system}.default}/bin/ekko-graphics-demo >out.txt 2>err.txt || status=$?
          test "$status" -eq 2
          test ! -s out.txt
          test -s err.txt
          status=0
          EKKO_GRAPHICS_FIXTURE=unknown ${self.packages.${pkgs.system}.default}/bin/ekko-graphics-demo invalid.bin >out.txt 2>err.txt || status=$?
          test "$status" -eq 2
          test ! -e invalid.bin
          test ! -s out.txt
          test -s err.txt
        '';
        build = self.packages.${pkgs.system}.default;
        packaged-smoke = pkgs.runCommand "ekko-packaged-smoke" {} ''
          sh ${./scripts/smoke.sh} ${self.packages.${pkgs.system}.default}/bin/ekko
          touch $out
        '';
      });
      devShells = forEachSystem (pkgs: {
        default = pkgs.mkShell { packages = [ pkgs.sbcl pkgs.zlib pkgs.python3 ]; };
      });
    };
}
