# Ekko v2

Ekko opens with centered, dark window titlebars, colored borders and a fixed
bottom taskbar. The taskbar reserves its own row, so it never covers applications.
Each window keeps its accent color: open windows have filled entries, `▸` marks
focus, and minimized entries have contrasting gray backgrounds. Click the focused
entry to minimize, another entry to focus, or a minimized entry to restore.
Titlebar `_`, `□` and `×` controls minimize, maximize/restore and close.

Drag a shared tiled border to resize the neighboring windows. Hover highlights
resize handles; dragging shows outlines, and releasing applies the new sizes.
Escape cancels. Right-click a titlebar for **Float window**, or press **Ctrl-p,
then t** to toggle floating/tiling. Floating windows resize from their sides,
bottom, all four corners, and the short top-edge handle beside the upper-left
corner. The rest of the titlebar moves the window. Drag a floating or tiled
window to the outermost content cell to snap there: the top cell maximizes, the
left or right cell takes half. Double-click a titlebar to maximize or restore.

The taskbar shows a clock at its right while Normal mode is active, a `●` on
windows that produced output since you last focused them, and a `+N` chip when
entries do not fit. Wheel over an entry cycles focus; middle-click closes that
window. **Alt-Tab** lists windows most-recently-used; **Super-1** through
**Super-9** focus a taskbar slot directly. With every window minimized, the
desktop shows a centered backdrop with the session name and clock.

A cramped desktop shows as many windows as fit: when the viewport is smaller
than every window's minimum, the newest unfocused windows are hidden until the
rest fit. Hidden windows keep their PTYs and dimensions and return when you
focus them from the taskbar, the `+N` window list or **Alt-Tab**, or when the
terminal grows.

This is the default experience; no profile is required:

```sh
nix run . -- run --session workspace "$SHELL" -i
# With the configured Cudaterm launcher:
cudaterm-finix -e nix run . -- run --session workspace "$SHELL" -i
```

## Text selection and scrollback

Drag the left mouse button over shell text to highlight a character range.
Release to automatically copy it to Ekko's buffer and request the terminal's system clipboard
using OSC 52. Wide characters and combining marks stay intact; reverse and
multiline selections are supported. Clipboard access depends on the host
terminal's settings. `nix run . -- buffer workspace` also exports the text.

Copied text flashes pale yellow for 200 ms, then returns to the selection highlight.
Keyboard copy flashes before returning to live output. Typing dismisses the flash
immediately and forwards the key to the application. The flash confirms Ekko's
copy; the terminal may still deny the system clipboard request.

The mouse wheel scrolls frozen history. Scroll down to the bottom, press Escape,
or type to return to live output. Typing resumes the application without losing
the first key; Escape only dismisses the pointer selection. Pane applications
continue running during selection. Applications requesting mouse tracking keep
their mouse events instead of starting Ekko selection.

Selection retains the original terminal colours and formatting, trims trailing
blanks, and separates physical rows with newlines. Word/rectangle selection, drag
autoscroll, soft-wrap-aware copying, and full Zellij search behavior remain open.
Existing sessions need to be restarted with the rebuilt runtime for these features.

A Linux/SBCL terminal multiplexer with real PTYs, a persistent session daemon,
text terminals, and a limited Kitty graphics implementation. Each pane can run
an ordinary shell, a terminal application, or terminal-browser. Ekko manages the
panes; the Kitty graphics protocol draws images inside them.

An optional [Zellij 0.43.1 Lisp profile](docs/zellij/README.md) is under development.
It currently demonstrates Normal/Locked input routing through public keymaps;
full functional and visual parity remains open. The differential runner records
known differences and exposes a strict, currently failing acceptance gate.

Open your shell beside terminal-browser in a new Kitty window:

```sh
nix run .#workspace
```

Open the browser/Slack benchmark:

```sh
nix run .#benchmark
```

Both launchers use Ekko's pinned terminal-browser source, with a small patch
to read transport preferences from the requesting browser session. The browser
build wrapper comes from `~/dev/sandbox/terminal-slack`. The benchmark runs that checkout's real Slack
wrapper in the second pane. Slack starts at its sign-in screen when its browser
profile has no session. First launch can require downloads and a browser build.

Options: `--current-terminal`, `--session NAME`, and `--browser-url URL`.
`EKKO_SLACK_SOURCE` selects another checkout; `EKKO_SHELL` selects the workspace
shell; `TERMINAL_SLACK_URL` selects the Slack URL. Default session names are
`workspace` and `benchmark`. Running a launcher again attaches to its existing
session; use a different name to create another workspace.

Inside an existing terminal, launch arbitrary argument vectors with `:::`
between pane commands (up to 16). For example, two shells:

```sh
nix run . -- run --session shells "$SHELL" -i ::: "$SHELL" -i
```

Commands execute directly, without shell interpolation. Use `sh -c '...'`
explicitly when needed. A single command also works.

