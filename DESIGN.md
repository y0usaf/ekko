# ekko — Design

## Pane layout

The daemon's `[ui].pane_layout` is `manual` by default, retaining the BSP
split geometry. `equal` resolves composite pane counts to the divisor-pair grid whose cells are closest to square in pixels, counting cell height twice; prime counts of five or more use one near-half proportional cut whose halves are themselves grids. Greedy per-level halving was replaced because it could flip the cut axis during recursion and produce T-junctions.

ekko is a terminal multiplexer built extension-first: a small core of
mechanism exposing a public extension API, with **all** stock behavior
implemented through that API. The approach follows two sibling projects —
phi (`phi-ext`/`phi-builtins`) and takhti (core-as-mechanism, dogfooded
`wm.lua`) — and exists so that the API is proven by the product itself.

## Locked decisions

- **2026-07-02 — The Rule:** If a feature can be an extension, it must be an
  extension. Built-in features use the same public extension API as user
  extensions and live in `ekko-builtins`. The extension API's test suite is
  `ekko-builtins` itself; when a builtin cannot be expressed, the API grows
  rather than being bypassed. Builtins register first and duplicate names are
  hard errors. There is no privileged path: a user extension can replace any
  builtin by disabling it and registering its own.
- **2026-07-02 — Bare harness:** The acceptance criterion, checked by
  `nix flake check`, is that building without `ekko-builtins`, `ekko-keycast`,
  and `ekko-lua` leaves a bare-but-functional harness: attach, raw key
  passthrough, full-screen grid, and a minimal fallback palette. If removing
  builtins breaks core, core was cheating.
- **2026-07-02 — Snapshot reads, action writes:** Extensions never see `&mut`
  host state. They read an immutable `ClientSnapshot` (or event payload) made
  before each entry point, and return `UiAction`/`EventReturn` values which
  the host applies afterward. `ekko-client/src/actions.rs::apply_ui_action`
  is the single write path. Builtin actions are closed and grow first-class
  variants when needed; user extensions use `UiAction::Custom { name, payload }`,
  interpreted through `ExtensionHost::register_action_interpreter`. Unknown
  names become an error note, and resolved actions use the same
  recursion-bounded interpreter.
- **2026-07-02 — Bounded dispatch:** `AppRuntime::dispatch` chooses the
  mechanism by event class. Hot notifications (including per-frame
  `GridUpdated`) use persistent per-handler workers with bounded mailboxes of
  64; a full mailbox drops the event with a log line. Gates and one-shot
  lifecycle events use thread-per-dispatch with 500ms gate and 2s lifecycle
  timeouts, logging and continuing after errors and timeouts. ekko is
  synchronous end to end and uses no tokio.
- **2026-07-02 — Hot-path exception:** surface, overlay, and mode `draw`
  closures run directly and unguarded during rendering. They are trusted
  in-process Rust and write cells only. The `ekko-lua` bridge instead applies
  instruction budgets to every callback and buffers data-only draw operations,
  replaying them after a clean return; native extensions pay no bridge cost.
- **2026-07-02 — Extension state:** Extensions own state such as sidebar
  scroll in `Arc<Mutex<..>>` captured by closures; modes and overlays use
  host-stored type-erased state (`Box<dyn Any>`) created per activation.
- **2026-07-02 — Wire and event boundaries:** `ekko-proto` is deliberately
  independent of `ekko-event`: the wire contract changes rarely while the
  event vocabulary grows with builtins. The hub and client event loop are the
  only translation points. Every wire change bumps `WIRE_VERSION`; additive
  changes preserve old clients and breaking changes reject them clearly,
  never silently misparsing.
- **2026-07-02 — WS8 API lesson:** `ekko-keycast` lives outside builtins and
  depends only on `ekko-ext`, proving that a public extension can be separate.
  Its real gap grew the API with dynamic `SurfaceSpec::visible` and generic
  `OverlaySpec::build_payload`, removing the host's `OVERLAY_HELP` special
  case.
- **2026-07-13 — Pane ownership:** The daemon owns the pane set: stable
  `PaneId`s, one PTY/parser and bounded reader/writer path per pane, a binary
  split tree, and focus per attached client. Detach preserves all panes; the
  client owns neither canonical topology nor PTY state. Core owns pane
  mechanism and `ekko-builtins` owns stock commands, chords, and presentation.
  Pane operations enter through public `UiAction`s and are bridged to Lua,
  never through a builtin-only hub path.
