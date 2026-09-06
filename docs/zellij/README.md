# Zellij compatibility work

The acceptance target is **Zellij 0.43.1**, using its shipped default configuration
and default layout. The complete objective is functional and visual equivalence,
not shortcut or theme similarity. This work is incomplete.

The optional [Lisp profile](../../examples/profiles/zellij.lisp) currently implements
Normal/Locked mode routing and a partial Pane mode: entry, exits, locking, and
fullscreen followed by Normal. It uses the documented public keymap/action API and runs
in the ordinary isolated configuration worker. No Zellij runtime participates in
Ekko sessions. Zellij is only an independent test oracle.

The pending title slice uses public launch/name/OSC metadata for command titles,
implicit-shell pane numbers, rename precedence, and empty OSC titles. Its
regular/bare lifecycle test is `checks.pane-titles`; the paired workflow adds a
`titles` scenario. The fresh Nix/live regular and bare lifecycle checks pass;
see [title-metadata evidence](../evidence/zellij/title-metadata/). Unicode
frame widths, title-stack operations, and layout-name behavior remain
incomplete; see [title observations](title-observations.md).

## Reference and reproduction

[pin.json](../../tests/zellij/reference/pin.json) records the upstream tag, Nix
source content hash, and SHA-256 hashes of verbatim `setup --dump-config` and
`setup --dump-layout default` output. The existing `flake.lock` pins the package;
`packages.zellij-reference` asserts its version, source hash, and Nixpkgs revision
to prevent accidental upgrades.
Upstream: [release](https://github.com/zellij-org/zellij/releases/tag/v0.43.1),
[source](https://github.com/zellij-org/zellij/tree/v0.43.1). Upstream code and
configuration are MIT licensed (see the pinned source's LICENSE.md). The profile
is independent Lisp policy; the reference files are upstream test fixtures.

```sh
nix build .#zellij-reference
EKKO_CONFIG="$PWD/examples/profiles/zellij.lisp" nix run . -- run --session compatibility /bin/sh
# Edit the loaded init file to load another profile, then:
nix run . -- config reload compatibility
nix run .#zellij-differential -- --output /tmp/ekko-zellij
nix run .#zellij-differential -- --output /tmp/ekko-zellij-strict --require-parity
```

The last command intentionally fails while differences or incomplete acceptance
coverage remain. The report distinguishes `modeled_cell_parity` from
`full_parity` and `coverage_complete`; the latter two remain false. The routing Nix
check verifies only the named slice; a green check does not assert whole-profile
parity. Full acceptance must eventually run the strict gate across the full
[surface inventory](surface.md), with actual terminal screenshots too.

The current runner starts both programs through real PTYs with identical 80×24
cell and 8×16 pixel-cell dimensions (override `--cols`/`--rows`), locale,
`TERM=xterm-256color`, `COLORTERM=truecolor`, and deterministic application content.
It recreates the same isolated session/config/cache/data directories between
sequential runs (so fixture argv and HOME paths are identical) and uses the same `oracle`
session name. The reference layout's central application is replaced with the
byte-recording fixture; tab/status plugins and the reference config remain intact.
This is an explicit workload substitution, not an output normalization.

The initial screen and Escape dismissal are observed at each size without assuming
a release-notes popup. Settled routing is checked against the observed post-dismissal
input baseline; cumulative input comparison still includes every startup byte.
Each checkpoint retains complete application-visible input, bytes sent, input
deltas, pyte cells (including attributes), and cursor coordinates/visibility.
Raw output is retained in `.ansi` files even if a scenario fails. `report.json`
lists every differing modeled cell; no normalization is applied. Absolute fixture
paths, titles, and any nondeterministic output remain visible differences. The
pyte model does not establish full terminal-capability or grapheme fidelity.
Real Kitty/font-controlled startup/Escape screenshots are now captured below.
Broader visual coverage, resize sequences, Unicode/error cases, and
detach/reattach differential scenarios remain open.

## First observed discrepancies

- **Startup release notes:** fresh Zellij data at 80×24 opens its bundled release-notes
  plugin. A 20×8 probe did not show the popup and forwarded Escape to the application. Ekko has no counterpart. Identical Escape dismisses Zellij's notes but
  reaches Ekko's application. The runner preserves this input mismatch at every
  subsequent cumulative checkpoint; it does not delete the Escape from evidence.
- **Settled Normal/Locked routing:** after the startup/dismissal checkpoints, `a`, Ctrl-g,
  Ctrl-p plus `b`, Ctrl-g, and `c` have equal application-input deltas in the
  recorded run. Each transition gets its own checkpoint and settling interval.
  This proves only the exercised controls; other Normal-mode shortcuts are absent.
- **Batched transitions:** one outer write containing Ctrl-g Ctrl-p `b` caused
  pinned Zellij to enter Pane mode without forwarding Ctrl-p/`b`; Ekko queues
  input behind the mode change. Timing and batching semantics need investigation
  and a dedicated regression scenario. Separately settled tests do not waive it.
- **Chrome and cursor:** every recorded checkpoint differs in borders, titles,
  status/tab bars, spacing, and cursor state. Full cell differences are emitted;
  mode routing is not visual parity.

The smaller alternative of adding hard-coded Zellij branches to the input router
was rejected because users must be able to replace the policy without a build.
The new mechanism is owner-scoped declarative keymaps plus existing action-returning
callbacks; it does not add a compatibility-specific daemon path.

## Graphical capture

A fresh private-display probe on 2026-09-05 ran
`EKKO_ORACLE_OUTPUT_DIR=/tmp/ekko-zellij-kitty-probe nix run .#test-kitty`.
The package built, but the executable returned **77 (unavailable)**: Xvfb allocated
a display and Kitty failed to create a GLX framebuffer/GLFW window
(`No GLXFBConfigs returned`). No screenshot was produced. The
[probe report](../evidence/zellij/kitty-probe/report.json) and
[Kitty log](../evidence/zellij/kitty-probe/kitty.err) retain the precise blocker.
This blocks that isolated Xvfb capture path, not independent API, input, or cell
comparison work. A subsequent private Cage/Wayland capture succeeded with pinned Kitty, software
Mesa llvmpipe, and DejaVu Sans Mono. The [captured image](../evidence/zellij/wayland-precursor/screenshot.png)
was inspected and shows the deterministic terminal fixture at 1280×720;
[renderer evidence](../evidence/zellij/wayland-precursor/eglinfo.txt) and
[font selection](../evidence/zellij/wayland-precursor/font-match.txt) are retained.
The subsequent [paired capture report](../evidence/zellij/wayland-paired-v3/report.json)
records actual Zellij and Ekko startup/Escape screenshots with the same 98×28
outer terminal (1274×700 terminal pixels) inside 1280×720 Kitty images. A real PTY
proxy retains raw output and terminal replies. Both sessions stop before the
same working path is recreated. The [Zellij](../evidence/zellij/wayland-paired-v3/zellij/escape.png)
and [Ekko](../evidence/zellij/wayland-paired-v3/ekko/escape.png) Escape screenshots
were inspected and differ in 78,043 pixels. Child PTY geometry is 96×24 vs 98×26;
Escape reaches only Ekko's fixture. These are recorded differences, not
normalizations. This historical capture uses wire-v3 Ekko before the new
geometry primitives; [provenance](../evidence/zellij/wayland-paired-v3/provenance.json)
records its exact binary/profile. Full screenshot coverage remains required.

The [v4 paired capture](../evidence/zellij/wayland-paired-v4/report.json) adds
full pyte grids and cell differences for each screenshot checkpoint, an exact
pinned PATH, and post-stop checks for processes belonging to the private work
directory. Both cleanup records have zero remaining processes. After Escape,
78,243 pixels and 452 modeled cells differ. [Provenance](../evidence/zellij/wayland-paired-v4/provenance.json)
records the binaries and artifact hashes. Pixel counts are run-specific: literal fixture paths remain visible in titles,
and no paths are normalized.
The pure [frame helper](../../examples/profiles/zellij-frames.lisp) is tested
separately and is not loaded into the profile yet. Generic geometry primitives
are documented in the [public API](../customization.md).

## Saved verification

[`checks.json`](../evidence/zellij/checks.json) records command exit codes,
package paths, and artifact hashes. `nix build -L`, `nix flake check -L`, and CLI
help passed. The strict parity command exited 1; the graphical probe exited 77.
[80×24 summary](../evidence/zellij/80x24/summary.json) and
[20×8 summary](../evidence/zellij/20x8/summary.json) count the differing cells at
each of seven checkpoints. Both settled input slices pass. Full cumulative input
differs at 80×24 because of release notes; at 20×8 it matches for this sequence.
Cell/cursor output differs at both sizes. Full `.json.gz` grids/reports and raw
`.ansi` streams sit beside each summary; gzip is lossless storage, not normalization.

The subsequent custom two-pane check covers settled Pane entry, fullscreen,
returns, unbound input, and locking at both sizes. Its [80×24](../evidence/zellij/pane-controls/80x24/summary.json)
and [20×8](../evidence/zellij/pane-controls/20x8/summary.json) summaries retain
input/state/geometry/cell results; full compressed reports and ANSI are adjacent.
Both settled input slices and Ekko mode/zoom assertions pass. Every checkpoint
still differs visually. `nix build -L` and `nix flake check -L` (nine checks)
passed; [commands and hashes](../evidence/zellij/pane-controls/checks.json) record
this verification separately from the earlier routing evidence.

```sh
nix run .#zellij-pane-differential -- --output /tmp/ekko-pane-pair
EKKO_ORACLE_OUTPUT_DIR=/tmp/ekko-wayland nix run .#zellij-visual
```

The geometry/frame-helper integration also passes `nix build -L` and
`nix flake check -L` (nine checks). [Recorded package paths and logs](../evidence/zellij/geometry-v4/checks.json)
identify this v4 verification separately from the earlier Pane-control check.
Real regular/bare PTY tests cover geometry reload/removal, fullscreen, tiny
viewports, and unchanged pane PIDs. A renderer regression checks graphics
clipping inside a shifted content rectangle.

The next work must extend the same public mechanism to the remaining modes and
remaining Pane controls, investigate rapid input timing against upstream, and add
replaceable chrome/geometry primitives with differential rendering checks.
Tabs, floating/stacked panes, plugins, persistence, web, and the rest of the surface
ledger remain required. Paired screenshot comparisons remain required.

## Public decoration integration (2026-09-05)

The profile now loads its sibling frame helper and reserves one cell on each
pane edge through ordinary public options. `:decorate` contributions supply
frames; the default Ekko titles, dividers, and status line also use this API.
Attachment wire v5 carries the clipped spans. The profile enables the reusable
`:erase-display-history` option: ED2 retains materialized main-screen rows,
including blanks, so the frame counter reflects actual history.
See [the API contract](../customization.md) and [ED2 observations](../evidence/zellij/erase-probe/summary.json).

The custom two-pane oracle names both panes A/B and omits bars. Ekko uses a
wrapper with public rename actions and a zero viewport inset to reproduce that
layout. The default-layout runner remains separate. The first integrated 80×24
run matches all modeled cells and cursor state after Escape, including Pane
color changes and fullscreen transitions. Startup still differs because of
release notes, and cumulative application input retains the Escape difference.
The 20×8 run still has frame/content/cursor differences. The first integration runs expose a
startup PTY sizing race: Ekko's detached launch starts the applications
at 58×34, then attachment resizes them to the requested pane dimensions. The
reference starts at the final pane size. These resize histories are compared
without normalization; equal settled cells do not pass the complete parity gate.
The final [cell evidence](../evidence/zellij/decorations-v5/checks.json) observes
matching 80×24 dimension histories but retains the 20×8 mismatch. The child can
read its initial dimensions before or after attachment; the oversized spawn
request remains in the implementation.

The [v5 private Kitty capture](../evidence/zellij/wayland-paired-v5/report.json)
uses candidate `/nix/store/4ii4bljx14mi1ama75hmb5fmb42sgspd-ekko-0.1.0`,
before the final ED2 color-parser correction. Both application PTYs are 96×24
inside the same 98×28 outer terminal. Escape matches cursor state but differs
in 33,547 pixels and 290 modeled cells. The images retain missing bars,
command-title differences, and startup content/history differences. Both private
sessions stopped with no remaining owned processes. This is failing parity
evidence for the recorded executable, not a capture of every later source edit.

## Startup geometry and declared hook dependencies

The startup sizing race described above is fixed: the launcher hands its
viewport to the daemon before children spawn, and split actions calculate the
new pane's content size before spawning. The paired gate now requires equal
complete application dimension histories. Both 80×24 and 20×8 pass this gate;
80×24 settled modeled cells/cursor still match. The 20×8 reference remains
blank while Ekko wraps the fixture text. [Controlled reference probes](../evidence/zellij/small-startup/provenance.json)
retain that unexplained difference with effective release-note controls.

A full-suite failure also exposed stale hook validation against undeclared
snapshot fields. Hooks now validate only their declared dependencies; constant
hooks run once initially. This preserves the public coeffect contract while
pane output and commands run concurrently. The strict complete-parity gate
continues to fail for startup input, rendering, and incomplete surface coverage.

[Build/check and paired-cell evidence](../evidence/zellij/startup-v5/checks.json)
records the passing full Nix suite and expected failing strict gate for
`/nix/store/w31i5slxl8l6n6rgicr6d52wsqrfnr7l-ekko-0.1.0`.
The [same-binary private capture](../evidence/zellij/wayland-startup-v5/report.json)
differs after Escape in 33,390 pixels and 290 modeled cells, with matching cursor
and 96×24 application cell dimensions. Zellij initially reports zero pixel
metadata in this run, while Ekko reports 1248×600; both share the 98×28 outer
terminal. Bars, titles, content, initial input, and pixel metadata differences
remain visible. Both private sessions stopped cleanly with no owned PIDs left.

Move-mode work (2026-09-06): the optional profile now swaps tiled pane positions
through the ordinary public `:set-layout` action. Ctrl-h enters Move; n/Tab and
p select the next/previous pane in screen order, while h/j/k/l and arrows swap
with the adjacent pane selected by activation history. Movement retains the
focused pane and Move mode. Enter/Escape/Ctrl-h return to Normal, Ctrl-g locks,
and Ctrl-p enters Pane. Other shared modes and the complete reference surface
remain incomplete. The default-layout bar/hint rendering remains separate work.

The regular/bare `pane-moves` check exercises the actual keybindings, exact
swap/inverse rectangles, retained child PIDs, input suppression, exits, and
reload/reattach. `move` and `move-fullscreen` paired scenarios retain all input,
PTY histories, and cells. The private `move-workflow` capture compares startup,
mode entry, cyclic and directional movements, and exit. Results and limitations
are tracked in [Move evidence](../evidence/zellij/move-mode/README.md).

Rename-pane work (2026-09-06): Pane c enters the ordinary profile's rename
keymap. Input and bracketed paste invoke a public fallback command; saved
names live in public daemon-owned component state, survive reload/worker
restart, and are discarded when their owner is removed. Entry retains the
current name, Enter/Ctrl-c commit, and Escape restores the saved name and
returns to Pane. Paired ASCII/paste editing and private screenshots are
recorded in [rename evidence](../evidence/zellij/rename-mode/README.md).
Unicode frame width and size limits remain required discrepancies.

Unicode frame titles now use the public `display-width` function with shared,
checksum-pinned scalar width tables. Wide and combining titles, per-key delete,
empty rename placeholders, and mixed Unicode truncation have paired captures
and 20 settled matching screenshots with CJK font coverage. Batched DEL remains
a required input discrepancy; startup and small-terminal differences remain.
See [Unicode evidence](../evidence/zellij/unicode-titles/README.md).

Original stdin-read context is now public input metadata. The rename profile
uses it to match batched DEL as well as separately delivered keys; fifteen
settled batched-title screenshots match. Version-6 viewers remain supported
by the version-7 daemon. [Read-context evidence](../evidence/zellij/read-context/README.md)
retains startup, small-terminal, and cursor differences. Mixed mode-switch
reads and the rest of the reference surface still require coverage.


Frame-toggle work (2026-09-06): Pane z now toggles ordinary tiled frames and
returns to Normal through public owner geometry/state actions. Boundary glyphs,
focus colors, and geometry policy remain Lisp. The regular/bare lifecycle check
covers child WINCH, inverse toggles, reload, detach, and removal. Paired 80×24
and 20×8 input/focus/PTY histories pass the exercised functional slice. Fourteen
settled private Kitty screenshots match exactly, including the separate Finix
runtime with its overlay/copy/input patch. Startup, application content in the
PTY fixture, and unimplemented surfaces remain differences. See
[frame evidence](../evidence/zellij/frame-toggle/README.md) and
[Finix preview instructions](finix-preview.md).


Session/exit work (2026-09-06): Ctrl-o enters Session and `d` detaches; Ctrl-q
quits in the supported unlocked modes. The public Lisp profile chooses these
bindings and its exit text. Paired regular/bare lifecycle checks verify real
child survival/termination and termios restoration. Ten settled Session
screenshots match, and post-quit screenshot plus native Kitty text/cursor exports
match. Startup, plugin launchers, other modes and multi-client/persistence
semantics remain open. [Session evidence](../evidence/zellij/session-mode/README.md).
