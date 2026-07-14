# PLAN — Lua everywhere (complete) → tiled panes (next)

Goal: everything in ekko that is user-configurable is configurable from Lua.
Two concrete end states:

1. **Registry parity** — every `ExtensionHost` method is reachable from a
   `.lua` script, in both processes. A user can disable any builtin and
   replace it wholesale with a script.
2. **Config is Lua** — `~/.config/ekko/init.lua` supersedes `config.toml`
   as the settings source (shell, scrollback, sidebar width, keybind
   overrides, disabled extensions).

The irreducible non-Lua core (explicit non-goals, by construction):

- Wire protocol shape and `WIRE_VERSION` (`ekko-proto/src/socket.rs:39`) —
  a contract between binaries of one build, not configuration.
- Socket discovery scheme (`ekko-proto/src/socket.rs:55-93`) — client and
  server must agree before any config is evaluated; `$EKKO_SOCKET_DIR`
  stays the escape hatch.
- The path to `init.lua` itself and the instruction budget guarding its
  evaluation (config can raise budgets, but not the budget under which
  config is read).
- CLI subcommands (`crates/ekko/src/main.rs`) — parsed before a runtime
  exists.
- Raw PTY output bytes — deferred for perf reasons (DESIGN.md "Deferred
  hooks"), orthogonal to this plan. Extensions get `GridUpdated`.

Sequencing: **WS-A → WS-B → WS-C → WS-D**. A is pure parity work with no
design decisions left; B unlocks the server half of the event vocabulary;
C wants B done first (both processes then already link `ekko-lua`); D is
opportunistic follow-up. Each workstream lands independently; `nix build`
and `nix flake check` green after each (doctrine 7).

## Where we are

- [x] A1 `ekko.register_mode` — landed (dialect parser + cursor return in
      `convert.rs`; `examples/scroll-mode.lua` + builtin-parity bridge test
      cover the A-acceptance mode items).
- [x] A2 `ekko.register_spinner` — landed (pure-data walk; empty frames is
      a registration error, interval defaults to 80ms).
- [x] A3 `ekko.register_session_grouper` — landed (name-rehydration +
      trailing "ungrouped" group; error → `ekko_ext::fallback_group`, which
      was promoted from a private ekko-client fn so bridge and client share
      the no-grouper shape). All 10 `ExtensionHost` registries now bridged.
- [x] A4 `ctx.render_scrollbar` draw op — landed (table-form call, one
      spec table per plan; `track`/`thumb` glyphs optional, defaulting to
      `"│"`/`"█"` — note the sidebar builtin's thumb is `"┃"` U+2503, so a
      pixel-faithful sidebar clone passes `thumb = "┃"` explicitly).
      `DrawContext` is now fully bridged.
- [x] A5 surface `Scaled` size + `hide_below` — landed (`size` accepts an
      integer → `Fixed` or a `{ preferred, min, fraction, min_remaining }`
      table → `Scaled`; `preferred` is required, the rest default to layout
      no-ops (0/1/0); `hide_below = { cols =, rows = }`, missing fields → 0).
      **WS-A is complete** — full client registry + draw parity; README's
      Lua section now lists the whole bridged surface.
- [x] B1 `host` manifest field — landed (`"client"` default / `"server"` /
      `"both"`; unknown value or non-string is a load error — a server hook
      silently loading client-side would be a correctness bug;
      `load_extensions(dir, HostKind)` filters, client passes
      `HostKind::Client`).
- [x] B2 server wiring — landed (`lua` feature in ekko-server's defaults,
      mirroring the client; `build_runtime` loads `HostKind::Server`
      scripts after builtins).
- [x] B5 restart-story docs — landed alongside B1/B2 (README `host` +
      reload paragraph; DESIGN.md deferred-hooks list rewritten: the stale
      "modes/spinners/grouper not bridged" and "server-side Lua deferred"
      bullets replaced by the hot-reload caveat).
- [x] B3 payload marshaling audit — landed. Audit result: no gaps to fill —
      `payload_table`'s match is already exhaustive with no wildcard (all 17
      `EventPayload` variants render; a new variant is a compile error until
      it does), and `event_return_from_value` covers all 4 `EventReturn`
      shapes. The "unrenderable → explicit nil" contingency was never
      needed. What was missing was pinning: bridge.rs gains a table-driven
      test dispatching every payload variant through a real subscription
      (handler echoes the payload as a sorted `key=value` notice; also pins
      `exit_code = nil` when `None`, and that every `EventKind::ALL` name is
      subscribable), plus a `spawn_override` return test (shell/cwd/env) —
      `EmitNotice`/`PtySpawnOverride` previously had zero test coverage.
