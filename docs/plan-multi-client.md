# Plan: one daemon, one workspace, N clients

Status: implemented, then simplified. The design landed simpler than drafted:
instead of one server hosting many named sessions, each daemon owns exactly one
workspace and `--instance NAME` provides isolation. Attaching later changed to
takeover semantics: a new attach detaches the previous client and reuses the
session view, so only one client is attached at a time and `run` commands only
apply at workspace creation. Per-client views and thin clients are otherwise as
planned. This is not a rewrite of the pane runtime or the VT.

## Terminology (pinned)

- **server** — one Ekko daemon process per `--instance`. Holds the workspace.
- **workspace** — the daemon's arrangement of panes. Owns its layout, options,
  extension worker, panes, and durable store. Survives client detach; ends on
  `stop`, a signal, or the last pane exiting.
- **pane** — one PTY plus its VT and graphics store. The unit of content.
- **client** — a terminal process attached to the server. Owns the tty.
- **view** — server-side per-client state: focus, viewport, interaction state,
  render cache.
- **wire** — one length-prefixed IPC connection; **peer** is its socket object.

Dropped: "desktop" as a model level, and named sessions entirely; the
arrangement belongs to the workspace. `ekko/desktop` remains the visual theme
profile (`examples/profiles/desktop-style.lisp`).

## Done means

- `ekko [--instance NAME] run ...` starts the server if needed, creates the
  workspace if absent, and attaches this terminal.
- `ekko attach` mirrors the same workspace with independent focus.
- One rendering implementation (server-side), one IPC version, one daemon per
  workspace.
- Pane PTY sizes follow the min-over-views policy.
- Single-client behavior is unchanged from the user's point of view.

## Hard constraints

1. **One PTY, one winsize.** A pane's PTY size is the minimum over the rects of
   the views currently showing it. Larger views pad; smaller views pan or clip.
   A pane with no viewers keeps its last size. This is the only lossless policy.
2. **Structural edits are shared.** Split, close and new-pane change the
   workspace for every attached client.
3. **Per view, never shared:** focus, minimize, copy/scroll
   state, prefix/mode, drag/hover/popup/paste-target, render cache. Two clients
   typing must not share a key prefix, a mode, or copy mode.
4. **Shared and guarded:** close kills a process other clients may be watching.
   `stop` refuses while other clients are attached unless `--force`.
5. **Config isolation stays.** One extension worker per workspace, as today
   (`[[principle:functional-core]]`). The server holds the worker.
6. **Wire discipline.** `+wire-version+` is 16 and old attaches are rejected
   explicitly (`[[principle:daemon-thin-client]]`). The route is
   `(version view-id)`; there is no session name.
7. **No per-view layouts.** Arrangement is workspace-scoped, so extension
   geometry hooks (`:layout`, `:panes` rects) keep working unchanged.

## State partition

Slot names are from `src/server.lisp:3-18`.

| state | before | after |
| --- | --- | --- |
| panes, PTYs, VT, graphics, history, labels, status | session | **daemon** (the workspace) |
| tree, options, registry/worker, store, notices | session | **daemon** (the workspace) |
| cols/rows/cw/ch, focus, zoom, mode, prefix, key-fragment, drag, window-drag, window-hover, chrome-press, popup, paste-target/fallback/buffer, last-click, input-read-*, transition | session | **view** |
| copy-* (pointer/anchor/lines/cursor/top/mark/search), selection, scroll offset | pane | **view** |
| outer image IDs, row cache, transport probe state | client | **view** (moves to server) |
| writer | one wire | **set of wires** |

## Phases

**Phase 1 — server and workspace.**

- One socket (`$XDG_RUNTIME_DIR/ekko/instance-NAME.sock`) and one lock per
  instance; routing is `(version view-id)` in the route packet.
- `serve` runs a process-level reactor over a single workspace object; all pane,
  view, layout and worker state lives directly on the daemon.
- `run` = ensure server, ensure workspace, attach. `attach`, `status`, `inspect`,
  `buffer`, `stop`, `config`, `command`, `split`, `rename` route over the socket;
  `--view ID` selects a view where needed.

**Phase 2 — views and thin clients.**

- Move `render-scene` (`src/client.lisp:205`), the row cache, the `ekko/client`
  attachment (outer image IDs), clipping, and transport probing into the server,
  one per view.
- The client keeps: raw mode, viewport reporting (`send-size`), input decode with
  escape timing, frame writes, terminal enter/leave, and forwarding host graphics
  replies from stdin to the server.
- `publish-scene` (`src/server.lisp:561`) fans out per view; each scene carries
  that view's rects.
- Delete the second-attach rejection (`src/server.lisp:628-629`); tag input with
  the view id.

**Phase 3 — sizing policy and multi-attach hardening.**

- Min-over-views PTY sizing with a debounce; relayout on attach, detach and
  resize.
- Prefix, mode and copy state per view; guard close under co-viewers.
- Optional `ignore-size` client flag for a mirror-only client that does not
  constrain PTY size.

**Deferred.**

- Maximize to a dedicated arrangement, moving or linking panes across
  workspaces, per-view layouts, panning a window larger than the client, and zoom
  semantics. Each needs pane-sharing rules and a size policy beyond min; see Open
  questions.

## Verification

- `nix build` and `nix flake check` must pass (`[[principle:nix-verify]]`).
- Existing single-client behavior must not regress.
- Manual acceptance: two clients on one workspace with independent focus;
  detach and reattach; a version-15 attach rejected with a clear error; the
  server survives client death; PTY size equals the minimum after a resize.

## Open questions

Resolved by the flattening: the workspace owns the extension worker and store;
panes cannot span workspaces; `stop` refuses with attached clients unless
`--force`; hooks fire per view with a view id; the client is a thin terminal
host.
