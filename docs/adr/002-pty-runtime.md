# ADR-002: Controlling PTYs and the live benchmark

The acceptance workspace uses local terminal-slack next to terminal-browser,
with ordinary shells using the same pane mechanism.

The pinned SBCL 2.5.4 `run-program :pty t` probe produced terminal descriptors,
but the child was not a session leader and `tcgetpgrp` returned ENOTTY. Repeating
with the supported input settings did not establish a controlling terminal.
The smaller built-in launch path cannot supply shell job control here.

A C adapter uses `openpty`, `fork`, and `login_tty`, installs initial dimensions,
closes unrelated descriptors, resets signals, and calls `execvp` with the supplied
argv. A close-on-exec error pipe reports exec failure. No Lisp executes between
fork and exec. VT state, graphics semantics, layout, input policy, IPC framing,
and rendering remain in Lisp. Zlib supplies the image codec.

Packaged tests check controlling-terminal ownership, shell job control, resizing,
independent input, and daemon survival across client loss. The graphics subset
was selected from the actual terminal-browser version pinned by terminal-slack.
Optional file/shared-memory probes are rejected; inline native frames work
without application patches. Native Kitty source rectangles clip placements.

Pinned Kitty 0.42.1 works on the live Wayland desktop. The Xvfb/GLX precursor
remains unavailable. Live visual verification and synthetic checks are recorded
separately. This delivers a testable preview; remaining release and extension
gates in GOAL.md are not waived.
