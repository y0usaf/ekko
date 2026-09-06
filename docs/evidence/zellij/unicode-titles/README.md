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

## Per-key dejavu visual

Source: `/tmp/ekko-unicode-visual-per-key`. Provenance command:

    nix run 'path:.#zellij-visual' -- compare --zellij /nix/store/wxjfyl2ksqkh263zwikp2igqh2a23y8r-zellij-0.43.1/bin/zellij --ekko /nix/store/ls477hk0b0xrp0qshlwr9wffamvadsqb-ekko-0.1.0/bin/ekko --profile /home/y0usaf/dev/maintaining/ekko_v2/examples/profiles/zellij.lisp --reference /home/y0usaf/dev/maintaining/ekko_v2/tests/zellij/reference --output /tmp/ekko-unicode-visual-per-key --scenario unicode-title-workflow

This report contains 20 settled checkpoints. All settled checkpoints are zero pixels and zero cells. Startup-screen differs by 50,736 pixels and 1,159 cells. Cleanup exited zero with no remaining PIDs; `full_parity` and coverage are false. The captured CJK labels render as tofu because the visual environment lacks the required font; this is a font-environment limitation, not an accepted parity result.

## Per-key 20x8

Source: `/tmp/ekko-unicode-20x8-per-key`. Provenance command:

    nix run 'path:.#zellij-pane-workflow' -- --only unicode-title --cols 20 --rows 8 --output /tmp/ekko-unicode-20x8-per-key

The report contains 21 stages. Startup differs by 29 cells; every later stage differs by 43 cells. Cursor comparison is false at every stage; the other reported input, event, geometry, dimensions, grid, focus, and probe flags are true. This is a bounded result with `full_parity` and coverage false.

## Final per-key CJK visual

Source: `/tmp/ekko-unicode-visual-cjk`. Provenance command:

    nix run 'path:.#zellij-visual' -- compare --zellij /nix/store/wxjfyl2ksqkh263zwikp2igqh2a23y8r-zellij-0.43.1/bin/zellij --ekko /nix/store/ls477hk0b0xrp0qshlwr9wffamvadsqb-ekko-0.1.0/bin/ekko --profile /home/y0usaf/dev/maintaining/ekko_v2/examples/profiles/zellij.lisp --reference /home/y0usaf/dev/maintaining/ekko_v2/tests/zellij/reference --output /tmp/ekko-unicode-visual-cjk --scenario unicode-title-workflow

The capture uses pinned Noto CJK Sans alongside DejaVu. It contains 20 settled checkpoints, each at zero differing pixels and zero differing cells. `startup-screen` differs by 50,654 pixels and 1,159 cells. Cleanup exited zero for Zellij and Ekko with no remaining PIDs. `full_parity` and coverage remain false; the overall goal is open.

## Verification checks

`checks/all-checks.log.gz`, `checks/all-checks.exit`, and `checks/checks-argv.json` preserve the broad 20-check run. The argv is machine-readable in `checks/checks-argv.json`; the run exited `0` and excludes `zellij-pane-workflow`. `checks/final-contracts.log.gz` preserves the later pane-rename and pane-titles contract run, which exited `0`. `checks/text-width.log.gz` preserves the text-width check output (`/nix/store/3v299dfsyvzsdbqzh5yz1xkn68kk913k-ekko-text-width`). Small plain result store outputs named by the logs are copied under `checks/results`; large binary and differential outputs are represented by their paths in the logs and are not duplicated here.

The final generator reproducibility check also passed after the broad run:
`nix build --no-link -L --print-out-paths 'path:.#checks.x86_64-linux.text-width'`.
It verifies pinned input hashes, regenerates and compares the checked-in CL
source, then compares every Unicode scalar to the Rust oracle. Its log and
result are in `checks/text-width-reproducible.*`. No runtime code changed in
this final verification step.