- [x] B4 failure-containment tests — landed, no production code needed
      (dispatch timeouts + budgets already bound both failures). bridge.rs
      pins: a runaway `BeforePtySpawn` gate budget-aborts and degrades to
      no-override while a later gate in the *same* script still answers
      (budget abort releases the lock cleanly); and the one hole budgets
      can't cover — a single C call (pathological pattern backtracking)
      never yields to the instruction hook, outlives the dispatch timeout,
      and abandons the script's lock — is contained: later callbacks into
      that script block on the lock, hit their own dispatch timeout, and
      are skipped, while other scripts (separate Lua states) answer
      normally.
- [x] B-acceptance — landed. `examples/spawn-hook.lua` (`host = "server"`)
      gates `BeforePtySpawn` (env stamp from the payload) and, because
      `SessionCreated` fires before any client is attached (hub dispatches
      it before `attached.insert` — a notice there is dropped), stashes the
      payload and surfaces it as a `Notice` on `ClientAttached`. The seam
      test (`crates/ekko-server/tests/extensions.rs`) loads the example
      through `load_extensions(_, HostKind::Server)` from a real extensions
      dir (a coexisting `host = "client"` script is asserted not to load),
      then asserts the injected env var in real shell output and the notice
      over the wire (`ServerNotice.source` is the handler *label*,
      `"id:event"`, not the bare manifest id). **WS-B is complete.**
      Gotcha: the flake's `src = self` excludes untracked files — a test
      referencing a new `examples/*.lua` passes locally but fails
      `nix build` until the file is at least staged.
- [x] WS-C — landed in one slice (C1+C2+C3+acceptance). `ekko-lua` gains a
      `config` module: `load_config(path)` evaluates `init.lua` under
      `HANDLER_BUDGET` in a throwaway state and deserializes the returned
      table straight into `Config` via mlua's `serialize` feature (untagged
      `Keybind` round-trips fine), then runs `normalize()` (now `pub`);
      `load_config_cascade()` (+ `load_config_cascade_in(dir)`, the test
      seam) is called by both processes' `run()` behind `#[cfg(feature =
      "lua")]`. Broken `init.lua` is a hard error (no TOML fall-through);
      broken TOML still degrades to defaults (now with a warning) as
      before. Unknown top-level keys warn; unknown *nested* keys are
      silently ignored by serde — accepted. `ekko.config` is a serialized
      copy set on the collector's ekko table (`LuaExtension::set_config`;
      `load_extensions` grew a `&Config` param — all call sites updated).
      `crates/ekko-lua/tests/config.rs` (9 tests) covers round-trip,
      normalize, unknown keys, hard errors + budget, cascade precedence,
      no-fall-through, `ekko.config` (custom + default), and the
      end-to-end disable-and-replace (init.lua disables
      `ekko-builtins.scroll-mode`, the example script re-registers
      "scroll"; build success proves it, since duplicate names are hard
      errors — needed `ekko-builtins` as a dev-dep of `ekko-lua`).
      **WS-C is complete** — README `## Configuration` section + crate
      rows and DESIGN.md crate map updated.
