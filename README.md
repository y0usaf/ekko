# ekko

An **extension-first** terminal multiplexer with zellij-class robustness and
an unboxed chrome themed from the host terminal's own colors —
project/session sidebar, one fullscreen terminal, a single statusbar row — in
front of a detachable session daemon.

Every stock feature (sidebar, statusbar, command mode, keybinds, theme,
resurrection manifests, ...) is implemented as an extension in `ekko-builtins`
through the public `ekko-ext` API — see `DESIGN.md` for The Rule and the
extension surface contract. Building with `--no-default-features` yields the
bare harness: attach, raw key passthrough, fullscreen grid, nothing else.

## Design

- **Client/server**: a daemon owns the PTYs and per-session `vt100` state;
  sessions survive client exit. Clients attach over a versioned Unix socket
  (`$XDG_RUNTIME_DIR/ekko/wire_vN/<session>`); incompatible builds never find
  each other's sockets.
- **Structured frames, client-side chrome**: the server streams cell-grid
  updates for the watched session; the client composites with a
  damage-tracked cell surface + diffed ANSI writer. All chrome is drawn by
  surface extensions that claim docked regions of the frame.
- **Core as mechanism, extensions as policy**: both the client and the
  daemon host an extension runtime. Extensions register commands, keybinds,
  modes, surfaces, overlays, themes, and lifecycle-event handlers; they read
  immutable snapshots and write only through returned actions, so the render
  loop never blocks on an extension.
- **Terminal fidelity**: server-side scrollback with a client scroll mode
  and wheel scrolling, drag-select + OSC 52 copy, mouse passthrough to
  mouse-aware TUIs (SGR and legacy encodings), bracketed paste re-wrapped
  for the child, window-title and OSC 52 clipboard passthrough, DECSCUSR
  cursor shapes, focus reporting (mode 1004), grapheme clusters (combining
  marks, ZWJ emoji), and italic rendering.
- **Robustness**: per-child reaper threads (SIGTERM escalation, no zombies),
  panic hooks routed onto the server's message bus, dedicated PTY reader and
  writer threads with byte-capped backpressure in both directions, debounced
  SIGWINCH, slow-client eviction, and session resurrection manifests
  (`~/.cache/ekko/...`).

## Workspace

| Crate | Responsibility |
|---|---|
| `ekko` | CLI binary: `attach`, `new`, `ls`, `kill`, hidden `--server` mode |
| `ekko-event` | Extension event vocabulary (`EventKind`, `EventReturn`, `UiAction`) |
| `ekko-ext` | Public extension API: registries, runtime, dispatch, `DrawContext`, dock layout |
| `ekko-builtins` | **All stock features**, registered through `ekko-ext` like any extension |
| `ekko-proto` | Wire messages, bincode framing, versioned socket paths |
| `ekko-pty` | PTY spawn (openpty + login_tty), reaper, non-blocking writes |
| `ekko-server` | Daemon: hub, session actors, pty writer, extension host |
| `ekko-resurrection` | Session-manifest I/O library (used by the resurrection builtin and `ekko ls`) |
| `ekko-client` | Attach client: event loop, snapshot building, extension host, action interpreter |
| `ekko-keycast` | Keystroke display for screencasts (`:keycast`) — a non-builtin extension in its own crate |
| `ekko-lua` | Lua scripting bridge: `~/.config/ekko/extensions/*.lua` become extensions, with instruction budgets and buffered draw ops |
| `ekko-grid` | Cell surface, damage tracking, diffed ANSI renderer (from phi-grid) |
| `ekko-tui` | Raw mode, terminal caps, cell-width/spinner primitives (from phi-tui / pi-harness) |
| `ekko-config` | `~/.config/ekko/config.toml` (`[keybinds]`, `[extensions] disabled`) |

`ref/` holds local checkouts of zellij, phi, and pi-harness used as design
references; it is not part of the build.

