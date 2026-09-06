# Pane title lifecycle (pinned Zellij 0.43.1)

This is source analysis only; it contains no new visual capture. The pinned
source checkout is `/nix/store/2q437kxp07ki50dkh6a4nmmcc4nlylqw-source`,
corresponding to `tests/zellij/reference/pin.json` (`v0.43.1`, nixpkgs
revision `ac62194c3917d5f474c1a844b6fd6da2db95077d`).

`terminal_pane.rs:338-381` selects the frame title in this order: temporary
frame override, rename/search prompts, explicit pane name, terminal grid title,
then stored initial title. The same steady-state precedence is explicit at
`terminal_pane.rs:458-466` and `:768-783`: explicit name wins; otherwise OSC
0/2 grid title wins; otherwise the initial title wins. The OSC parser accepts
codes 0 and 2 and stores the value in Grid (`grid.rs:2545-2567`). Ekko parses
those codes into `terminal-title` (`src/vt.lisp:296-300`), but the current
profile chooses note text or pane `:label` (`examples/profiles/zellij.lisp:24-40`).
The Rust `Option::as_deref().unwrap_or(...)` means an explicitly set empty OSC
title (`Some("")`) still wins over the initial title; only absence falls back.
The reference trims surrounding whitespace before storing OSC titles
(`grid.rs:2554-2565`); Ekko instead removes control characters and truncates
to 120 characters (`src/vt.lisp:300`). These parser differences also require
coverage when exposing titles to the profile. The reference has title-stack
push/pop state (`grid.rs:2023-2038`), another required title lifecycle case.

`TerminalPane::new` defaults an absent initial title to exactly `Pane #N`
(`terminal_pane.rs:899-936`). For a layout command, the layout applier passes
`RunCommand::to_string()` as the initial title (`tab/layout_applier.rs:484-510`);
that formatter is the executable path followed by each argument separated by
spaces, with no shell quoting or escaping (`zellij-utils/src/input/command.rs:49-81`).
A layout `name` is the explicit pane name and therefore wins. A pane with no
layout command title (including an implicit/default shell) therefore reaches
the `Pane #N` fallback unless it emits OSC 0/2. Interactive splits pass
their optional initial title into the constructor (`tab/mod.rs:2019-2054` and
`:2079-2114`). The allocator returns current tiled terminal count plus current
floating terminal count plus one (`tab/mod.rs:640-653`, repeated at `:2868-2880`).
A close removes the pane before allocation, so a later pane number can be
reused; it is not historical monotonic state. `rename` writes `pane_name`
(`terminal_pane.rs:800-804`), making it highest priority until cleared.

Ekko stores `argv`, `label`, and `vt` (`src/server.lisp:3-6`). The current
snapshot does not export `argv`; `context-data` in `src/commands.lisp` exports
labels, geometry, process and history state, and activation order. Startup and
split creation set `label` to `(file-namestring (first argv))`
(`src/server.lisp` and `src/commands.lisp`), explaining the basename title.
Rename mutates that label (`src/commands.lisp:426`), and the default formatter
renders pane ID plus display label (`src/builtins.lisp:19-26`). The public
snapshot exports `label` and `display-label`, but neither `argv` nor
`terminal-title` nor title provenance. Ekko currently stores absent and empty
OSC titles as the same empty string; the pinned reference distinguishes them.

The missing observations needed for a faithful ordinary Lisp policy are the
original startup command vector (including whether the command is an implicit
shell), the current terminal OSC title including the empty-string case, and
whether the label came from a layout name or an interactive rename. Exposing
those as plain snapshot data would let a hook choose policy without imposing a
new global title-policy API. No implementation was made here.

Source file hashes from the pinned checkout:

* `terminal_pane.rs`: `8d0ce5c30ffb9ca0660f43c72a57be9104ce82b3f65187e7c757d65ef81e892d`
* `grid.rs`: `ac6e61a518e7697f4db3486efb9a8bb5beaa2395e4a2c781d41409044e08e0e0`
* `layout_applier.rs`: `2647b48fd463991961b8d18a4ff340d5f8442752e7aa26ec3aaeda0a44f0df67`
* `tab/mod.rs`: `6f9050999d4d0d1e57c51467dddaead9c6369dd6e028397e25bf6ee8f775bf2c`
