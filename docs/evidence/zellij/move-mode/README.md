# Move mode evidence

The ordinary Lisp profile now implements tiled Move: Ctrl-h entry, n/Tab and p
cycling, directional letters/arrows, Normal exits, locking, and Pane transition.
It swaps stable pane-ID leaves through public `:set-layout`, retaining focus and
processes. Pinned Tab handlers ignore tiled movement while fullscreen
(`zellij-server/src/tab/mod.rs:3086–3234`); the profile does the same.

Final observed results:

* `paired-80x24`: every tiled Move stage matches input delta, focus receiver,
  and complete cell/pixel PTY histories. Fourteen cell differences remain from
  the earlier startup Escape forwarded to Ekko but consumed by release notes.
* `paired-20x8`: input, focus, and complete cell/pixel histories match. Output
  differences remain: 43 cells before movement, 29 afterward.
* `fullscreen-fixed`: movement and exit match input delta, focus, and complete
  PTY histories after the no-op guard. Startup/14-cell differences remain.
* `visual`: nine settled checkpoints have zero differing pixels and cells;
  startup differs by 50,794 pixels/1,159 cells. Both sessions cleaned up with
  no remaining children. This capture precedes the fullscreen-only guard.
* `checks`: all 17 checks except the known-failing full workflow check passed
  before the fullscreen guard. After it, core tests and regular/bare Move,
  keymap, and pane-workflow checks passed. Exact commands and outputs retained.

The first fullscreen probe encountered scenario-specific harness assumptions
about focus and zoom; readiness now preserves prior focus/zoom except where a
scenario explicitly changes them. This permits measurement rather than timing
out on the new scenario. No observed histories were removed or normalized.

`investigation/` retains an early-known-pixel run and the rejected selective
refresh experiment. The experiment was reverted: when both FIRST samples have
zero pixels, Zellij refreshes the untouched pane through its frame-update path.
`paired-80x24/move-fullscreen` preserves the pre-guard failure for comparison
with `fullscreen-fixed`.

This is bounded tiled Move evidence. Floating/stacked/grouped panes, other
shared mode transitions, bars/hints, startup timing/rendering, and the complete
remaining reference surface still require implementation and comparison.
Full parity remains false.
