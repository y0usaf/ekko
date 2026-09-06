# Pinned Zellij frame-toggle observation

This is a pinned-Zellij-only live observation, not a Nix acceptance check and
not a candidate parity result. It used the reference binary
`/nix/store/wxjfyl2ksqkh263zwikp2igqh2a23y8r-zellij-0.43.1/bin/zellij`, the
reference config/layout fixture, and the existing `run_side` implementation.
The exact standalone driver is `frame_toggle_probe.py`.

The stage plan at 80x24 was:

`startup -> dismiss-release-notes (Escape) -> pane-enter (Ctrl-p) -> frames-off (z) -> pane-enter-off (Ctrl-p) -> frames-on (z)`.

Run invocation (the driver embeds the pinned binary and repository paths):

```sh
PYTHONPATH=tests/zellij:/nix/store/mbk230kf0aaxlnyd8ywc5qf39iklg8qa-python3.12-pyte-0.8.2/lib/python3.12/site-packages:/nix/store/p40skimzr30vlwi6fs4l02ypnva1617r-wcwidth-0.6.0/lib/python3.12/site-packages python /tmp/frame_toggle_probe.py
```

The outer terminal ioctl stayed `[24, 80, 640, 384]`. The fixture child
`FIRST` and `WINCH` histories (rows, columns, pixel width, pixel height) were:

```text
startup:    A 22x38  B 10x38  C 10x38
frames-off: A 24x39  B 11x40  C 12x40
frames-on:  A 22x38  B 10x38  C 10x38
```

Each toggle generated one WINCH per child. `zellij.json.gz` retains the decoded
stage snapshots and `zellij.ansi.gz` retains raw ANSI; `stages.json.gz` is the
complete run serialization. `zellij-cleanup.json` records the cleanup result.
