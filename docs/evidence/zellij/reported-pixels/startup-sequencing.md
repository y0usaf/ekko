# Reported PTY pixels and the first child sample

This note records the startup ordering found in the pinned Zellij source and
the retained workflow evidence. It does not claim a universal scheduler order.

## Observed evidence

The archived `visual/report.json.gz` records the startup fixture's
first `TIOCGWINSZ` sample as `[26,47,611,650]` for A and
`[12,47,611,300]` for B/C on Zellij. The corresponding Ekko samples are the
same rows and columns with `[0,0]` pixels. This is a run-specific observation:
the harness intentionally retains every `FIRST` and `WINCH` event.

The stored matrix at
`/nix/store/nh96axfbvhw6j0bg55sap4xr7hvkbczz-ekko-zellij-pane-workflow/report.json`
shows that startup pixel geometry matches in `navigation`, `new-r`,
`close-x`, and `fullscreen-navigation`, while it differs in `titles`, `new-n`,
and `new-d`. Thus “FIRST has pixels” is not a stable property of every sample.
The same report says cell geometry is equal at startup in all these cases;
the discrepancy is specifically the pixel fields and subsequent rendering.

## Source-derived order

The fixture's child records `FIRST` immediately after `tty.setraw` and before
its first `read` or output in `tests/zellij/pane_workflow_differential.py`
(`inspect_spawn_source`, around lines 218–286). Its `size()` calls
`TIOCGWINSZ`, so the recorded value is whatever the PTY has when the child is
scheduled.

In Zellij, `Pty::spawn_terminals_for_layout` first extracts and spawns all run
instructions (`zellij-server/src/pty.rs:976–1014`). The OS path calls `openpty(None, &orig_termios)` in `handle_terminal`
(`zellij-server/src/os_input_output.rs:223–245`), which does not establish
the layout's pixel dimensions. The server then applies the layout:
`Screen::apply_layout` calls `Tab::apply_layout`, and afterward
`tab.resize_whole_tab(self.size)` (`zellij-server/src/screen.rs:1548–1580`).
The layout applier sends `resize_pty!` with the current shared character cell
size when it creates/places panes (`zellij-server/src/tab/layout_applier.rs:
484–530` and `965–992`). The macro emits `PtyWriteInstruction::ResizePty`
with columns, rows, and optional pixel width/height
(`zellij-server/src/tab/mod.rs:75–119`); the writer performs the OS resize in
`zellij-server/src/pty_writer.rs:49–67`.

The source establishes a spawn-then-layout-resize pipeline. The shared
character cell dimensions also depend on query reply processing
(`zellij-server/src/screen.rs:1244` onward). The retained samples are consistent
with metric availability and child scheduling interacting with this pipeline;
they do not isolate which ordering caused each FIRST value. Zero pixels with
correct rows/columns must not be described as proof that FIRST preceded every
layout resize. A synchronized event trace is still needed to separate these
causes.

## Implemented mechanism and remaining investigation

The current Ekko slice already keeps two values per pane: the renderer's physical cell metrics and the last
kernel PTY tuple. Before spawning a child, compute the final pane geometry and
apply one PTY resize carrying `(cols, rows, pixel_width, pixel_height)`; record
that tuple only after the spawn/resize succeeds. Later layout changes compare
the desired tuple with the recorded tuple and issue a resize when different.
This is a generic PTY lifecycle primitive and naturally preserves `WINCH`.

When the outer client learns physical cell size after startup, publish that as
daemon state and make the next normal layout/reconciliation perform the
resize. Packet receipt itself should update state and revision without forcing
layout. No sleep, retry loop, or Zellij-specific delegation can make the
child's first scheduler observation deterministic; the evidence must retain
whether `FIRST` preceded or followed the resize.


This mechanism alone does not resolve the early-known-pixel discrepancy: Ekko
currently launches children before its viewer receives metric replies. No
startup handshake change was implemented from this investigation. The next
step is a trace of query emission/reception, layout application, kernel resize,
and child FIRST observations under controlled event ordering, then a decision
about a reusable launch/attachment sequencing mechanism. Full resize histories
and every existing mismatch remain acceptance requirements.
