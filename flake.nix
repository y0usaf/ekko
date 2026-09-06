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
        zellij-reference = let pin = builtins.fromJSON (builtins.readFile ./tests/zellij/reference/pin.json); in
          assert pkgs.zellij.version == pin.release;
          assert pkgs.zellij.src.outputHash == pin.source_nar_hash;
          assert nixpkgs.rev == pin.nixpkgs_revision;
          pkgs.zellij;
        zellij-pane-probe = pkgs.writeShellScriptBin "ekko-zellij-pane-probe" ''
          exec ${pkgs.python3.withPackages (p: [ p.pyte ])}/bin/python ${./tests/zellij}/pane_probe.py \
            --zellij ${self.packages.${pkgs.system}.zellij-reference}/bin/zellij "$@"
        '';
        zellij-pane-differential = pkgs.writeShellScriptBin "ekko-zellij-pane-differential" ''
          exec ${pkgs.python3.withPackages (p: [ p.pyte ])}/bin/python ${./tests/zellij}/pane_differential.py \
            --zellij ${self.packages.${pkgs.system}.zellij-reference}/bin/zellij \
            --ekko ${self.packages.${pkgs.system}.default}/bin/ekko \
            --profile ${./examples/profiles}/zellij.lisp "$@"
        '';
        zellij-pane-workflow = pkgs.writeShellScriptBin "ekko-zellij-pane-workflow" ''
          exec ${pkgs.python3.withPackages (p: [ p.pyte ])}/bin/python ${./tests/zellij}/pane_workflow_differential.py \
            --zellij ${self.packages.${pkgs.system}.zellij-reference}/bin/zellij \
            --ekko ${self.packages.${pkgs.system}.default}/bin/ekko \
            --profile ${./examples/profiles}/zellij.lisp \
            --reference ${./tests/zellij}/reference "$@"
        '';
        zellij-session-lifecycle = pkgs.writeShellScriptBin "ekko-zellij-session-lifecycle" ''
          exec ${pkgs.python3.withPackages (p: [ p.pyte ])}/bin/python ${./tests/zellij}/session_lifecycle.py \
            --zellij ${self.packages.${pkgs.system}.zellij-reference}/bin/zellij \
            --ekko ${self.packages.${pkgs.system}.default}/bin/ekko \
            --profile ${./examples/profiles}/zellij.lisp \
            --reference ${./tests/zellij}/reference "$@"
        '';
        zellij-differential = pkgs.writeShellScriptBin "ekko-zellij-differential" ''
          exec ${pkgs.python3.withPackages (p: [ p.pyte ])}/bin/python ${./tests/zellij}/differential.py \
            --zellij ${self.packages.${pkgs.system}.zellij-reference}/bin/zellij \
            --ekko ${self.packages.${pkgs.system}.default}/bin/ekko \
            --profile ${./examples/profiles}/zellij.lisp "$@"
        '';
        browser-source = pkgs.stdenvNoCC.mkDerivation {
          name = "ekko-terminal-browser-source";
          src = terminal-browser;
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
        zellij-visual = import ./nix/zellij-visual.nix { inherit pkgs; };
        default = pkgs.stdenv.mkDerivation {
          pname = "ekko";
          version = "0.1.0";
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [
              ./ekko.asd
              ./examples/profiles/zellij-frames.lisp
              ./examples/profiles/zellij-pane.lisp
              (pkgs.lib.fileset.fileFilter (file: file.hasExt "lisp" || file.hasExt "c") ./src)
              (pkgs.lib.fileset.fileFilter (file: file.hasExt "lisp" || file.hasExt "py") ./tests)
              ./scripts/build.sh ./scripts/build.lisp ./scripts/build-demo.lisp
              ./scripts/test.sh ./scripts/test.lisp ./scripts/smoke.sh
              ./scripts/generate-text-width.py ./scripts/text-width-oracle.rs
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
        zellij-pane-probe = {
          type = "app";
          meta.description = "Observe pinned Zellij Pane mode and batched input";
          program = "${self.packages.${pkgs.system}.zellij-pane-probe}/bin/ekko-zellij-pane-probe";
        };
        zellij-pane-differential = {
          type = "app";
          meta.description = "Settled Pane-mode PTY differential; visual parity remains open";
          program = "${self.packages.${pkgs.system}.zellij-pane-differential}/bin/ekko-zellij-pane-differential";
        };
        zellij-session-lifecycle = {
          type = "app";
          meta.description = "Pinned quit/detach lifecycle comparison; full parity remains open";
          program = "${self.packages.${pkgs.system}.zellij-session-lifecycle}/bin/ekko-zellij-session-lifecycle";
        };
        zellij-differential = {
          type = "app";
          meta.description = "Differential Zellij reference runner; full parity remains open";
          program = "${self.packages.${pkgs.system}.zellij-differential}/bin/ekko-zellij-differential";
        };
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
        zellij-visual = {
          type = "app";
          meta.description = "Private Wayland Kitty visual capture precursor";
          program = "${self.packages.${pkgs.system}.zellij-visual}/bin/ekko-zellij-visual";
        };
        default = { type = "app"; meta.description = "Ekko terminal multiplexer CLI"; program = "${self.packages.${pkgs.system}.default}/bin/ekko"; };
      });
      checks = forEachSystem (pkgs: {
        text-width = pkgs.runCommand "ekko-text-width" {
          nativeBuildInputs = [ pkgs.python3 pkgs.rustc pkgs.sbcl pkgs.gnutar pkgs.stdenv.cc ];
          unicodeWidthCrate = pkgs.fetchurl {
            url = "https://static.crates.io/crates/unicode-width/unicode-width-0.1.10.crate";
            sha256 = "c0edd1e5b14653f783770bce4a4dabb4a5108a5370a5f5d8cfe8710c361f6c8b";
          };
        } ''
          mkdir crate
          tar -xf $unicodeWidthCrate -C crate
          python ${./scripts/generate-text-width.py} \
            crate/unicode-width-0.1.10 --rustc rustc --output generated.lisp
          cmp generated.lisp ${./src/text-width.lisp}
          rustc --crate-name unicode_width --crate-type lib --edition=2018 \
            crate/unicode-width-0.1.10/src/lib.rs -o libunicode_width.rlib
          rustc ${./scripts/text-width-oracle.rs} \
            --extern unicode_width=$PWD/libunicode_width.rlib -o rust-oracle
          ./rust-oracle > rust-widths
          sbcl --noinform --non-interactive --load ${./src/text-width.lisp} \
            --eval '(loop for cp from 0 to #x10ffff do
                      (unless (<= #xd800 cp #xdfff)
                        (format t "~D~%" (ekko/text:display-width (code-char cp)))))' \
            > lisp-widths
          cmp rust-widths lisp-widths
          printf 'exhaustive Unicode scalar widths match unicode-width 0.1.10\n' > $out
        '';
        pane-modes = pkgs.runCommand "ekko-pane-modes" { nativeBuildInputs = [ pkgs.python3 ]; } ''
          python ${./tests}/pane_modes.py ${self.packages.${pkgs.system}.default}/bin/ekko ${./examples/profiles}/zellij.lisp > $out
          python ${./tests}/pane_modes.py ${self.packages.${pkgs.system}.default}/bin/ekko-bare ${./examples/profiles}/zellij.lisp bare >> $out
        '';
        pane-workflow = pkgs.runCommand "ekko-pane-workflow" { nativeBuildInputs = [ pkgs.python3 ]; } ''
          python ${./tests}/pane_workflow.py ${self.packages.${pkgs.system}.default}/bin/ekko ${./examples/profiles}/zellij.lisp regular 80 24 > $out
          python ${./tests}/pane_workflow.py ${self.packages.${pkgs.system}.default}/bin/ekko ${./examples/profiles}/zellij.lisp regular 20 8 >> $out
          python ${./tests}/pane_workflow.py ${self.packages.${pkgs.system}.default}/bin/ekko-bare ${./examples/profiles}/zellij.lisp bare 80 24 >> $out
          python ${./tests}/pane_workflow.py ${self.packages.${pkgs.system}.default}/bin/ekko-bare ${./examples/profiles}/zellij.lisp bare 20 8 >> $out
        '';
        keymap-input = pkgs.runCommand "ekko-keymap-input" { nativeBuildInputs = [ pkgs.python3 ]; } ''
          python ${./tests}/keymap_input.py ${self.packages.${pkgs.system}.default}/bin/ekko > $out
          python ${./tests}/keymap_input.py ${self.packages.${pkgs.system}.default}/bin/ekko-bare bare >> $out
          python ${./tests}/keymap_input.py --read-bytes ${self.packages.${pkgs.system}.default}/bin/ekko >> $out
          python ${./tests}/keymap_input.py --read-bytes ${self.packages.${pkgs.system}.default}/bin/ekko-bare >> $out
        '';
        keymaps = pkgs.runCommand "ekko-keymaps" { nativeBuildInputs = [ pkgs.python3 ]; } ''
          python ${./tests}/keymaps.py ${self.packages.${pkgs.system}.default}/bin/ekko ${./examples/profiles}/zellij.lisp > $out
          python ${./tests}/keymaps.py ${self.packages.${pkgs.system}.default}/bin/ekko-bare ${./examples/profiles}/zellij.lisp bare >> $out
        '';
        decorations = pkgs.runCommand "ekko-decorations" { nativeBuildInputs = [ pkgs.python3 ]; } ''
          python ${./tests}/decorations.py ${self.packages.${pkgs.system}.default}/bin/ekko ${./examples/profiles}/zellij.lisp > $out
          python ${./tests}/decorations.py ${self.packages.${pkgs.system}.default}/bin/ekko-bare ${./examples/profiles}/zellij.lisp bare >> $out
        '';
        pane-notes = pkgs.runCommand "ekko-pane-notes" { nativeBuildInputs = [ pkgs.python3 ]; } ''
          python ${./tests}/pane_notes.py ${self.packages.${pkgs.system}.default}/bin/ekko ${./examples/profiles}/zellij.lisp > $out
          python ${./tests}/pane_notes.py ${self.packages.${pkgs.system}.default}/bin/ekko-bare ${./examples/profiles}/zellij.lisp bare >> $out
        '';
        pane-titles = pkgs.runCommand "ekko-pane-titles" { nativeBuildInputs = [ pkgs.python3 ]; } ''
          python ${./tests}/pane_titles.py ${self.packages.${pkgs.system}.default}/bin/ekko ${./examples/profiles}/zellij.lisp > $out
          python ${./tests}/pane_titles.py ${self.packages.${pkgs.system}.default}/bin/ekko-bare ${./examples/profiles}/zellij.lisp bare >> $out
        '';
        pane-rename = pkgs.runCommand "ekko-pane-rename" { nativeBuildInputs = [ pkgs.python3 ]; } ''
          python ${./tests}/pane_rename.py ${self.packages.${pkgs.system}.default}/bin/ekko ${./examples/profiles}/zellij.lisp > $out
          python ${./tests}/pane_rename.py ${self.packages.${pkgs.system}.default}/bin/ekko-bare ${./examples/profiles}/zellij.lisp bare >> $out
        '';
        viewer-exit = pkgs.runCommand "ekko-viewer-exit" { nativeBuildInputs = [ pkgs.python3 ]; } ''
          python ${./tests}/viewer_exit.py ${self.packages.${pkgs.system}.default}/bin/ekko > $out
          python ${./tests}/viewer_exit.py ${self.packages.${pkgs.system}.default}/bin/ekko-bare >> $out
        '';
        initialization = pkgs.runCommand "ekko-initialization" { nativeBuildInputs = [ pkgs.python3 ]; } ''
          python ${./tests}/initialization.py ${self.packages.${pkgs.system}.default}/bin/ekko > $out
          python ${./tests}/initialization.py ${self.packages.${pkgs.system}.default}/bin/ekko-bare >> $out
        '';
        pane-frames = pkgs.runCommand "ekko-pane-frames" { nativeBuildInputs = [ pkgs.python3 ]; } ''
          python ${./tests}/pane_frames.py ${self.packages.${pkgs.system}.default}/bin/ekko ${./examples/profiles}/zellij.lisp > $out
          python ${./tests}/pane_frames.py ${self.packages.${pkgs.system}.default}/bin/ekko-bare ${./examples/profiles}/zellij.lisp >> $out
        '';
        pane-moves = pkgs.runCommand "ekko-pane-moves" { nativeBuildInputs = [ pkgs.python3 ]; } ''
          python ${./tests}/pane_moves.py ${self.packages.${pkgs.system}.default}/bin/ekko ${./examples/profiles}/zellij.lisp > $out
          python ${./tests}/pane_moves.py ${self.packages.${pkgs.system}.default}/bin/ekko-bare ${./examples/profiles}/zellij.lisp bare >> $out
        '';
        pane-layouts = pkgs.runCommand "ekko-pane-layouts" { nativeBuildInputs = [ pkgs.python3 ]; } ''
          python ${./tests}/pane_layouts.py ${self.packages.${pkgs.system}.default}/bin/ekko > $out
          python ${./tests}/pane_layouts.py ${self.packages.${pkgs.system}.default}/bin/ekko-bare bare >> $out
        '';
        pane-pixels = pkgs.runCommand "ekko-pane-pixels" { nativeBuildInputs = [ pkgs.python3 ]; } ''
          python ${./tests}/pane_pixels.py ${self.packages.${pkgs.system}.default}/bin/ekko > $out
          python ${./tests}/pane_pixels.py ${self.packages.${pkgs.system}.default}/bin/ekko-bare bare >> $out
        '';
        startup-geometry = pkgs.runCommand "ekko-startup-geometry" { nativeBuildInputs = [ pkgs.python3 ]; } ''
          python ${./tests}/startup_geometry.py ${self.packages.${pkgs.system}.default}/bin/ekko ${./examples/profiles}/zellij.lisp regular 80 24 > $out
          python ${./tests}/startup_geometry.py ${self.packages.${pkgs.system}.default}/bin/ekko ${./examples/profiles}/zellij.lisp regular 20 8 >> $out
          python ${./tests}/startup_geometry.py ${self.packages.${pkgs.system}.default}/bin/ekko-bare ${./examples/profiles}/zellij.lisp bare 80 24 >> $out
          python ${./tests}/startup_geometry.py ${self.packages.${pkgs.system}.default}/bin/ekko-bare ${./examples/profiles}/zellij.lisp bare 20 8 >> $out
        '';
        zellij-session-lifecycle = pkgs.runCommand "ekko-zellij-session-lifecycle" {} ''
          mkdir -p $out
          ${self.packages.${pkgs.system}.zellij-session-lifecycle}/bin/ekko-zellij-session-lifecycle --output $out/regular
          ${self.packages.${pkgs.system}.zellij-session-lifecycle}/bin/ekko-zellij-session-lifecycle \
            --ekko ${self.packages.${pkgs.system}.default}/bin/ekko-bare --only detach --output $out/bare
        '';
        zellij-pane-workflow = pkgs.runCommand "ekko-zellij-pane-workflow" {} ''
          mkdir -p $out
          ${self.packages.${pkgs.system}.zellij-pane-workflow}/bin/ekko-zellij-pane-workflow --output $out
        '';
        zellij-routing = pkgs.runCommand "ekko-zellij-routing" {} ''
          ${self.packages.${pkgs.system}.zellij-differential}/bin/ekko-zellij-differential --output $out/80x24
          ${self.packages.${pkgs.system}.zellij-differential}/bin/ekko-zellij-differential --cols 20 --rows 8 --output $out/20x8
        '';
        zellij-pane-differential = pkgs.runCommand "ekko-zellij-pane-differential" {} ''
          ${self.packages.${pkgs.system}.zellij-pane-differential}/bin/ekko-zellij-pane-differential --output $out/80x24
          ${self.packages.${pkgs.system}.zellij-pane-differential}/bin/ekko-zellij-pane-differential --cols 20 --rows 8 --output $out/20x8
        '';

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
