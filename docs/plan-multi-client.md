# Plan: one server, many sessions, N clients

Status: draft for review. Scope: replace the per-session daemon with one server
that hosts every session, with per-client views and thin clients. This is not a
rewrite of the pane runtime or the VT.

## Terminology (pinned)

- **server** — the single Ekko process, one per user. Holds every session.
- **session** — a named arrangement of panes that clients attach to and switch
  between. Owns its layout, options, extension worker, and (for now) its panes.
  Survives client detach.
- **pane** — one PTY plus its VT and graphics store. The unit of content.
- **client** — a terminal process attached to the server. Owns the tty.
- **view** — server-side per-client state: current session, focus, viewport,
  interaction state, render cache.
- **wire** — one length-prefixed IPC connection; **peer** is its socket object.

Dropped: "desktop" as a model level; the arrangement belongs to the session.
`ekko/desktop` remains the visual theme profile
(`examples/profiles/desktop-style.lisp`).

## Done means

- `ekko run --session NAME ...` starts the server if needed, creates the session
  if absent, and attaches this terminal.
- A second `ekko attach NAME` mirrors the same session with independent focus.
- One rendering implementation (server-side), one IPC version, no per-session
  daemons.
- Session PTY sizes follow the min-over-views policy.
- Single-client behavior is unchanged from the user's point of view.

## Hard constraints

1. **One PTY, one winsize.** A pane's PTY size is the minimum over the rects of
   the views currently showing it. Larger views pad; smaller views pan or clip.
   A pane with no viewers keeps its last size. This is the only lossless policy.
2. **Structural edits are shared.** Split, close and new-pane change the session
   for every attached client.
3. **Per view, never shared:** focus, current session, minimize, copy/scroll
   state, prefix/mode, drag/hover/popup/paste-target, render cache. Two clients
   typing must not share a key prefix, a mode, or copy mode.
4. **Shared and guarded:** close kills a process other clients may be watching.
   It must be rejectable or confirmable when co-viewers exist.
5. **Config isolation stays.** One extension worker per session, as today
   (`[[principle:functional-core]]`). The server holds the workers.
6. **Wire discipline.** Bump `+wire-version+` (13 -> 14) and reject old attaches
   explicitly (`[[principle:daemon-thin-client]]`).
7. **No per-view layouts.** Arrangement is session-scoped, so extension geometry
   hooks (`:layout`, `:panes` rects) keep working unchanged.

## State partition

Slot names are from `src/server.lisp:3-18`.

| state | today | after |
| --- | --- | --- |
| panes, PTYs, VT, graphics, history, labels, status | session | session (unchanged) |
| tree, options, registry/worker, store, notices | session | session (unchanged) |
| cols/rows/cw/ch, focus, zoom, mode, prefix, key-fragment, drag, window-drag, window-hover, chrome-press, popup, paste-target/fallback/buffer, last-click, input-read-*, transition | session | **view** |
| copy-* (pointer/anchor/lines/cursor/top/mark/search), selection, scroll offset | pane | **view** |
| outer image IDs, row cache, transport probe state | client | **view** (moves to server) |
| writer | one wire | **set of wires** |

## Phases

**Phase 1 — server and registry.**

- One socket (`$XDG_RUNTIME_DIR/ekko/ekko.sock`) and one lock; the session name
  becomes a routing field in the attach packet. `socket-path`
  (`src/wire.lisp:70-82`) becomes one path.
- `serve` splits into a process-level reactor plus a per-session object.
- `run` = ensure server, ensure session, attach. `attach`, `status`, `inspect`,
  `buffer`, `stop`, `config`, `command`, `split`, `rename` route by name over the
  shared socket.
- `ekko run --session NAME` keeps its current output and semantics.

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

- Maximize to a dedicated arrangement or session, moving or linking panes across
  sessions, per-view layouts, panning a window larger than the client, and zoom
  semantics. Each needs pane-sharing rules and a size policy beyond min; see Open
  questions.

## Verification

- `nix build` and `nix flake check` must pass (`[[principle:nix-verify]]`).
- Existing single-client behavior: the `tests/` suites and the zellij
  differential runner must not regress.
- Manual acceptance: two clients on one session with independent focus;
  detach and reattach; a version-13 attach rejected with a clear error; the
  server survives client death; PTY size equals the minimum after a resize.

## Open questions

1. If sessions can be transient, does a session still own an extension worker, or
   does the server own workers with sessions as namespaces?
2. Can a pane appear in more than one session (tmux `link-window`)? If so, who
   owns its worker and store namespace, and how is PTY size computed across
   sessions?
3. What does `stop NAME` mean when other clients are attached?
4. `:viewport` is session-scoped in the extension context today. Per-view
   viewports mean hooks fire per view; do hooks receive a view id, or a
   synthesized session viewport (the min)?
5. Does the client keep any rendering, or is it a pure byte pump?
