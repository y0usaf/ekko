# ekko — Design

ekko is a terminal multiplexer built extension-first: a small core of
mechanism exposing a public extension API, with **all** stock behavior
implemented through that API. The approach follows two sibling projects —
phi (`phi-ext`/`phi-builtins`) and takhti (core-as-mechanism, dogfooded
`wm.lua`) — and exists so that the API is proven by the product itself.

## The Rule

> If a feature can be an extension, it must be an extension. Built-in
> features are implemented through the same public extension API user
> extensions use, and live in `ekko-builtins`.

Corollaries:

- The extension API's test suite is `ekko-builtins` itself. If a built-in
  can't be expressed through the API, the fix is to **grow the API**, not to
  bypass it.
- Built-ins register first; duplicate names are hard errors. There is no
  privileged path — a user extension can replace any built-in wholesale by
  disabling it and registering its own.
- **Acceptance criterion** (CI-checked): building without `ekko-builtins`
  leaves a bare-but-functional harness — attach, raw key passthrough,
  full-screen grid, minimal fallback palette, nothing else. If deleting the
  builtins breaks core, core was cheating.

## Crate map

| Crate | Role | Core or policy |
|---|---|---|
| `ekko-event` | Event vocabulary (`EventKind`/`EventPayload`/`EventReturn`, `UiAction`) shared by both hosts | core (vocabulary) |
| `ekko-ext` | Extension API: `Extension`/`ExtensionHost` traits, registries, `RuntimeBuilder`/`AppRuntime`, `DrawContext`, dock layout resolver | core (mechanism) |
| `ekko-builtins` | Sidebar, statusbar, command mode, commands, keybindings, help overlay, theme, spinner, session grouping, session naming, resurrection, spawn hooks | **policy** |
| `ekko-client` | Attach client host: threads, socket glue, snapshot building, `apply_ui_action`, `DrawContext` adapter | core |
| `ekko-server` | Per-session daemon host: hub actor, PTY lifecycle, render tick, hook dispatch sites | core |
| `ekko-proto` | Wire contract (framing, socket, messages). Small and stable; independent of `ekko-event` | core |
| `ekko-grid` | Cell surface, damage tracking, optimizing ANSI diff renderer | core |
| `ekko-tui` | Terminal primitives: raw mode, color probing, cell-width math, spinner math | core |
| `ekko-pty` | PTY spawn/IO/reaping | core |
| `ekko-config` | Config schema + TOML parsing; the `init.lua` settings source that supersedes `config.toml` is evaluated in `ekko-lua` (the schema crate stays dependency-free). Holds binding *strings*; binding *meanings* live in builtins | core |
| `ekko-resurrection` | Manifest I/O library (used by the resurrection builtin and by `ekko ls`) | core (I/O) |
| `ekko-keycast` | Keystroke display (`:keycast`). The WS8 extension: lives outside the builtins, depends only on `ekko-ext` | **policy** |
| `ekko-lua` | Lua scripting bridge: `~/.config/ekko/extensions/*.lua` scripts become `Extension`s, guarded by instruction budgets + buffered draw ops; also evaluates `init.lua` into `ekko_config::Config` and exposes the resolved config to scripts as `ekko.config` | core (bridge) |

## Extension surface contract

- **Snapshot reads, action writes** (the takhti discipline): extensions
  never see `&mut` host state. Reads come from an immutable
  `ClientSnapshot` (or the event payload) built before each entry point;
  writes happen only through returned `UiAction`/`EventReturn` values,
  applied by the host after the callback returns. `apply_ui_action` in the
  client is the single write path.
- **Bounded dispatch**: `AppRuntime::dispatch` runs each subscribed handler
  on its own thread with a per-kind timeout (notifications 100ms, gates
  500ms, one-shot lifecycle 2s), logging and continuing past errors and
  timeouts. ekko is synchronous end to end — no tokio.
- **Hot-path exception**: surface/overlay/mode `draw` closures are called
  directly and unguarded on the render pass. They are trusted in-process
  Rust and write cells only. The `ekko-lua` bridge adds its own guard —
  instruction budgets on every callback, and draw calls buffered as
  data-only ops replayed after the Lua call returns cleanly — behind the
  same `DrawContext` trait; native extensions pay nothing for it.
- **Extension-owned state**: extensions hold their own state (e.g. sidebar
  scroll) via `Arc<Mutex<..>>` captured in their closures; modes and
  overlays get host-stored type-erased state (`Box<dyn Any>`) created per
  activation.
- Client and server each run their **own** `AppRuntime` (they are separate
  processes), built with the same `RuntimeBuilder`. `ekko-event` is one flat
  vocabulary; each side dispatches only its own subset.

## Wire protocol vs. event vocabulary

`ekko-proto` and `ekko-event` are deliberately independent. The wire contract
changes rarely and every change is a `WIRE_VERSION` bump; the event
vocabulary grows continuously with the builtins. The hub and the client
event loop are the only translation points (e.g. `EventReturn::EmitNotice`
→ `ServerToClient::Notice`).

## Deferred hooks (and why)

- **Raw PTY output chunks**: up to 64KB per read inside the 16ms render
  budget — the daemon's hottest path. If output observation is ever wanted,
  hook the already-coalesced `GridUpdate` via a persistent worker with a
  bounded, non-blocking mailbox (real queued-write-ops), not
  thread-per-dispatch.
- **Server-side Key/Paste interception**: every keystroke would pay a
  dispatch round trip; the client owns input policy.
