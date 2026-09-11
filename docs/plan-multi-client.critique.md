## Verdict

**Sound-with-fixes.** The plan is not ready to implement: its state partition omits state that determines geometry, input ordering, extension behavior, and presentation ownership. It enables multiple clients before establishing the invariants that make them safe. The largest unresolved issue is the extension contract, not worker ownership or client rendering. “Single-client behavior is unchanged” also conflicts with deferring existing zoom semantics and changing hidden-pane sizing.

## Holes and contradictions

### The state table is incomplete

> “Slot names are from `src/server.lisp:3-18`.”

> “tree, options, registry/worker, store, notices … session (unchanged)”

The actual structs and their consumers require these additions or corrections:

| State | Required treatment |
| --- | --- |
| `pane-x/y`, `pane-outer-x/y/cols/rows` | Per-view computed geometry. `layout` currently writes these into the pane; scene generation, hit testing, and extension snapshots read them. Leaving them there makes the last layout win. |
| `pane-minimized` | Per-view, as the hard constraint says, but missing from the table. |
| `pane-unseen-output` | Per-view activity tracking. Output currently marks activity relative to session focus, and focusing clears it. One client must not clear another’s unread indicator. |
| `pane-activation-order`, `session-activation-sequence` | Explicit decision required. Focus updates these, and they determine floating-window stacking. Keeping them shared makes independent focus reorder another client’s windows. |
| `reported-cw/ch`, `key-fragment-mode`, `paste-overflow` | Missing companions to state already assigned to views. |
| `copy-cells`, `copy-end`, `copy-flash-until`, `search-input`, `search-text` | Explicitly include these. Copy state needs a map keyed by pane within each view, not one copy-mode slot per client. |
| `input-queue`, `input-bytes`, pending command origin | Input buffering must retain originating view and session binding. Worker serialization can remain session-scoped. |
| `hook-context`, queued hooks, decorations, status contributions, geometry contributions, `component-state` | Cannot remain implicitly session-wide if callbacks consume per-view context. Define scope for each. |
| `revision`, presentation stamp | Need view invalidation as well as shared pane/session invalidation. |
| Clipboard and notices | Separate the shared Ekko buffer and session errors from origin-specific clipboard delivery and interaction feedback. |

Evidence: [pane/session structs and layout](/home/y0usaf/dev/maintaining/ekko/src/server.lisp:3), [deferred input](/home/y0usaf/dev/maintaining/ekko/src/commands.lisp:172), [extension action storage](/home/y0usaf/dev/maintaining/ekko/src/commands.lisp:664), [clipboard delivery](/home/y0usaf/dev/maintaining/ekko/src/selection.lisp:15).

There is no obvious shared pane-content state wrongly moved wholesale. The dangerous ambiguity is `cw/ch`: a view owns host cell measurements, but the shared VT still requires canonical cell metrics. Moving session measurements does not eliminate that shared requirement.

“Store … unchanged” is also misleading. The in-memory store is session-owned, but durable filenames are keyed only by component namespace, without a session name. Multiple session copies can overwrite the same persistent namespace. Choose a server-owned authoritative store preserving that existing namespace scheme, or explicitly introduce session namespaces and migration. [Store implementation](/home/y0usaf/dev/maintaining/ekko/src/store.lisp:14)

### A shared tree does not preserve geometry-hook semantics

> “No per-view layouts. Arrangement is session-scoped, so extension geometry hooks (`:layout`, `:panes` rects) keep working unchanged.”

That conclusion is false. `session-rectangles` depends on viewport, focus, minimize, zoom, and floating-window stacking. Even ordinary focus can change geometry when a small layout collapses. The tree can remain shared while its computed rectangles differ.

`context-data` exposes far more than `:viewport`: focus, mode, zoom, visibility, activity, display labels, activation order, and rectangles. A synthesized minimum viewport cannot describe both clients’ interaction state. [Geometry](/home/y0usaf/dev/maintaining/ekko/src/server.lisp:152), [extension context](/home/y0usaf/dev/maintaining/ekko/src/commands.lisp:72)

Per-view hooks also cannot write into one shared `decorations` or `component-state` slot without overwriting each other. A view ID alone does not fix storage scope or stale callback results.

### Min sizing is underspecified and oversold

> “A pane's PTY size is the minimum over the rects of the views currently showing it.”

> “This is the only lossless policy.”

Specify **componentwise minimum of content dimensions after insets**, not outer rectangles. A 100×20 view and a 60×40 view yield 60×20.

Delete “only lossless.” Resizing can alter application layout and discard screen content; padding does not undo that. The policy ensures that the canonical cell grid fits participating views once resizing settles.

The missing pixel policy blocks implementation. `pty-size-for`, VT resizing, graphics scrolling, clipping, and pixel mouse input all depend on cell metrics. Clients with different fonts cannot each supply the shared application’s canonical pixel geometry. Define canonical metrics and coordinate conversion explicitly. [PTY sizing](/home/y0usaf/dev/maintaining/ekko/src/server.lisp:19), [mouse forwarding](/home/y0usaf/dev/maintaining/ekko/src/server.lisp:383)

Also define whether copy-mode views, fully occluded panes, and minimized panes constrain live PTYs. Add initial sizing for panes created with no viewers, and resize triggers for focus, minimize, restore, geometry hooks, and structural edits.

### “Set of wires” is not an ownership model

> “writer | one wire | **set of wires**”

Some operations broadcast; others have exactly one recipient. Detach, clipboard export, popup results, command errors, and delayed mode changes need an originating view.

