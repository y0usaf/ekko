# Invalid startup query ordering probe: initial attempt

This directory is retained as raw evidence only. The initial attempt used an
invalid gate (`all([])`), so it is not an accepted startup-order experiment and
must not support a conclusion. The JSON and ANSI payloads below are compressed
with reproducible gzip; their contents are unchanged.

Command:

```sh
PYTHONPATH=/nix/store/mbk230kf0aaxlnyd8ywc5qf39iklg8qa-python3.12-pyte-0.8.2/lib/python3.12/site-packages:/nix/store/p40skimzr30vlwi6fs4l02ypnva1617r-wcwidth-0.6.0/lib/python3.12/site-packages python tests/zellij/startup_pixel_probe.py --zellij /nix/store/wxjfyl2ksqkh263zwikp2igqh2a23y8r-zellij-0.43.1/bin/zellij --ekko /nix/store/k4a7qsrg1rn53w9jskp6rxcs1wjlm07h-ekko-0.1.0/bin/ekko --profile examples/profiles/zellij.lisp --reference tests/zellij/reference --output docs/evidence/zellij/startup-query-order
```

Exit status: `0`. `report.json` retains startup fixture sizes, query/response
hex, and raw ANSI per side for immediate, gated-after-FIRST, and no-response
schedules at 80x24.

Immediate and gated runs produced Zellij FIRST pixel sizes `[304,352]` for A
and `[304,160]` for B/C, while Ekko remained `[0,0]`. With no response both
sides recorded `[0,0]`. This isolates the available metric response from the
child's scheduling: in this run, gating did not alter the result. It does not
establish a universal ordering. These observations remain invalid because the
initial gate was constructed with `all([])`.
