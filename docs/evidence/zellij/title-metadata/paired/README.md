# Paired title and pane workflow evidence

Runs used the current `nix run 'path:.#zellij-pane-workflow'` package and the
current `examples/profiles/zellij.lisp`, with query responses enabled. The
runner returned exit 1 for both runs because differences were found; all cells,
inputs, terminal queries, and cleanup records are retained without
normalization.

| viewport | scenarios | result |
|---|---|---|
| 80x24 | `titles`, `new-n`, `new-d`, `new-r`, `fullscreen-navigation` | exit 1; functional slice differs |
| 20x8 | `titles`, `new-r` | exit 1; functional slice differs |

At 80x24, startup has 1045 differing cells (Zellij's release notes popup and
chrome); each later title checkpoint has 14 differing cells, with one modeled
cell in the per-scenario title reports. The title fixture's OSC 0, OSC 2,
empty, long, and Unicode-whitespace requests were consumed and recorded. Input
is equal only at startup: Zellij consumes Escape to dismiss release notes while
Ekko forwards it to pane A. `input_delta_equal` is true for each title request;
the cumulative input remains different. Creation stages select Ekko pane 4;
their application cell geometry differs after the split, while outer dimensions
remain equal. Fullscreen focus-right reaches zero differing cells, but the
fullscreen geometry stages still differ.

At 20x8, cumulative and delta input are equal at every checkpoint, including
all title requests and `new-pane-right`. Startup differs by 29 modeled cells,
title stages by 43–51 cells, and `new-pane-right` by 37 cells. Outer dimensions,
grid shape, application input receiver, and probe input are equal; pixel
geometry and terminal query sets differ at every checkpoint. The recorded A
child geometry is 6x8 initially and 6x3 after right split.

The runner's full reports are compressed beside this file, with raw ANSI and
cleanup records. `provenance.json` records package paths and hashes.
