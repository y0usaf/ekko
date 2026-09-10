# Implementation progress

Style refinement (2026-09-08): frozen selection now retains styled VT/history
cells and uses the live row renderer; only selected backgrounds change. Headers
span each pane in its border colour with centred, display-width-clipped titles.
Removed the previous middle-truncation/scroll-label fitting code. Pane order in
the top strip is stable. Nix build plus mouse-selection, pane-frames, pane-titles,
daily and runtime checks exited 0. Cudaterm's font-atlas builder now aligns the
eleven light box glyphs at common cell centres; 11,498 other glyphs compare
byte-for-byte unchanged. This deliberately favours the requested Ekko styling
over exact Zellij screenshot parity.

Bars (2026-09-08): added an owned Lisp session/pane strip and contextual mode
hints in the already reserved top/bottom rows. The component clips by display
width, follows focus/title/mode changes, and leaves zero-inset oracle layouts
unchanged. Nix config validation and the decorations, pane-modes, pane-frames,
and mouse-selection checks exited 0. A private live daemon verified Normal,
Pane, Move, Session and Locked labels without hook errors. The bars were visually
inspected in Cudaterm; live reload preserved the shell PID. These are usable Ekko
bars, not real tabs or complete Zellij tab/status plugin parity.

Text usability (2026-09-08): character-range mouse highlighting and release-to-copy,
wheel scrollback, and viewer-owned OSC 52 clipboard export are implemented in
the parity worktree. Pointer actions share public validation with profile actions;
mouse-aware applications retain their input. Wire 12 keeps legacy buffer support.
This is a usability slice, not full Zellij selection/search parity. See README.md
for controls and remaining text-selection limitations.
Final `nix flake check -L .` exited 0, including the regular/bare mouse-selection
integration and all 14 existing paired pane workflows. An earlier full run found
an extra same-cell-size WINCH in the move scenario; it did not recur in the final
run. A focused move probe against the unchanged baseline runtime also fails the
geometry gate. This observation remains recorded rather than normalized out of the oracle.
[Verification receipt](docs/evidence/text-selection/verification.json).

Active outcome: complete functional and visual parity with pinned Zellij 0.43.1
through an optional, replaceable public Lisp profile, while preserving Ekko's
independent daemon, transactional reload, reversible ownership, and graphics
isolation. The full acceptance gate is in GOAL.md; it remains open.

Current isolated Finix preview: pinned to
`a9a129c6ee99a4b725b9cdf465726c763ece7e17`. Final nested `nix flake check`
passes initialization, viewer-exit, launcher and frame checks (regular/bare where
applicable). `cd ~/finix && nix run path:./previews/ekko-zellij` launches the
separate Lisp profile; add `-- --keep-session` for explicit detach/reattach.
[Launch/review instructions](docs/zellij/finix-preview.md).
[Final receipt](docs/evidence/zellij/initialization/finix-preview-receipt.json).
This is an intermediate deliverable; full parity remains active.

Initialization continuation (2026-09-06): added documented public
`register-component :initialize` callbacks. They return only owned state, keymap,
status and decoration actions, validated as a group on a detached session before
configuration commit. Startup callbacks run before child spawn; reload callbacks
receive immutable committed snapshots and reject stale declared dependencies.
Seven invalid/error/timeout candidates leave generation, state, mode and child PID
unchanged; deferred application bytes replay through the old map on failure.
Regular and bare real-daemon checks pass, including removal/reinstallation and
failed startup without child execution. Aggregate-limit/freshness checks also
pass. The final Nix suite passed all 25 named checks and 14 workflow scenarios
(the Nix build queue reports 26 checks). Logs and exact sources are recorded
with the evidence.
[Evidence](docs/evidence/zellij/initialization/README.md).
The Finix shared-runtime preview passed its four checks at pinned source
`01ab2dee4eab6d89a26f220a7519e4b9c7b83174`, with no separate patch and the custom
menu preserved. The live main Finix inputs/configuration/patch remain untouched.
Next bounded action: public durable namespaced state with atomic updates and
failure reporting, sufficient for version markers without direct profile file
writes. Then implement startup floating/plugin presentation and input against
the pinned oracle. Release notes and full parity remain unimplemented/open.

