#!/usr/bin/env bash
set -euo pipefail

slack_source=${EKKO_SLACK_SOURCE:-$HOME/dev/sandbox/terminal-slack}
mode=${EKKO_WORKSPACE_MODE:-browser-slack}
case "$mode" in
  browser-slack) default_session=benchmark; window_title='Ekko v2 — browser + Slack' ;;
  shell-browser) default_session=workspace; window_title='Ekko v2 — shell + browser' ;;
  *) printf 'Unknown workspace mode: %s\n' "$mode" >&2; exit 2 ;;
esac
session=${EKKO_BENCHMARK_SESSION:-$default_session}
browser_url=${EKKO_BROWSER_URL:-https://example.com}
current_terminal=false
while (($#)); do
  case "$1" in
    --current-terminal) current_terminal=true; shift ;;
    --session|--browser-url)
      if (($# < 2)); then printf '%s requires a value\n' "$1" >&2; exit 2; fi
      if [[ $1 == --session ]]; then session=$2; else browser_url=$2; fi
      shift 2 ;;
    --help|-h)
      printf '%s\n' 'Usage: nix run .#benchmark -- [--current-terminal] [--session NAME] [--browser-url URL]' \
        '       nix run .#workspace -- [--current-terminal] [--session NAME] [--browser-url URL]' \
        'benchmark: browser + Slack. workspace: your shell + browser. Both open a Kitty window.' \
        'EKKO_SLACK_SOURCE overrides ~/dev/sandbox/terminal-slack. TERMINAL_SLACK_URL overrides the Slack URL.'
      exit 0 ;;
    *) printf 'Unknown benchmark option: %s\n' "$1" >&2; exit 2 ;;
  esac
done

test -f "$slack_source/flake.nix" || { printf 'Missing terminal-slack checkout: %s\n' "$slack_source" >&2; exit 1; }
browser_options=()
if [[ -n ${EKKO_BROWSER_SOURCE:-} ]]; then
  browser_options=(--override-input terminal-browser "path:$EKKO_BROWSER_SOURCE" --no-write-lock-file)
fi
browser_package=$(nix build --no-link --print-out-paths "path:$slack_source#browser" "${browser_options[@]}")
benchmark_dir=${XDG_STATE_HOME:-$HOME/.local/state}/ekko-v2/benchmark
mkdir -p "$benchmark_dir/bin"
ln -sfn "$browser_package/bin/terminal-slack-browser" "$benchmark_dir/bin/terminal-browser"

# Let the browser negotiate the supported local transport.
# An explicit inline override remains useful for transport comparisons.
export TERMINAL_BROWSER_FRAMES=${TERMINAL_BROWSER_FRAMES:-auto}
export TERMINAL_BROWSER_GRAPHICS=kitty
export FONTCONFIG_FILE="$EKKO_FONTCONFIG"
unset TMUX STY HERDR_SOCKET HERDR_PANE_ID
if [[ $mode == shell-browser ]]; then
  command=("$EKKO_BINARY" run --session "$session" "${EKKO_SHELL:-${SHELL:-bash}}" -i
           ::: "$benchmark_dir/bin/terminal-browser" open "$browser_url")
else
  slack_package=$(nix build --no-link --print-out-paths "path:$slack_source" "${browser_options[@]}")
  command=("$EKKO_BINARY" run --session "$session" "$benchmark_dir/bin/terminal-browser" open "$browser_url"
           ::: "$slack_package/bin/terminal-slack")
fi
if "$current_terminal"; then
  exec "${command[@]}"
fi
exec "$EKKO_KITTY" --config NONE --class "ekko-v2-$default_session" --title "$window_title" \
  --override initial_window_width=160c --override initial_window_height=48c \
  --override font_size=11 --override confirm_os_window_close=0 "${command[@]}"