- **Lua hot reload**: both processes evaluate scripts once at runtime
  build. The client re-reads them on the next attach; the daemon on the
  next session (`ekko kill` + resurrection is the reload path). A script
  declares where it runs via the manifest's `host` field (`"client"`
  default / `"server"` / `"both"` — the latter is two independent Lua
  states, one per process, sharing nothing).

## Workstreams

- [x] WS0 — `ekko-event` + `ekko-ext` scaffolding, sync dispatch, this document
- [x] WS1 — surfaces & dock layout; sidebar/statusbar as builtins
- [x] WS2 — command registry; command mode as a builtin
- [x] WS3 — keybinding registry; default chords as a builtin
- [x] WS4 — session grouper; surface-routed mouse input
- [x] WS5 — help overlay from live registries; theme + spinner as builtins
- [x] WS6 — server hooks (`BeforePtySpawn`, lifecycle, bell, heartbeat);
      resurrection as a builtin; `.ekko-env` spawn override; wire `Notice`;
      `WIRE_VERSION` 2
- [x] WS7 — bare-harness build (`--no-default-features` feature `builtins`)
      + integration test (`crates/ekko-server/tests/extensions.rs`)
- [x] WS8 — `ekko-keycast`: a keystroke display built purely against the
      public API from its own crate. Proved a real gap and grew the API for
      it: dynamic surface visibility (`SurfaceSpec::visible`), plus the
      generic overlay payload builder (`OverlaySpec::build_payload`) that
      removed the host's `OVERLAY_HELP` special case
- [x] WS9 — `ekko-lua`: scripts from `~/.config/ekko/extensions/*.lua`
      terminate in the same `Extension`/`ExtensionHost` traits (the phi-lua
      pattern); instruction budgets + buffered draw ops guard the render
      path (`crates/ekko-lua/tests/bridge.rs`)
- [x] WS10 — terminal fidelity + scrollback (`WIRE_VERSION` 4). Fixed the
      client's per-frame full repaint (`CellSurface::resize` same-size
      no-op); scroll mode as a builtin (grew the API:
      `ModeOutcome::ContinueWith`, `UiAction::Scroll`/`ScrollToBottom`,
      `ClientSnapshot::scrollback`) over server-side vt100 scrollback with
      wheel scrolling and a statusbar indicator; mouse passthrough to
      mouse-aware children (`TermModes` on every `GridUpdate`, SGR + legacy
      re-encoding); drag selection + OSC 52 copy (wired the dormant
      `ekko-grid::selection`); bracketed paste re-wrapped (and
      injection-stripped) server-side; OSC 0/2 title, child OSC 52,
      DECSCUSR cursor shape, and focus reporting (1004) passthrough;
      grapheme clusters + wide-cell spans + italic on the wire and in the
      renderer; read-direction PTY backpressure cap mirroring the writer's
- [x] WS11 — leader key + which-key panel as a builtin. Grew the mechanism:
      mode-scoped keybindings now *dispatch* (the client matches
      `match_keybinding(bytes, Some(mode))` before the active mode's
      `on_key`, so any extension extends a mode's vocabulary without owning
      the mode), `KeybindingInfo` carries its mode scope (statusbar hints
      filter to normal, help groups per mode, the panel renders its own
      scope), and the chord vocabulary gained `ctrl+space`, `space`, and
      case-sensitive bare printables for mode-scoped maps. The leader map
      itself is policy: registry entries under `mode = "leader"`
      (`crates/ekko-builtins/src/leader.rs`, user side
      `examples/leader-map.lua`)


## Pane MVP contract (next milestone)

Panes are the next core multiplexer mechanism. The minimum useful vertical
slice is deliberately smaller than Zellij: tiled terminal panes with split,
directional focus, and close. Floating panes, tabs, stacks, pane
rename/move/zoom, layout files, synchronized input, and restoring an exact pane
topology after daemon death are follow-ups.

Locked ownership and boundaries:

- **Daemon owns the pane set.** A session hub owns stable `PaneId`s, one PTY +
  parser + bounded reader/writer path per pane, a binary split tree, and focus
  per attached client. Detach must preserve all panes. The client never owns
  canonical topology or PTY state.
- **Core owns mechanism, builtins own product policy.** Core implements pane
  identity, split-tree mutation/geometry, directional neighbor selection,
  lifecycle cleanup, wire transport, and action application. `ekko-builtins`
  alone chooses stock commands/chords and presentation. Pane operations enter
  through public `UiAction`s and are bridged to Lua; no builtin-only hub path.
- **Snapshot in, actions out.** Client extensions read pane metadata from
  `ClientSnapshot` and request split/focus/close through returned actions. The
  client translates those actions to versioned wire requests; the hub is the
  single canonical write path.
- **Thin client.** The wire carries a workspace snapshot: complete pane
  metadata/topology projection each frame, sparse/full grid payloads per pane,
  and the receiving client's focused pane. Client state is a discardable cache
  used for composition, mouse hit-testing, and selection.
- **One canonical canvas.** As today, the daemon sizes a session to the
  smallest attached terminal-pane area. All viewers receive the same pane
  geometry, while each viewer may focus a different pane. Keyboard/paste/scroll
  route through that client's focus; a mouse hit names the target pane.
- **Bare behavior stays real.** Without builtins, a session still starts with
  one full-canvas pane, supports raw passthrough, detach/attach, and renders
  correctly. It has no stock gesture for creating more panes.

The initial split policy is an explicit 50/50 split of the focused leaf to the
right or downward, with a minimum viable child size; invalid splits are
rejected without mutating topology. New panes start the configured shell in the
session cwd. Inheriting the live foreground process cwd is deferred because it
requires platform-specific process inspection rather than pane mechanism.
