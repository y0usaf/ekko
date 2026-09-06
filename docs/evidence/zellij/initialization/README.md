# Public initialization mechanism evidence

This slice adds `register-component :initialize`; it does not implement Zellij
release notes or persistent plugin state. The callback group returns bounded,
owned state/mode/status/decoration actions and is staged before commit. Startup
runs after inert layout and before child spawn. Reload preserves child PIDs.

The real-daemon check runs regular and bare binaries. It covers initial input
interception, dismissal, retained state on reload, owner removal/reinstallation,
seven rejected candidates (invalid later actions, errors and timeouts), queued
input replay after rejection, and failed startup before any child executes.
The exact application input is `61626263` (`abbc`). Unit checks additionally cover
aggregate state rejection, unchanged live terminal geometry during staging,
declared dependency invalidation and intervening retained state.

The first test attempt incorrectly asserted that all decorations disappear when
one owner is removed; regular builds retain their built-in chrome. The archived
failure was fixed to assert removal of the requested owner's contributions only.
No runtime assertion or acceptance gate was relaxed.

`finix-zellij-shared-final-check.log.gz` records the separately pinned shared-runtime
preview at commit `01ab2dee4eab6d89a26f220a7519e4b9c7b83174`: four checks passed,
including regular/bare frame and viewer-exit tests and ephemeral/retained launcher
paths. The main Finix configuration and live runtime patch were not changed.

Full Zellij parity and coverage remain false. The next required startup mechanism
is durable namespaced state with atomic updates and failure semantics; pointer,
floating, release-note and tip behavior are separate required work.

Final `nix flake check -L path:.` exited 0: 25 named checks and 14 paired workflow scenarios. The Nix queue reports 26 build checks; `checks.json` lists all named attributes. Runtime: `/nix/store/9cjxh7999gfhrb98ynkcrxrw7pfdv5dh-ekko-0.1.0`.
