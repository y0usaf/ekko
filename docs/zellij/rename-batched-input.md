# Rename input batching observations

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

## Client read/event boundary (pinned source)

`ClientOsApi::read_from_stdin` uses `fill_buf`, copies all currently available
bytes, consumes that buffer, and returns it as one `Vec<u8>`
(`zellij-client/src/os_input_output.rs:180-200`). The stdin loop then appends
that whole read to `current_buffer` (`stdin_handler.rs:89`), asks termwiz to
emit a sequence of decoded `InputEvent`s (`stdin_handler.rs:108-116`), and sends
one `InputInstruction::KeyEvent` per decoded event. The first event drains the
whole `current_buffer`; subsequent events in the same read receive an empty
vector (`stdin_handler.rs:118-143`). Thus the read boundary is retained only
on the first event, while event boundaries come from termwiz. This explains
the live DEL DEL result: two decoded delete events can carry `[0x7f,0x7f]`
then `[]`, so neither reaches the server's exact one-byte delete branch.

The client passes each event's supplied bytes through `cast_termwiz_key` and
`handle_key` (`input_handler.rs:162-170`) to the server key message; rename
fallback stores those bytes directly in `PaneNameInput` (`keybinds.rs:70-87`).
The smallest compatible fallback context is therefore two distinct values:
the existing per-event decoded/raw bytes, plus an optional original read batch
attached only to its first event. The batch must not replace per-event bytes.
A batch containing a bound mode-switch key followed by text is parsed under
the pre-switch mode; later text cannot be retroactively routed to rename
without an explicit ordering policy, so that case needs a behavior test rather
than an inferred fix.

## Implemented read context

The public fallback event now carries optional `:read-bytes` independently of
its existing decoded-key `:bytes`. The ordinary profile selects original-read
context when supplied; it does not need a special host rename path. Packet 16
preserves this context through the isolated worker's asynchronous input queue.
Fifteen settled batched-title screenshots now match exactly, including the
previously different DEL checkpoint. Paired 80×24 and 20×8 retain startup/content
and cursor differences; read batching is not a full input-parity claim.
See [read-context evidence](../evidence/zellij/read-context/README.md).

Mixed mode-switch reads, parser/error edge cases, and the complete remaining
reference input surface still require paired coverage beyond the implemented
regular/bare API contracts. Earlier failed captures remain valid historical
evidence of the behavior that changed.
