# Pane workflow visual recovery

The 2026-09-06 recovery attempt could not start the private compositor. The
comparator reached Zellij launch, but Cage exited before workflow fixture
readiness, so no screenshot, ANSI checkpoint, red `CAN'T SPLIT!` frame, or
restoration frame exists for this attempt.

The retained failure is under `attempt-20260906/`. `input-sha256.txt` records
the current comparator/profile and pinned reference/tool binaries;
`artifact-sha256.txt` records every retained attempt artifact.

Failure evidence from `zellij/cage.err`:

* `/tmp/.X11-unix not owned by root or us`
* `No display available in the first 33`
* `Cannot create XWayland server`
* `Unable to open Wayland socket: No such file or directory`

The run used the private Cage headless harness, Kitty, DejaVu Sans Mono, and
the pinned Zellij 0.43.1 reference. No user desktop surface was captured.
