# Pane title lifecycle (pinned Zellij 0.43.1)

This records source analysis and the title metadata implementation. Fresh Nix
regular/bare lifecycle checks pass; the private workflow has exact settled
pixel/cell matches but startup and broader behavioral gaps remain. See
[evidence](../evidence/zellij/title-metadata/README.md). The pinned
source checkout is `/nix/store/2q437kxp07ki50dkh6a4nmmcc4nlylqw-source`,
corresponding to the pinned `v0.43.1` reference (nixpkgs
revision `ac62194c3917d5f474c1a844b6fd6da2db95077d`).

`terminal_pane.rs:338-381` selects the frame title in this order: temporary
frame override, rename/search prompts, explicit pane name, terminal grid title,
then stored initial title. The same steady-state precedence is explicit at
`terminal_pane.rs:458-466` and `:768-783`: explicit name wins; otherwise OSC
0/2 grid title wins; otherwise the initial title wins. The OSC parser accepts
codes 0 and 2 and stores the value in Grid (`grid.rs:2545-2567`). Ekko exposes
those codes as `:terminal-title`; the ordinary profile now chooses note text
or the result of `zellij-pane-title`.
The Rust `Option::as_deref().unwrap_or(...)` means an explicitly set empty OSC
title (`Some("")`) still wins over the initial title; only absence falls back.
The reference trims surrounding whitespace before storing OSC titles
(`grid.rs:2554-2565`); Ekko removes control characters but preserves surrounding
spaces so profiles choose whitespace policy. The former 120-character cap is
removed; the existing 16 KiB OSC parser bound remains. The reference has title-stack
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

Ekko retains its existing basename `:label` for builtin decoration. Its public
snapshot now also exports copied `:argv`, `:name`, `:terminal-title`,
`:launch-kind`, and immutable `:creation-position`. Rename updates both the
existing label and the separate explicit name. Absent OSC titles are now `nil`,
distinct from empty strings. Inspect JSON exposes the same observations with
underscore names for `launch_kind`, `creation_position`, and `terminal_title`.

The profile now consumes the plain metadata needed for a faithful policy:
original startup command vector, `:launch-kind`, creation position, rename
`:name`, and terminal OSC title including the empty-string case. Its pure
`zellij-pane-title` helper joins command argv literally, trims Unicode
White_Space from OSC titles, and applies the pinned precedence. Layout-name
versus interactive rename provenance, title stack behavior, and saved-layout
restore behavior remain outside the current public metadata contract.
Unicode whitespace trimming is covered by the policy tests, but interior
non-ASCII titles still exceed the frame helper's supported width contract.
Full title parity remains unproven; this is a required outstanding gap.

Source file hashes from the pinned checkout:

* `terminal_pane.rs`: `8d0ce5c30ffb9ca0660f43c72a57be9104ce82b3f65187e7c757d65ef81e892d`
* `grid.rs`: `ac6e61a518e7697f4db3486efb9a8bb5beaa2395e4a2c781d41409044e08e0e0`
* `layout_applier.rs`: `2647b48fd463991961b8d18a4ff340d5f8442752e7aa26ec3aaeda0a44f0df67`
* `tab/mod.rs`: `6f9050999d4d0d1e57c51467dddaead9c6369dd6e028397e25bf6ee8f775bf2c`