- [x] WS-D — landed. `Config` gains a `lua` section (`LuaLimits`:
      `draw_budget` / `handler_budget`, defaults = the old constants, now
      `ekko_config::LUA_*_BUDGET_DEFAULT`; `normalize()` repairs 0 → default
      since a zero budget would abort every callback instantly). The bridge
      reads budgets from the extension's config at register time (closures
      capture the plain `u32`s); the sole remaining constant is
      `BOOTSTRAP_BUDGET` for the two paths no config can govern — evaluating
      `init.lua` itself and a script's top-level chunk (both run before any
      config attaches; `register()` runs after `set_config`, so it *is*
      config-governed). D-acceptance pinned in `bridge.rs::
      config_raises_the_instruction_budgets`: one 3M-iteration loop fails a
      command handler and draws nothing under `Config::default()`, then
      completes on both paths once config raises the budgets — proving the
      defaults still bind *and* the raise reaches the callbacks. Config
      round-trip + zero-repair covered in the config tests; `"lua"` added to
      the known-top-level-keys warn list.

**All four workstreams are complete.** The answer to "what can't Lua
configure?" is now exactly the irreducible list at the top of this file.
Remaining known wart: the pre-existing ekko-client clippy warning noted
below (non-gating).

Tree note: `crates/ekko-lua/tests/which_key_real.rs` (pinned to a local
`~/.config/ekko` path), `examples/window-frame.lua`, and `.claude/` are
deliberately-untracked local scratch — not part of this plan's work; a
dirty-looking `git status` showing only these is fine. (`result` is now
gitignored; `nix build` no longer dirties the tree.) A pre-existing
clippy warning in ekko-client (`items_after_test_module`, lib.rs ~287)
predates WS-C and does not gate the flake — ignore or fix as a drive-by.

---

## WS-A — Bridge parity (client-side registries)

Close the gap between `ExtensionHost`'s 10 methods
(`crates/ekko-ext/src/traits.rs:20-51`) and the 7 registries the bridge
walks (`crates/ekko-lua/src/lib.rs:212-232`). Missing: `register_mode`,
`register_spinner`, `register_session_grouper` — plus three smaller holes
in already-bridged surfaces/draw.

### A1. `ekko.register_mode` (the big one)

`ModeSpec` (`crates/ekko-ext/src/mode.rs:41-46`) is structurally the
overlay pattern the bridge already implements at
`crates/ekko-lua/src/lib.rs:436-606`: type-erased per-activation state +
key handler + render. Reuse all of it.

```lua
ekko.register_mode({
  name = "jump",
  init = function() return { input = "" } end,          -- optional; default {}
  on_key = function(state, bytes, snapshot)
    if bytes == "\027" then return "exit" end
    state.input = state.input .. bytes
    return nil                                           -- Continue
    -- or: { scroll = -1 }            → ContinueWith(actions)
    -- or: { "exit", { switch_session = state.input } }  → ExitWith(actions)
  end,
  render = function(ctx, state, snapshot)                -- optional
    ctx.put_text(0, ctx.size().rows - 1, "jump: " .. state.input, "fg", "bg")
    return { row = ctx.size().rows - 1, col = 6 + #state.input }  -- cursor, or nil
  end,
})
```

Implementation:

- `crates/ekko-lua/src/lib.rs` — add `modes = {}` + `register_mode` to the
  collector chunk (lines 185-202), a walk loop (after line 221), and
  `fn register_mode(...)` modeled on `register_overlay`:
  - State: `struct LuaModeState(RegistryKey)` mirroring `LuaOverlayState`
    (line 460); `ModeInitFn` stores the Lua value in the registry. Note
    `ModeKeyFn` takes `&mut ModeState` — the Lua value in the registry is
    interior-mutable from Lua's side, so mutation "just works" the same
    way overlay state does; the `&mut` is only threaded for native
    extensions.
  - `on_key` marshaling → `ModeOutcome` (`mode.rs:28-38`), conventions
    matching the overlay `handle_key` dialect (lib.rs:528-573):
    `nil`/no-return → `Continue`; string `"exit"` → `Exit`; array with
    `"exit"` head → `ExitWith(actions)`; any other action table/array →
    `ContinueWith(actions)`. Parse via the existing `actions_from_value`
    (`convert.rs:270-335`).
  - `render` under `DRAW_BUDGET` with buffered `DrawOp` replay, exactly
    like the overlay render fn (lib.rs:489-505). Return value: `nil` or
    `{ row =, col = }` → `Option<(i32, i32)>` hardware cursor.
  - `on_key` runs under `HANDLER_BUDGET`; on error, log + `Exit` (a broken
    mode must not trap input — same philosophy as overlay's
    error-→-`Close`, lib.rs:523-526).
