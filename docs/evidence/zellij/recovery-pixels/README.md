# Zellij cell-pixel lifecycle recovery

This evidence is for the pinned `zellij 0.43.1` reference (`tests/zellij/reference/pin.json`) at an outer PTY size of 80x24. The event rows below are a reconstruction from the prior conversation, not a fresh trace; the prior raw files were lost on reboot.

* Before an explicit `n`/`d` split, the reconstructed rows were A `FIRST [22,38,0,0]`, B/C `FIRST [10,38,0,0]`.
* After that split, the affected pane received a `WINCH` with known pixels (`304x160` for 38x10).
* Do not infer that assignment of `character_cell_size` itself resizes PTYs. Source shows assignment only updates shared state; the next layout operation invokes the resize macro.
* `TIOCSWINSZ` is issued by the resize backend, but this evidence does not claim Linux emits `SIGWINCH` for identical complete winsize values. The observed meaningful transition is pixel fields `0,0` to known values.

The source explanation below is from the authoritative pinned Nix source path `/nix/store/2q437kxp07ki50dkh6a4nmmcc4nlylqw-source`; the release/source authority is `tests/zellij/reference/pin.json`.

* The pinned parser has the startup CSI 14t/16t query in `zellij-client/src/stdin_ansi_parser.rs:63-69`, not `stdin_handler.rs`.
* `zellij-client/src/input_handler.rs:258-263` forwards pixel replies to the server.
* `zellij-server/src/screen.rs:1244-1256` merges and assigns the shared cell size; this function performs no PTY resize.
* `zellij-server/src/screen.rs:4234-4245` sends `QueryTerminalSize` after layout setup. `zellij-client/src/lib.rs:610-614` answers with a normal terminal resize. The source does not establish that this query itself synchronously resizes every pane; resize call sites consume the shared metric on their subsequent layout/resize operation.
* `zellij-server/src/tab/mod.rs:97-123` computes optional pixel dimensions for pane resize; `zellij-server/src/os_input_output.rs:50-74` maps absent values to zero and performs `TIOCSWINSZ`.

The remaining implementation question is how to separate renderer cell metrics
from application-visible PTY metrics through the ordinary public API. The exact
event timing must be reproduced before choosing that contract. No core change
or parity claim follows from this source analysis alone.

Live rerun attempt on 2026-09-06 used the pinned binary and existing `tests/zellij/pane_workflow_probe.py`; it could not start the reference children in this rebooted environment (`RuntimeError: apps failed to start`, empty PTY stream), and cleanup returned empty stderr. Nix rebuild was blocked first by fetcher-cache SQLite access and then, with `XDG_CACHE_HOME=/tmp`, by the Nix daemon socket returning `EPERM`. No live trace is represented as successful here. Exact pinned source excerpts and hashes are in `source-excerpts.txt`.