| Keys | Action |
| --- | --- |
| Ctrl-p, then r / d / n | New pane right / down / automatic |
| Ctrl-p, then Tab | Cycle focus, restoring minimized windows |
| Ctrl-p, then h / j / k / l or arrows | Focus a neighboring window |
| Ctrl-p, then m / f / x | Minimize / maximize or restore / close |
| Ctrl-p, then t | Toggle floating / tiled |
| Ctrl-p, then c | Rename; Enter saves, Escape restores the previous name |
| Ctrl-h, then h / j / k / l | Move a window |
| Ctrl-o, then d | Detach, keeping applications running |
| Ctrl-g | Lock/unlock Ekko shortcuts |
| Ctrl-q | Stop the session |
| Escape / Enter in a mode | Return to normal (rename Escape returns to pane mode) |
| Mouse drag / wheel | Copy text / scroll history |
| Mouse wheel over a taskbar entry | Cycle focus |
| Mouse middle-click a taskbar entry | Close that window |
| Double-click a titlebar | Maximize / restore the window |
| Alt-Tab | Window switcher (Tab or arrows move, Enter focuses, Escape cancels) |
| Super-1 through Super-9 | Focus that taskbar slot |


```sh
nix run . -- attach workspace
nix run . -- status workspace    # JSON: PIDs, dimensions, frames, errors, bytes
nix run . -- stop workspace
nix run . -- doctor --restore-terminal
```

Closing the window leaves the session running. `stop` terminates its owned
process groups. Recovery restores terminal modes after an unclean client exit.

Lisp init files now customize commands, keymaps, options, and status hooks, with
live reload in an isolated worker. Dynamic split trees, bounded scrollback, and
line-based copy/search are available. See [customization](docs/customization.md)
and [the example init file](examples/init.lisp). This remains a preview, not a
P0/P1/P2 release or a complete Emacs-style interactive environment.
Text emulation covers the tested shell baseline, not complete xterm behavior.
Graphics currently support direct RGB/RGBA, optional zlib compression, local
uncompressed RGB/RGBA shared-memory transfers, native
pixel placements, pane-scoped deletion, clipping, updates, and reconnect. PNG,
child file/temporary-file transport, Unicode placeholders, scaled placements, animation,
and complete keyboard/clipboard negotiation are unsupported. The launchers negotiate local shared-memory frames. Ekko snapshots them into
owned files and negotiates file delivery with the outer terminal; hosts without
local-file access receive inline frames. No frame-rate or resolution cap is imposed.
Set `TERMINAL_BROWSER_FRAMES=inline` to compare the older transport.

Attachment IPC is now version 7, including original-read input context; version-6 viewers remain supported. Existing daemons keep their executable;
start a **new session name** to use these features. Incompatible attachments
are rejected explicitly.

Build and verify the packaged executable:

```sh
nix build
nix flake check
nix run . -- --help
```

Checks include independent graphics receivers, real PTYs, reply and input routing,
clipping, scoped deletion, client death, reconnect, zoom/swap, shell job control,
terminal restoration, configuration rollback, callback termination, dependency hooks,
bare builds, split trees, and persistent copy buffers. The original synthetic fixtures remain under
`nix run .#demo-graphics`. The isolated Xvfb precursor is `nix run .#test-kitty`;
its GLX limitation is separate from the working live Wayland launch.

`nix develop` supplies SBCL, a C toolchain, zlib, and Python. Native fallback:
`sh scripts/build.sh`, then `sh scripts/test.sh`. The executable disables Lisp
init files, needs no Quicklisp cache, and ships its OS adapter in the Nix closure.

Measure fixed idle, scrolling-text, 1 MiB graphics, and 16 KiB paste workloads:

```sh
nix run .#performance -- --seconds 5 > performance.json
nix run .#performance -- --seconds 5 --direct > direct-pty.json
```

For larger animated pages, increase frame size and rate:

```sh
nix run .#performance -- --seconds 5 --graphics-size 1274 1368 --graphics-fps 30
nix run .#performance -- --seconds 5 --workload graphics --transport shared \
  --graphics-size 1274 1368 --graphics-fps 30
```

`--graphics-frame /path/to/frame.rgba` replays raw RGBA pixels of that size;
the benchmark replaces the first eight bytes with its timestamp.

The JSON records per-process CPU, sampled RSS, voluntary context switches,
terminal bytes, input latency, graphics transfer latency, and daemon allocation/GC
counters. `--direct` runs the same fixture directly in a PTY. These synthetic
measurements exclude Kitty rendering and physical display latency; the live
browser/Slack launcher remains `.#benchmark`. See [performance](docs/performance.md)
for workload details and recorded comparisons.

See [PROGRESS.md](PROGRESS.md), [architecture](docs/architecture.md), and the
[compatibility ledger](docs/compatibility.md). The destination specification is
[GOAL.md](GOAL.md). Sibling Ekko implementation code was not used.

The optional `examples/profiles/desktop.lisp` installs the same desktop through
the public extension API in `ekko-bare`. Regular Ekko includes it by default.
The Zellij profile remains an explicit compatibility experiment, not the default.