- Entering the mode already works from Lua: `UiAction::EnterMode` is
  marshaled (`convert.rs`), and `KeybindingSpec.mode` lets scripts bind
  chords *into* the new mode's scope.
- Update the stale doc comment at `crates/ekko-lua/src/lib.rs:36-39`
  ("Modes, spinners, and the session grouper are not yet bridged").

### A2. `ekko.register_spinner`

`SpinnerSpec` (`crates/ekko-ext/src/visual.rs:149-153`) is pure data — no
callbacks, no budget needed:

```lua
ekko.register_spinner({ name = "dots", frames = { "⠋", "⠙", "⠹" }, interval_ms = 80 })
```

Collector entry + a ~20-line walk that builds
`SpinnerSpec { name, frames: Arc::new(vec), interval_ms }`.

### A3. `ekko.register_session_grouper`

`SessionGrouperSpec` (`crates/ekko-ext/src/snapshot.rs:83-88`):
`Vec<SessionEntry> -> Vec<ProjectGroup>`.

```lua
ekko.register_session_grouper({
  name = "by-basename",
  group = function(sessions)   -- array of { name, cwd, state, created_at_secs }
    -- return array of { name = "group", sessions = { "sess-a", "sess-b" } }
  end,
})
```

Marshaling decisions:

- In: array of tables from `SessionEntry` (`snapshot.rs:25-30`); `state`
  as a string (reuse whatever `snapshot_table` already emits for session
  state in `convert.rs`).
- Out: groups reference sessions **by name**; the bridge rehydrates
  against the input entries. Scripts cannot fabricate entries, and any
  input session not claimed by a group is appended to a trailing
  `"ungrouped"` group so a buggy script can't make sessions vanish from
  the sidebar. On error: log + return the flat single-group fallback
  (match whatever the host does when no grouper is registered).

### A4. `ctx.render_scrollbar` draw op

The one `DrawContext` method missing from the Lua ctx
(`crates/ekko-ext/src/draw.rs:116-123`). Add to
`crates/ekko-lua/src/draw.rs`:

- `DrawOp::Scrollbar { col, row, rows, visible_items, total_items, scroll_from_top, fg, bg, track_glyph, thumb_fg, thumb_glyph }`
- `ctx.render_scrollbar({ col =, row =, rows =, visible =, total =, from_top =, fg =, bg =, track = "│", thumb_fg =, thumb = "█" })`
  with color values resolved through the existing hex/role resolver
  (`convert.rs:387-420`); replay builds `ScrollbarModel`/`ScrollbarStyle`
  (`draw.rs:67-81`).

### A5. Surface spec parity: `SurfaceSize::Scaled` + `hide_below`

`register_surface` currently hard-codes `Fixed` size and `hide_below:
None` (`crates/ekko-lua/src/lib.rs:403-404`). Accept:

```lua
size = 4                                                        -- Fixed (unchanged)
size = { preferred = 30, min = 10, fraction = 3, min_remaining = 20 }  -- Scaled
hide_below = { cols = 80, rows = 10 },                          -- (min_frame_cols, min_remaining_rows)
```

Map the table form onto `SurfaceSize::Scaled`'s fields
(`crates/ekko-ext/src/surface.rs:28-33`); integer stays `Fixed`.

### A-acceptance

- `crates/ekko-lua/tests/bridge.rs`: one test per new registry —
  register from Lua source, drive the spec's closures, assert marshaling
  both directions (mode outcome dialect gets its own table-driven test).
- New `examples/scroll-mode.lua`: a working reimplementation of the
  scroll-mode builtin (`crates/ekko-builtins/src/scroll_mode.rs`) — the
  proof that a mode builtin is now replaceable from Lua via
  `[extensions] disabled` + same-name re-registration.
- After A, the only `ExtensionHost` capabilities not reachable from Lua
  are server-side dispatch (→ WS-B).