Today keyboard commands commonly pass `nil` as the command peer. Delayed actions then resolve their target through current session focus. Preserve the initiating view and stable target identity through dispatch and completion; define cancellation after detach or session switching. Replacing `writer` with a collection does none of this. [Command dispatch](/home/y0usaf/dev/maintaining/ekko/src/commands.lisp:229), [action target resolution](/home/y0usaf/dev/maintaining/ekko/src/commands.lisp:728)

Focus reporting also needs aggregation. `set-focus` emits application focus-out/focus-in events, and host focus packets are forwarded separately. One client leaving a pane must not report it unfocused while another active client still focuses it.

### Thin-client IPC lacks its essential contract

> “Move `render-scene` … into the server”

> “`publish-scene` … fans out per view; each scene carries that view's rects.”

Are scenes now internal renderer inputs, or still wire payloads? Specify frame-byte messages, frame identity, completion acknowledgements, and host-reply routing.

Currently the client acknowledges only after terminal output drains and outstanding file uploads complete. Server socket drainage is not terminal drainage. Snapshot leases must survive until the corresponding terminal-side completion, including during switching and detach. [Client acknowledgement loop](/home/y0usaf/dev/maintaining/ekko/src/client.lisp:408), [wire leases](/home/y0usaf/dev/maintaining/ekko/src/wire.lisp:64)

Session switching introduces identity collisions: pane IDs restart at 1 per session, while renderer caches use pane/image IDs and the allocator call hardcodes incarnation `1`. Add session incarnation to identity or explicitly retire presentation state at a switching boundary without reusing IDs that can receive late replies. [Renderer allocation](/home/y0usaf/dev/maintaining/ekko/src/client.lisp:247)

### Compatibility and lifecycle are missing

> “`ekko run --session NAME` keeps its current output and semantics.”

Today a new daemon inherits the invoking process’s environment and working directory. Later sessions created inside an existing server will inherit the first server launch’s context unless creation requests carry that information. `serve` also sets `EKKO_SESSION_NAME` process-wide. Child launch context must become explicit.

Routing only the attach packet is insufficient: control clients send commands without attaching. Specify versioned routing for those connections too. Separate resize from attach; `send-size` currently reuses the attach packet.

Define session-switch behavior, concurrent creation of the same name, failed initialization cleanup, last-pane closure, and server lifetime with zero sessions.

## Risks and ordering

The current order is backwards:

> Phase 2: “Delete the second-attach rejection”

> Phase 3: “Prefix, mode and copy state per view”

Isolation and canonical sizing are prerequisites for accepting a second client, not hardening afterward.

Use this order:

1. Resolve state ownership, extension contexts/actions, canonical pixel geometry, and lifecycle semantics.
2. Extract views while retaining one attached client. Preserve existing behavior.
3. Introduce the server registry, explicit child launch context, routed controls, and session-local teardown.
4. Implement sizing, command provenance, close guards, focus aggregation, and switching cleanup.
5. Enable multiple clients.
6. Move rendering across the wire in a separate step.

Centralizing rendering adds CPU and allocation work to the reactor serving every PTY. Bound work per client and session. Preserve independent backpressure and snapshot leases; one stalled client must not stall others.

Do not trust the architecture document’s timeout claim: it says ten-second presentation timeout, but the reactor explicitly retains attached viewers awaiting acknowledgements indefinitely. Multiple stalled viewers make that retention policy materially more expensive. [Actual peer servicing](/home/y0usaf/dev/maintaining/ekko/src/server.lisp:803)

The proposed manual checks are inadequate. Add automated cases for interleaved prefixes, delayed commands plus trailing input, copy isolation, mixed cell sizes, minimize/focus resizing, switching with uploads in flight, close while another view has a drag or paste target, and stopping one session while another remains operational.

## Cut or defer

- Remove worker ownership and pane linking from “Open questions.” Constraint 5 already chooses one worker per session; cross-session panes are already deferred.
- Defer `ignore-size`. It adds oversized-content and input-mapping cases before the basic policy is defined.
- Separate server-side rendering from the multi-client correctness milestone. Existing client renderers already provide attachment-local caches.
- Do not defer existing zoom semantics while promising unchanged single-client behavior. Retain per-view zoom using the shared tree, or explicitly narrow compatibility. Zoom within a session does not require cross-session pane sharing.
- Cut “pure byte pump” terminology. The specified client still owns escape timing, terminal lifecycle, resize observation, reply classification, and output completion.

## Concrete edits

Replace the open questions with decisions:

1. **Worker ownership — not blocking:** one worker and candidate lifecycle per session; the server supervises them. Transience changes teardown timing, not ownership.
2. **Cross-session panes — not blocking:** unsupported in this change.
3. **`stop NAME` — blocking:** destroy only the named session and disconnect its views. Other sessions survive. Reject destructive operations when other viewers exist unless explicitly forced; apply the rule to control RPC and extension actions, including last-pane closure.
4. **Extension context — blocking:** callbacks run with an explicit originating view where applicable; shared `:layout` remains the tree, while viewport, rects, focus, visibility, mode, and presentation outputs are view-scoped. Define headless initialization and shared geometry-action semantics separately. Remove “unchanged.”
5. **Client rendering — already answered:** the final client performs terminal I/O and framing, with no scene-to-terminal renderer. Document terminal-drained frame acknowledgements and server-owned upload completion tracking.

Add explicit contracts for:

- A view’s pane-state map, stable focus identity, and behavior when switching away and back.
- Session-level mutation serialization plus view-tagged command/input queues.
- Canonical PTY/VT dimensions and pixel metrics, view coordinate transforms, and resize debounce timing.
- Connection-owned presentation identity and leases that remain valid across session changes.
- Server-owned assets, quotas, startup reclamation, and session-local cleanup.
- View-aware `status`/`inspect`; their current singular focus, geometry, and copy-mode fields become ambiguous.

No files modified.