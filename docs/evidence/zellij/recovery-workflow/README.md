# Post-reboot workflow recovery evidence

This directory separates historical successful Nix outputs from fresh attempts
that could not start in the current sandbox. The pinned reference is
[`tests/zellij/reference/pin.json`](../../../../tests/zellij/reference/pin.json).

## Historical Nix outputs

These files were copied from existing Nix store paths and retain the original
check output. They are labeled historical and are not claims about a
fresh build:

- [`historical-store/pane-workflow`](historical-store/pane-workflow): regular
  and bare runs at 80×24 and 20×8, all passed.
- [`historical-store/pane-notes`](historical-store/pane-notes): regular and
  bare runs, both passed.
- [`historical-store/startup-geometry`](historical-store/startup-geometry):
  regular and bare runs at 80×24 and 20×8, all passed.
- [`historical-store/SHA256SUMS`](historical-store/SHA256SUMS): copied-output
  hashes.

## Fresh attempts

- [`nix-functional-checks.log`](nix-functional-checks.log) and its exit file
  record the requested Nix checks. They were blocked by Nix daemon socket
  permissions after relocating the cache to `/tmp`.
- [`pane-workflow-regular-80x24.log`](pane-workflow-regular-80x24.log),
  [`pane-workflow-regular-20x8.log`](pane-workflow-regular-20x8.log), and the
  corresponding bare logs record `connect failed (errno 1)` from the existing
  Ekko binary before fixture startup.
- [`pairedworkflow-80x24.log`](pairedworkflow-80x24.log) and
  [`pairedworkflow-20x8.log`](pairedworkflow-20x8.log) timed out waiting for
  fixture event files before differential stages began.

The exact commands, status, limitations, and no-normalization policy are in
[`provenance.json`](provenance.json). `source-boundary.log` is a native Python
static check of the split thresholds; it is not Nix validation.