---

## WS-B — Server-side Lua

Lua scripts currently load only in the client
(`crates/ekko-client/src/lib.rs:113-118`); the server builds from builtins
alone (`crates/ekko-server/src/lib.rs:30-35`). DESIGN.md already blesses
this move: "the bridge terminates in the shared traits, so the daemon can
host scripts; only the client loads them today."

### B1. Host declaration on the script manifest

One mechanism, no second directory (doctrine 5): a script declares where
it runs.

```lua
local ext = {
  id = "user.envlog",
  host = "server",        -- "client" (default) | "server" | "both"
}
```

- `LuaExtension::from_source` (`crates/ekko-lua/src/lib.rs:100-135`)
  reads the optional `host` field; unknown values are a load error.
- `load_extensions(dir)` (lib.rs:150-169) grows a filter:
  `pub fn load_extensions(dir: &Path, host: HostKind) -> Vec<Box<dyn Extension>>`
  with `enum HostKind { Client, Server }`; `"both"` matches either.
  Update the client call site to pass `HostKind::Client`.
- `"both"` means two independent Lua states (one per process) — no shared
  state, ever. Document in the crate doc comment.

### B2. Server wiring

- `crates/ekko-server/Cargo.toml`: `ekko-lua` optional dep behind a `lua`
  feature, in `default` — mirror `crates/ekko-client/Cargo.toml`.
- `build_runtime` (`crates/ekko-server/src/lib.rs:30-35`): after builtins,
  `#[cfg(feature = "lua")] register_boxed_extensions(ekko_lua::load_extensions(&ekko_config::config_dir().join("extensions"), HostKind::Server))`.
  Builtins-first ordering is preserved automatically.

### B3. Payload marshaling audit

`convert.rs::payload_table` / `event_return_from_value` must cover the
server-dispatched kinds (`crates/ekko-event/src/lib.rs:64-78`):
`BeforePtySpawn` (gate — `PtySpawnOverride` is *already* marshaled per the
`EventReturn` support in `convert.rs:338-381`), `SessionCreated`,
`ClientAttached`, `ClientDetached`, `SessionExited`, `PtyResized`,
`Heartbeat`, plus server-side `Bell`. Audit `payload_table` against every
`EventPayload` variant and fill gaps; any variant it can't render should
become an explicit `nil`-payload rather than an error.

### B4. Failure containment (mostly free, verify it)

Dispatch already runs each handler on its own thread with per-kind
timeouts (`crates/ekko-ext/src/runtime.rs:29-46`), and every Lua callback
runs under `HANDLER_BUDGET`. Two things to verify with tests rather than
build:

- A Lua handler that blows its budget on `BeforePtySpawn` must degrade to
  "no override", not a failed spawn.
- A timed-out handler abandons a locked `SharedLua`; subsequent callbacks
  into the same script must fail cleanly (poisoned/held lock → logged
  error), not wedge the hub. This is the same exposure the client already
  accepts; the test just pins it for the daemon.

### B5. Restart story (documentation, not code)

The daemon evaluates scripts once at session start and outlives config
edits. Editing a server script takes effect on the next session; `ekko
kill` + resurrection is the reload path. Hot reload is explicitly out of
scope. Document in README + DESIGN.md "Deferred hooks" (remove the
client-only caveat, add the reload caveat).

### B-acceptance

- `crates/ekko-server/tests/extensions.rs` (the existing injection seam,
  line ~92) gains: a Lua `host = "server"` script subscribing to
  `SessionCreated` + gating `BeforePtySpawn` with a `PtySpawnOverride`
  (e.g. injected env var), asserted end-to-end through a real spawn.
- A `host = "client"` script is asserted *not* to load in the server and
  vice versa.
- New `examples/spawn-hook.lua` (`host = "server"`).

---

## WS-C — `init.lua` config

Replace the settings *source* with Lua while keeping `ekko-config`'s
`Config` struct as the internal representation. The contract "binding
strings live in config, binding meanings live in builtins" is preserved —
`init.lua` returns data, it does not register callbacks.

