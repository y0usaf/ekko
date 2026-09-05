# Ekko v2

A Linux/SBCL terminal multiplexer with real PTYs, a persistent session daemon,
text terminals, and a limited Kitty graphics implementation. Each pane can run
an ordinary shell, a terminal application, or terminal-browser. Ekko manages the
panes; the Kitty graphics protocol draws images inside them.

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
between the two pane commands. For example, two shells:

```sh
nix run . -- run --session shells "$SHELL" -i ::: "$SHELL" -i
```

Commands execute directly, without shell interpolation. Use `sh -c '...'`
explicitly when needed. A single command also works.

| Keys | Action |
| --- | --- |
| Ctrl-b, then Tab / 1 / 2 | Switch focus |
| Mouse click | Focus and interact with the clicked application |
| Ctrl-b, then z | Toggle focused-pane zoom |
| Ctrl-b, then s | Swap panes |
| Ctrl-b, then < / > | Resize the divider |
| Ctrl-b, then d | Detach, keeping applications running |
| Ctrl-b, then x | Close the focused pane's process group |
| Ctrl-b, then q | Stop the session |
| Ctrl-b, then Ctrl-b | Send Ctrl-b to the application |

```sh
nix run . -- attach workspace
nix run . -- status workspace    # JSON: PIDs, dimensions, frames, errors, bytes
nix run . -- stop workspace
nix run . -- doctor --restore-terminal
```

Closing the window leaves the session running. `stop` terminates its owned
process groups. Recovery restores terminal modes after an unclean client exit.

This is a two-pane preview, not a P0/P1/P2 release. Dynamic pane creation,
arbitrary split trees, scrollback/copy mode, and extensions are not implemented.
Most runtime policy is Common Lisp, but customization currently means editing
source and rebuilding. A user init file, public commands/hooks, configurable
keymaps, and live reload are planned; Emacs-style customization is not yet available.
Text emulation covers the tested shell baseline, not complete xterm behavior.
Graphics currently support direct RGB/RGBA, optional zlib compression, local
uncompressed RGB/RGBA shared-memory transfers, native
pixel placements, pane-scoped deletion, clipping, updates, and reconnect. PNG,
child file/temporary-file transport, Unicode placeholders, scaled placements, animation,
and complete keyboard/clipboard negotiation are unsupported. The launchers negotiate local shared-memory frames. Ekko snapshots them into
owned files and negotiates file delivery with the outer terminal; hosts without
local-file access receive inline frames. No frame-rate or resolution cap is imposed.
Set `TERMINAL_BROWSER_FRAMES=inline` to compare the older transport.

This changes attachment IPC to version 2. Existing daemons keep their executable;
start a **new session name** to use the new transport. Incompatible attachments
are rejected explicitly.

Build and verify the packaged executable:

```sh
nix build
nix flake check
nix run . -- --help
```

Checks include independent graphics receivers, real PTYs, reply and input routing,
clipping, scoped deletion, client death, reconnect, zoom/swap, shell job control,
and terminal restoration. The original synthetic fixtures remain under
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
