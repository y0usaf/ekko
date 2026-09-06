# Full read-context workflow output

The Nix check exited 0 and produced the captured output at `/nix/store/pzl0b2f55zg7q31r1krk7vnjcrba0590-ekko-zellij-pane-workflow`. The runner log is retained as `runner.log`.

Provenance command:

    nix build --no-link -L --print-out-paths 'path:.#checks.x86_64-linux.zellij-pane-workflow'

The run covers 12 scenarios at 80x24: `rename`, `titles`, `navigation`, `new-n`, `new-d`, `new-r`, `close-x`, `fullscreen-navigation`, `move`, `unicode-title-batched`, `unicode-title`, and `move-fullscreen`. All scenarios report settled input, focus, cell, and PTY checks true, and all pixel histories are true in this sample. Every scenario still reports `full_input_parity=false` and `cell_parity=false`; coverage is false. This is evidence for this captured run only and does not establish universal first-frame timing or full parity.

JSON and ANSI files are reproducibly compressed with `gzip -n -9`; raw non-JSON/ANSI files and the runner log are retained.
