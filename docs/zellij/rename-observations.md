# Pinned Zellij pane rename observations

Source pin: `/nix/store/2q437kxp07ki50dkh6a4nmmcc4nlylqw-source` (Zellij
0.43.1). This records source behavior, live observations, and the current
profile implementation boundary.

## Entry, commit, and cancel

The reference configuration enters rename mode from Pane mode with
`c`: `SwitchToMode "RenamePane"; PaneNameInput 0`
(`tests/zellij/reference/config.kdl:39`). The KDL parser accepts the numeric
byte argument for `PaneNameInput` at
`zellij-utils/src/kdl/mod.rs:1475-1477`. Entering RenamePane first stores the
active pane's prior name, but byte zero does not clear the editable name:
`update_active_pane_name` removes NUL with
`clean_string_from_control_and_linebreak`, then `update_name("")` appends an
empty string. This was confirmed against the pinned binary by the
[archived reference-only probe](../evidence/zellij/rename-mode/README.md#reference-only-probe): a blank pane
showed `Enter name...` on `rename-enter`, then `ABC` after typing.

The mode's direct bindings are `Ctrl c -> SwitchToMode Normal` and
`Esc -> UndoRenamePane; SwitchToMode Pane`
(`tests/zellij/reference/config.kdl:110-112`). Enter and Escape also come
from the shared binding `shared_except "normal" "locked"` at
`config.kdl:195-197`, so Enter commits the current edit by leaving the mode
and returning Normal. Ctrl-C likewise leaves the mode and keeps the edit.
Escape runs undo first and returns to Pane mode.

Mode transition into RenamePane snapshots the active pane name with
`store_pane_name` at `zellij-server/src/screen.rs:2060-2067`. The terminal
stores the backup only when the current name differs
(`zellij-server/src/panes/terminal_pane.rs:619-627`). Reentering rename mode
therefore captures the currently active pane's current name again. In the same
live probe, after committing `ABC`, reentry followed by `X` displayed `ABCX`;
Escape restored `ABC`. An empty original name is preserved as an empty backup,
while a non-empty name is retained on entry.

## Editing bytes

When a key is not consumed by a RenamePane binding, the mode's default action
is `PaneNameInput(raw_bytes)`
(`zellij-utils/src/input/keybinds.rs:74-86`). The route sends that action to
`ScreenInstruction::UpdatePaneName`
(`zellij-server/src/route.rs:437-445`), and screen dispatch updates the
currently active pane (`zellij-server/src/screen.rs:4084-4092`).

`Tab::update_active_pane_name` requires valid UTF-8. DEL (0x7f) and
backspace (0x08) are passed through as editing commands; every other control,
newline, carriage return, U+2028, and U+2029 is removed by
`clean_string_from_control_and_linebreak`
(`zellij-server/src/tab/mod.rs:4615-4631`,
`zellij-utils/src/shared.rs:38-48`). The terminal then appends the cleaned
string, or pops one Unicode scalar for DEL/backspace
(`zellij-server/src/panes/terminal_pane.rs:468-480`). Invalid UTF-8 fails the
update rather than being lossy.

Bracketed paste is handled by the client as one `PaneNameInput` payload in
RenamePane mode (`zellij-client/src/input_handler.rs:176-208`). The client
does not send paste begin/end wrappers in this mode. The same UTF-8/control
cleaning and Unicode-scalar backspace behavior applies to the pasted text.

While RenamePane is active, the pane title renderer displays `Enter name...`
when the editable name is empty; otherwise it displays the editable name
(`zellij-server/src/panes/terminal_pane.rs:458-466`). The explicit empty
name therefore remains observable during editing.

## Focus and reentry edge cases

Name input resolves the active pane at the time each payload is processed
(`tab.update_active_pane_name(raw_bytes, client_id)` at
`screen.rs:3608-3618`). Shared bindings can change focus while RenamePane is
active, including Alt-h/Alt-l/Alt-j/Alt-k in
`config.kdl:180-187`. Consequently, typing after such a focus change edits
the new active pane. Undo likewise resolves the active terminal at the time
of the UndoRenamePane command (`tab/mod.rs:4655-4669`); it does not carry a
pane ID captured at mode entry. This is source-derived behavior and should be
covered by a live test before claiming parity.

Leaving RenamePane by an unbound mode transition does not invoke undo or a
separate commit operation. The edited `pane_name` remains, while the stored
previous name remains available to a later UndoRenamePane until another
RenamePane entry updates it.

## Implemented profile and remaining boundary

The profile now uses a named keymap fallback for raw input and daemon-owned
component state for per-pane saved names. Entry keeps the current name;
Escape restores the active pane's saved name without deleting that backup.
A fresh entry replaces the saved name, and closed-pane entries are pruned.
ASCII editing, DEL, filtered bracketed paste, commit, cancel, and reload during
editing have regular/bare real-worker and paired evidence in
[rename-mode](../evidence/zellij/rename-mode/README.md).

The underlying `:rename` replacement remains bounded to 512 characters;
callback paste is bounded to 4096 bytes, and component state to 16 KiB.
Unicode scalar editing is covered by pure helper tests, but Unicode display
width remains unsupported by the frame helper. These are required gaps, not
parity exceptions. Focus changes through unimplemented shared Alt bindings,
large names/pastes, invalid-input error reporting, and the complete rename
surface still need live comparison.
