# Read-context Unicode title evidence

This archive contains the successful read-context captures. JSON and ANSI traces use reproducible `gzip -n -9`; PNG files remain intact. `SHA256SUMS` covers all archived files except this README. The full workflow outputs are archived under `checks/full-workflow` and `checks/full-workflow-startup-history`; the earlier final flake check failure is retained under `checks/flake-check.log.gz`.

## Visual

Source: `/tmp/ekko-read-batched-visual`. Provenance:

    nix run 'path:.#zellij-visual' -- compare --zellij /nix/store/wxjfyl2ksqkh263zwikp2igqh2a23y8r-zellij-0.43.1/bin/zellij --ekko /nix/store/ghvd77hxfbnk31gigzixcdq81ka5kc0i-ekko-0.1.0/bin/ekko --profile /home/y0usaf/dev/maintaining/ekko_v2/examples/profiles/zellij.lisp --reference /home/y0usaf/dev/maintaining/ekko_v2/tests/zellij/reference --output /tmp/ekko-read-batched-visual --scenario unicode-title-batched-workflow

There are 16 checkpoints: 15 settled checkpoints at zero differing pixels and cells, plus `startup-screen` at 50,714 differing pixels and 1,159 differing cells. Both cleanup paths exited zero with no remaining PIDs. The capture precedes the final queued-writer ownership fix and has the same visible workflow path. `full_parity` and coverage are false.

## Paired 80x24

Source: `/tmp/ekko-read-batched-80x24`.

    nix run 'path:.#zellij-pane-workflow' -- --only unicode-title-batched --cols 80 --rows 24 --output /tmp/ekko-read-batched-80x24

The report contains 16 stages. Startup differs by 1,045 cells; each later stage differs by 14 cells. Input delta is false only at `dismiss-release-notes` and true thereafter. PTY focus receiver equality is true at every stage; cursor equality is false at every stage. The cumulative input startup discrepancy remains in this capture. `full_parity` and coverage are false.

## Paired 20x8

Source: `/tmp/ekko-read-batched-20x8`.

    nix run 'path:.#zellij-pane-workflow' -- --only unicode-title-batched --only unicode-title --cols 20 --rows 8 --output /tmp/ekko-read-batched-20x8

The report contains 16 batched stages; the per-key workflow record contains 21 stages with the same startup 29-cell and later 43-cell differences. Cursor equality is false at every stage; all other reported input, event, geometry, dimensions, grid, focus, and probe flags are true. `full_parity` and coverage are false.

## Check contracts

The `checks` directory preserves the successful broad check run (`all-checks.log.gz`, `all-checks.exit`, and machine-readable `checks-argv.json`), the successful final v3 input contracts (`input-contracts-v3.log.gz`), and the rejected initial/v2 fixture attempts under `checks/invalid`. The broad run exited 0 and excludes `zellij-pane-workflow`. The v3 contracts exited 0 and cover the new client protocol version 7 plus version 6 negotiated scene behavior. Small plain result outputs named by the logs are copied under `checks/results`; large binaries remain represented by their logged store paths. The full workflow check completed with exit 0 and is archived under `checks/full-workflow`; its run-scoped README records measured status and limitations. A corrected startup-history rerun also completed with exit 0 and is archived under `checks/full-workflow-startup-history`, with every FIRST/WINCH event retained for comparison. The earlier top-level flake check exited 1 because its startup geometry assertion assumed a single initial event while that run produced an initial `0x0` record followed by a `WINCH` record; the raw failure log is retained as `checks/flake-check.log.gz` with `checks/flake-check.exit`. The corrected rerun does not establish universal first-frame timing or full parity.

The final `nix flake check -L 'path:.'` exited 0 after the startup assertion
correction. Its complete log and exit marker are `checks/flake-final.log.gz`
and `checks/flake-final.exit`. This establishes the current Nix check baseline;
the workflow's full input/cell parity and coverage flags remain false.
