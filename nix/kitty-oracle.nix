{ pkgs }:

pkgs.writeShellApplication {
  name = "ekko-kitty-red-pixel";
  runtimeInputs = [
    pkgs.kitty
    pkgs.xorg.xorgserver
    pkgs.xorg.xwd
    pkgs.imagemagick
    pkgs.coreutils
    pkgs.gnugrep
    pkgs.gnused
    pkgs.mesa
    pkgs.fontconfig
    pkgs.dejavu_fonts
    pkgs.bash
  ];
  text = ''
    export LIBGL_DRIVERS_PATH="${pkgs.mesa}/lib/dri"
    export FONTCONFIG_FILE="${pkgs.makeFontsConf { fontDirectories = [ pkgs.dejavu_fonts ]; }}"
    exec ${pkgs.bash}/bin/bash ${./../scripts/oracle/kitty-red-pixel.sh}
  '';
}
