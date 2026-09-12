# Pane mode observations (Zellij 0.43.1)

The executable at
`/nix/store/wxjfyl2ksqkh263zwikp2igqh2a23y8r-zellij-0.43.1/bin/zellij`
was driven through a real PTY by [`pane_probe.py`](../../tests/zellij/pane_probe.py).
The probe creates two named, deterministic terminal panes (`A` and `B`),
uses an explicit two-terminal layout without tab/status plugins (a separate
layout scenario, not the pinned default layout), records the complete pyte cell grid and raw ANSI stream at every checkpoint,
records bytes read by each child, and records each child's `SIGWINCH` size.
For example:

```sh
nix run .#zellij-pane-probe -- --output /tmp/ekko-zellij-pane-probe
nix run .#zellij-pane-probe -- --cols 20 --rows 8 --output /tmp/ekko-zellij-pane-probe-small
```

At 80×24, each tiled app receives 38×22 cells. At startup Zellij displays
the 0.43.1 release notes; the probe's Escape dismisses it and reaches no app.
At 20×8 there is no release-notes overlay, so that same Escape reaches the
focused A app (`1b`). This is retained as observed input, not normalized.

From Normal, a settled `Ctrl-p` enters Pane mode and reaches neither app.
The Pane `f` action toggles the focused A pane fullscreen and then returns to
Normal. The app receives `SIGWINCH 78x22`; B remains 38×22. Repeating
`Ctrl-p`, `f` restores the split, sends `SIGWINCH 38x22` to A, and leaves both
panes tiled. At 20×8 the corresponding sizes are 8×6 → 18×6 → 8×6. The first
grid rows are, respectively:

```text
80×24 tiled:      ┌ A ───────────────────── SCROLL:  0/1 ┐┌ B ───────────────────── SCROLL:  0/1 ┐
80×24 fullscreen: ┌ A ───────────────────────────────────────────────────────────── SCROLL:  0/1 ┐
20×8 tiled:       ┌ A ─────┐┌ B ─────┐
20×8 fullscreen:  ┌ A ───────────────┐
```

## Frame text and colors

The pyte grid records every frame character as bold with the frame foreground
color and the default background. In settled captures, focused A is
`afff00` (ANSI-256 color 154, the default `frame_selected.base` green) in
Normal and Locked, and `d75f00` (ANSI-256 color 166, the default
`frame_highlight.base` orange) in Pane. Unfocused B has `fg=default` because
the default `frame_unselected` style is absent. These choices follow
`zellij-server/src/ui/pane_contents_and_ui.rs:302-349`, the default palette
constants in `zellij-utils/src/shared.rs:83-101`, and the frame declarations
in `zellij-utils/src/data.rs:1333-1347`. Frame characters are made bold and
given that foreground in `zellij-server/src/ui/pane_boundaries_frame.rs:12-28`.

The title comes from the layout pane name (`terminal_pane.rs:338-381`) and is
paired with `grid.scrollback_position_and_length()` (`terminal_pane.rs:383-391`).
For the short A/B names, the settled 80×24 title row is:

```text
┌ A ───────────────────── SCROLL:  0/1 ┐┌ B ───────────────────── SCROLL:  0/1 ┐
```

With long names, the 80×24 tiled row uses the full title and the shortened
right counter:

```text
┌ LONG-PANE-A-0123456789 ───────── 0/1 ┐┌ LONG-PANE-B-9876543210 ───────── 0/1 ┐
```

At 20×8, the two long titles are truncated around the middle as
`┌ L[..]9 ┐┌ L[..]0 ┐`; fullscreen A at that size is
`┌ LONG-P[..]456789 ┐`. The truncation and right-side counter fallback are
implemented in `pane_boundaries_frame.rs:410-458` and `:166-231`; title-line
placement and horizontal fill are in `:461-697`. The observed `(0, 1)` counter
therefore changes from `SCROLL:  0/1` to `0/1` when title width consumes the
available space, then disappears when even the short form cannot fit.