### C1. Shape

`~/.config/ekko/init.lua` evaluates to a table congruent with `Config`
(`crates/ekko-config/src/lib.rs:21-27`):

```lua
return {
  general = { default_shell = "/run/current-system/sw/bin/nu", scrollback_lines = 50000 },
  ui = { sidebar_width = 28 },
  keybinds = { detach = "ctrl+q", session_next = { "ctrl+j", "ctrl+down" } },
  extensions = { disabled = { "ekko-builtins.sidebar" } },
}
```

Being Lua, users get conditionals/env-dispatch for free; ekko only sees
the returned table.

### C2. Implementation

- Evaluation lives in `ekko-lua` (not `ekko-config` — the dumb store stays
  dumb and dependency-free): `pub fn load_config(path: &Path) -> Result<ekko_config::Config>`.
  - `crates/ekko-lua/Cargo.toml`: add `ekko-config.workspace = true` (no
    cycle: `ekko-config` depends on nothing internal) and mlua's
    `serialize` feature; deserialize the returned table straight into
    `Config` via `LuaSerdeExt` + serde, then run the existing
    `normalize()` (`ekko-config/src/lib.rs:144-148`). Unknown keys:
    warn, don't fail (config files outlive binaries).
  - Evaluate under `HANDLER_BUDGET` in a throwaway `Lua` state.
- Precedence, applied identically at both load sites
  (`crates/ekko-client/src/lib.rs` `run()` and
  `crates/ekko-server/src/lib.rs:44`): `init.lua` if present → else
  `config.toml` → else defaults. A broken `init.lua` is a **hard error
  with a clear message**, not a silent fall-through to TOML — silently
  ignoring the user's config is worse than refusing to start. Factor the
  cascade into one helper (behind `#[cfg(feature = "lua")]`, e.g.
  `ekko_lua::load_config_cascade()`), called by both processes.
- `[extensions] disabled` needs zero new plumbing: config is loaded
  before `RuntimeBuilder::build` in both processes already, so a
  Lua-produced disabled list flows through `with_disabled`
  (`crates/ekko-ext/src/builder.rs`) unchanged. **This closes the last
  gap in "replace any builtin from Lua"**: disable in `init.lua`,
  re-register from a script (modes/spinners/grouper included, post-A/B).
- Expose resolved config to scripts: add a read-only `ekko.config` table
  (serde-serialized `Config`) to the collector env
  (`crates/ekko-lua/src/lib.rs:185-202`). Requires threading
  `&Config` into `load_extensions` — the client/server call sites both
  have it in hand.

### C3. Docs & conformance

- DESIGN.md crate map: `ekko-config` row becomes "config schema +
  TOML/`init.lua` loading (eval in `ekko-lua`)"; note the divergence in
  the doctrine-conformance table if the "dumb TOML store" phrasing was
  load-bearing anywhere.
- README: `init.lua` reference section with the full schema.

### C-acceptance

- `crates/ekko-lua/tests/`: `init.lua` → `Config` round-trip (all four
  sections), unknown-key warning, budget-exceeded error, precedence over
  a coexisting `config.toml`.
- End-to-end: `init.lua` disabling a builtin + a script replacing it, per
  the A-acceptance scroll-mode example.

---

## WS-D — Promote constants (selective, follow-up)

Only promote what script authors will actually hit. First (and possibly
only) batch — the Lua budgets, since WS-A/B/C make them the binding
constraint on what scripts can do:

```lua
lua = { draw_budget = 200000, handler_budget = 2000000 },
```

- New `Config` section in `ekko-config`; consumed at
  `crates/ekko-lua/src/lib.rs:63-66` (constants become
  `Config`-sourced with the current values as defaults). The budget for
  evaluating `init.lua` itself stays the hard-coded default — the
  bootstrap exception.

Deliberately **not** promoted until someone needs them (each is one
config field away when that day comes): dispatch timeouts
(`ekko-ext/src/runtime.rs:29-46`), render tick / settle
(`ekko-server/src/hub.rs:34-41`), heartbeat interval (`hub.rs:44`), PTY
chunk/backpressure caps (`pty_io.rs`, `pty_writer.rs`). Constants are
fine as constants; an unused knob is just surface area.

