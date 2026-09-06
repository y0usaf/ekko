# Public layout replacement

`:set-layout :tree` lets ordinary components replace a tiled arrangement using
existing stable pane IDs. The daemon validates every leaf exactly once, branch
shape and percentages, and a maximum of 31 nodes before applying the action.
Invalid batches leave state unchanged. It copies the accepted tree and uses
the existing layout/PTY resize path, preserving focus, fullscreen, and children.

The regular/bare real-PTY test rearranges three children while fullscreen,
checks exact unzoomed rectangles and kernel winsizes, rejects a duplicate
leaf, detaches, removes/reloads the component, and verifies the arrangement
and PIDs survive. Nix unit tests cover missing/unknown IDs, malformed branches,
percentages, excessive depth, and snapshot isolation. The neighboring pixel
and pane-workflow contracts also pass. See [command and outputs](result.json).

This is a reusable mechanism for profile-driven layout switching; automatic
Zellij layout selection and its spawn/resize side effects remain incomplete.
No reference discrepancies were waived and no new screenshot parity is claimed.
