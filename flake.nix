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
              (pkgs.lib.fileset.fileFilter (file: file.hasExt "lisp") ./examples/profiles)
              (pkgs.lib.fileset.fileFilter (file: file.hasExt "lisp" || file.hasExt "c") ./src)
              ./scripts/build.sh ./scripts/build.lisp ./scripts/build-demo.lisp
              ./scripts/smoke.sh
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
