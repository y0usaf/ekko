{
  description = "Ekko v2 terminal multiplexer";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-25.05";
  inputs.terminal-browser = {
    url = "github:zenbu-labs/terminal-browser/cce10b6131d15bf46a3e4b8dc827e0544ff7fc65";
    flake = false;
  };
  inputs.cudaterm-data = {
    url = "github:y0usaf/cudaterm/c1acd3202c27275a8d8061ed7547dd45eb42637d";
    flake = false;
  };
  outputs = { self, nixpkgs, terminal-browser, cudaterm-data }:
    let
      textWidthData = pkgs: pkgs.runCommand "ekko-cudaterm-width-data" rec {
        nativeBuildInputs = [ (pkgs.python3.withPackages (p: [ p.unicodedata2 ])) pkgs.gnutar pkgs.gzip ];
        unifontSource = "mirror://gnu/unifont/unifont-17.0.05/unifont-17.0.05.tar.gz";
        unifont = pkgs.fetchurl {
          url = unifontSource;
          hash = "sha256-8ofP+ybiJyOqNuZoSGmw8/87+4IsSwEAi9hHkR7BtjE=";
        };
      } ''
        tar -xzf $unifont --strip-components=3 \
          unifont-17.0.05/font/precompiled/unifont-combining-17.0.05.txt
        mkdir -p $out
        tar -xOf $unifont --wildcards '*/COPYING' > $out/UNIFONT-COPYING
        printf '%s\n' "$unifontSource" > $out/unifont-source.txt
        substitute ${cudaterm-data}/tools/build_widths.py build-widths.py \
          --replace-fail 'import unicodedata' 'import unicodedata2 as unicodedata'
        python build-widths.py \
          --combining unifont-combining-17.0.05.txt --out $out/widths.bin \
          --offsets-out $out/offsets.bin --version-out $out/unicode-version.txt
      '';
      systems = [ "x86_64-linux" ];
      forEachSystem = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in {
      packages = forEachSystem (pkgs: {
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
          widthData = textWidthData pkgs;
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [
              ./ekko.asd
              (pkgs.lib.fileset.fileFilter (file: file.hasExt "lisp") ./examples/profiles)
              (pkgs.lib.fileset.fileFilter (file: file.hasExt "lisp" || file.hasExt "c") ./src)
              ./scripts/build.sh ./scripts/build.lisp
              ./scripts/smoke.sh
              ./scripts/generate-text-width.py
            ];
          };
          nativeBuildInputs = [ pkgs.sbcl ];
          buildInputs = [ pkgs.zlib (pkgs.zstd.override { static = true; }) ];
          dontConfigure = true;
          # A saved SBCL core is appended to the ELF runtime. Stripping loses it.
          dontStrip = true;
          dontPatchELF = true;
          doCheck = true;
          buildPhase = ''
            export XDG_CACHE_HOME=$TMPDIR/ekko-build-cache
            export EKKO_SOURCE_DIR=$PWD
            export EKKO_OUTPUT=$PWD/ekko
            sh scripts/build.sh
          '';
          checkPhase = ''
            sh scripts/smoke.sh "$PWD/ekko"
          '';
          installPhase = ''
            install -Dm755 ekko $out/bin/ekko
            ln -s ekko $out/bin/ekko-bare
            install -Dm644 $widthData/UNIFONT-COPYING $out/share/doc/ekko/UNIFONT-COPYING
            install -m644 $widthData/unifont-source.txt ${cudaterm-data}/data/UNICODE-LICENSE.txt \
              $out/share/doc/ekko/
          '';
        };
        graphics-demo = pkgs.stdenv.mkDerivation {
          pname = "ekko-graphics-demo";
          version = "0.1.0";
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [
              ./ekko.asd ./scripts/build-demo.lisp
              (pkgs.lib.fileset.fileFilter (file: file.hasExt "lisp") ./src)
            ];
          };
          nativeBuildInputs = [ pkgs.sbcl ];
          dontConfigure = true;
          dontStrip = true;
          dontPatchELF = true;
          buildPhase = ''
            export XDG_CACHE_HOME=$TMPDIR/ekko-build-cache
            EKKO_SOURCE_DIR=$PWD EKKO_OUTPUT=$PWD/ekko-graphics-demo sbcl --no-userinit --no-sysinit \
              --non-interactive --load scripts/build-demo.lisp
          '';
          installPhase = ''
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
          program = "${self.packages.${pkgs.system}.graphics-demo}/bin/ekko-graphics-demo";
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
        # Enforced byte identity: ekko.org is the source of every file it
        # tangles, so a generated file that was hand-edited behind org's back
        # must fail this check.  It copies the tree twice, re-tangles one copy
        # from ekko.org, and compares the result against the untouched copy.
        tangle = pkgs.runCommand "ekko-tangle" {
          nativeBuildInputs = [ pkgs.emacs-nox ];
          input = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [
              ./ekko.org ./ekko.asd ./flake.nix ./src ./examples ./scripts ./nix
            ];
          };
        } ''
          cp -r $input expected
          cp -r $input work
          chmod -R u+w expected work
          export HOME=$TMPDIR
          cd work
          emacs --batch --eval '(progn (require (quote org)) (org-babel-tangle-file "ekko.org"))'
          cd ..
          status=0
          for rel in ekko.asd flake.nix $(cd expected && find src examples scripts nix -type f | sort); do
            if cmp -s "expected/$rel" "work/$rel"; then
              echo "same     $rel"
            else
              echo "CHANGED  $rel -- the committed file is not what ekko.org tangles to" >&2
              status=1
            fi
          done
          if [ "$status" -ne 0 ]; then
            echo "ekko.org is the source; re-tangle it (scripts/tangle.sh) rather than editing generated files" >&2
            exit 1
          fi
          touch $out
        '';
        text-width = pkgs.runCommand "ekko-text-width" {
          nativeBuildInputs = [ pkgs.python3 pkgs.sbcl ];
          widthData = textWidthData pkgs;
        } ''
          python ${./scripts/generate-text-width.py} $widthData ${cudaterm-data}/data \
            --output generated.lisp --info-output original-info
          cmp generated.lisp ${./src/text-width.lisp}
          sbcl --noinform --non-interactive --load ${./src/text-width.lisp} \
            --eval '(with-open-file (widths "lisp-widths" :direction :output :element-type (quote (unsigned-byte 8)))
                      (with-open-file (info "lisp-info" :direction :output :element-type (quote (unsigned-byte 8)))
                        (loop for cp from 0 to #x10ffff do
                          (write-byte (ekko/text:code-info cp) info)
                          (unless (<= #xd800 cp #xdfff)
                            (write-byte (ekko/text:display-width (code-char cp)) widths)))))'
          python -c "from pathlib import Path; w = Path('$widthData/widths.bin').read_bytes(); Path('original-widths').write_bytes(w[:0xd800] + w[0xe000:])"
          cmp original-widths lisp-widths
          cmp original-info lisp-info
          printf 'exhaustive Unicode scalar widths and cluster properties match pinned cudaterm data\n' > $out
        '';
        build = self.packages.${pkgs.system}.default;
        graphics-demo = self.packages.${pkgs.system}.graphics-demo;
        packaged-smoke = pkgs.runCommand "ekko-packaged-smoke" {} ''
          sh ${./scripts/smoke.sh} ${self.packages.${pkgs.system}.default}/bin/ekko
          touch $out
        '';
        bare-core = self.packages.${pkgs.system}.default.overrideAttrs (old: {
          pname = "ekko-bare-core";
          buildPhase = "export EKKO_BUILD_SYSTEM=ekko/core\n" + old.buildPhase;
          checkPhase = old.checkPhase + ''
            export HOME=$TMPDIR/bare XDG_CONFIG_HOME=$TMPDIR/bare/config XDG_RUNTIME_DIR=$TMPDIR/bare/run
            export EKKO_INSTANCE=bare-core SHELL=/bin/sh
            mkdir -p $XDG_CONFIG_HOME
            mkdir -m 700 $XDG_RUNTIME_DIR
            ./ekko run --detached sh -c 'exec sleep 600' || { cat $XDG_RUNTIME_DIR/ekko/*log >&2; exit 1; }
            ./ekko list > list.json
            pids=$(grep -o '"[a-z_]*pid":[0-9]*' list.json | cut -d: -f2)
            test $(echo $pids | wc -w) -eq 3
            ./ekko stop --force
            for pid in $pids; do
              for i in $(seq 50); do kill -0 $pid 2>/dev/null || break; sleep 0.1; done
              if kill -0 $pid 2>/dev/null; then echo "process $pid outlived ekko stop" >&2; exit 1; fi
            done
            for log in $XDG_RUNTIME_DIR/ekko/*log; do
              if test -s $log; then cat $log >&2; exit 1; fi
            done
          '';
        });
        pty-harness = pkgs.runCommand "ekko-pty-harness" {
          nativeBuildInputs = [ (pkgs.python3.withPackages (p: [ p.pyte ])) ];
        } ''
          python ${./scripts/pty-harness.py} ${self.packages.${pkgs.system}.default}/bin/ekko
          touch $out
        '';
      });
      devShells = forEachSystem (pkgs: {
        default = pkgs.mkShell { packages = [ pkgs.sbcl pkgs.zlib pkgs.python3 ]; };
      });
    };
}