- **2026-07-13 — Pane snapshot and canvas:** The wire carries complete pane
  metadata/topology, sparse or full grids, and each client's focused pane.
  Client state is a discardable composition, hit-testing, and selection cache.
  The daemon sizes one canonical session canvas to the smallest attached
  terminal-pane area; viewers share geometry but may focus independently.
  Input routes through the client's focus and mouse hits name a target pane.
  A bare session still has one full-canvas pane, raw passthrough,
  detach/attach, and correct rendering, but no stock pane-creation gesture.
  Splits are explicit 50/50 right/down splits with a minimum child size;
  invalid splits do not mutate topology. New panes start the configured shell
  in the session cwd; inheriting a foreground process cwd is deferred because
  it requires platform-specific process inspection.

- **2026-07-13 — Dependency replacement boundary:** A dependency is replaced only when our replacement is smaller than the API surface we used from it, or when std already provides the exact API. Fewer crates alone does not justify more of our code: that loses to the prime directive of writing less code. This clearly removed interprocess because `std::os::unix::net` is the same API and ekko is Unix-only; tempfile because it was test-only and six crates bought one `mkdtemp`; nix and close_fds because libc was already a dependency and the fd-close path must be async-signal-safe, with unconditional fallback on any `close_range` failure so seccomp EPERM cannot leak every PTY master and the session socket into a child; daemonize because a documented double fork is about 50 lines; and itoa because it lived inside vendored vt100, which we already own. The rule stops at signal-hook: the daemon registers two independent Signals instances for SIGINT/SIGTERM (its signal thread and one reaper thread per PTY), exactly the fan-out provided by signal-hook-registry; libc::sigaction would silently replace the first registration, while a correct self-pipe, fan-out, and SA_RESTART implementation is 150–300 lines of unsafe global state, so three crates are not worth it. thiserror was removed only barely—the hand-written impls cost 59 lines for two crates—and is the marginal case calibrating the rule. mlua stays because Lua scripting is the product: `vendored` statically compiles Lua 5.4 from C source, costing 16 build-time crates but no runtime library dependencies; `ldd` shows only libc, libm, and libgcc_s. Safe `Lua::new()` calls `disable_c_modules()`, so user scripts cannot dlopen native modules and the interpreter cannot reintroduce a runtime dependency. serde/bincode stay because hand-rolling the wire format is more code than it deletes. unicode-width stays because correctness depends on a generated Unicode table, not code.

- **2026-08-01 — TOML configuration removal:** `config.toml` support and the `toml` dependency are removed because `init.lua` superseded TOML; the replacement is nothing, not more code, consistent with the 2026-07-13 dependency replacement boundary. Reversing this requires a demonstrated need for TOML plus a smaller complete implementation than the dependency and its integration.

- **2026-08-03 — ASCII border glyphs:** Pane separator glyphs can be replaced by an optional three-character table (`ui.border_glyphs = { horizontal, vertical, junction }`) that collapses the box-drawing table to one glyph per line direction plus one junction glyph for every corner/tee/cross. The setting is client-local rendering only — the daemon still owns separator-cell reservation and announces the border *style* over the wire, but the glyph table never touches the wire or a client snapshot, so no `WIRE_VERSION` change and no extension-API change.

The extension boundary for [[canon:functional-core]] is the public trait
surface in `ekko-ext`; `ekko-client/src/actions.rs::apply_ui_action` is the
single host write path. The daemon state required by
[[canon:daemon-thin-client]] is the session's pane topology, pane PTYs and
parsers, split geometry, and per-session workspace state; it survives client
detach and reattach.

The wire's current workspace contract carries complete pane metadata and
client focus. Since `WIRE_VERSION` 8, `ServerToClient::Workspace` carries the
pane projection and per-pane grids while `ClientToServer` carries split,
focus, and close requests. Version 9 added `WorkspaceUpdate.border_style`:
pane separation is session-level configuration (`compact` reserves shared
separator cells; `frame` draws a ring), and clients draw zellij-style
junction-resolved box glyphs from `ekko-client/src/borders.rs`. Version 10
added scrollback search/dump (`SearchScrollback`/`DumpScrollback` to
`SearchResults`/`ScrollbackDump`) in absolute history rows and
`GridUpdate.history` for the scroll indicator. Version 11 added out-of-band
`Inject` and `DumpSession` scripting verbs for `ekko send` and `ekko dump`;
they need no attach or TTY and act on the session's primary pane.

## Architecture

