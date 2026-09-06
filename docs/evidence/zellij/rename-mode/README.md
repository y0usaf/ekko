# RenamePane evidence

This archive preserves three bounded Ekko rename captures and one reference-only probe. JSON and ANSI traces are compressed with `gzip -n -9`; PNG files are retained unchanged. `SHA256SUMS` covers every archived file except this README.

## Paired 80x24

Source: `/tmp/ekko-rename-80x24`, query responses enabled, fixture shell SHA256 `a0402312561c81005bd2b092583414dd4ed70b229ca7bb2fa3b1fafa43bdd7e5`. The run predates the retaining undo-backup fix; the visible rename path is the same. It has 14 stages. Startup differs by 1,045 cells; the remaining 13 stages differ by 14 cells each. At startup, input and input delta match; at `dismiss-release-notes` both differ; from `pane-enter` onward input differs while input delta matches. Event comparison ignoring PID differs at every non-startup stage. Cursor comparison differs at every stage. Application cell geometry, pixel geometry, dimensions, grid shape, outer ioctl, terminal queries, focus receiver, and probe input match at every checkpoint. Cleanup exited zero with no remaining PIDs.

## Paired 20x8

Source: `/tmp/ekko-rename-20x8`, query responses enabled. It has 14 stages. Input, input delta, application cell geometry, pixel geometry, dimensions, grid shape, events ignoring PID, focus receiver, and probe input match at every checkpoint. Cursor comparison differs at every stage. Startup differs by 29 cells; the remaining 13 stages differ by 43 cells each. Cleanup exited zero with no remaining PIDs. This run includes the retaining undo-backup path and exercises the paired `rename-paste` stage.

## Visual capture

Source: `/tmp/ekko-rename-visual`. Provenance command:

    nix run 'path:.#zellij-visual' -- compare --zellij /nix/store/wxjfyl2ksqkh263zwikp2igqh2a23y8r-zellij-0.43.1/bin/zellij --ekko /nix/store/ypzqdrx03rf53wa72zkga4mlz3rcpkrl-ekko-0.1.0/bin/ekko --profile /home/y0usaf/dev/maintaining/ekko_v2/examples/profiles/zellij.lisp --reference /home/y0usaf/dev/maintaining/ekko_v2/tests/zellij/reference --output /tmp/ekko-rename-visual --scenario rename-workflow

The visual report contains 9 settled checkpoints with zero pixel and cell differences. Its startup screen has 50,706 differing pixels and 1,159 differing cells. Cleanup exited zero with no remaining PIDs. `full_parity` is false; this visual capture is a bounded ASCII baseline.

## Reference-only probe

Source: `/tmp/ekko-rename-reference-probe2`. Files preserve the stages and raw traces for the pinned Zellij reference rename behavior. The exact invocation was not recorded in the source directory, so provenance is explicitly unavailable. The probe showed entry placeholder `Enter name...`, `ABC`, reentry append `ABCX`, and Escape restoration to `ABC`.

The paired captures exercise filtered paste; the visual capture does not. Unicode display width and large-input parity remain unproven.

## Nix checks

`checks/other-checks.log.gz` is the recorded successful run of these 19 checks (all checks except `zellij-pane-workflow`), using this argv:

    nix build --no-link -L --print-out-paths path:.#checks.x86_64-linux.build path:.#checks.x86_64-linux.daily path:.#checks.x86_64-linux.decorations path:.#checks.x86_64-linux.fake-host path:.#checks.x86_64-linux.keymap-input path:.#checks.x86_64-linux.keymaps path:.#checks.x86_64-linux.packaged-smoke path:.#checks.x86_64-linux.pane-layouts path:.#checks.x86_64-linux.pane-modes path:.#checks.x86_64-linux.pane-moves path:.#checks.x86_64-linux.pane-notes path:.#checks.x86_64-linux.pane-pixels path:.#checks.x86_64-linux.pane-rename path:.#checks.x86_64-linux.pane-titles path:.#checks.x86_64-linux.pane-workflow path:.#checks.x86_64-linux.runtime path:.#checks.x86_64-linux.startup-geometry path:.#checks.x86_64-linux.zellij-pane-differential path:.#checks.x86_64-linux.zellij-routing

The 19-check run preceded the final inspector JSON-state escaping fix. The parent observed exit 0; the log includes the pane differential 80x24 and 20x8 passes.

`checks/final-contracts.log.gz` records the final two contract checks, both exit 0, with these commands:

    nix build --no-link -L --print-out-paths path:.#checks.x86_64-linux.pane-rename path:.#checks.x86_64-linux.keymap-input

The corresponding final outputs were `/nix/store/ix3dh6z6lfj9qhrhsqxvlaq4z08a65hj-ekko-pane-rename` and `/nix/store/krn9w8jg0f6bm09jgajijipy925kim2g-ekko-keymap-input`. These final checks ran after the inspector JSON-state escaping fix. The parent observed exit 0 for both commands; the archived logs preserve their output.

The final store output contents are preserved in `checks/pane-rename.result`
and `checks/keymap-input.result`; each records regular and bare suite passes.