Shared-runtime continuation (2026-09-06): integrated the existing Finix opaque
overlay, graphics-cropping, explicit input and copy-fallback changes through the
same public runtime. Both profiles can use unpatched wire 11, retaining supported
legacy attachments. The main Finix config, live runtime patch and sessions were
preserved. `nix flake check -L path:.` exited 0 with 24 checks and 14 workflow
scenarios. The unpatched candidate also passes existing menu, frame, viewer-exit
and launcher tests. Its private Kitty menu capture passes graphics occlusion,
exact pixel restoration, live output, lock and split checks. An older-Pillow
harness failure and the compatibility-only copied-test fix are retained.
[Evidence](docs/evidence/zellij/shared-overlays/README.md).
[Startup UI source findings](docs/zellij/startup-ui-investigation.md) explain the
seen-marker timing and small-viewport popup exclusion. Release notes remain
unimplemented. Next bounded action: a transactional owner initialization contract
before input routing, with durable state and pointer/floating UI handled through
public mechanisms. Full parity remains the active unfinished goal.

Session/exit continuation (2026-09-06): added ordinary Lisp Session entry/exits,
locking, single-client detach and Quit in the five supported unlocked modes.
The paired lifecycle test verifies restored termios, stable child PIDs through
detach/reattach, Normal input on reattach, and child termination on quit;
regular and bare cases pass. A rejected action-order attempt is retained.
The initial full Nix check hit reference FIRST 0×0 then WINCH versus Ekko's
final-size FIRST. That minimized failure is archived without relaxing any gate.
After the generic exit-text work, `nix flake check -L path:.` exited 0 with
24 checks, 14 workflow scenarios, and the lifecycle matrix. The optional
`:viewer-exit-text` is owned, reloadable, validated plain text; its wording stays
in Lisp. Base wire 9 preserves viewers 6/7; 8 remains reserved for the existing
Finix overlay build, and the combined preview uses 10. Ten settled Session
screenshots match. Quit now matches the actual screenshot and native Kitty
styled-text/wrap/cursor export. Pyte's alternate-screen discrepancy is preserved
and identified as a model limitation. [Exact evidence](docs/evidence/zellij/session-mode/README.md).
The Finix candidate passes its frame, launcher, viewer-exit and preserved-menu
Nix checks. Full parity remains false. Next bounded action: startup UI
initialization and ownership for a replaceable Lisp release-notes overlay;
all remaining source-ledger surfaces and startup size/input differences remain.

Frame-toggle continuation (2026-09-06, session 01a076eb-d25e-74b3-a610-fe3d78b2fa45):
work is isolated in `../ekko-zellij-parity` on `zellij-parity-01a076eb`, starting
from the existing `1736077` implementation. Concurrent Finix/menu changes and
running sessions were inspected and preserved. The interrupted geometry code
failed a fresh Nix build; fixed its outer/content assertion and duplicate-key
validation, then corrected ordinary Lisp shared-boundary color/junctions and
binding ownership. Regular/bare live frame tests pass inverse toggles, exact
child WINCH, unchanged PIDs, reload/rollback, detach, and component removal.
`nix flake check -L path:.` exited 0 with 22 checks and 13 paired workflow
scenarios. Paired 80×24 and 20×8 frame runs pass the named functional slice;
full input/cell parity and coverage remain false. Fourteen settled screenshots
match exactly in both base and Finix patched runtimes. Startup differs by
50,698 / 50,612 pixels respectively, with 1,159 modeled cells; child pixel
startup ordering and the separate PTY fixture's content/cursor differences
remain recorded. [Exact evidence](docs/evidence/zellij/frame-toggle/README.md).
A separate `~/finix/previews/ekko-zellij` flake builds the selectable Lisp profile
with the preserved overlay/copy/input fixes. Its frame and real-PTY launcher
checks pass. [Launch/reload/review instructions](docs/zellij/finix-preview.md).
The user's main Finix flake, live runtime patch, custom menu, and existing
sessions remain untouched by this continuation. The preview is intermediate;
full Zellij parity is still the active goal. Next bounded action: public Lisp
session-mode routing and quit/detach with real-child and terminal-restoration
comparison; the entire unimplemented ledger remains in scope.

