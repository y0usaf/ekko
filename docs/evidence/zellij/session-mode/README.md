# Session mode, quit/detach and viewer exit evidence

The ordinary Lisp profile now implements Session entry/exits, transitions to the
supported Pane/Move modes, locking, single-client detach, and quit from Normal,
Pane, Move, RenamePane and Session. Locked Ctrl-q/Ctrl-o remain literal child
input. Plugin launchers and transitions into unimplemented modes remain open.

The paired real-child lifecycle test verifies restored host termios, retained
child PIDs across detach/reattach, Normal-mode input after explicit detach, and
child termination on quit. It runs all five quit origins in the regular runtime
and detach/quit in the bare runtime. These are functional slice checks, not
complete session, persistence or multi-client parity.

The first detach action batch put the keymap transition before its primary
action. Validation rejected it before any detach event. The corrected ordinary
Lisp batch follows the public ordering rule. The failure and raw output are
retained in `failed-action-order`; no generic detach runtime change was needed.

The first full suite in this slice failed because the reference child's FIRST
size was 0×0, followed by WINCH to 22×38. Ekko's FIRST was already 22×38. Both
complete histories are retained in `failed-full-startup`. No comparison gate was
relaxed or normalized. After the exit-text work, `nix flake check -L path:.`
passed all 24 checks, including 14 workflow scenarios and the lifecycle matrix.
This successful sample does not erase the known reference startup nondeterminism.

## Actual terminal evidence

[Session-mode screenshots](visual/summary.json) have 10 exact settled pixel/cell
matches. Startup differs by 50,772 pixels and 1,159 modeled cells.

Quit initially differed by 1,569 pixels: both viewers restored the host main
screen, but Ekko lacked the farewell line. A new public owner-scoped
`:viewer-exit-text` option carries plain bounded text; all wording remains in
Lisp. Regular/bare tests verify replacement, rejected controls, owner removal,
unchanged pane PIDs, and termios restoration. The viewer also rejects invalid
wire text before changing retained state.

The new [native-cell capture](exit-native/summary.json) compares unchanged
screenshots and Kitty's own `get-text --extent screen --ansi --add-cursor
--add-wrap-markers` exports over a private control socket. Both the settled
startup and post-quit exports match byte-for-byte, with zero differing pixels.
The proxy holds the final terminal image without writing observer text; it
records the actual child exit status. Its private processes are cleaned up.
The pinned CLI help is retained in `kitty-get-text-help.txt.gz`.

Pyte still reports hundreds of post-exit differences because its interpretation
of alternate-screen restoration differs from the actual Kitty terminal. Those
reports remain unchanged. They are explicitly modeled-cell results, not an
actual-terminal discrepancy. Kitty exports retain styled text, wrapping markers,
and cursor position/style; screenshots remain necessary for glyph rasterization
and graphics. No ANSI, cells, startup input, or screenshots were normalized.

![Reference exit](exit-native/zellij/quit.png)

![Ekko exit](exit-native/ekko/quit.png)

The [Finix combined runtime](finix-exit/summary.json) also has exact settled
startup and post-quit pixel/native-text/cursor matches. Its startup popup differs
by 50,760 pixels; all exports and process cleanup records are retained. The
Finix candidate passes the existing custom-menu test as well as frame, exit-text
and launcher checks, including a retained-session launcher path. The menu test
and configuration snapshot hashes are retained without modifying the live menu.

## Reproduce

```sh
nix run .#zellij-session-lifecycle -- --output /tmp/session-lifecycle
nix run .#zellij-pane-workflow -- --only session-routing --output /tmp/session-routing
nix run .#zellij-pane-workflow -- --only session-routing --cols 20 --rows 8 --output /tmp/session-small
nix flake check -L path:.
```

Full acceptance remains false. Startup notes, immediate child size/query order,
failure-specific exit messages, custom defaults on reattachment, independent
clients, all remaining modes, tabs/layouts/floating/stacked panes, bundled plugins,
persistence, CLI, web and the rest of the source-derived ledger remain required.
The next bounded action is to isolate startup UI initialization and ownership
requirements for a replaceable Lisp release-notes overlay, without suppressing
the recorded startup-input or geometry discrepancies.
