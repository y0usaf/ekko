#!/usr/bin/env bash
set -euo pipefail

# Capture Kitty through a private wlroots headless compositor.  Cage's
# headless output is intentionally fixed at 1280x720 by this wlroots backend; the
# dimensions are checked from the decoded PNG below.
CAGE_BIN=${CAGE_BIN:-cage}
KITTY_BIN=${KITTY_BIN:-kitty}
GRIM_BIN=${GRIM_BIN:-grim}
EGLINFO_BIN=${EGLINFO_BIN:-eglinfo}
FC_MATCH_BIN=${FC_MATCH_BIN:-fc-match}
IDENTIFY_BIN=${IDENTIFY_BIN:-identify}
CONVERT_BIN=${CONVERT_BIN:-convert}

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/ekko-zellij-visual.XXXXXX")
output_dir=${EKKO_ORACLE_OUTPUT_DIR:-}
if test -n "$output_dir"; then
  mkdir -p "$output_dir"
fi

run_dir="$tmp_dir/runtime"
mkdir -m 700 "$run_dir"
export XDG_RUNTIME_DIR="$run_dir"
export WAYLAND_DISPLAY=wayland-0
export HOME="$tmp_dir/home"
export XDG_CONFIG_HOME="$tmp_dir/config"
export XDG_CACHE_HOME="$tmp_dir/cache"
export XDG_DATA_HOME="$tmp_dir/data"
mkdir -p "$HOME" "$XDG_CONFIG_HOME" "$XDG_CACHE_HOME" "$XDG_DATA_HOME"

# Keep both compositor and client rendering on software Mesa, and make the
# only font visible to Kitty explicit and reproducible.
export LIBGL_ALWAYS_SOFTWARE=1
export MESA_LOADER_DRIVER_OVERRIDE=llvmpipe
export WLR_BACKENDS=headless
export WLR_RENDERER=pixman
export WLR_LIBINPUT_NO_DEVICES=1
export WLR_HEADLESS_OUTPUTS=1
export WLR_NO_HARDWARE_CURSORS=1
export LC_ALL=C
export LANG=C

compositor_pid=''
cleanup() {
  test -z "$compositor_pid" || kill "$compositor_pid" 2>/dev/null || true
  test -z "$compositor_pid" || wait "$compositor_pid" 2>/dev/null || true
  if test -n "$output_dir"; then
    cp -f "$tmp_dir"/* "$output_dir"/ 2>/dev/null || true
  fi
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

for tool in "$CAGE_BIN" "$KITTY_BIN" "$GRIM_BIN" "$EGLINFO_BIN" "$FC_MATCH_BIN" "$IDENTIFY_BIN" "$CONVERT_BIN"; do
  command -v "$tool" >/dev/null || {
    printf '{"status":"unavailable","oracle":"zellij-visual","reason":"missing-tool"}\n' >"$tmp_dir/report.json"
    printf 'zellij-visual: unavailable (missing tool: %s)\n' "$tool" >&2
    exit 77
  }
done

cat >"$tmp_dir/environment.txt" <<EOF
wayland_display=$WAYLAND_DISPLAY
backend=headless
compositor=cage
compositor_renderer=pixman
client_renderer=llvmpipe
font_family=DejaVu Sans Mono
fontconfig_file=$FONTCONFIG_FILE
dimensions=1280x720
EOF

"$FC_MATCH_BIN" -f '%{family}|%{style}|%{file}\n' 'DejaVu Sans Mono' >"$tmp_dir/font-match.txt"
font_match=$(cut -d'|' -f1-2 <"$tmp_dir/font-match.txt")
if test "$font_match" != 'DejaVu Sans Mono|Book'; then
  printf '{"status":"unavailable","oracle":"zellij-visual","reason":"font-mismatch"}\n' >"$tmp_dir/report.json"
  printf 'zellij-visual: unavailable (font match: %s)\n' "$font_match" >&2
  exit 77
fi

# Cage supplies one private Wayland socket and one headless output.  Kitty is
# the sole client, so no existing desktop display or surface is involved.
"$CAGE_BIN" -d -- "$KITTY_BIN" --config NONE \
  --override linux_display_server=wayland \
  --override font_family='DejaVu Sans Mono' \
  --override font_size=16 \
  --override remember_window_size=no \
  --class ekko-zellij-visual \
  sh -c 'printf "EKKO ZELLIJ PRIVATE WAYLAND\\nbackend=kitty-wayland\\nrenderer=llvmpipe\\nfont=DejaVu Sans Mono\\n"; sleep 8' \
  >"$tmp_dir/cage.out" 2>"$tmp_dir/cage.err" &
compositor_pid=$!

for _ in $(seq 1 100); do
  test -S "$run_dir/$WAYLAND_DISPLAY" && break
  kill -0 "$compositor_pid" 2>/dev/null || break
  sleep 0.1
done
if ! test -S "$run_dir/$WAYLAND_DISPLAY"; then
  printf '{"status":"unavailable","oracle":"zellij-visual","reason":"wayland-socket"}\n' >"$tmp_dir/report.json"
  printf 'zellij-visual: unavailable (private Wayland socket was not created)\n' >&2
  exit 77
fi

sleep 1
"$EGLINFO_BIN" -B -p wayland >"$tmp_dir/eglinfo.txt" 2>"$tmp_dir/eglinfo.err"
if ! grep -Fq 'OpenGL core profile renderer: llvmpipe' "$tmp_dir/eglinfo.txt"; then
  printf '{"status":"unavailable","oracle":"zellij-visual","reason":"renderer-mismatch"}\n' >"$tmp_dir/report.json"
  printf 'zellij-visual: unavailable (EGL did not report llvmpipe)\n' >&2
  exit 77
fi

sleep 1
image="$tmp_dir/screenshot.png"
"$GRIM_BIN" "$image" >"$tmp_dir/grim.out" 2>"$tmp_dir/grim.err"
dimensions=$($IDENTIFY_BIN -format '%wx%h' "$image")
if test "$dimensions" != '1280x720'; then
  printf '{"status":"fail","oracle":"zellij-visual","reason":"dimensions","dimensions":"%s"}\n' "$dimensions" >"$tmp_dir/report.json"
  printf 'zellij-visual: FAIL (expected 1280x720, got %s)\n' "$dimensions" >&2
  exit 1
fi
maxima=$($CONVERT_BIN "$image" -format '%[fx:maxima]' info: 2>/dev/null)
if test "$maxima" = '0' || test -z "$maxima"; then
  printf '{"status":"fail","oracle":"zellij-visual","reason":"blank-image"}\n' >"$tmp_dir/report.json"
  printf 'zellij-visual: FAIL (decoded image is blank)\n' >&2
  exit 1
fi

sha256sum "$image" >"$tmp_dir/screenshot.sha256"
printf '{"status":"pass","oracle":"zellij-visual","compositor":"cage","backend":"headless","compositor_renderer":"pixman","client_renderer":"llvmpipe","font":"DejaVu Sans Mono Book","dimensions":"%s","image":"screenshot.png"}\n' "$dimensions" >"$tmp_dir/report.json"
printf 'zellij-visual: PASS (%s, Cage headless/pixman, Kitty Wayland/llvmpipe, DejaVu Sans Mono)\n' "$dimensions"