Original-read input context (2026-09-06): public fallback events now expose
optional `:read-bytes` separately from decoded-key bytes. The first semantic
event consumes the original read; later events explicitly carry nil. The
ordinary rename profile now matches pinned batched DEL and mixed Unicode
appending. Wire 7 carries read context, while wire-6 viewers remain accepted
and receive negotiated version-6 scenes. Deferred input belongs to its original
writer, preventing old input from crossing into a replacement attachment.
Regular/bare framed-input, legacy-input, UTF-8 fragmentation, command deferral,
and lifecycle contracts pass. All 20 checks other than `zellij-pane-workflow`
passed through Nix. A fresh full workflow check also passed its 12 named
scenarios, including complete PTY pixel histories in this sample. Every
scenario still reports full input/cell parity false; startup ordering remains
a known variable and coverage remains incomplete. A subsequent full flake
check failed because Zellij produced FIRST 0×0 then WINCH; the harness
incorrectly required exactly one startup event. Its readiness assertion now
allows that observed history while preserving every event for comparison;
the corrected full workflow then passed all 12 scenarios. The final
`nix flake check -L path:.` exited 0. This is the current check baseline,
not complete reference parity.
Fifteen settled CJK-font batched-title screenshots match at zero pixels/cells;
startup differs by 50,714 pixels and 1,159 cells. Paired 80×24 batched input and
20×8 batched/per-key runs match settled input deltas, focus, and complete PTY
histories. They retain startup/content and cursor differences (14 cells at
80×24, 43 at 20×8 after startup). See
[read-context evidence](docs/evidence/zellij/read-context/README.md).
Mixed mode-switch read ordering, parser errors, input limits, startup, and the
entire unimplemented reference surface remain required. Full parity is open.
The next frame-toggle mechanism is investigated in
[frame-toggle findings](docs/zellij/frame-toggle-investigation.md); it must use
generic runtime geometry contributions with ordinary Lisp policy.

Unicode title slice (2026-09-06): added pure public `display-width` and shared
licensed Unicode tables for profile fitting, VT, copy, and decoration clipping.
The Nix `text-width` oracle matches all 1,112,064 Unicode scalars against the
checksum-pinned `unicode-width` 0.1.10 crate. The profile now accepts printable
Unicode titles and shows `Enter name...` after deleting a rename to empty.
Regular/bare Unicode rename/OSC lifecycle and clipping contracts pass. All 20
checks other than the still-failing `zellij-pane-workflow` passed through Nix;
the exact argv and results are archived. Twenty settled visual checkpoints
match at zero differing pixels/cells, including wide glyphs, combining marks,
per-key deletion, and mixed-title truncation with pinned DejaVu/CJK fonts.
Startup differs by 50,654 pixels and 1,159 cells. At 20×8, per-key title stages
match application input/focus and PTY histories, but retain 43 differing cells
and cursor differences (startup 29 cells). Batched DEL still differs: the
reference retains the name while Ekko deletes it. Initial failed/batched and
missing-CJK-font captures remain archived; no differences are normalized.
See [Unicode evidence](docs/evidence/zellij/unicode-titles/README.md) and
[batched input investigation](docs/zellij/rename-batched-input.md).
Application-content combining behavior, emoji shaping against the actual
terminal, remaining title variants, input bounds, startup, and the entire
unimplemented reference surface remain required. Full parity remains open.

RenamePane and public input/state slice (2026-09-06): ordinary Lisp now implements
rename entry, incremental input, delete, filtered paste, commit, and undo.
Public named keymap fallbacks receive original input through the isolated worker;
component-owned daemon state preserves undo across detach, reload, and worker
restart, and is removed with its owner. State validation and snapshot copying
preserve the transactional action boundary. Regular/bare real-child contracts
cover input routing, paste bounds, failed reload, and lifecycle preservation.
All 19 checks other than the still-failing `zellij-pane-workflow` passed before
the last inspect JSON encoding correction; afterward core tests and
`nix build --no-link -L --print-out-paths 'path:.#checks.x86_64-linux.pane-rename'
'path:.#checks.x86_64-linux.keymap-input'` passed again. Paired 80×24 and 20×8
rename stages match input deltas, focus, and full PTY cell/pixel histories;
startup input and terminal content differences remain. Nine settled visual
checkpoints match exactly; startup differs by 50,706 pixels and 1,159 cells.
See [rename evidence](docs/evidence/zellij/rename-mode/README.md).
Unicode frame width, 512-character names, 4096-byte fallback paste, bounded
component state, remaining shared transitions, and the complete unimplemented
reference surface remain required. Full parity and the full Nix gate are open.
The next Unicode investigation is recorded in
[title-width findings](docs/zellij/unicode-title-investigation.md); existing VT
scalar widths are not yet proven equivalent to pinned Zellij string widths.
Fresh parent access checks pass for the Nix daemon, AF_UNIX bind/listen, and
both GPU render nodes; no earlier sandbox blocker is being carried forward.

