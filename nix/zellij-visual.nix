{ pkgs }:

let
  visualTools = with pkgs; [
    cage
    kitty
    grim
    imagemagick
    mesa-demos
    fontconfig
    dejavu_fonts
    (python3.withPackages (p: [ p.pillow p.pyte ]))
    coreutils
    gnugrep
    bash
  ];
  fontConfig = pkgs.writeText "ekko-zellij-visual-fonts.conf" ''
    <?xml version="1.0"?>
    <!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
    <fontconfig>
      <dir>${pkgs.dejavu_fonts}</dir>
      <cachedir prefix="xdg">fontconfig</cachedir>
    </fontconfig>
  '';
in pkgs.writeShellApplication {
  name = "ekko-zellij-visual";
  runtimeInputs = visualTools;
  text = ''
    export PATH="${pkgs.lib.makeBinPath visualTools}"
    export LIBGL_DRIVERS_PATH="${pkgs.mesa}/lib/dri"
    export __EGL_VENDOR_LIBRARY_FILENAMES="${pkgs.mesa}/share/glvnd/egl_vendor.d/50_mesa.json"
    export FONTCONFIG_FILE="${fontConfig}"
    if test "''${1:-}" = compare; then
      shift
      exec python ${./../scripts/oracle/zellij-visual-compare.py} "$@"
    fi
    exec ${pkgs.bash}/bin/bash ${./../scripts/oracle/zellij-visual.sh} "$@"
  '';
}
