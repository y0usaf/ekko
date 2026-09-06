# Reported pixel evidence

This directory records bounded visual and workflow probes for reported PTY
pixel metrics. It does not claim full parity.

* [paired](paired/README.md) contains current 80x24 and 20x8 all-scenario
  runs plus 120x24 `new-n` query/no-query comparisons. The 120x24 query case
  retains a measured auto-layout geometry difference; the no-query case passes
  its named slice.
* [visual](visual/) contains the completed private-display visual tree,
  including PNGs, compressed JSON/ANSI, cleanup records, and checksums.
* [startup-order](startup-order/) contains the failed paired Nix check output
  from `/nix/store/nh96axfbvhw6j0bg55sap4xr7hvkbczz-ekko-zellij-pane-workflow`.
* [contracts](contracts/) contains the passing pane-notes and pane-pixels Nix
  contract outputs.

The `paired` runs are pre-focus-fix workflow evidence. The `visual` run is a
post-focus-fix candidate: 13 settled checkpoints have zero pixel and cell
differences, while startup-screen differs by 50,794 pixels and 1,159 cells;
`full_parity` remains false. The visual run cleaned up with zero remaining
owned PIDs.

All archives preserve raw mismatches and use reproducible gzip headers.

[Current Nix checks](checks/result.json): all 15 checks other than the known
failing `zellij-pane-workflow` passed. This is not a full flake-check pass.
