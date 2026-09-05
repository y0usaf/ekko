# ADR-000: Initial scope and capability truthfulness

Status: accepted for M00 planning

## Decision

Build a Linux x86-64 SBCL terminal multiplexer around daemon-owned virtual
terminals and pane-owned Kitty graphics state. Start with Kitty 0.42.1 as the
real-host oracle, pinned by git commit in [`../sources.json`](../sources.json).
Maintain one compatibility-ledger row per protocol family. Since M00 contains
no protocol implementation or verification, all rows start as `untested`; no
feature is advertised as supported until executable evidence exists.

Synthetic producers and a restricted fake host are necessary P0 test
instruments, but P0 also requires real Kitty visible proof; fake-host evidence
alone cannot establish P0. The names
`terminal-browser` and `terminal-slack` remain labels until candidate projects,
revisions, launch argv, and graphics traces are recorded and accepted.

## Smaller alternative rejected

A text-only multiplexer or raw Kitty passthrough would take less code, but it
cannot establish pane isolation, reconstruction after detach, or independent
image identifiers. A broad compatibility claim would also hide untested
semantics. The ledger is therefore deliberately explicit and incomplete.

## Evidence

The protocol authorities, immutable repository pins, and M00 toolchain pins are listed in
[`../sources.json`](../sources.json). The pinned Kitty documentation defines
the graphics APC, transfer formats/media, placement, deletion, and query
semantics; its keyboard document defines progressive enhancement and event
encoding. Xterm control sequences remain the VT baseline. The M00 CLI runner
exists and is covered by build checks; protocol tests and real-host evidence
remain pending in `PROGRESS.md`.

## Consequences and reversal conditions

Every implementation ticket must update the affected ledger row and add test
IDs without silently broadening support. If a pinned authority changes
normative semantics, pin a new revision, update the row, and add a regression.
If no target application requires relative placement or animation, those rows
remain deferred to P3; a target trace promotes them only with evidence.
