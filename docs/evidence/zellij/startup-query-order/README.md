# Startup query ordering probe

The standalone probe runs three schedules at 80×24: immediate query replies,
replies held until the current A/B/C event files each contain FIRST, and no
replies. Every side/schedule uses a fresh temporary session directory. Gated
runs assert the gate opened and replies were sent after it. Parser-release
timestamps bound response processing; they are not child event timestamps or
initial arrival timestamps for buffered query bytes.

The initial corrected run (`report.json.gz`) used candidate k4a7qs… and observed
known Zellij FIRST pixels with immediate replies, zero with gated/no replies;
Ekko recorded zero for every schedule. It was invoked directly using Nix-store
Python dependencies and is supplementary evidence.

The independent [Nix invocation](nix-command.txt), candidate z8hwiia…, exited
zero and is archived under `nix/`. All FIRST samples in that run had zero pixels,
including immediate Zellij. Both runs retain raw ANSI, query/response records,
and complete child event histories. Neither run proves startup parity: the
immediate response schedule can yield different FIRST observations across runs.
The existing early-known-pixel discrepancy remains required.

Earlier faulty gate implementations are retained separately in
`../startup-query-order-invalid-initial-probe/` and
`../startup-query-order-invalid-second/`, explicitly invalid evidence.
No acceptance histories were normalized and this is not a full parity pass.
