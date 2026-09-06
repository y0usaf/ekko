# Automatic no-preference pane layout investigation

This is an evidence record and design note. No automatic-layout behavior was
changed during this investigation.

The current paired runner starts with the tree

```lisp
(:columns 50 1 (:rows 50 2 3))
```

The existing profile already implements the largest-splittable-pane policy in
`zellij-pane-find-room-for-new-pane`. It scores an eligible pane as
`rows * ratio * cols`, chooses the greatest score, and returns the existing
generic `(:split :pane ID :axis AXIS :argv ARGV)` action. In the current paired
run the reported physical cell is 8x16, so `ratio` is 2. The fallback ratio 4
is used only when reported metrics are unavailable.

Pinned Zellij has the same policy in
`zellij-server/src/panes/tiled_panes/tiled_pane_grid.rs:1350-1394`:
`find_room_for_new_pane` uses `unwrap_or(DEFAULT_CURSOR_HEIGHT_WIDTH_RATIO)`;
the constant is 4 at `:19`, and the reported ratio is rounded in
`tiled_panes/mod.rs:1755-1762`. The direction is horizontal (`:rows`) when
`rows * ratio > cols` and there is sufficient height; otherwise it is vertical
(`:columns`) when there is sufficient width. `new_no_preference_pane` starts
with `PaneGeom::default()` (`tab/mod.rs:1331-1425`), then
`add_tiled_pane` invokes the auto-layout path (`tab/mod.rs:5014-5035`). The
subsequent swap/layout and resize path is in `tab/mod.rs:916-958` and
`tab/swap_layouts.rs:206-276`.

The policy uses outer pane rectangles (including frames), as does the current
Lisp helper. Its calculation is:

| viewport | measured largest candidate | ratio | score | existing action plan |
|---|---:|---:|---:|---|
| 80x24 | pane 1, 40x24 | 2 | 1920 | split pane 1 by `:rows` |
| 120x24 | pane 1, 60x24 | 2 | 2880 | split pane 1 by `:columns` |
| 20x8 | pane 1, 10x8 | 2 | 160 | no eligible split under the minimum checks |

These are action plans from the existing profile policy, not asserted final
trees. At 80x24 the initial tree is observed as
`(:columns 50 1 (:rows 50 2 3))`; at 120x24 it is also that tree; at 20x8 the
captured no-preference checkpoint retains that tree on the Ekko side.

The captured 120x24 query-response records show the mismatch at the
`new-pane` checkpoint. The exact event records are retained in
`paired/120x24-query/report.json.gz` and the corresponding raw frame in
`paired/120x24-query/zellij.ansi.gz`:

* Zellij A remains `22x58` cells with no pixel metadata; B and C receive
  `WINCH 10x58, 464x160`; the new process records `FIRST 21x58, 464x336`.
* Ekko A records `WINCH 22x28, 224x352`; B and C remain `10x58, 464x160`;
  the new process records `FIRST 22x28, 224x352`.

The raw Zellij frame places the new-pane title between A and the B/C column at
that captured checkpoint. It does not include a later Zellij pane-geometry
snapshot, so the record establishes the observed checkpoint and event order,
not an eventual settled rectangle or a settled tree. The source sequence
explains why a default geometry exists before relayout, but does not identify a
different selection policy. The no-query 120x24 slice has equal application
and pixel geometry through creation, which further bounds this result to the
query-response path.

At 20x8, Zellij records A `WINCH 6x8,64x96`, B/C `WINCH 2x8,64x32`, and a new
process `FIRST 0x0`; Ekko records no corresponding new-pane event in the
retained checkpoint. This is an observed small-viewport lifecycle difference,
not evidence for a ratio-4 policy.

The current Ekko action API is sufficient to express the measured policy. The
remaining investigation target is the query-response creation/relayout timing
and a settled geometry observation for the Zellij 120x24 checkpoint. Adding a
new generic largest-pane query or changing the profile selection rule would be
speculative and is not justified by these records.
