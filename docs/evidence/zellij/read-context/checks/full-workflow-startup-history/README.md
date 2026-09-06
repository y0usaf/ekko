# Corrected full-workflow startup-history output

This corrected run exited 0 and produced the captured output at `/nix/store/xz0craicj0jpclag6fxny75m25mp5vxp-ekko-zellij-pane-workflow`. The runner log is retained as `runner.log`.

Provenance command:

    nix build --no-link -L --print-out-paths 'path:.#checks.x86_64-linux.zellij-pane-workflow'

The run covers 12 scenarios at 80x24: `rename`, `titles`, `navigation`, `new-n`, `new-d`, `new-r`, `close-x`, `fullscreen-navigation`, `move`, `unicode-title-batched`, `unicode-title`, and `move-fullscreen`. All scenarios report settled input, focus, and PTY checks true; pixel histories are true in this sample. `full_input_parity` and `cell_parity` remain false. The corrected harness retains every FIRST/WINCH event for comparison and allows the startup sequence observed in the failed flake run. This successful sample does not establish universal first-frame timing or full parity.

JSON and ANSI files are reproducibly compressed with `gzip -n -9`; raw non-JSON/ANSI files and the runner log are retained.
