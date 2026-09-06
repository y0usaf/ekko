# Isolated Finix Zellij profile preview

From the existing Finix checkout:

```sh
cd ~/finix
nix run path:./previews/ekko-zellij
```

This is a development preview of the partial profile. Ctrl-p opens Pane mode;
`r`/`d` split right/down, `h`/`j`/`k`/`l` focus, `z` toggles frames, `f` toggles
fullscreen, and `c` renames. Ctrl-h enters Move mode. Ctrl-o enters Session;
`d` detaches and Ctrl-q quits outside Locked mode. The full Zellij mode/bar/tab,
floating, plugin, session, CLI, and other surface requirements remain open.

The launch prints a private configuration path and exact Attach, Stop and reload
commands for
that session. Edit the copied Lisp files there and run the printed reload
command; the runtime does not rebuild and the pane applications keep their
PIDs. Closing the viewer ends this disposable preview and cleans only its
private runtime. The printed Stop command also ends only this preview.

To test detach/reattach while retaining the children, use:

```sh
nix run path:./previews/ekko-zellij -- --keep-session
```

After Ctrl-o then d, use the printed Attach command to return to the same
session. The printed Stop command ends it. Its private directory remains
available for reviewing the copied configuration; remove that printed directory
when finished. Without `--keep-session`, closing the viewer cleans the preview.

The existing custom menu remains separately selectable:

```sh
EKKO_CONFIG="$PWD/modules/shell/ekko/init.lisp" nix run path:./previews/ekko-zellij
```

To choose the child explicitly, append `-- /bin/sh -i`. To build and verify:

```sh
nix build path:./previews/ekko-zellij
nix flake check -L path:./previews/ekko-zellij
```

The nested preview flake has its own lock and source input pointing at the
isolated parity worktree `~/dev/maintaining/ekko-zellij-parity`. It does not change
Finix's main Ekko input, system configuration, deployed menu, or running sessions.
Its separate `runtime.patch` retains the existing overlay, graphics clipping,
copy-input and scrollback fixes. The two already-integrated geometry fixes are
omitted, and the metadata/protocol hunks are ported onto base wire 9, using wire
10 for this combined preview. The original custom menu also passes its existing
Nix-backed integration test against this candidate. The
live `modules/shell/ekko/runtime.patch` remains untouched.

Frames, modes, bindings and geometry choices are replaceable Lisp. Generic
runtime actions validate contributions, enforce ownership, resize PTYs, and clip
graphics. Zellij is used only as the pinned independent test oracle. The
[frame evidence](../evidence/zellij/frame-toggle/README.md) includes screenshots
of the actual Finix patched binary. A useful preview does not pass the full
acceptance gate in GOAL.md.
