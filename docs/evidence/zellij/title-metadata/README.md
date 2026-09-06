# Title metadata verification

Fresh Nix builds pass for the executable and `checks.pane-titles`, including
regular and bare profile lifecycle tests. Native unit/config checks also pass.
Exact commands, store outputs, hashes, and earlier restricted-access attempts
are preserved in `status.json` and the accompanying logs.

The [private visual workflow](visual/summary.json) has zero differing pixels
and modeled cells at all 13 settled checkpoints, including pane creation,
fullscreen focus, the red failed-split flash, and restoration. The startup
release-notes screen differs by 50,672 pixels and 1,159 cells. PNG pairs, ANSI,
cell reports, conditions, cleanup records, and checksums are archived under
`visual/`; both owned sessions stopped successfully. These are workflow results,
not complete surface coverage.

[Paired 80×24 and 20×8 runs](paired/README.md) verify application input deltas
and focus but retain startup-input, small-terminal output, and PTY resize-history
differences. At 80×24, the title stages retain 14 differing cells from the
startup input effect. No observed difference is normalized away.

The full flake check failed the reference workflow fixture's `/usr/bin/env`
launcher inside Nix. Using the harness's pinned Python interpreter fixes that
startup failure. The corrected workflow check completes its scenarios and
fails the actual resize-history gate; its complete output is under
`nix-workflow/`. The full flake check and full Zellij parity remain incomplete.