## Usage

```sh
ekko                  # start + attach a fresh session in the current directory
ekko new [name]       # create + attach a session (named, or auto-named)
ekko attach <name>    # attach; respawns from a resurrection manifest if needed
ekko ls               # list live + resurrectable sessions
ekko kill <name>      # kill a session
```

Unnamed sessions are named by the registered session-namer extension; the
stock policy is the tilde-abbreviated working directory plus a random word
pair — `~/Dev/ekko polished-lemur` — so `ekko ls` and `EKKO_SESSION_NAME`
read like places, and the sidebar's project grouping (by parent directory
of each session's cwd) stays orthogonal to display names. A user extension
(Rust or Lua `register_session_namer`) replaces the scheme wholesale; the
host still sanitizes, uniquifies, and falls back to `session-<hex>` if no
namer is registered.

Inside, navigation lives on the alt layer (nothing steals the control bytes
your shell depends on):

| Keys | Action |
|---|---|
| `alt+j` / `alt+k` (or `alt+↓`/`alt+↑`) | next / prev session |
| `alt+h` / `alt+l` (or `alt+←`/`alt+→`) | prev / next project |
| `alt+n` | new session |
| `alt+x` | kill session (lands on a neighbor) |
| `alt+e` | command mode (`:q`, `:detach`, `:new [name]`, `:switch <name>`, `:kill`, `:help`, `:keycast`) |
| `alt+s` | scroll mode (`j`/`k` line, `u`/`d` half page, PgUp/PgDn page, `g` top, `G` live, `q`/Esc exit) |
| `alt+/` | help overlay |
| `ctrl+space` | leader: a which-key panel of every `mode = "leader"` binding (`e` command mode, `s` scroll, `n` new session, `d` detach, `?` help) |
| `ctrl+q` | detach |

The mouse wheel scrolls history directly (arrow keys on the alternate
screen); dragging with the left button selects text and copies it to the
system clipboard via OSC 52 on release. When the program inside requests
mouse tracking, mouse events are forwarded to it instead.

The statusbar shows the live chord set, so the defaults are always on
screen. All keybinds are configurable under `[keybinds]` in the config
(`ctrl+<letter>`, `ctrl+space`, `alt+<char>`, arrow-key chords, and — for
mode-scoped bindings like the leader map — bare printables and `space`; one
action can take a list of chords). Leader entries rebind as
`"leader.<action>" = "<key>"`, the chord itself as `leader = "..."`.

## Lua extensions

Scripts in `~/.config/ekko/extensions/*.lua` load as extensions with the same
standing as the builtins (duplicate names fail the build loudly; broken
scripts are skipped with a logged warning). A script returns its manifest
plus a `register(ekko)` function:

```lua
local ext = { id = "user.hello", name = "hello", version = "0.1.0" }

function ext.register(ekko)
  ekko.register_command({
    name = "hello",
    description = "say hello",
    handler = function(args)
      return { { set_status_note = { text = "hello " .. args, kind = "ok" } } }
    end,
  })
  ekko.register_surface({
    name = "hello-bar", dock = "top", size = 1,
    draw = function(ctx, snapshot)
      ctx.put_text(0, 0, 40, "accent", "surface", "session: " .. snapshot.session_name)
    end,
  })
  ekko.subscribe("bell", function(payload) end)
end

return ext
```

`ekko.register_command` / `register_keybinding` / `register_surface` /
`register_overlay` / `register_theme` / `subscribe` are bridged; every
callback runs under an instruction budget, and draw calls are buffered ops
replayed only if the callback returns cleanly — a runaway script errors out
instead of wedging the terminal.

A keybinding registered with `mode = "leader"` becomes a leader-map entry:
the host dispatches it while the leader mode is active, and the which-key
panel lists it automatically. `examples/leader-map.lua` shows leaf entries
(return `"exit_mode"` ahead of the action) and a sticky, repeatable one.
