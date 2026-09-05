# Implementation progress

Active outcome: a live, customizable multiplexer preview. The browser/Slack benchmark
runs in Kitty on the Wayland desktop. A separate workspace launcher opens an
interactive shell beside terminal-browser. The wider GOAL.md release and extension
gates remain open; no P0, P1, or P2 release is claimed.

Daily-use foundation (2026-09-05): Lisp init and reload, public commands/keymaps,
declared status hooks, owner reconstruction, split trees, scrollback, and copy/search
are implemented. Builtins use the public API; a separate bare executable is checked
with an external init command. The worker has load/dispatch deadlines and recovers
without stopping PTYs. Attachment IPC is now **version 3**; use a new session name.
See [customization](docs/customization.md) for limits and examples. This is an
initial extension surface, not completion of the wider release gates.
`nix build -L`, `nix flake check -L`, CLI help, and the example init check exited
zero. Final replay p95 was 6.82 ms, with about 7.0% combined Ekko CPU; the new
worker adds 123.8 MiB sampled RSS. Evidence: [checks](docs/evidence/daily-checks.json)
and [replay](docs/evidence/daily-performance.json).

Local transport update (2026-09-05): raw shared-memory ingress, immutable daemon
snapshots, filename IPC, capability-probed Kitty file egress, and acknowledged
scene lifetimes are implemented. CookUnity-sized replay p95 frame receipt is
97.66 → 8.84 ms; combined Ekko CPU is about 70% → 9% of one core. No FPS or
resolution cap is used. A pinned dependency patch makes browser transport
preferences session-owned. The `cookunity-shared` workspace visibly uses the new
path; physical display latency remains unmeasured. This update introduced attachment
IPC version 2 (superseded by version 3 above). See docs/adr/003-local-frame-transport.md and the
performance report for resource ownership, fallback, evidence and limitations.
`nix build -L`, `nix flake check -L`, and `nix run . -- --help` exited zero.

Performance update (2026-09-04): cached text rows, placement-only image updates,
batched input, constant-time output queue insertion, and deadline-based idle
polling are implemented. `nix run .#performance` provides repeatable JSON
measurements with daemon allocation/GC counters. `nix build` and
`nix flake check -L` passed with new input/rendering/timeout regressions.
See [performance measurements](docs/performance.md) for the saved baseline and
optimized results; this does not extend the preview's compatibility claims.

Graphics follow-up: profiled and removed repeated Base64 alphabet searches,
decode/re-encode validation, payload string conversions, and per-byte APC buffering.
The same synthetic graphics workload improved from 132.6 ms to 60.9 ms p95 frame
receipt, with about 76% fewer daemon allocations. The Nix build and full checks
passed, including new codec/fragmentation/quota regression tests. Website visual
quality still requires separate live validation; see the performance report.

CookUnity follow-up: specialized byte loops, exact-size upload assembly, and
batched client uploads improve a captured 1274 × 1368 frame replay from 61 to 89
frames in five seconds (p95 receipt 147.8 → 97.6 ms). The benchmark now supports
larger frames, frame rates, and captured RGBA input. Nix build/checks passed,
including fragmented multi-chunk uploads with full pixel validation. An updated
`cookunity-fast` session is running; existing daemons retain their old executable.
See the performance report for evidence and display-latency limitations.

Implemented:

- A C OS adapter for controlling PTYs, direct argv execution, initial dimensions,
  nonblocking I/O, sockets, terminal modes, process groups, and zlib.
- A Lisp session daemon, bounded local IPC, and reconnectable client.
- Basic text VT state, colors, cursor/margins/erase, alternate screen, child
  replies, keyboard modes, pixel mouse routing, paste, and focus.
- Pane-owned direct RGB/RGBA graphics with zlib, native placement, clipping,
  independent host IDs, scoped deletion, repeated frames, and reconstruction.
- Mixed split trees (up to 16 panes), dynamic creation/close, zoom, swap, resize, and rename.
- Bounded main-screen history, frozen copy mode, line selection/search, and a daemon buffer.
- Public Lisp commands/keymaps/options/status hooks, isolated worker deadlines, atomic reload, and owner inspection.
- `nix run .#benchmark` for terminal-browser + local terminal-slack.
- `nix run .#workspace` for the user's shell + terminal-browser.
- Packaged integration tests for collision/clipping/reply/input isolation,
  shell job control, SIGTERM restoration, client death, reconnect, and teardown.

The pinned SBCL process API was probed first: child descriptors were terminals,
but there was no controlling terminal. The C adapter establishes it without
executing Lisp between fork and exec; see [ADR-002](docs/adr/002-pty-runtime.md).

Actual applications: terminal-browser from terminal-slack's locked upstream
revision `cce10b6131d15bf46a3e4b8dc827e0544ff7fc65`, and the local terminal-slack
checkout at `03a8d78273159c7592b5555db36fc5f7da3b91f2` (pre-existing deleted docs).
The original preview needed no application source changes and selected inline
Kitty RGBA frames. The local-transport follow-up above adds the scoped browser
environment patch and enables shared-memory negotiation.

Live verification on 2026-09-04: Kitty 0.42.1 launched successfully with Wayland.
The browser and Slack sign-in page were visibly present in independent panes;
concurrent browser updates preserved Slack's image. A window-only screenshot
was inspected in /tmp, not added to the repository. The earlier Xvfb/GLX precursor
still has its separate unavailable result in docs/evidence/kitty-precursor.

The original three M01 synthetic fixtures and their earlier evidence are retained.
The current Nix runtime check additionally exercises the installed executable
through actual PTYs and an independent restricted Kitty receiver. Final build
and check results are recorded in docs/evidence/runtime.json.

Limitations: one attached writer, incomplete VT/terminfo compatibility, line-only
copy selection, no history reflow or host clipboard, and a bounded initial extension API.
Graphics support is deliberately limited to the benchmark's native direct
RGB/RGBA subset. Full PNG, placeholders, remaining transfer adapters, scaled placement,
animation, compositor layers, and the remaining GOAL.md gates remain open.
Slack authentication beyond the displayed sign-in screen is user-controlled.
