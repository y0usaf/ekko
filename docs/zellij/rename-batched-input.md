# Rename input batching discrepancy

The live paired capture at `/tmp/ekko-unicode-visual/report.json` recorded a
real difference at `rename-clear-combining`: the pinned Zellij pane retained
`界面` after the bundled `DEL DEL` write, while Ekko processed the two DEL bytes
as two deletions. The per-key capture at
`/tmp/ekko-unicode-visual-per-key/report.json` has separate DEL checkpoints and
does not show that discrepancy. These are live capture observations, not a
normalization or a pass claim.

The source explains the behavior. Zellij's keybind fallback maps RenamePane
input to `Action::PaneNameInput(raw_bytes)` in
`zellij-utils/src/input/keybinds.rs:74-86` (pinned source tree
`/nix/store/2q437kxp07ki50dkh6a4nmmcc4nlylqw-source`). The client dispatches that
action for a key event in `zellij-client/src/input_handler.rs:170-208`.
The server then handles the complete payload in
`zellij-server/src/tab/mod.rs:4615-4631`: the payload is treated as the
single-byte delete command only when it equals `0x7f` or `0x08`; otherwise
control and line-break cleaning removes the controls before passing the result
to `TerminalPane::update_name`. That method, at
`zellij-server/src/panes/terminal_pane.rs:468-482`, pops one Unicode scalar only
for an exact delete/backspace payload and otherwise appends the supplied text.

Therefore a single raw packet containing `0x7f 0x7f` is not equivalent to two
key events in the reference: it does not match the exact delete branch. This is
a source-derived explanation consistent with the live captures.

Ekko already dispatches separate semantic keys, which explains its different
result for this batched write. The missing evidence is how the pinned client
parser associates each semantic event with the original read buffer: trace
that association before selecting a public API change. A reusable event may
need both decoded-key bytes and original input-batch bytes, with a documented
boundary for fragmented input. Merely splitting more aggressively would
preserve the current discrepancy. Test batched and separately delivered DEL,
ordinary text, and mixed bound/unbound input before claiming a fix.
