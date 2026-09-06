# Pinned Zellij pixel lifecycle probe

The paired `new-d` workflow was run against pinned Zellij 0.43.1 at 80x24
with terminal-query responses enabled and disabled, and at 120x24 with
responses enabled. Reports retain every query, response, cursor, fixture
dimension event, and modeled cell; no resize or pixel history was normalized.

Measured behavior:

* With responses enabled, Zellij emits startup queries `CSI 14 t` (text area)
  and `CSI 16 t` (cell size). The response is `640x384`/`8x16` at 80x24 and
  `960x384`/`8x16` at 120x24. Ekko emits only `CSI 16 t`.
* In these earlier 80x24 and 120x24 samples, Zellij's initial fixture `FIRST`
  events reported pixel dimensions `[0,0]` for A/B/C, both with and without
  query responses. A later current run observed known pixels at `FIRST` on
  Zellij while Ekko still reported zero, so this is a sample result rather
  than a universal Zellij invariant.
* After `new-pane-down`, the responding Zellij run reports a `WINCH` for all
  existing panes at `[304,160]` (80x24) or `[464,160]` (120x24), while the
  no-response Zellij run records no pixel update (`[0,0]`). Ekko resizes only
  A to `[304,160]` and leaves B/C unchanged.
* The 80x24 query run returned exit 1 with `application_cell_geometry_equal`
  false at `new-pane-down`; the no-query run returned exit 0. The 120x24
  query run returned exit 1 with the same geometry mismatch. This isolates
  the mismatch to pixel geometry/terminal-query behavior in this probe.

Interpretation: the pinned server creates child PTYs before the client’s
pixel-dimension response has updated `ScreenContext`; `Screen::pixel_dimensions`
starts at its default and is later used for pane resize. The response changes
the later resize path, but cannot retroactively change the initial `FIRST`
event. This ordering is inferred from the source and confirmed by the traces;
the traces alone establish ordering and values, not internal scheduling.

Relevant pinned source: `route.rs:1066-1073` forwards
`TerminalPixelDimensions`; `screen.rs:795` initializes pixel dimensions to the
default and `screen.rs:1244-1251` merges the response and derives cell size;
`tab/mod.rs:52-82` sends pixel-aware `ResizePty` instructions; and
`pty_writer.rs:12-16,54-73` applies them to the child PTY. Exact source hashes
are in `source-sha256.txt`.