| Crate | Role | Core or policy |
|---|---|---|
| `ekko-event` | Event vocabulary (`EventKind`/`EventPayload`/`EventReturn`, `UiAction`) shared by both hosts | core (vocabulary) |
| `ekko-ext` | Extension API traits, registries, runtime, draw context, dock resolver | core (decision-making mechanism) |
| `ekko-builtins` | Sidebar, statusbar, command mode, commands, keybindings, help, theme, spinner, sessions, resurrection, spawn hooks | **policy** |
| `ekko-client` | Attach host, threads, socket glue, snapshots, single action application, draw adapter | core (machinery) |
| `ekko-server` | Session daemon hub, PTYs, render tick, hook dispatch | core (machinery) |
| `ekko-proto` | Stable wire framing, sockets, and messages | core (machinery) |
| `ekko-grid` | Cells, damage tracking, ANSI diff renderer | core (machinery) |
| `vt100` | Vendored parser plus scrollback accessors | vendored machinery |
| `ekko-tui` | Raw mode, color probing, cell widths, spinner math | core (machinery) |
| `ekko-pty` | PTY spawn, I/O, and reaping | core (machinery) |
| `ekko-tmp` | Zero-dependency temp-directory helper used only by tests | **machinery** |
| `ekko-paths` | XDG/env path resolution for disk and socket roots | core (policy boundary) |
| `ekko-config` | Config schema, Lua cascade policy, binding strings, shared border vocabulary | core (policy) |
| `ekko-resurrection` | Manifest I/O used by resurrection and `ekko ls` | core (machinery) |
| `ekko-keycast` | Keystroke display extension | **policy** |
| `ekko-lua` | Lua-to-Extension bridge, instruction budgets, buffered draw ops, config evaluation | core (bridge machinery) |

Both client and server are separate processes with their own `AppRuntime`
built by the same `RuntimeBuilder`; each dispatches only its event subset.

## Deferred

- **Raw PTY output chunks:** up to 64KB per read inside the 16ms render
  budget is the daemon's hottest path. If observation is wanted, hook the
  already-coalesced `GridUpdate` through a persistent worker with a bounded,
  non-blocking mailbox, not thread-per-dispatch.
- **Server-side Key/Paste interception:** every keystroke would pay a
  dispatch round trip, so the client owns input policy.
- **Lua hot reload:** both processes evaluate scripts once per runtime build;
  the client rereads on next attach and the daemon on next session. `ekko kill`
  plus resurrection is the reload path. A manifest `host` field selects
  `client` (default), `server`, or `both`; `both` means independent Lua
  states sharing nothing.
- **Foreground-process cwd inheritance:** it requires platform-specific
  process inspection rather than pane mechanism.
- **Floating panes, tabs, stacks, pane rename/move/zoom, layout files,
  synchronized input, and exact pane-topology restoration after daemon death:**
  these are follow-ups to the deliberately smaller tiled-pane vertical slice.
- **Dependency alternatives:** signal-hook stays for independent signal fan-out
  (reconsider only if the daemon has one safe registration design); serde_json
  stays rather than hand-rolling JSON (reconsider only if its use becomes a
  smaller API surface); unicode-width stays for its
  generated Unicode table (reconsider only with an equally correct generated
  replacement); vendoring vte was rejected because it is not smaller code
  (reconsider only if we own or need to change the parser).

## Roadmap

Each phase has a criterion that can be checked without treating this document
as a status ledger:

- **WS0 — event/ext foundation:** the workspace exposes the event vocabulary,
  extension traits, and synchronous bounded dispatch.
- **WS1 — surfaces and dock:** sidebar and statusbar are extensions registered
  through the public API and the dock resolver lays them out.
- **WS2 — commands:** command registration and command mode operate through
  the extension API.
- **WS3 — keybindings:** default chords are registry entries, not host paths.
- **WS4 — session and mouse:** session grouping and surface-routed mouse input
  reach extensions through snapshots and actions.
- **WS5 — help/theme/spinner:** help is generated from live registries and
  theme and spinner are builtins using the same surface API.
- **WS6 — server hooks and resurrection:** lifecycle, PTY, bell, and heartbeat
  hooks dispatch through the runtime; resurrection and `.ekko-env` are
  builtins and notices cross the wire.
- **WS7 — bare harness:** `nix flake check` compiles the workspace binary with
  `--no-default-features`.
- **WS8 — keycast:** a separate public-API extension can provide keystroke
  display without a builtin-only dependency.
- **WS9 — Lua bridge:** extension scripts load through `Extension`/
  `ExtensionHost`, with instruction budgets and buffered draw operations.
- **WS10 — terminal fidelity:** scrollback, selection, mouse/paste/OSC
  behavior, wide cells, cursor/focus reporting, and PTY backpressure are
  represented and rendered correctly.
- **WS11 — leader and which-key:** mode-scoped registry bindings dispatch
  before mode handlers and the panel presents their scopes.
- **WS-P — tiled panes:** daemon-owned pane topology supports split, directional
  focus, close, canonical geometry, workspace transport, and bare one-pane
  behavior; invalid splits are rejected without mutation.
- **WS-PB — pane borders:** configured session-level border style is carried
  over the wire and clients render compact and framed separation correctly.
- **WS-SB — scrollback UX:** search and dump return absolute history matches
  and dumps, while clients expose the scroll indicator from grid history.
