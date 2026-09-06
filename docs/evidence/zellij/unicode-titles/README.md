# Initial Unicode title evidence

This is an initial bounded archive; later corrected captures may supersede it. JSON and ANSI files are compressed with `gzip -n -9`; PNG files are retained. `SHA256SUMS` covers all archived files except this README.

## Initial batched visual

Source: `/tmp/ekko-unicode-visual`.

    nix run 'path:.#zellij-visual' -- compare --zellij /nix/store/wxjfyl2ksqkh263zwikp2igqh2a23y8r-zellij-0.43.1/bin/zellij --ekko /nix/store/f2bx0xmaf3l76np3fs9sv09gm8vav7qr-ekko-0.1.0/bin/ekko --profile /home/y0usaf/dev/maintaining/ekko_v2/examples/profiles/zellij.lisp --reference /home/y0usaf/dev/maintaining/ekko_v2/tests/zellij/reference --output /tmp/ekko-unicode-visual --scenario unicode-title-workflow

The report contains 16 checkpoints. `startup-screen` differs by 50,620 pixels and 1,159 cells; `startup` through `rename-enter-combining` are zero pixels/cells. `rename-clear-combining` differs by 4,312 pixels and 46 cells; `rename-combining` and the following combining checkpoints differ by 1,138 pixels and 9 cells. Long-title stages range from 1,138 to 2,195 pixels and 9 to 18 cells; clear-long is 4,317 pixels and 46 cells. Cleanup exited zero for both Zellij and Ekko with no remaining PIDs. `full_parity` and coverage are false.

## Initial batched paired 80x24

Source: `/tmp/ekko-unicode-80x24-v2`.

    nix run 'path:.#zellij-pane-workflow' -- --only unicode-title --cols 80 --rows 24 --output /tmp/ekko-unicode-80x24-v2

The report has 16 Unicode-title stages. Startup differs by 1,045 cells; later stages range from 14 to 51 cells. Application cell geometry is equal for every stage. Input parity passes only at startup; the run is not full parity and coverage is false. This capture predates the placeholder fix and uses the earlier batch DEL scenario naming.

## Failed readiness

`failed-readiness/paired80.log` and `failed-readiness/paired-raw` preserve `/tmp/ekko-unicode-paired80.log` and `/tmp/ekko-unicode-80x24`. The run failed with `KeyError: 'focus'` in `pane_workflow_differential.py` while waiting for candidate readiness. It is retained as invalid readiness evidence and supports no parity conclusion; the harness was subsequently corrected to use status focus.
