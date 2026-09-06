# Zellij pane-frame toggle investigation

This is a source-only investigation against the pinned checkout
`/nix/store/2q437kxp07ki50dkh6a4nmmcc4nlylqw-source` (Zellij 0.43.1). It makes
no implementation or parity claim.

## Pinned behavior

The reference binding is `z` → `TogglePaneFrames` followed by
`SwitchToMode "Normal"` (`tests/zellij/reference/config.kdl:35-37`). The
configuration default is enabled (`zellij-utils/src/setup.rs:792`; KDL option
parsing at `zellij-utils/src/kdl/mod.rs:2285-2286`).

`ScreenInstruction::TogglePaneFrames` flips one screen-level boolean, applies
it to every tab, renders, and reports session state
(`zellij-server/src/screen.rs:4114-4121`). This is global to the session's
tabs, rather than a focus-local property. Reconfiguration similarly replaces
the value from configuration and reapplies it to all tabs
(`screen.rs:2666-2717`). The inspected paths show runtime screen state and
configuration reapplication; they do not establish that a toggle is persisted
through session serialization.

`TiledPanes::set_pane_frames` updates each pane's frame flag, computes content
offsets, resizes each PTY, and resets boundaries
(`zellij-server/src/panes/tiled_panes/mod.rs:538-596`). With frames enabled,
non-borderless panes get a one-cell frame offset on their content. With frames
disabled, offsets depend on pane location and viewport boundaries; stacked
panes have additional top/bottom rules (`:551-590`). The layout rectangles
remain the layout geometry, but application content geometry and PTY rows and
columns can change. This PTY resize is a direct source observation.

The no-frame rendering branch must be read narrowly. In tiled rendering,
`tiled_panes/mod.rs:1066-1098` skips `render_pane_frame` for ordinary panes and
calls it only for stacked panes, where the result is a title-only treatment;
it then renders shared boundaries for that stack. Separately,
`pane_boundaries_frame.rs:810-839` contains the title-only path for a frame
render invocation when frames are disabled (and for one-row panes). The source
does not support a blanket claim that every frame-disabled tiled pane emits a
title line.

Frame colors still depend on focus, mode, hover, and pane groups even when the
frame is drawn (`pane_contents_and_ui.rs:204-280, 302-350`). Fullscreen and
stacked geometry therefore require separate tests; the toggle itself remains
session-wide.

## Current public-profile blocker

The profile installs static `:pane-insets`, `:viewport-insets`, and
`:split-gaps` (`examples/profiles/zellij.lisp:52-55`). `:set-layout` can change
the tree, but there is no public runtime decoration contribution that can
change per-pane content offsets and trigger the corresponding PTY resize.
Making a core `pane-frames` boolean or a privileged toggle would encode a
Zellij policy in the host and would prevent an ordinary Lisp component from
choosing the policy.

## Smallest reusable vertical slice

Add a generic, profile-owned decoration geometry contribution to the existing
layout/decoration path. A component should be able to read its own
`:component-state` boolean and return per-pane content offsets plus decoration
spans for the current layout. The host should merge the contributions,
recompute content dimensions, resize PTYs when those dimensions change, and
invalidate boundaries. The profile can then implement `z` as an ordinary
command that toggles its component state and returns to Normal; no host action
named `TogglePaneFrames` is needed.

The contribution contract must specify precedence when multiple components
write an edge, and must include stacked/viewport boundary inputs. State
preservation across reload or detach should use the existing daemon-owned
component-state preservation path if the profile elects to preserve the
boolean; the pinned source alone does not prove Zellij's toggle persistence.

## Tests to add

Extend the profile/helper and live workflow coverage with: two tiled panes;
toggle on/off/on; focus changes; ordinary frame-off rendering; stacked panes;
fullscreen; exact content rows/columns and WINCH after each offset change;
detach/reload state according to the chosen component-state contract; and a
second attached client if multi-client state is supported. Assert raw spans,
layout geometry, PTY observations, and rendered cells independently. Keep a
test proving that only stacked panes take the title-only frame path when the
source conditions apply.

The API and tests above are design proposals inferred from the source and
current public interfaces. The global toggle, offset/PTY resize, and stacked
render branch are directly observed at the cited locations.

## Controlled live observation

I ran a pinned-Zellij-only probe with the existing `run_side` machinery. The
stage plan was `startup`, release-note dismissal, Pane entry, `z`, Pane entry,
and `z` again at 80×24. The exact command was:

```
PYTHONPATH=tests/zellij:/nix/store/mbk230kf0aaxlnyd8ywc5qf39iklg8qa-python3.12-pyte-0.8.2/lib/python3.12/site-packages:/nix/store/p40skimzr30vlwi6fs4l02ypnva1617r-wcwidth-0.6.0/lib/python3.12/site-packages \
  python /tmp/frame_toggle_probe.py
```

Raw output is retained under `/tmp/ekko-reference-frame-toggle`. The fixture
reported these child PTY histories (rows, columns, pixel width, pixel height):

```
startup:    A 22x38  B 10x38  C 10x38
frames-off: A 24x39  B 11x40  C 12x40
frames-on:  A 22x38  B 10x38  C 10x38
```

Each `z` produced one WINCH per child; the outer terminal ioctl remained
`[24, 80, 640, 384]`. The three off-state sizes differ, so a single global
content inset cannot reproduce this transition. This is an observation from
the deterministic fixture, not a universal claim about every layout.
