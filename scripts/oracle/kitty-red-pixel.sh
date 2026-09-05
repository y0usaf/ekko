#!/usr/bin/env bash
set -euo pipefail

# Isolated Kitty oracle precursor. This deliberately captures a private Xvfb
# display and checks decoded pixels; successful escape-sequence emission alone
# is never considered a pass.
KITTY_BIN=${KITTY_BIN:-kitty}
XVFB_BIN=${XVFB_BIN:-Xvfb}
XWD_BIN=${XWD_BIN:-xwd}
CONVERT_BIN=${CONVERT_BIN:-convert}

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/ekko-kitty-oracle.XXXXXX")
output_dir=${EKKO_ORACLE_OUTPUT_DIR:-}
if test -n "$output_dir"; then
  mkdir -p "$output_dir"
fi
display=''
xvfb_pid=''
kitty_pid=''
cleanup() {
  test -z "$kitty_pid" || kill "$kitty_pid" 2>/dev/null || true
  test -z "$xvfb_pid" || kill "$xvfb_pid" 2>/dev/null || true
  test -z "$kitty_pid" || wait "$kitty_pid" 2>/dev/null || true
  test -z "$xvfb_pid" || wait "$xvfb_pid" 2>/dev/null || true
  if test -n "$output_dir"; then
    cp -f "$tmp_dir"/* "$output_dir"/ 2>/dev/null || true
  fi
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

for tool in "$KITTY_BIN" "$XVFB_BIN" "$XWD_BIN" "$CONVERT_BIN"; do
  command -v "$tool" >/dev/null || {
    printf 'missing required tool: %s\n' "$tool" >&2
    printf '{"status":"unavailable","reason":"missing-tool","tool":"%s"}\n' "$tool" >"$tmp_dir/report.json"
    exit 77
  }
done

"$XVFB_BIN" -displayfd 3 -screen 0 800x600x24 -ac +extension GLX +iglx +render \
  -nolisten tcp 3>"$tmp_dir/display" >"$tmp_dir/xvfb.log" 2>&1 &
xvfb_pid=$!
for _ in $(seq 1 50); do
  test -s "$tmp_dir/display" && break
  kill -0 "$xvfb_pid" 2>/dev/null || {
    printf 'Xvfb exited before allocating a display; log: %s\n' "$tmp_dir/xvfb.log" >&2
    printf '{"status":"unavailable","reason":"xvfb-exited"}\n' >"$tmp_dir/report.json"
    exit 77
  }
  sleep 0.1
done
test -s "$tmp_dir/display" || {
  printf 'Xvfb did not allocate a display; log: %s\n' "$tmp_dir/xvfb.log" >&2
  printf '{"status":"unavailable","reason":"xvfb-no-display"}\n' >"$tmp_dir/report.json"
  exit 77
}
read -r display_number <"$tmp_dir/display"
display=":${display_number}"
for _ in $(seq 1 50); do
  if DISPLAY="$display" "$XWD_BIN" -root -silent > /dev/null 2> /dev/null; then
    break
  fi
  kill -0 "$xvfb_pid" 2>/dev/null || {
    printf 'Xvfb lost before becoming ready; log: %s\n' "$tmp_dir/xvfb.log" >&2
    printf '{"status":"unavailable","reason":"xvfb-lost"}\n' >"$tmp_dir/report.json"
    exit 77
  }
  sleep 0.1
done
DISPLAY="$display" "$XWD_BIN" -root -silent > /dev/null

# One raw RGB red pixel, uploaded through Kitty's graphics protocol.
DISPLAY="$display" LIBGL_ALWAYS_SOFTWARE=1 \
  MESA_LOADER_DRIVER_OVERRIDE=llvmpipe \
  "$KITTY_BIN" --config NONE \
  --override linux_display_server=x11 --class ekko-oracle sh -c \
  'printf "\033_Ga=T,f=24,s=1,v=1,m=0;/wAA\033\\"; sleep 30' \
  >"$tmp_dir/kitty.out" 2>"$tmp_dir/kitty.err" &
kitty_pid=$!
sleep 1
kill -0 "$kitty_pid" 2>/dev/null || {
  printf 'Kitty exited before capture; log: %s\n' "$tmp_dir/kitty.err" >&2
  sed -n '1,80p' "$tmp_dir/kitty.err" >&2
  if grep -Eiq 'GLX|OpenGL|GLFWwindow|fbconfig' "$tmp_dir/kitty.err"; then
    printf '{"status":"unavailable","reason":"kitty-glx"}\n' >"$tmp_dir/report.json"
    exit 77
  fi
  printf '{"status":"unavailable","reason":"kitty-exited"}\n' >"$tmp_dir/report.json"
  exit 77
}
"$XWD_BIN" -display "$display" -root -silent |
  "$CONVERT_BIN" xwd:- -depth 8 txt:- >"$tmp_dir/pixels.txt"

if grep -Fq '(255,0,0)' "$tmp_dir/pixels.txt"; then
  printf '{"status":"pass","oracle":"kitty-red-pixel"}\n' >"$tmp_dir/report.json"
  printf 'kitty-red-pixel: PASS (captured red pixel on isolated display)\n'
else
  printf '{"status":"fail","oracle":"kitty-red-pixel","reason":"pixel-mismatch"}\n' >"$tmp_dir/report.json"
  printf 'kitty-red-pixel: FAIL (no captured red pixel)\n' >&2
  sed -n '1,80p' "$tmp_dir/kitty.err" >&2
  exit 1
fi