Move-mode slice (2026-09-06): ordinary Lisp now implements tiled cyclic and
directional swaps through `:set-layout`, with reference mode bindings and
fullscreen no-op behavior. Regular/bare real-child tests verify swap/inverse
rectangles, focus/PIDs, input suppression, exits, and reload/reattach. All 17
other Nix checks passed; after the final fullscreen guard, core, Move, keymap,
and pane-workflow checks passed again. Final paired tiled runs at 80×24 and
20×8 and fullscreen at 80×24 match input deltas, focus, and complete PTY
cell/pixel histories for their movement stages. Startup differences remain.
Nine settled private Move screenshot checkpoints match exactly; startup and
small-terminal output do not. Rejected pixel-refresh and pre-guard fullscreen
results remain archived. See [Move evidence](docs/evidence/zellij/move-mode/README.md).
Floating/stacked/grouped movement, remaining shared modes and bars/hints, and
the entire unimplemented reference surface remain required; full parity is open.

Public layout replacement (2026-09-06): added ordinary `:set-layout :tree`
action for arrangements of existing stable pane IDs. It validates shape,
percentages, exact pane membership, and bounded traversal before applying the
batch, preserves focus/fullscreen, and resizes existing PTYs without replacing
children. The accepted layout survives detach and component removal/reload.
Nix checks `pane-layouts`, `pane-pixels`, and `pane-workflow` pass, including
regular/bare real-child lifecycle tests; the candidate's core unit tests pass.
See [layout replacement evidence](docs/evidence/zellij/layout-replacement/README.md).
A corrected standalone query-order probe also ran through Nix: gated replies
follow all three FIRST observations, and raw histories are preserved. Immediate
Zellij FIRST pixels differ between corrected runs; this remains an ordering gap.
See [query-order evidence](docs/evidence/zellij/startup-query-order/README.md).
This provides a needed public layout mechanism, not automatic-layout parity.
The full reference discrepancies and remaining surface stay required.

Reported PTY pixels and access recovery (2026-09-06): Nix daemon access,
AF_UNIX bind/listen, and direct opens of both GPU render nodes now pass. Earlier
sandbox blockers below are historical. Public `:pty-pixel-source` separates
physical rendering metrics from terminal-reported PTY metrics; reported facts
survive profile removal/reload, and packet 15 (wire version 6) records replies
without immediately resizing applications. Ordinary focus no longer causes an
unnecessary PTY resize when pane rectangles are unchanged. The optional profile
uses reported metrics through the public API. Core Nix tests and
`nix build --no-link --print-out-paths 'path:.#checks.x86_64-linux.pane-notes'
'path:.#checks.x86_64-linux.pane-pixels'` pass, including regular/bare real-child
pixel observations and profile removal/restoration. The pane-note test now reads
one snapshot per predicate, avoiding a race at note expiry without changing its
deadline. Fresh visual evidence matches all 13 settled checkpoints exactly
(zero differing pixels/cells); startup differs by 50,794 pixels and 1,159 cells.
Both visual sessions cleaned up with no remaining child PIDs.

All 15 other Nix checks pass on the current candidate, including runtime, daily,
regular/bare profile contracts, startup geometry, routing, and pane differential
checks. The exact command and outputs are in
[check evidence](docs/evidence/zellij/reported-pixels/checks/result.json).
The full Nix gate remains failing: Zellij can receive terminal metrics before
children first observe their PTYs, whereas Ekko currently starts them with zero
reported pixels. Earlier samples with zero initial pixels do not establish a
universal startup order. Preserve both outcomes and every WINCH event. Automatic
layouts at 120×24 and failed no-preference spawning at 20×8 also remain gaps.
See [reported-pixel evidence](docs/evidence/zellij/reported-pixels/README.md).
Full parity and the complete remaining surface are still required.

Title metadata slice (2026-09-06): public pane snapshots and inspect output now
expose launch arguments/kind, immutable creation position, explicit rename, and
OSC title. Absent and empty OSC titles are distinct; the former 120-character
title cap is removed while the bounded parser remains. The ordinary profile
selects rename, OSC title, command argv, or `Pane #N`, with temporary notes still
overriding the frame. Added a regular/bare real-worker regression and paired
OSC 0/2, empty, long, and whitespace-title stages. With full access restored,
`nix build --no-link -L --print-out-paths path:.#default` and the corresponding
`checks.x86_64-linux.pane-titles` command passed, including regular/bare lifecycle
tests. The private visual workflow now matches exactly at all 13 settled
checkpoints (zero differing pixels/cells), including the failed-split flash and
restoration; startup release notes still differ. Paired 80×24/20×8 runs retain
startup-input/small-terminal output and PTY resize-history discrepancies.
`nix flake check -L --keep-going path:.` failed the workflow fixture launcher;
after fixing its Nix-sandbox interpreter path, the workflow check reaches its
real resize-history parity failure. Full Nix and full parity are not green.
See [title evidence](docs/evidence/zellij/title-metadata/README.md).
Unicode frame widths, title stacks/layout names, resize-history timing, and the
complete remaining surface stay required.

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