Each of `Esc`, `Enter`, and `Ctrl-p` was sent in its own settled write after
entering Pane; each returned to Normal, and none reached an app. An unbound
`q` in Pane also produced no app input. `Ctrl-g` in Pane entered Locked;
`q` in Locked reached A as exactly byte `71`, and the configured `Ctrl-g`
returned to Normal. B received no test input throughout. The child logs and
all cells at each checkpoint are retained in the probe's output JSON.
Lossless compressed JSON, raw ANSI captures, and SHA-256 metadata for the
repeat runs are archived under
[`docs/evidence/zellij/pane-reference`](../evidence/zellij/pane-reference/).

The settled app-input deltas at 80×24 were therefore:

| PTY write | A delta | B delta |
| --- | --- | --- |
| `1b` startup dismissal | empty | empty |
| `10` Pane entry | empty | empty |
| `66` Pane fullscreen | empty | empty |
| `1b`, `0d`, or `10` Pane returns | empty | empty |
| `71` unbound Pane key | empty | empty |
| `07` Pane → Locked | empty | empty |
| `71` Locked key | `71` | empty |
| `07` Locked → Normal | empty | empty |

The shipped public keymap has the relevant action order in
[`config.kdl`](../../tests/zellij/reference/config.kdl): Pane `f` is
`ToggleFocusFullscreen; SwitchToMode "Normal"` (lines 35–35), Pane `Ctrl-p`
is `SwitchToMode "Normal"` (line 24), and `Enter`/`Esc` are shared returns
(lines 195–200). Shared `Ctrl-g` enters Locked (lines 177–179); Locked's
`Ctrl-g` returns to Normal (lines 7–9). The reference keybind lookup falls
back to `NoOp` in non-default modes and to `Write` in Locked/Normal, as shown
by `default_action_for_mode` in the pinned source.

## Batched mode transitions

The rapid case writes `Ctrl-g Ctrl-p b` as one PTY write from Normal. In every
run, none of those three bytes reached A or B. The resulting mode is observable
with a following `f` and `Ctrl-p`: depending on scheduling, the reference either
treats `f` as Pane `f` (fullscreen, A gets `SIGWINCH 78x22`) or forwards `f` to
A. In the forwarding branch, the subsequent `Ctrl-p` is also forwarded as
byte `10`, proving the mode was still Locked rather than Normal. Two repeated
runs at each size produced both outcomes:

| size | repeat 1 | repeat 2 |
| --- | --- | --- |
| 80×24 | Pane result: fullscreen; no app input | Locked result: A receives `66`, then `10` |
| 20×8 | Pane result: fullscreen; no app input | Locked result: A receives `66`, then `10` |

This is a race, not a stable size rule. The source explains why: each parsed
`ClientToServerMsg::Key` looks up the current mode in the route thread
(`zellij-server/src/route.rs`, lines 973–985), while `SwitchToMode` queues
`ServerInstruction::ChangeMode` to the server (`route.rs`, lines 92–111).
The next key can therefore be interpreted before or after that queued mode
change is applied. The probe deliberately leaves this result visible and
does not assert or normalize one winner. A profile implementation should
model settled writes deterministically and preserve a separate race regression
case until the candidate's batching policy is explicitly made equivalent.

## Erase and scrollback counters

The initial fixture writes `CSI 2J CSI H` before its ready text. In the
custom two-pane layout this settled output gives the reference frame counter
`SCROLL:  0/1` at 80×24. A cursor-home without erase gives no counter. The
following matrix was run independently at both sizes with two deterministic
panes; each row is the visible pyte frame after startup has settled:

| PTY output before `PANE-* READY` | 80×24 frame counter | 20×8 frame counter |
| --- | --- | --- |
| `CSI 2J CSI H` | `0/1` | omitted |
| `CSI H` | omitted | omitted |
| `CSI 2J CSI H CSI 2J CSI H` | `0/23` | omitted |
| `ONE`, `TWO`, `THREE`, then `CSI 2J CSI H` | `0/4` | omitted |
| `ONE`, `TWO`, `THREE`, then `CSI 0J CSI H` | omitted | omitted |
| `ONE`, `TWO`, `THREE`, then `CSI 1J CSI H` | omitted | omitted |
| `ONE`, `TWO`, `THREE`, then `CSI 3J CSI H` | omitted | omitted |