---

## Risks / open decisions

| Risk | Position |
|---|---|
| Mode `on_key` dialect diverging from overlay `handle_key` dialect | Deliberately mirrored (`"exit"`/`"close"` string + array-head form). If a third dialect ever appears, extract a shared outcome parser in `convert.rs`. |
| Buggy server script degrades all sessions' daemon | Accepted: budgets + dispatch timeouts + log-and-continue already bound it; B4 pins it with tests. |
| `init.lua` and daemon lifetime skew (daemon started under old config) | Same skew TOML has today; no regression. Documented in B5. |
| mlua `serialize` feature pulling serde through the bridge | Small, vendored Lua already dominates; acceptable. |
| Grouper scripts dropping sessions | Prevented structurally (name-rehydration + trailing ungrouped group, A3). |

## Milestone summary

| # | Deliverable | Proof |
|---|---|---|
| A | Full client registry parity (`register_mode`/`register_spinner`/`register_session_grouper`, scrollbar op, Scaled/hide_below) | `examples/scroll-mode.lua` replaces the builtin; bridge tests |
| B | Scripts run in the daemon (`host = "server"`/`"both"`) | `examples/spawn-hook.lua` overrides a real PTY spawn in the seam test |
| C | `init.lua` supersedes `config.toml`; `ekko.config` readable from scripts | disable-and-replace a builtin entirely from Lua, no TOML present |
| D | Lua budgets configurable | budget raise observable in a bridge test |

After C, the answer to "what can't Lua configure?" is exactly the
irreducible list at the top of this file — the bootstrap contract and
nothing else.


---

## WS-P — Tiled pane MVP

Goal: cross the threshold from a detachable single terminal to a real terminal
multiplexer with the smallest coherent pane model. `DESIGN.md` “Pane MVP
contract” is locked for this workstream.

### MVP user journey

1. Start a session: one full-canvas shell, unchanged from today.
2. Split the focused pane right or down: a second independent shell starts.
3. Focus left/right/up/down: subsequent key, paste, scroll, cursor, and mouse
   behavior belongs to that pane.
4. Close the focused pane, or let its child exit: its sibling absorbs the space.
5. Detach/re-attach: all live panes and topology remain. Closing/exiting the last
   pane ends the session as the single PTY does today.

Explicitly deferred: floating panes, tabs, stacks, pane move/rename/zoom,
manual ratio resize, arbitrary layout files, synchronized input, exact topology
resurrection after daemon death, and live-process cwd inheritance. These do not
block the pane threshold.

### P1. Extract the one-pane actor — **serial**

- [ ] Introduce stable `PaneId` and a server-owned terminal-pane object that
      contains the current parser/callback state, VT compatibility filter, PTY
      handle, writer sender, backlog counter, title, render diff base, and
      force-full state.
- [ ] Tag PTY bytes/exits/panics with pane identity + generation so output from
      a retired pane cannot mutate a replacement.
- [ ] Move the hub from flattened singular fields to a one-entry pane map while
      preserving the current wire and visible behavior exactly.
- [ ] Prove: existing daemon tests stay green; pane exit reclaims writer/reader
      resources; stale output is ignored; byte caps remain per pane; one-pane
      detach/attach and no-builtins tests still pass.

This gate is intentionally behavior-preserving. Multi-pane state must not be
built on duplicated copies of the current flattened hub.

### P2. Add canonical topology + multi-PTY lifecycle — **serial after P1**

- [ ] Add a pure binary split tree (`Leaf(PaneId)` / horizontal or vertical
      split with ratio) and deterministic rect resolver. Split is transactional:
      reject dimensions below the minimum without spawning/leaking a child.
- [ ] Add pure mutations: split focused leaf right/down, remove leaf + promote
      sibling, and directional neighbor selection based on resolved geometry.
- [ ] Store focus per attached `ClientId`; attach chooses a valid pane, detach
      drops only that focus, and pane removal repairs every client's focus.
