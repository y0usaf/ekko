# Reported pixel paired workflow

Current core/profile, query responses enabled unless stated. Every run returned
the runner's expected exit 1 due functional or visual differences; no report
normalization was applied. Full scenario artifacts are under `80x24/scenarios`
and `20x8/scenarios`.

* 80x24 all scenarios: application and pixel geometry are equal at startup,
  titles, creation, close, and fullscreen checkpoints. Directional focus
  stages still have application/pixel geometry differences. Startup differs by
  1045 cells; later differences include release-note Escape/input and chrome.
* 20x8 all scenarios: title/new-pane-down/new-pane-right/close/fullscreen
  application and pixel geometry are equal. Directional focus and new-pane
  (no preference) geometry differ. Input is equal in this small viewport.
* 120x24 `new-n` with query responses: geometry diverges at creation (212
  cells). Zellij sends WINCH to B and C at 464x160; Ekko additionally resizes
  A to 224x352. Initial A/B/C pixel values are 0x0 on both sides.
* 120x24 `new-n` without query responses: application and pixel geometry remain
  equal through creation; Zellij's query responses are absent and its new-pane
  WINCH retains 0x0 pixels. This run exits 0 for the named slice.

The 120x24 result is a measured aspect/fallback difference, not waived. Exact
commands, package paths, hashes, query traces, and cleanup records are in
`provenance.json` and the per-run directories.

## Auto layout trace

The 120x24 query `new-pane` checkpoint is geometrically explicit. Zellij keeps
A at `22x58` cells with no pixel metadata, resizes B and C to `10x58` and
`464x160`, and starts the new pane at `21x58` with `464x336`. Ekko keeps the
same initial A/B/C cell sizes, but resizes A and starts pane 4 at `22x28` with
`224x352`; its B/C remain `10x58`, `464x160`. The captured frame shows
Zellij's new pane between A and the B/C column, while Ekko's pane occupies a
30-column middle column. This is an auto-layout geometry difference, not a
query-response interpretation.

Pinned source supports the ordering: `tab/mod.rs:1331-1425` constructs a
no-preference pane with `PaneGeom::default()` and inserts it through
`add_tiled_pane`; `tab/mod.rs:5014-5035` marks auto layout damaged and invokes
`relayout_tiled_panes`; `tab/mod.rs:916-958` swaps a candidate layout and then
resizes the tiled pane tree. `swap_layouts.rs:206-276` selects the next layout
whose constraint fits the new pane count. The exact captured values are in the
120x24 reports; the source establishes the mechanism but does not by itself
prove which candidate was selected.

At 20x8 no-preference creation, Zellij records A `WINCH` to `6x8,64x96`, B/C
`WINCH` to `2x8,64x32`, and a new pane `FIRST` at `0x0`; Ekko records no WINCH
and no corresponding new-pane event in the retained checkpoint. This is an
observed small-viewport creation gap and remains unnormalized.