At 20×8 the application cells are 8×6 inside 10×8 outer panes. The earlier
explanation that no short counter could fit was incorrect: the source permits
a position-only ` 0 ` indication at this width. The blank application and absent
counter require investigation of actual startup output/state; width alone does
not explain them. The complete settled grids are
retained in the compressed evidence files
[`80x24.json.gz`](../evidence/zellij/erase-probe/80x24.json.gz) and
[`20x8.json.gz`](../evidence/zellij/erase-probe/20x8.json.gz), with a
summary and SHA-256 manifest beside them.

The source-level cause is specific to full erase. `grid.rs:2740-2801`
dispatches `CSI J`; type 2 resets the scroll region and calls `fill_viewport`,
while types 0 and 1 only clear below or above the cursor and type 3 clears
existing lines above. `fill_viewport` at `grid.rs:1283-1294` calls
`transfer_rows_to_lines_above(self.viewport.len())` on the main screen before
replacing the viewport with blank rows. Thus the first full erase transfers
the one materialized initial row (`0/1`), repeating it after the viewport is
populated transfers the 22 pane rows (`0/23`), and three printed rows before
the erase transfer four rows (`0/4`). `grid.rs:600-606` then reports
`(lines_below.len(), scrollback_buffer_lines + lines_below.len())`; the runs
stayed at the viewport bottom, so position remained zero.

Historical note from before the generic erase policy was integrated: Ekko's
`scroll-lines` only remembered rows when ordinary main-screen scrolling
occurred (`src/vt.lisp:47-60`), and `history-count` exposed stored rows above
the viewport (`src/history.lisp:17-24`). Its `CSI 2J` path therefore needed to
transfer materialized main-screen rows into history to reproduce the reference
counter. The public `:erase-display-history` option now supplies that policy;
remaining parity gaps include below-viewport accounting, Unicode edge cases,
and resize interaction. A profile must continue deriving the length from
actual history and below-viewport state; substituting a fixture constant would
hide this discrepancy.

## Startup PTY sizing

The paired custom two-pane run starts its outer PTY at the requested size before
launching either program (the differential runner's `Terminal`
constructs a PTY, applies `TIOCSWINSZ`, then `Popen`). Zellij receives that PTY
as its session terminal and creates each pane at its final content rectangle:
`READY 38x22` at 80×24 and `READY 8×6` at 20×8.

Before the startup sizing fix, Ekko followed a different call path. `run` calls `run-session` in
`src/client.lisp`; when no socket exists, `run-session` starts `--serve` with
`sb-ext:run-program` and `:input nil`, then connects and calls
`attach-session`. `serve` in `src/server.lisp` loads the worker registry,
spawns every child with the fixed `59×34` request, builds the pane tree, and
calls `layout`; the first attachment sends the real viewport only afterward.
With the profile's one-cell pane insets, the startup layout can settle at
58×34 before the attachment resize. One captured run recorded these histories:

| Outer PTY | Ekko A/B | Zellij A/B |
| --- | --- | --- |
| 80×24 | `READY 58×34`, `WINCH 38×22` | `READY 38×22` |
| 20×8 | `READY 58×34`, `WINCH 8×6` (B's `WINCH` may precede `READY`) | `READY 8×6` |

These histories are retained in
`/nix/store/0gimgx8bvphh4dhdpqr246fvf48pdw2c-ekko-zellij-pane-differential/80x24/{ekko,zellij}.json`
and the corresponding `20x8` directory.

The later final 80×24 archive
(`/nix/store/7sgkrijiqclasdhb492q60x4q8nzg092-ekko-zellij-pane-differential/80x24`)
recorded `READY 38×22` directly for both Ekko panes, matching Zellij. The
difference shows a startup race: if attachment and resize win before the child
writes its first READY line, the oversized request is not observable; if the
child writes first, the initial archive's `READY 58×34` followed by `WINCH`
appears. The later 20×8 archive still records `READY 58×34` followed by
`WINCH 8×6`, so the small-viewport case remains exposed.

The implemented follow-up has `run-session` query its inherited terminal size
and pass `(cols rows cw ch)` through an internal startup argument. `serve` now
sets that viewport, builds the pane tree from the loaded public options, and
calculates content rectangles before spawning. Direct detached `--serve` without
a viewport retains the 120×36, 8×16 default. Initially collapsed hidden panes
start at 1×1. Split actions use a separate planned layout and spawn at the new
content size before committing live state; failed spawning leaves existing
geometry and processes intact.

The paired input/geometry gate now requires matching complete application
dimension histories. The fixed implementation passes it at 80×24 and 20×8,
with `READY 38x22` and `READY 8x6` respectively and no extra startup resize.
This does not resolve the separate small-viewport rendering difference.

At 20×8 the Zellij settled frame has blank pane interiors even though both
applications report `READY 8×6` and no resize. Its raw stream contains a
startup full erase and frame repaint but no `PANE-* READY` text. Therefore the
tiny-viewport blank repaint is a separate rendering observation; it is not
evidence that Zellij started the applications at another size. After the sizing fix Ekko wraps READY at the correct eight-column width, while
the reference still shows blank interiors. The earlier truncated text and the
current wrapped text are retained in their respective captures.

### Small startup controls

The [retained reference probes](../evidence/zellij/small-startup/provenance.json)
vary child text, ED2, and delays through four seconds. At 20×8, children log
`READY 8x6` and `OUTPUT 8x6` without a resize, but the settled outer ANSI contains
neither PANE nor READY text. One or three Escape bytes do not reveal it.
Effective top-level `show_release_notes false` and the source-defined seen-notes
cache marker both retain the blank result. Positive 80×24 controls show the
application text without the release-notes overlay. An earlier nested
`options { show_release_notes false; }` control was invalid and is not the
basis for this conclusion.

These controls rule out startup sizing, the tested output delays, ED2 alone,
and release notes as explanations for this observed 20×8 difference. The exact
internal reference rendering branch remains unresolved. Ekko does not suppress
small-pane content to imitate this observation without an understood mechanism.

### Three-pane workflow and split-error probes

The retained [workflow reference evidence](../evidence/zellij/workflow-v1/80x24/summary.json)
uses A on the left and B/C stacked on the right, each branch divided equally.
Directional focus chooses the most recently active adjacent overlapping pane;
`p` cycles by screen position `(y, x)`. Fullscreen navigation selects tiled
neighbors while keeping the new focus fullscreen. Closing A selects the most
recent survivor C and expands the B/C column. These policies require daemon
activation history and geometry before fullscreen expansion, now exposed through
the public pane snapshot.

Pinned automatic `n` first selects the largest eligible pane (lower ID wins an
area tie), then chooses its axis using the rounded terminal cell aspect ratio.
Both dimensions must be at least five, and at least one must exceed ten.
Explicit `d`/`r` instead permit exactly ten cells on the divided axis. The paired
harness is being extended to supply identical terminal pixel-query capabilities;
unknown reference capabilities use an upstream ratio of four and must not be
silently equated with Ekko's effective fallback dimensions.

The [split-error evidence](../evidence/zellij/split-error-v1/20x8/summary.json)
records failed `d`: unchanged tiled geometry, Normal mode, and a 1000 ms bold
red (indexed color 124) frame titled `CAN'T SPLIT!`, clipped to `C[..]!` in a
ten-column frame. Failure while fullscreen first restores the tiled layout.
The source spawns and closes an attempted PTY; the profile's preflight rejection
currently avoids that attempted process. Any observable child side effects remain
a discrepancy to measure, alongside startup/small-terminal rendering differences.
Temporary frame data uses the ordinary public `:pane-note` contribution and the
profile's decoration hook. Full Pane parity remains unproven.
