# Ekko v2 — an extensible terminal multiplexer in SBCL

Status: implementation specification; no implementation or verification is claimed.
Prepared: 2026-09-04. Project directory: `~/dev/maintaining/ekko_v2`.

## 0. Read this first

Build a new terminal multiplexer in Common Lisp, targeting SBCL. Its defining
capability is **correct multiplexing of independent Kitty Graphics applications**:
two applications may use identical image identifiers, draw concurrently, scroll,
resize, switch screens, and reconnect without corrupting one another's graphics.
The motivating workspace is a terminal browser next to terminal Slack.

This is a fresh design. Do not read, import, copy, or reverse-engineer the sibling
`ekko` project. Do not inherit its architecture, configuration, dependencies, or
compatibility promises. Upstream protocol documentation and independent terminal
implementations may be consulted with attribution and license review.

“Perfectly extensible” is a direction, not an excuse for an infinite framework.
Translate it into tested extension contracts, complete ownership, reversible
mounting, explicit compatibility, and a reproducible build. Write the smallest
implementation that meets these contracts. Do not build a generic application
platform before demonstrating graphics in two panes.

This document specifies the destination and an ordered path. An implementation
agent should implement it, not replace it with another plan. Work one bounded
ticket at a time, preserving a passing build. Do not mark the project complete
after a parser, a screenshot, a passthrough demo, or a text-only multiplexer.

### Authority and evidence

1. Follow user instructions and applicable ancestor `AGENTS.md` files.
2. Preserve the product outcomes and invariants in this document.
3. Use the pinned upstream specifications for exact external protocol semantics.
4. Resolve implementation choices locally; record consequential choices in ADRs.
5. If a normative rule contradicts an assumption here, correct the assumption,
   preserve isolation, add a regression, and explain the change.

The applicable `~/dev/AGENTS.md` requires least code, declarative policy,
reversible components, a functional core, public APIs for built-ins, daemon-owned
durable session state, and Nix verification. These are implementation constraints.
Do not edit that file. A local exception requires evidence and the exception
mechanism defined there; this document does not waive its rules.

## 1. Product contract

The user starts a workspace, splits it, launches arbitrary executable argument
vectors, and uses each pane as a terminal. Layouts, commands, bindings, themes,
status UI, and workspace recipes are replaceable through documented interfaces.
Applications do not need Ekko-specific patches for supported terminal behavior.

The daemon owns applications and their virtual terminals. Attaching a client is
opening a view of those terminals. Closing that view does not close applications.
Graphics survive detachment because image content and logical placements belong
to the daemon, not to the outer terminal's cache.

The flagship demonstration must show two actual applications, identified by
repository URL, revision, launch argv, and graphics mode. The names
`terminal-browser` and `terminal-slack` are illustrative until their exact projects
are identified. Never invent an executable, upstream URL, or successful demo.
Build synthetic browser-like and chat-like fixtures first; they are necessary
for deterministic tests but do not substitute for real application acceptance.
Use local pages and non-sensitive chat fixtures where practical. External login
or unavailable software is a named blocker for that integration gate only.

### Release boundaries

| Level | Required outcome | May be claimed |
| --- | --- | --- |
| P0: feasibility | Two synthetic producers, same IDs, clipped graphics, resize, reply isolation, text over graphics | Graphics architecture proven, only for tested cases |
| P1: useful alpha | PTYs, VT baseline, splits, focus, static graphics, detach/reattach, Nix artifact | Usable alpha with published limitations |
| P2: flagship v1 | All mandatory gates below, actual browser + Slack evidence, third-party extension exercise | Ekko v2 flagship release |
| P3: compatibility expansion | Animation, broader hosts, nested mux paths, extra platforms | Only the specific additional tested capabilities |

P2 requires native placements **and** incoming Unicode placeholder images, direct
transmission, raw RGB/RGBA and PNG, compression, image lifecycle, local transfer
adapters, pane-local responses, clipping, replay, and mixed text/image behavior.
Relative placement support is required by P2 if either target application uses
it; otherwise it is an explicitly rejected P3 capability. Full animation is P3
unless target traces promote it. Repeated static updates are required in P2.

Do not advertise complete Kitty protocol conformance at P2. Maintain an exact
capability table with supported, rejected, and untested states. Generic graphics
support has no magic negotiation bitmask for every subfeature; use documented
operation responses and application tests, not an invented negotiation protocol.

### Explicit initial exclusions

Linux x86-64 is the initial supported platform; Linux aarch64 is a later build
target. Start with a pinned Kitty host. Do not initially implement Windows,
macOS, GPU rendering, SSH transport, a package marketplace, collaborative input,
session resurrection after daemon death, or arbitrary Lisp sandboxing. Do not
write a browser or a Slack client. Do not embed tmux/zellij as the multiplexer.
Other terminals get a truthful text fallback until tested.

## 2. Non-negotiable invariants

Give each invariant a test identifier and include it in release evidence.

| ID | Invariant |
| --- | --- |
| I01 | Pane output can change only its own virtual terminal and owned resources. |
| I02 | Only the client renderer writes outer-terminal control sequences. |
| I03 | Application image identities never become global outer IDs unchanged. |
| I04 | A pane cannot delete, cover, move, or receive acknowledgements for another pane's images. |
| I05 | Every visible image pixel belongs to that pane's current visible content rectangle after clipping and UI occlusion. |
| I06 | Parsing is invariant under every possible read boundary, including single-byte reads. |
| I07 | Logical state survives client loss; a new client can reconstruct the view without application retransmission. |
| I08 | Input, terminal replies, and control RPCs have distinct decoding and routing paths. |
| I09 | Queues, parser buffers, image memory, history, and extension work have enforced limits. |
| I10 | Every live resource has one owner and a tested teardown path. |
| I11 | Extensions read detached snapshots and propose validated actions; they receive no mutable host handles. |
| I12 | Unmount removes component-owned contributions without undoing later unrelated user changes. |
| I13 | Resize, image completion, and rendering use explicit geometry/content generations. |
| I14 | Incremental rendering converges to the same visible state as a full reconstruction. |
| I15 | Advertised child terminal capabilities are backed by implementation and tests. |
| I16 | An unsupported or malformed sequence cannot escape into outer-terminal execution. |
| I17 | A clean pinned build runs without the developer's Lisp init files, Quicklisp cache, or source checkout. |
| I18 | Failure in an optional extension or a slow client cannot stop all pane I/O. |

