# Implementation progress

Active outcome: complete functional and visual parity with pinned Zellij 0.43.1
through an optional, replaceable public Lisp profile, while preserving Ekko's
independent daemon, transactional reload, reversible ownership, and graphics
isolation. The full acceptance gate is in GOAL.md; it remains open.

Reboot recovery (2026-09-06): the previous goal turn made implementation and
verification progress; the complete goal remains active. The worktree and stored
Nix packages survived. Background jobs and the latest temporary paired/Kitty
captures did not survive. Replacement runs were attempted without reusing old
PIDs, but could not reach fixture startup. Fresh `nix build --no-link -L` is denied
access to the Nix daemon socket under the new sandbox; this is an environmental
validation blocker, not a passing result. See [recovery status and build failure](docs/evidence/zellij/recovery/status.json).
Recovered [historical Nix outputs](docs/evidence/zellij/recovery-workflow/README.md)
confirm regular/bare pane workflow and startup geometry at 80×24 and 20×8,
plus pane-note checks. Fresh daemon and compositor runs are also socket-blocked;
there is no new paired or screenshot pass. The paired harness now correctly
expects a right split at 20×8 and evaluates automatic split eligibility using
all three initial rectangles. Native Python syntax and boundary checks passed;
these do not replace Nix validation. Source investigation records
[title lifecycle gaps](docs/zellij/title-observations.md) and
[pixel metric timing](docs/evidence/zellij/recovery-pixels/README.md).
Full resize histories, titles, cursor raster differences, and the unimplemented
surface remain required; the latest lost traces are labeled reconstructed.

Pane workflow extension (in progress, 2026-09-05): Luna agents are implementing
and probing directional/cyclic focus, automatic and explicit splits, close focus,
and failed-split frames. The reusable core now exposes activation order, tiled
rectangles during fullscreen, effective cell pixels, targeted split/close focus,
startup layout trees, horizontal arrow keys, and expiring owner-scoped pane notes.
`nix build --no-link -L` passed for the core and unit contracts (log
`/tmp/ekko-workflow-notes-build.log`); final workflow integration and the full Nix
suite are pending. Reference observations and remaining differences are in
[Pane observations](docs/zellij/pane-observations.md). This extends the existing
partial slice; it does not satisfy the full parity gate.

Zellij foundation (2026-09-05): pinned the exact release/default config/default
layout before implementing the profile, inventoried the upstream user-facing
surface, and added public owner-scoped keymaps, active-mode snapshots, and
validated keymap-switch actions. The optional Lisp profile currently implements
Normal/Locked routing and partial Pane controls. It is not a full Zellij
compatibility profile yet.
The real-PTY differential harness retains all cell and application-input
differences and raw streams. The reference's fresh release-notes UI causes an
Escape-input mismatch; the initial foundation captures have different chrome/cursor
state. Batched mode transitions also need further upstream investigation.
The strict acceptance gate remains failing; no differences are waived.
See [reference and evidence](docs/zellij/README.md),
[complete surface worklist](docs/zellij/surface.md), and
[public APIs](docs/customization.md). The private Kitty/Xvfb capture probe returned
77: no GLX framebuffer configuration; its logs are recorded. Screenshot acceptance
remains required. Earlier interrupted-turn audit: no Zellij work or live task
process was found; resumed work made concrete implementation and oracle progress.

Verification: `nix build -L`, `nix flake check -L` (all eight checks), and
`nix run . -- --help` exited zero. Real worker/PTY tests cover Unicode key
matching/forwarding, ownership, rollback, and profile switching without changing
pane PIDs in both ordinary and bare executables. The settled routing slice passes
at 80×24 and 20×8; modeled cells differ at every checkpoint. Full input parity is
false at 80×24 (release notes) and true only for this small sequence at 20×8.
`--require-parity` exits 1. Evidence: [commands and hashes](docs/evidence/zellij/checks.json),
[80×24](docs/evidence/zellij/80x24/summary.json),
[20×8](docs/evidence/zellij/20x8/summary.json). Raw ANSI and compressed full cell
reports accompany the summaries; no normalization is applied.

Pane controls (2026-09-05): public action batches now accept one session action
followed by one validated keymap transition, with trailing mode/status changes
committed only after the primary action succeeds. The optional profile implements
Pane entry/exits, locking, ignored unbound input, and fullscreen followed by
Normal. Real two-PTY tests exercise regular/bare executables, exact geometry
restoration, reload, and preserved application PIDs. A separate custom two-pane
reference scenario records resize events and all cells; its geometry differs
from Ekko and does not replace default-layout acceptance. Rapid reference mode
keys can leave Locked or Pane depending on scheduling; this discrepancy remains
open. See [observations](docs/zellij/pane-observations.md).
`nix build -L` and `nix flake check -L` (nine checks) passed. Both paired
settled Pane input slices pass at 80×24 and 20×8; all visual checkpoints
differ. [Verification and hashes](docs/evidence/zellij/pane-controls/checks.json).