- [ ] Generalize spawn, resize, PTY input/replies, scrollback, bell/title/
      clipboard, dirty scheduling, diff bases, and exit cleanup to N panes.
      Last-pane exit retains current session-exit semantics.
- [ ] Prove topology invariants/property cases: every live pane appears exactly
      once, rects are non-overlapping/in-bounds, removal leaves no unary split,
      all focused IDs are live, and every successful split owns one fully wired
      PTY.

No client-facing controls yet; focused internal operations are exercised through
hub seams. This keeps topology/lifecycle failures separate from protocol and UI.

### P3. Carry pane workspaces over the wire — **serial after P2**

- [ ] Bump `WIRE_VERSION`. Replace the singular visual contract with a workspace
      update containing complete pane metadata (`PaneId`, rect, title), the
      receiving client's focused pane, and optional sparse/full `GridUpdate`
      data per pane. Complete metadata makes removal/topology recovery
      idempotent; grid payloads remain incremental.
- [ ] Add client→server split-right, split-down, focus-direction, focus-by-id,
      and close-focused requests. Requests from unattached clients and unknown
      IDs are ignored safely.
- [ ] Generalize slow-client recovery/coalescing: a dropped workspace frame
      forces full grids for every live pane on that client’s next update;
      coalescing may not lose a pane’s earlier row patches or newer topology.
- [ ] Replace client singular `GridState` with a pane-state map. Compose each
      pane grid at its server-provided rect inside the terminal region; show the
      hardware cursor/title/modes for only the focused pane.
- [ ] Route mouse hit-testing/selection to pane-local coordinates. Clicking a
      pane focuses it before forwarding; keyboard, paste, and scroll use server
      focus. Resize reports the client terminal-pane canvas and the daemon
      resizes every resolved child PTY.
- [ ] Prove round-trip/framing, workspace merge/resync, stale epoch rejection,
      multi-grid composition, independent client focus, and one-pane visual
      equivalence.

### P4. Expose stock pane management through the public API — **serial after P3**

- [ ] Add public `UiAction` forms for split right/down, focus direction, and
      close focused pane. `apply_ui_action` remains the client’s only action
      interpreter and translates these to wire requests.
- [ ] Bridge every new action through `ekko-lua` in the same action dialect and
      expose immutable pane metadata + focused ID in `ClientSnapshot` /
      `snapshot_table`.
- [ ] Add `ekko-builtins` commands and leader-map entries through ordinary
      command/keybinding registration. Suggested minimum: `:split right|down`,
      `:pane-focus <direction>`, `:pane-close`; leader `|`, `-`, `h/j/k/l`, `x`.
      Resolve collisions explicitly rather than granting pane policy priority.
- [ ] Grow status/help/which-key only from live registries/snapshot data. No
      pane-specific branch in core rendering or input.
- [ ] Prove disable-and-replace from a file-backed Lua extension and retain the
      no-builtins one-pane harness.

### P5. End-to-end pane acceptance — **serial after P4**

- [ ] A daemon seam test starts two distinguishable shells, verifies independent
      output, switches focus and routes input correctly, closes/exits either
      child, and confirms sibling expansion + last-pane session exit.
- [ ] Detach/re-attach preserves both live children, topology, scrollback, and
      valid focus. Two simultaneous clients may focus different panes without
      redirecting each other’s input.
- [ ] Terminal resize reaches every pane with its resolved dimensions; a
      resize/split flood stays bounded and converges to the latest geometry.
- [ ] Mouse-aware child, bracketed paste, selection/OSC52, title, cursor shape,
      focus reporting, and alternate-screen wheel behavior are pinned to the
      focused/hit pane rather than regressing to session-global state.
- [ ] `cargo fmt --check`, focused suites, `cargo test --workspace`,
      `nix build`, and `nix flake check` pass; the exact commands/outcomes are
      recorded before this workstream closes.

### Dependency order

`P1 → P2 → P3 → P4 → P5`. All are serial because each evolves the same hub,
wire, or action contract. Parallel waves become safe only after P3 if follow-up
features own disjoint consumers; do not invent concurrency inside the MVP.