The main test oracle is isolation plus reconstruction equivalence. A successful
image upload is not an adequate graphics oracle.

## 3. Architecture and ownership

```text
application A <-> PTY A -> byte parser -> VT A + logical image store A --+
application B <-> PTY B -> byte parser -> VT B + logical image store B --+
                                                                     |
                    daemon: reducer, layout, extensions, scene builder
                                                                     |
                       versioned local IPC: scene + asset references
                                                                     |
                   client: capability probe, diff, graphics scheduler
                                                                     |
                                      outer terminal (initially Kitty)

outer input -> client decoder -> semantic input event -> daemon -> focused PTY
outer replies -> client reply tracker -> presentation state (never focused PTY)
child queries -> daemon protocol handlers -> originating PTY
```

### 3.1 Minimal module boundaries

Use ASDF systems/packages to enforce direction; do not create a file for every
class. Extract more modules only when the boundary has a real consumer.

| Module | Responsibility | Must not own |
| --- | --- | --- |
| `ekko/core` | IDs, immutable event/action values, validation, reducer, ownership | FDs, terminal writes, extension execution |
| `ekko/vt` | Incremental octet parser, screens, modes, history, logical cursor | Outer coordinates, client image IDs |
| `ekko/graphics` | Child graphics semantics, image assets, logical placements, quotas | Raw outer output |
| `ekko/scene` | Layout geometry, visibility, text/image scene, damage | PTYs, terminal escape serialization |
| `ekko/platform` | Linux PTYs, poll, sockets, monotonic clock, signals, filesystem adapters | Layout or keybinding policy |
| `ekko/server` | Reactor, action commit, lifetime, RPC, work scheduling | Terminal-specific display policy |
| `ekko/client` | Outer capability state, input decode, rendering and transport | Authoritative application state |
| `ekko/extensions` | Versioned API, registry, dispatch, manifests, mounting | Direct mutation shortcuts |
| `ekko/builtins` | Default layouts, commands, keys, theme, status and overlays | Private kernel imports |

Protocol correctness, clipping enforcement, resource accounting, and IPC framing
are kernel mechanisms. Do not make them replaceable by untrusted actions. User
experience policies sit in built-ins through the same API outsiders use.

### 3.2 Data model

Specify concrete structs, typed identifiers, and field invariants before adding
methods. Prefer `defstruct` and ordinary functions for data and pure transforms;
use CLOS where multiple real implementations benefit from generic dispatch.

- `session`: pane registry, workspace tree, geometry lease, component context.
- `pane`: stable ID plus incarnation, PTY owner, process status, VT state,
  graphics namespace, input modes, quotas, title metadata.
- `vt-state`: main/alternate screens, cursor and saved cursor, scroll region,
  rendition, tab stops, terminal modes, incremental UTF-8/parser state.
- `cell`: grapheme data, width/continuation marker, rendition, hyperlink ID,
  optional semantic image-placeholder metadata. Preserve original semantics.
- `image-asset`: immutable validated content, dimensions, format, content
  generation, owner, byte accounting; optional normalized raster/cache.
- `logical-image`: pane-local identity, number lookup if applicable, current
  asset, image generation, reference/lifetime state.
- `placement`: logical image reference, pane/screen ownership, anchor, source
  rectangle, destination extent, offset, layer, placement identity, generation.
- `client-view`: capabilities, cell metrics, viewport, last acknowledged scene,
  terminal cache mappings, pending transactions, presentation generation.
- `component`: manifest, registration owner token, declared dependencies,
  contributions, subscriptions, timers/jobs, mount generation.

Keep image bytes out of repeated snapshots. Snapshots carry immutable asset
handles and metadata; only the host can resolve handles to owned data.
Mutable internal arrays are permitted behind exclusive ownership, but snapshots
must never alias them. Avoid copying full screens for every character: use
batched events, owned buffers, and detached snapshots at extension boundaries.

### 3.3 Reactor and actions

Start with one state-owning daemon reactor and nonblocking I/O. Reads and worker
completions become events. A reducer validates actions and commits coherent
state changes; an imperative executor performs declared I/O effects and returns
completion events. No callback may re-enter a half-finished commit.

Use worker processes for expensive image decoding and user Lisp dispatch when
hard termination is required. A worker result contains owner ID and generation;
discard stale results. Never kill a thread that may hold daemon locks to enforce
an extension timeout. Start simple: one serialized extension worker, with bounded
queues and restart on timeout, is acceptable before measured demand for a pool.

OS side effects are not magically transactional. Resource creation uses pending
ownership, then commit-on-success; failure closes pending resources. Irreversible
actions such as sending input or killing a process are explicit actions, never
described as reversible registry contributions.

## 4. PTY and process foundation