Shared runtime frame capture: all 14 settled screenshots and native Kitty text/cursor exports match exactly; initial startup remains 50,578 pixels / 1,159 modeled cells different. Raw evidence: `docs/evidence/zellij/shared-overlays/frame-native/`.

### Desktop profile

- Added `examples/profiles/desktop.lisp`: centered window headers with yellow
  minimize, green maximize/restore and red close controls; replaced the top
  strip with a fixed one-row task dock. Ctrl-p then m minimizes; Ctrl-p then
  Tab cycles/restores. Buttons use public decoration actions, not profile
  branches in the runtime.
- Minimized state stays in the daemon; layout removes hidden leaves from a
  derived tree and preserves the original splits and hidden PTY dimensions.
  Application keyboard/paste input is suppressed on an empty desktop.
- Controls activate on matching press/release, never in application cells;
  removing their decoration owner removes hit targets. No scene wire change.
- Verification passed: `nix build --no-link -L .#checks.x86_64-linux.desktop
  .#checks.x86_64-linux.mouse-selection .#checks.x86_64-linux.pane-frames
  .#checks.x86_64-linux.daily .#checks.x86_64-linux.runtime` (one command).
  Desktop tests exercise regular and bare runtimes with live child PTYs,
  all-minimized state, restore geometry, input isolation, close, reattachment,
  and owner removal. Final profile check `nix build --no-link -L
  .#checks.x86_64-linux.desktop` also passed after keyboard bindings were added.
- Opened `ekko-desktop` in configured Cudaterm with the corrected font atlas;
  visually checked the live controls, centered header and reserved dock.

### Desktop becomes the default

The default owner now installs the complete desktop experience: centered dark
headers with colored Unicode edge strokes, macOS-colored window controls,
stable per-window accents, and a fixed bottom taskbar. Open windows have filled
entries, focus has a separate arrow, minimized entries use contrasting gray,
and spaces separate entries. Taskbar clicks minimize the focused window, focus
another open window, or restore a minimized window.

The old default chrome and prefix key bindings have been replaced. Ctrl-p pane,
Ctrl-h move, Ctrl-o session, Ctrl-g lock and Ctrl-q quit match the preview. The
shared desktop style and pane bindings also load through the public API in the
bare profile; the default owner remains fully removable by custom configs.
Existing public CLI command names remain available. Rename state is scoped to
the installing owner, and keyboard copy mode has its own explicit keymap.

Default and bare-profile integration tests cover real PTYs, keyboard mode and
rename behavior, button and taskbar clicks, process/layout preservation,
minimized reattachment, input isolation, and removal of the desktop owner.
Graphics tests now use the desktop bindings and account for reserved borders;
custom-prefix tests explicitly opt into their own keymap policy.

Final validation: `nix flake check --keep-going -L .` exited zero across all 28 checks. Opened the built-in default in configured Cudaterm with an empty configuration and confirmed `defaults` is the only installed owner. Evidence: `docs/evidence/desktop-default/verification.json`.

### Desktop interaction slice

- Runtime: decoration spans accept `:hover-sgr` (pointer repaint),
  `:wheel-command` (`:direction` -1/1) and `:middle-command`; `bind-key`
  accepts `M-`/`Super-` chord specs and an optional `:arguments` list; the
  snapshot adds wall-clock `:time` and per-pane `:activity`; floating and tiled
  moves snap at the outermost content cell (top maximizes, sides halve) ahead
  of window drop targets; a titlebar double-click toggles zoom.
- Profile: dock clock, `+N` overflow chip with a most-recent window list,
  `●` activity markers, hover styles on entries and controls, Alt-Tab window
  switcher, Super-1..9 focus slots, and a centered empty-desktop backdrop.
- Fixed the uncommitted refactor's syntax errors in `src/layout.lisp` (extra
  close parenthesis and `aw`/`bw` unbound across `destructuring-bind`) and the
  `row-cost` declaration in `src/history.lisp` that typed the SGR list as a
  string, which had left the working tree unable to build.
- Verification: `nix flake check --keep-going -L .` exited 0 (28 checks). A
  live smoke run against the built binary confirmed the clock, activity badge
  and clear-on-focus, `Super-2` focus, the Alt-Tab popup, double-click zoom,
  wheel cycle, middle-click close, top-edge snap to `(0 0 120 39)`, and the
  minimized backdrop. New behaviors have no automated coverage.
[Verification receipt](docs/evidence/desktop-interaction/verification.json).