The private screenshot environment now works using pinned Cage headless/pixman,
Kitty Wayland, Mesa llvmpipe, and DejaVu Sans Mono. The [1280×720 fixture capture](docs/evidence/zellij/wayland-precursor/screenshot.png)
was inspected; renderer/font logs accompany it. This is capture infrastructure,
not application screenshot parity. The earlier Xvfb failure remains recorded.

Geometry and capture follow-up (2026-09-05): public pane/viewport insets and
column/row split gaps now drive outer rectangles, PTY content dimensions, cursor
placement, and image clipping. Reload reapplies geometry without restarting pane
processes. Attachment wire **version 4** carries outer rectangles and geometry
metadata. Defaults preserve the previous layout. At this checkpoint the Zellij profile did not yet
use these options. A pure Lisp frame helper and pinned tests covered the observed
printable-ASCII title/counter formatting; integration follows below. `nix build -L` and
`nix flake check -L` (nine checks) passed after integration.
[Checks and package paths](docs/evidence/zellij/geometry-v4/checks.json).

Paired private Kitty screenshots now run both actual multiplexers with the same
fixture argv, paths, terminal environment, and outer dimensions. The v4
[report](docs/evidence/zellij/wayland-paired-v4/report.json) retains startup and
Escape screenshots, full compressed cell grids/diffs, raw ANSI/replies, and clean
session shutdown records. Escape differs in 78,243 pixels and 452 modeled cells;
application geometry and input also differ. These are failing parity evidence.
The earlier [v3 capture](docs/evidence/zellij/wayland-paired-v3/report.json) is
preserved separately. No complete functional or visual parity is claimed.

Decoration integration (2026-09-05): public `:decorate` actions now replace
component-owned, bounded styled spans. The daemon clips them outside application
content and composes them in registration order. Default chrome and the optional
profile both use this API; attachment wire **version 5** carries the result.
Snapshots expose viewport, outer/content rectangles, visibility, zoom, and actual
main-screen history counts. The profile loads its frame helper and enables ED2
history retention through an ordinary option. Regular and bare real-daemon
decoration tests pass, including overlapping owners, removal, reload, invalid
spans, clipping, and preserved PTYs. Keymap and pane-mode checks also pass.
The integrated custom 80×24 Pane scenario matches all modeled cells and cursor
after Escape; release notes, initial PTY sizing, small terminals, default bars,
and the broader surface still fail or lack acceptance coverage. `nix build -L` and all 11 `nix flake check -L` checks pass.
[Final build and cell evidence](docs/evidence/zellij/decorations-v5/checks.json)
and [private Kitty evidence](docs/evidence/zellij/wayland-paired-v5/report.json)
retain candidate paths and known differences; the screenshot candidate predates
the final ED2 color-parser fix.

Startup sizing and hook follow-up (2026-09-05): `run` now passes its viewport
before daemon startup, and the daemon computes public-option geometry before
spawning application PTYs. Split actions likewise spawn at the planned content
size and preserve existing state on failure. Paired tests now require matching
application dimension histories; the fixed 80×24 and 20×8 runs pass this gate.
Regular/bare tests cover first ioctl sizes with 9×17 pixel cells, reattachment,
new splits, failed spawn, terminal-less fallback, and invalid startup arguments.

The full suite exposed a coeffect race: unrelated pane output could discard a
focus-only hook result because stale validation compared the entire snapshot.
Validation now compares declared reads, with deterministic regressions for
unrelated changes, changed dependencies, and initially running constant hooks.
`nix build -L` and the full `nix flake check -L` suite pass after the fix;
the strict gate exits 1 for known parity gaps.
[Checks and paired cells](docs/evidence/zellij/startup-v5/checks.json) and
[verified-binary screenshots](docs/evidence/zellij/wayland-startup-v5/report.json)
retain all differences. The pinned small-screen
probe rules out the tested delays, ED2, and release notes as explanations for
blank reference content at 20×8; the internal cause remains unresolved.
[Reference controls](docs/evidence/zellij/small-startup/provenance.json) retain
raw output and effective release-note controls. Full parity remains open.

The browser/Slack benchmark and separate shell/browser launcher remain earlier
preview outcomes. The wider GOAL.md release and extension gates remain open;
no P0, P1, or P2 release is claimed.

Daily-use foundation (2026-09-05): Lisp init and reload, public commands/keymaps,
declared status hooks, owner reconstruction, split trees, scrollback, and copy/search
are implemented. Builtins use the public API; a separate bare executable is checked
with an external init command. The worker has load/dispatch deadlines and recovers
without stopping PTYs. That foundation introduced attachment IPC **version 3**, superseded by v4 above.
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