Use real PTYs with controlling-terminal and job-control behavior. A subprocess
with pipes is not a terminal. The Linux PTY routines are documented in
[openpty/forkpty/login_tty](https://www.man7.org/linux/man-pages/man3/forkpty.3.html).

First test whether the pinned SBCL process API satisfies controlling TTY,
foreground process groups, resize, descriptor ownership, and exec-error reporting.
If it does, use it. Otherwise use a small compiled C launcher/platform shim for
OS mechanics. Keep all mux policy, VT state, graphics semantics, and extension
logic in Lisp. Do not implement a second core in C or Rust.

The launcher must establish the session/controlling terminal, duplicate the
slave onto stdin/out/err, close unrelated descriptors, set initial window size,
and execute an argv directly. Use a close-on-exec error channel to distinguish
exec failure from normal exit. Do not run arbitrary Lisp between fork and exec
in a multithreaded runtime. Explain the selected spawn path in ADR-002.

Required behaviors: interactive shell, Ctrl-C, Ctrl-Z, `fg`, exit status, EOF,
SIGHUP, child reaping, SIGWINCH, nonblocking short writes, EINTR/EAGAIN, and Linux
PTY hangup behavior. A pane close targets its job/process-group ownership, not
unrelated PIDs. The shutdown policy has a bounded grace period and escalation.

Daemons use a per-user local socket in a private runtime directory. Handle
concurrent startup and stale sockets without deleting a live daemon's socket.
Authenticate local peers using OS credentials; restrict directory/socket access.
Do not expose an unauthenticated network listener or an eval RPC.

## 5. Virtual terminal baseline

Use an explicit byte state machine. Do not parse terminal output using regex
replacement. Separate UTF-8 text decoding from escape/control parsing. Bound
control headers, parameters, strings, nesting, and incomplete-sequence lifetime.
Specify discard/recovery states so oversized strings cannot turn their suffix
into executable outer controls.

Ship a checked-in support matrix and matching `ekko-256color` terminfo entry.
Use [xterm's control-sequence reference](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html)
for exact baseline behavior. Implement the subset needed by advertised terminfo
and the target applications, including:

- Cursor movement/addressing, wrap-pending state, origin mode, scroll margins,
  insertion/deletion, erase operations, tabs, save/restore, and reset.
- Main and alternate screen transitions, independent state where required,
  history ownership, resize rules, and a documented initial no-reflow policy.
- Indexed and true color, rendition reset, hyperlinks, cursor visibility/style,
  title updates, bracketed paste, mouse/focus modes, and synchronized updates.
- Child terminal/device queries answered from virtual state, including cursor
  position and pane-sized character/pixel geometry where supported.
- Wide characters, combining marks, grapheme boundaries, malformed UTF-8,
  right-edge behavior, erase of half a wide cell, and explicit ambiguous width.

Pin a Unicode data version and test the host width assumptions. Do not claim
perfect shaping parity with every terminal. Preserve Kitty placeholder metadata
before general text transformations; a combining-mark cleanup must not destroy
image references. History search/copy offers text-oriented output that omits
placeholder encoding noise while a diagnostic export preserves semantic data.

Pane output must never set global outer modes directly. Unknown OSC/DCS/APC
families are consumed safely, counted, and exposed through diagnostics. Handle
clipboard operations by explicit policy; do not forward arbitrary OSC 52.
If target apps use a known passthrough wrapper, add a bounded decoder that feeds
the inner sequence back into the same parser. It must not bypass isolation.

## 6. Graphics virtualization: the flagship subsystem

The upstream [Kitty Graphics specification](https://sw.kovidgoyal.net/kitty/graphics-protocol/)
is the wire authority. Read its control reference and terminal-interaction rules
when implementing each operation. The architecture and tests below are Ekko
design requirements, not a replacement protocol specification.

### 6.1 Three distinct identities

Model these separately:

1. Application identity, scoped by pane incarnation and logical image generation.
2. Daemon asset/placement identity, independent of any attached terminal.
3. Client-specific outer image/placement identity, scoped by attachment generation.

Allocate outer IDs through one client-owned allocator with explicit exhaustion
handling. Never implement `outer-id = pane-id * constant + child-id`; arbitrary
child IDs and reuse break that scheme. Never treat an image number as the same
concept as an image ID. Keep lookup semantics in the child graphics handler.

Tombstone retired mappings until their outstanding replies are consumed or the
attachment is reset. Wraparound must not alias a pending transaction. Reuse of a
pane ID after process restart creates a new incarnation, preventing stale jobs
and replies from affecting the new application.

### 6.2 Ingress and transaction boundaries

Every pane has its own graphics parser and upload assembly. A complete validated
operation commits logical state atomically; partial uploads are private pending
resources. A resize during upload is a normal interleaving: use the logical
cursor/state at the protocol-defined commit point, then current scene geometry.

Normalize completed uploads into daemon-owned immutable assets. Validate dimensions,
format, compressed and decoded sizes before accepting expensive work. Enforce
limits during decompression, not merely after it. Truncated payloads, missing final
chunks, invalid headers, and cancelled uploads release their pending ownership.

Different panes may upload concurrently. The outer renderer serializes each
outgoing graphics transaction according to the protocol; it must never interleave
another image's continuation data into an active upload. Control scheduling and
image scheduling share one output writer. Reserve queue capacity for input/replies
and bound the largest transaction so one image cannot stall interaction indefinitely.

Do not retain an unbounded raw escape transcript as the image database. Keep only
the canonical content and logical state needed to reconstruct current supported
behavior. Captures are optional bounded diagnostic artifacts.

### 6.3 Initial renderer decision and feasibility gate

Default design: consume both child placement forms into one logical scene, then
emit explicitly positioned native outer placements. This keeps outer rendering
independent of the application's chosen encoding. Placeholder cells become
semantic references with their own movement/lifetime rules, not opaque glyphs.

Before building general UI, prove this renderer with a tiny real-Kitty experiment:

- Two panes reuse the same application image and placement identifiers.
- One image crosses every edge of its pane, including partial-cell offsets.
- Transparent images overlap text at each supported layer relation.
- A small overlay occludes the middle of an image.
- A resize occurs during an incomplete upload; a later full replay agrees.

Native placement cropping/scaling may have host rounding or compositing limits.
The correctness fallback is a bounded raster crop/resample adapter that produces
exact-size fragments with a tested one-to-one pixel placement. Use a maintained
decoder/resampler dependency; do not write PNG or a general image library.
Cache derived assets by source generation, source crop, destination geometry,
pixel metrics, and rendering policy. Never crop a previously cropped result.

If this cannot faithfully represent required placeholder/text compositing, test
an outer-placeholder renderer behind the same scene interface. Record a measured
decision in ADR-001. Do not silently simplify transparency, stacking, or clipping
to make the demo pass. Do not invest in two production renderers before one passes.

### 6.4 Geometry and clipping algorithm

Use explicit coordinate types: pane cells, screen/history rows, client cells,
source pixels, and outer pixels. Use half-open rectangles internally. Convert
protocol indexing only at ingress/egress. Borders and status rows are not part of
the pane content rectangle.

For every visible placement:

1. Resolve the anchor using the pane's current screen/history transform.
2. Compute its intended destination rectangle `D` in client pixels.
3. Intersect `D` with pane content, client viewport, and visible screen region.
4. Subtract opaque Ekko UI rectangles, producing disjoint visible fragments.
5. Map each fragment back through the original destination-to-source transform.
6. Apply documented integer rounding once; use exact raster fallback if necessary.
7. Emit owned fragments or no placement when the visible set is empty.

For a horizontal source interval `[sx, sx+sw)` and destination
`[dx, dx+dw)`, a destination coordinate `u` maps to
`sx + (u-dx)*sw/dw`; do the analogous calculation vertically. Keep rational/fixed
precision until the final pixel operation. Test non-integral scale ratios.

Clamp all dimensions and multiplications before allocation. Negative offsets
must be clipped, not converted to enormous unsigned values. An image with an
extreme layer value still cannot cover another pane or Ekko chrome. Pane isolation
is enforced geometrically, not by hoping a sufficiently large z-index hides it.

Define one ordering for text backgrounds, below-text images, text glyphs,
above-text images, and UI, calibrated against the pinned host. Bound fragmentation
and cache growth. If a pathological scene exceeds limits, degrade that placement
with a diagnostic; never escape clipping to reduce work.

### 6.5 Lifecycle, history, and reconstruction

Separate removing visible placements from freeing image data. Resolve every
child deletion selector against that child's logical state, then reconcile the
affected client fragments. Never emit an outer global delete in response to a
child request. Own-ID cleanup is also required on detach and shutdown.

Handle image replacement, placement reuse, screen changes, erasure, scrolling,
history eviction, pane destruction, and daemon quota eviction as explicit events.
Do not assume that erasing text has identical effects on native placements and
placeholder references. Derive behavior from the pinned protocol and fixtures.

Anchor scrolling images to screen/history records, not permanently to terminal
row numbers. History remains bounded and retains referenced assets under quota.
When history eviction removes the last reference, reclaim the asset. Copy mode
is a view transform; it must not mutate the application's live screen.

Every client attachment starts with an empty presentation cache and its own ID
mapping. Rebuild visible assets and placements from daemon state. On reconnect,
remove known stale owned IDs when possible, use a new namespace generation, and
never assume a surviving outer cache is trustworthy. Outer uploads are disposable
cache population, not the source of truth.

### 6.6 Replies and support queries

The daemon is the terminal seen by the application. It answers child graphics
operations after logical validation/commit, according to the supported virtual
contract. A logical success means accepted into Ekko's virtual terminal, not a
promise that a disconnected viewer displayed the image.

Outer query/upload responses belong to the client's own transactions and update
presentation health. They never become child keystrokes and never get routed by
current focus. Honor child response suppression and identifiers at the child
boundary. Avoid duplicate acknowledgements from both daemon and host.

Pending outer replies carry request kind, outer identity, attachment generation,
deadline, and expected response shape. Unknown/late replies are bounded diagnostics.
Probe before ordinary input where feasible; do not assume the terminal input
stream provides cryptographic provenance for escape-shaped user input. Resolve
ambiguous response-shaped bytes conservatively with documented matching rules.

Use a fake host to test delayed, reordered, missing, duplicate, and stale replies.
An outer storage failure invalidates the client's cache and triggers bounded
retry/fallback; it does not erase accepted daemon assets. Publish the behavior
when no client is attached or the host lacks graphics.

### 6.7 Transfer adapters and less common operations

Direct payload transport is the baseline and the only required outer transport.
Local file/temp-file/shared-memory requests are handled where the child lives,
then normalized to owned data; never send a child filesystem path to the viewer.
Adapters validate size/offset, object type, access policy, and ownership before
read or cleanup. Follow normative transfer cleanup rules; do not invent blanket
unlink behavior. Error paths need the same cleanup testing as success.

Disable local adapters for future remote/untrusted ingress by default. If a
safe adapter policy intentionally rejects an otherwise valid request, report the
limitation and a proper failure; do not silently read arbitrary files or claim
full local-transfer conformance. No credentials or image payloads in default logs.

Incoming placeholder references require semantic decoding and reconstruction
tests, including identifier reuse, incomplete references, colored ordinary text,
scrolling, and deletion. Do not rewrite every foreground color as an image ID.

Relative placement, if enabled, uses a pane-owned dependency graph with bounded
depth, cycle checks, missing-parent errors, and transitive invalidation. Animation,
if promoted, needs bounded frame storage, a daemon timeline, deterministic clock
tests, attach catch-up, and frame composition; raw passthrough is not a shortcut.

### 6.8 Required capability ledger

Create `docs/compatibility.md` with one row per operation family and columns:
ingress semantics, emitted host operation, target-app need, test IDs, exact host
version, status, limitations. Include transport, encoding, compression, placement
forms, crop/scale/offset, layers, IDs/numbers, deletes, quiet responses, queries,
scroll/erase/reset behavior, relative placement, and animation. A family marked
supported must enumerate supported keys/values and malformed-input handling.

## 7. Input, geometry, and client behavior

Decode outer input into key/text/mouse/paste/focus events before applying bindings.
Encode unhandled events according to the focused pane's own terminal modes.
Implement [Kitty keyboard negotiation](https://sw.kovidgoyal.net/kitty/keyboard-protocol/)
as per-pane virtual state, separate from the client's host negotiation. Legacy
fallback must work; enhanced events must not leak into an application that did
not request them. Paste contents must not execute mux commands.

Mouse hit testing maps client coordinates to a pane's content coordinates.
Chrome interactions belong to the UI. A drag captures its destination at press
time until release/cancel; crossing a pane boundary must not split one gesture
across applications. Define pixel-mouse behavior when metrics are unavailable.

P2 has one interactive geometry owner per session. Additional clients are
read-only mirrors with their own presentation caches. Reject a second writer
unless the user explicitly transfers the lease. Smaller mirrors clip/pan the
canonical view; they do not resize application PTYs. Different pixel metrics use
each viewer's rendering transform while the child's reported geometry follows
the owner. Releasing the owner preserves the last canonical geometry.

Resize sequence: receive metrics -> commit layout generation -> set PTY sizes ->
enqueue redraw -> accept application redraw against the new size. Coalesce resize
bursts without dropping the final size. Reject zero-sized PTYs; at tiny terminal
sizes present a minimum-size view while preserving valid child dimensions.

Client entry/exit owns raw mode, alternate screen, mouse/focus modes, keyboard
negotiation, cursor, title policy, and owned graphics. Restore state in unwind and
signal paths where possible. SIGKILL cannot run cleanup: provide a tested
`ekko doctor --restore-terminal` recovery command instead of promising otherwise.

## 8. Extensibility and customization

### 8.1 Public contract

Start with a small versioned API: component registration, command registration,
binding contributions, layout proposals, UI scene contributions, dependency
subscriptions, and host actions. Document the actual public exports on one screen.
Do not expose raw terminal writes, arbitrary host objects, or private package
symbols. Built-ins import only the public API plus general utility libraries.

Sketch, not a pre-existing API or requirement to build a macro language:

```lisp
(register-component
  :id :example-status
  :api-version 1
  :reads '(:session/focus :pane/title)
  :handler #'example-status-handler)

;; Handler receives detached data and returns a bounded action list.
(defun example-status-handler (snapshot event)
  (declare (ignore event))
  (list (status-contribution :key :focused-title
                             :text (focused-title snapshot))))
```

Resolve accessors against the declared dependency set. A read outside it is a
diagnostic failure, not an invisible subscription. Events contain data; no callback
closures smuggle host references. Validate action schema, ownership, revision,
capability, count, and byte size before applying an atomic logical batch.

### 8.2 Reversible components

One host-owned context contains registries and named resource contributions.
Mount yields an owner token; every committed contribution/resource carries it.
Prefer owner-tagged overlays and reconstruction over naive restoration of a stale
snapshot. Removing A's keybinding contribution must not erase a later B binding.

On unmount: stop new dispatch, invalidate generation, remove subscriptions,
cancel jobs/timers, remove contributions, dispose explicitly owned resources,
and rebuild affected context. Late worker completions are ignored. Dependency
notifications go to exactly the matching resolved consumers after a commit.
Define deterministic ordering and detect reactive update loops with a budget.

Preserved state is named: user-created sessions/panes and application VT/image
state survive removal of a status/layout/keybinding component. Ephemeral preview
panes explicitly owned by that component are disposed. User actions triggered
through a component are not all secretly component-owned reversible effects.

Test mount -> exercise every effect -> unmount -> context/resource diff, and
mount A -> mount B -> modify common key -> unmount A -> B remains correct.
Include failed mount, failed reload, timeout, and double-unmount idempotence.

### 8.3 Configuration and reload

Use declarative data for defaults: keys, colors, layout parameters, workspace argv,
and policies. Provide a schema, unknown-key errors, source locations, and resolved
configuration inspection. Lisp configuration is an explicit trusted code path,
never silently loaded from the current project directory.

Precedence: built-in defaults < user config < named workspace profile < explicit
CLI options < documented session overrides. Reload validates a candidate before
swapping contributions; failed reload preserves the active configuration and
reports the offending source. Keep optional trusted code out of the daemon's
mutable state domain. Worker isolation is a fault boundary, not an OS sandbox:
same-user arbitrary Lisp can still access files and processes.

P2 examples, each using only public API: an alternate layout, a custom command
with binding, and a status component with dependency updates. At least one is
packaged as a separate ASDF system outside `ekko/builtins` to detect private API
coupling. Replacing defaults must not require editing the core.

Bare-build gate: omit `ekko/builtins`, start the daemon, create a PTY through core
RPC, and attach a single rectangular viewport. This is a kernel primitive; tabs,
status, command palette, and default key policy are not required in the bare build.

## 9. IPC, automation, observability

Use a local versioned framed protocol. Start with a length-prefixed envelope and
JSON metadata, plus separately bounded binary asset frames. Do not deserialize
arbitrary Lisp forms from sockets. Specify byte order, maximum lengths, unknown
message handling, request IDs, sequence numbers, and connection role.

Maintain one integer wire revision, incremented for every wire change, and an
explicit compatibility table: negotiate an understood revision for additive
changes; reject incompatible peers before decoding normal messages. Version the
extension API separately. Old scene deltas must not apply to a different base.

The daemon sends snapshots/deltas with base and target revisions, asset handles,
and release notifications. Clients request missing assets and acknowledge applied
scene generations. A slow viewer may lose intermediate presentation deltas and
receive a new snapshot; it must not lose authoritative application events. Bound
each queue and prevent image traffic from starving command responses.

CLI target surface (implement progressively; document exact syntax):

```text
ekko new --session work -- <argv...>
ekko attach --session work
ekko split --session work --axis horizontal -- <argv...>
ekko list --json
ekko command <registered-command> --json <arguments>
ekko inspect --session work --json
ekko config check
ekko config reload
ekko doctor --json
ekko doctor --restore-terminal
```

Queries produce requested data; successful mutations are quiet unless output is
explicitly requested. Errors go to stderr with meaningful exit status. Attached
UI output belongs exclusively to the renderer, never ordinary logging streams.

Inspection exposes pane/process state, geometry, terminal modes, logical graphics
counts/bytes, pending uploads, client mappings, cache failures, queue occupancy,
component ownership, subscriptions, and recent typed errors. Redact payloads,
input, paths, environment secrets, and tokens by default. Opt-in capture files
have bounded size and restrictive permissions.

Use deterministic replay fixtures containing ordered pane-output chunks, input
events, geometry events, worker completions, and clock advances. Replay validates
state and planned output without a real TTY. Do not build a persistent event-sourced
database merely to support this test facility.

## 10. SBCL, dependencies, and reproducible builds

Use ASDF for systems and build/test entry points; see the
[ASDF manual](https://asdf.common-lisp.dev/asdf/). Produce a native-launchable SBCL
executable using the supported saved-image facility described in the
[SBCL manual](https://sbcl.org/manual/index.html#Saving-a-Core-Image). Create the
image before opening runtime FDs, worker processes, threads, or user configuration.
Initialize them at executable startup. A saved executable is not automatically
statically linked; package every runtime library and resource it actually needs.

Required future repository shape (create files when their milestone needs them):

```text
GOAL.md                 this contract
README.md               build, launch, defaults, honest support status
PROGRESS.md             active ticket and evidence, not a second specification
flake.nix / flake.lock  pinned build, dev shell, checks, runnable app
ekko.asd                explicit systems and dependency direction
src/                    small modules following section 3
native/                 only necessary OS shim/launcher
tests/{unit,property,integration,fixtures}/
scripts/                build/test/demo entry points
terminfo/               advertised child capability definition
examples/               workspaces and independent extensions
docs/{architecture.md,compatibility.md,protocol.md,extension-api.md}
docs/adr/               short decisions with evidence
docs/evidence/          machine-readable reports and visual-run references
```

Pin nixpkgs, SBCL, Lisp libraries, native libraries, Unicode data, and host test
versions. Use an existing pinned Nix Lisp package set or explicit fixed-source
derivations; do not download dependencies during compilation or first launch.
Evaluate one library per need: JSON, base64, compression, image decode, test runner.
Prefer existing adequate packages; record license, pin, maintenance evidence,
build proof, and a narrow adapter for foreign dependencies. Do not spend a phase
writing a package manager, generic FFI framework, or custom image codec.

Required commands when implemented:

```sh
nix build
nix flake check
nix run . -- --help
nix run .#test-integration
nix run .#test-kitty
nix run .#benchmark
```

`nix flake check` includes compilation, meaningful unit/property tests, PTY/IPC
integration, terminfo validation, packaging smoke tests, and bare-build tests.
Real Kitty tests run in an explicit GUI-capable integration job using a pinned
host and software rendering/headless display where reliable. They must never
silently skip and return “all passed.” If the environment cannot run them, return
a distinct unavailable result and leave the visual release gate open.

A clean build must be exercised with empty HOME/user caches and startup files
disabled, then run from a directory outside the checkout. Include installed
terminfo, examples/help resources, and native runtime libraries in the closure.
Test the same artifact distributed to users. Native ASDF/SBCL build instructions
are a contributor fallback; CI evidence comes from Nix commands.

## 11. Test program and concrete oracles

### 11.1 Testing layers

| Layer | Oracle | Catches |
| --- | --- | --- |
| Pure unit | Small expected state transitions | Parser, modes, geometry, ownership |
| Property/replay | Partition invariance and reconstruction equivalence | Boundary bugs, stale generations, leaks |
| PTY integration | Actual child observations and process status | Job control, resize, hangs, routing |
| Fake host | Independent restricted terminal/graphics receiver | Invalid outer streams, ID collisions, replies |
| Real Kitty | Calibrated screenshot/pixel regions and live protocol probes | Actual clipping, layering, cache behavior |
| Real apps | Recorded revisions and scripted workspace scenario | Unsupported assumptions in synthetic fixtures |

Do not test an encoder solely with its own decoder. Use independent fixtures,
known pixels, and a deliberately small host oracle. Real host acceptance cannot
be replaced with the fake host. Compare image interiors with stable pixel masks;
do not make font antialiasing a source of flaky full-screen screenshot equality.

### 11.2 Mandatory fixtures

| Test | Scenario and required observation |
| --- | --- |
| G01 collision | A uploads red, B blue, both reuse image 7/placement 1; each pane keeps its color. |
| G02 scoped deletion | A issues every supported broad and targeted delete; B's asset and visible pixels are unchanged. |
| G03 chunk boundaries | Split each small fixture at every byte; random partitions for large ones; identical final state/replies. |
| G04 concurrent transfer | Alternate A/B reads and force short outer writes; fake host sees only valid complete upload ordering. |
| G05 replies | Switch focus, delay/reorder host responses, close/recreate pane; no wrong PTY receives bytes. |
| G06 clipping | Oversized checkerboard crosses all pane edges at fractional scales; sentinel border/neighbor pixels never change. |
| G07 movement | Resize, swap, zoom, hide/show panes, and pan mirrors; compare with a fresh full reconstruction. |
| G08 scrolling | Full/partial scroll regions, history entry/eviction, and copy-mode scroll; anchors and references stay valid. |
| G09 screens | Enter/leave alternate screen and reset; check text and graphics against isolated reference behavior. |
| G10 placeholders | Move/copy/erase placeholder cells and reuse IDs; intended images move, ordinary colored text remains text. |
| G11 upload race | Resize, reset, quota-fail, or kill pane mid-upload; no stale placement, leaked pending bytes, or wrong cursor. |
| G12 reattach | Kill client, continue child updates detached, attach fresh client; current image scene is restored. |
| G13 formats | RGB/RGBA/PNG, compression, malformed/truncated input, decode expansion bomb; bounded behavior and truthful replies. |
| G14 local transport | Allowed file/temp/shared-memory, invalid ranges/types, denied access, cleanup failures; no unrelated file mutation. |
| G15 layering | Transparency over text, below-text images, overlay opening/closing; correct compositing and no UI coverage. |
| G16 cache failure | Host forgets image or rejects upload; daemon state survives, client retries/falls back without a retry storm. |
| G17 identity edges | Zero/omitted/max identifiers, number lookup, reuse/replacement, allocator exhaustion; no accidental alias. |
| G18 quota | One pane floods uploads; another still accepts input, quotas reclaim resources, accepted live assets follow policy. |
| T01 PTY | Interactive shell job control, signals, size query, exec failure, exit and reaping. |
| T02 input | Different key/mouse/paste modes in two panes; events encoded for the destination only. |
| T03 text | Wide/combining text at edges, invalid UTF-8, margins, erase/insert/delete, terminfo-driven programs. |
| E01 composition | Mount/exercise/unmount every component effect; compare context and resource ownership. |
| E02 dependencies | Change each declared key; exactly its consumers run; undeclared read fails. |
| E03 containment | Infinite-loop/throwing/oversized-output extension; worker terminated, no partial commit, PTYs continue. |
| E04 reload | Invalid replacement and concurrent owner contributions; old valid behavior persists. |
| B01 packaging | Clean Nix build and installed executable outside checkout with empty user caches. |
| B02 bare | No built-ins; daemon, PTY RPC and single viewport work. |
| R01 recovery | Slow/disconnected client, daemon shutdown, terminal recovery; bounded queues and owned cleanup. |

Generate property cases with fixed seeds and persist minimized failures. For
fragmentation tests, enumerate all single split points on short inputs and include
one-byte feeds; do not attempt exponentially many partitions of huge images.
Assertions inspect semantic state, observable PTY bytes, visible regions, resource
counts, and allocated-byte trends. “Did not crash” is insufficient.

### 11.3 Real flagship acceptance script

1. Build the pinned artifact and record versions, display backend, and cell metrics.
2. Launch browser and Slack apps in separate PTYs, using documented argv.
3. Display distinct images in both, then refresh them repeatedly and concurrently.
4. Scroll each independently; resize divider and outer window; swap and zoom panes.
5. Open command UI over an image; close it and check exact restoration.
6. Switch main/alternate screens where apps support it; exercise keyboard/mouse/paste.
7. Detach, allow new content, reattach; images and process identities survive.
8. Terminate one app; the other remains visually and interactively correct.
9. Save a short recording plus fixture/report references and all remaining limitations.

Never commit private Slack content, login tokens, or browser secrets. If application
access is unavailable, save the launch/integration harness and report that exact
gate as blocked. Continue all independent synthetic, protocol, build, and extension
work; do not label synthetic producers as the real app result.

## 12. Resource limits and performance

Initial defaults are engineering starting points, not measured claims:

| Resource | Initial bound/policy |
| --- | --- |
| Decoded single image | 32 MiB and maximum dimension 8192; validate products before allocation |
| Daemon graphics | 128 MiB/pane, 512 MiB global including pending and derived assets |
| Scrollback | 10,000 rows/pane plus an explicit byte cap |
| Non-image control string | 64 KiB; small independent header/parameter limits |
| IPC metadata frame | 1 MiB; separate bounded binary asset chunks |
| Queued presentation data | 8 MiB/client before resnapshot/disconnect policy |
| Extension dispatch | 50 ms wall deadline, bounded actions/output, bounded worker memory |
| Idle incomplete upload | Configurable 10 s inactivity deadline; release pending resources |

Account input buffers, compressed data, decoded data, derived images, and retained
history; avoid counting only the final asset. Deduplication may come later after
profiling and must retain per-owner logical accounting. Under pressure, evict
derived/presentation caches first. Reject new allocations or apply a documented
logical eviction policy; never silently destroy arbitrary live content.

Benchmark through the real packaged binary with fixed fixtures: eight text panes,
two panes each replacing a 1 MiB image at 5 Hz, repeated resize, and 100 attach/
detach cycles. Record hardware, SBCL/host versions, RSS, GC pauses, CPU, bytes
written, input-to-PTY latency, and frame latency separately.

Initial targets: p95 input dispatch below 20 ms and p99 below 50 ms under that
workload; static idle CPU below 1% of one core; no monotonic retained-resource
growth over repeated teardown cycles. These are targets to measure on a named
reference machine, not portable CI timing assertions. Correctness gates cannot
be waived for speed. Optimize only after recording the baseline and a profile.

## 13. Ordered implementation milestones

Each milestone ends with code, tests, documentation deltas, and an evidence entry.
Keep tickets to one behavior and usually a handful of files. Do not introduce
unneeded stubs for all future packages. A milestone is not complete if its named
tests are skipped, TODO, mocked at the boundary under test, or only described.

### M00 — Build spine and protocol ledger

Create ASDF/Nix executable, help/version, test runner, clean build check, and
PROGRESS.md. Pin source revisions and dependencies. Create the initial capability
ledger and ADR-000 scope. Identify actual target projects when available, without
reading Ekko. Exit: `nix build`, `nix flake check`, help smoke test succeed.

### M01 — Graphics risk experiment, before general UI

Create tiny fixture generators and a minimal Kitty output harness. Implement the
geometry transform, outer ID allocator, transaction writer, and red/blue collision
experiment. Exercise edges, transparency, overlays, and replay. Record ADR-001
native versus placeholder egress and exact evidence. Exit: P0 visible proof and
G01/G04/G06/G15 focused tests. This harness can use synthetic scene data; label it
as such and integrate it into the actual PTY path in M04.

### M02 — Correct PTYs and daemon lifetime

Implement platform adapter, spawn decision, daemon socket/startup, pane IDs,
process/reaping lifecycle, framed RPC, and one client. Exit: T01 and client death
without child death, with descriptor ownership checks. Do not build a full VT
engine inside the PTY adapter.

### M03 — Text virtual terminal and compositor

Implement parser, VT baseline, terminfo, one rectangular viewport, then two-pane
layout through the first narrow policy interface. Input and queries are virtual.
Exit: T02/T03, malformed-sequence isolation, partition properties, and real shell
plus a terminfo-driven full-screen application. Do not postpone all graphics
until after dozens of text-only features; use the M01 harness continuously.

### M04 — Real graphics path

Connect PTY APC parsing -> logical assets/placements -> scene -> client renderer.
Implement namespace isolation, ingress assembly, direct formats/compression,
logical replies, scoped deletion, and asynchronous decode ownership. Exit:
G01–G06, G11, G13, G17 through actual child PTYs, including independent fake host
and real Kitty evidence. This is the first genuine mux graphics milestone.

### M05 — Geometry, history, placeholders, replay

Implement viewport changes, image/text damage, screen/history lifecycle, incoming
placeholders, overlay clipping, detach reconstruction, and outer-cache failure.
Exit: G07–G12/G15/G16 and incremental/full reconstruction equivalence. P1 may be
declared only with a published capability table and identified omissions.

### M06 — Compatibility and actual applications

Add local transfer adapters, any required wrappers, and protocol features shown
by actual app traces. Add relative placements/animation only if promoted by the
target-app gate or as a separately tracked P3 task. Exit: G14, all P2 required
operation rows, and flagship script evidence. App-specific workarounds belong
in documented profiles only when a standards-correct solution is impossible.

### M07 — Complete extension lifecycle and customization

Turn the proven policy seams into the versioned public API; implement worker
dispatch, owner contributions, reactive dependencies, config reload, and separate
example extension packages. M03's first layout seam should grow here rather than
being discarded for a speculative framework. Exit: E01–E04 and B02. Keybindings,
layouts, theme, command UI, status, workspace recipes use that same mechanism.

### M08 — Hardening and release proof

Finish quotas/fairness, mirrors/lease rules, malformed-input properties, slow client
recovery, independent packaging, diagnostics, docs, and benchmarks. Exit: all P2
gates, B01/B02/R01/G18, clean Nix validation and real-app/host evidence. Publish
an explicit limitations list. Stop adding features until this gate is complete.

### M09 — Optional compatibility expansion

Animation and relative placements beyond target needs; other Kitty-capable hosts;
nested mux operation; remote transport; macOS; daemon crash recovery. Each needs
an ADR, capability rows, explicit acceptance tests, and an independent milestone.
No optional item can be used to defer a mandatory P2 correctness fix.

## 14. Execution protocol for an implementation model

At the start of each work session:

1. Read applicable instructions, this document's active sections, and PROGRESS.md.
2. Inspect only `ekko_v2` and relevant pinned upstream references; avoid sibling Ekko.
3. Select the earliest incomplete ticket whose prerequisites pass.
4. Name its expected observable behavior and failing/missing test.
5. Implement the smallest coherent change and run the appropriate Nix checks.
6. Update progress with exact commands, exit status, test IDs, and remaining failures.

If a task exceeds a useful context window, split it at a behavior boundary and
write a resumption note. Do not keep redesigning the whole system. Do not create
a second competing specification. Do not claim a test ran because its source
looks correct, or claim graphics fidelity because a bytestring matches itself.

Suggested `PROGRESS.md` format:

```text
Active milestone/ticket:
Required outcome:
Prerequisites and current baseline:
Files/contracts touched:
Evidence: command, exit status, test IDs, artifact path
Known failures and minimized reproducer:
Decisions made (ADR links):
Next smallest action:
External blockers (scope and independent work remaining):
```

When uncertain about protocol details, consult the pinned authority and create a
small probe. When uncertain about architecture, prefer the existing boundary and
smallest implementation. Ask the user only for a consequential product decision,
credentials/access, or an unavailable target identity that cannot be established.
Do not pause for routine library/API choices or permission already granted.

Do not download/run target applications with private accounts just to obtain a
visual fixture. Prepare the concrete integration harness first; credentials and
external access remain the user's input when needed.

### Short handoff prompt

> Implement GOAL.md in order, starting from the earliest uncompleted milestone.
> Work only in ekko_v2; do not read sibling Ekko. Use SBCL and reproducible Nix
> builds. Preserve the isolation and extension invariants. Maintain PROGRESS.md
> with exact verification evidence. Finish bounded vertical slices, not broad
> scaffolding. The project is not complete until the P2 release checklist passes;
> report unavailable external gates honestly and continue independent work.

## 15. P2 release checklist

- [ ] Fresh pinned Nix build, checks, packaged launch, and native fallback docs.
- [ ] Real PTYs, interactive job control, session daemon, detach/reconnect.
- [ ] Advertised VT/terminfo behavior and per-pane input modes verified.
- [ ] Two simultaneous independent graphics applications with colliding IDs.
- [ ] Required graphics capability ledger complete with test evidence.
- [ ] Native and placeholder ingress, clipping, layering, lifecycle, replay.
- [ ] Child/host replies isolated, stale generations and partial writes tested.
- [ ] File/temp/shared-memory adapters tested under their documented policies.
- [ ] Independent real Kitty oracle and actual browser + Slack scenario recorded.
- [ ] Public extension API, reversible mount/unmount, dependency and timeout tests.
- [ ] Defaults replaceable; separate extension examples; no-builtins CI gate.
- [ ] Resource limits, fairness, cleanup, slow client, and recovery tests pass.
- [ ] Headless inspection, machine-readable commands, useful errors, private logs.
- [ ] Benchmarks recorded; unmet targets and compatibility limits disclosed.
- [ ] No disabled mandatory tests, fake successful integrations, or passthrough bypass.

## 16. Sources and decision records

Research consulted on 2026-09-04. Links are authorities to pin during M00, not
claims that this unimplemented project has been tested against them. Record exact
revision/hash and section references in the compatibility ledger; upstream pages
can change. Avoid copying whole documentation into the repository unnecessarily.

- [Kitty Graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/): wire semantics.
- [Kitty keyboard protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/): enhanced input.
- [Xterm control sequences](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html): VT reference.
- [SBCL manual](https://sbcl.org/manual/index.html): runtime, processes, executable images.
- [ASDF manual](https://asdf.common-lisp.dev/asdf/): system/build organization.
- [Linux PTY routines](https://www.man7.org/linux/man-pages/man3/forkpty.3.html): OS mechanics.

ADRs to write as evidence becomes available: 000 scope/capabilities, 001 graphics
egress and clipping, 002 PTY spawn boundary, 003 image decoder/dependencies,
004 extension worker/ownership, 005 IPC compatibility and geometry lease.
Keep each short: decision, smaller alternative rejected and why, evidence,
consequences, reversal conditions. Do not write speculative ADRs to fill the list.
