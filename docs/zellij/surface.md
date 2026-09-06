# Zellij 0.43.1 surface and Ekko discrepancy ledger

This is a source inventory for the pinned Zellij release, followed by the
current Ekko status. It is a discrepancy ledger, not a compatibility claim.
`Observed` means the checked-in differential harness exercised that behavior;
`Partial` means a bounded part is implemented and verified, with required gaps recorded;
`Implemented` means the Ekko sources or product documentation describe it;
`Unimplemented` means the pinned surface has no corresponding Ekko surface;
`Inventory` means the source was catalogued but Ekko behavior has not been
tested or mapped. A row marked `Inventory` must not be read as support.

## Evidence and scope

The reference is Zellij tag `v0.43.1`, packaged as `nixpkgs.zellij` and pinned
by [`tests/zellij/reference/pin.json`](../../tests/zellij/reference/pin.json).
The checked-in KDL files are hash checked by the runner:

* [`config.kdl`](../../tests/zellij/reference/config.kdl) is the reference
  configuration, including its explicit keybindings, plugin aliases, and
  option documentation.
* [`default.kdl`](../../tests/zellij/reference/default.kdl) is the three-pane
  default layout (one-cell tab bar, central terminal pane, one-cell status
  bar).
* The complete built-in defaults are [`assets/config/default.kdl`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/assets/config/default.kdl)
  and [`assets/layouts/default.kdl`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/assets/layouts/default.kdl).

The differential runner starts one real PTY fixture under each program,
dismisses Zellij release notes, sends an ordinary byte, locks and unlocks with
`Ctrl-g`, and compares the PTY input slice plus a `pyte` screen snapshot. See
[`tests/zellij/differential.py`](../../tests/zellij/differential.py), especially
the exercised sequence at lines 104–122. It explicitly records that only
Normal/Locked routing is exercised and that the screen model is incomplete.
Ekko's checked-in profile unregisters the defaults and registers `:normal`,
`:locked`, and `:pane`. Normal/Locked forward unbound input; Pane ignores it.
The partial Pane controls cover Ctrl-p entry/exit, Enter/Escape exit, Ctrl-g
locking, and `f` fullscreen followed by Normal. See
[`examples/profiles/zellij.lisp`](../../examples/profiles/zellij.lisp) and the
[custom two-pane observations](pane-observations.md). Remaining bindings and rendering variants remain required. Public decoration
spans now drive the profile frames: the custom 80×24 settled Pane scenario
matches modeled cells/cursor, while startup, small-terminal, and default-layout
differences remain. See [integration evidence](README.md#public-decoration-integration-2026-09-05).

The first stage exposes a concrete startup difference: a fresh Zellij default
configuration at 80×24 opens a floating 0.43.1 release-notes pane, and `Escape` dismisses
it; Ekko forwards that `0x1b` to the fixture. At 20×8, a reference probe did not
show the popup and forwarded Escape; startup behavior must be observed at each size. The harness retains this input
mismatch. Timing is also part of the observed surface: writing `Ctrl-g Ctrl-p b`
as one batch to Zellij can leave it in `Pane` mode with `Ctrl-p`/`b` absent from
the application input, while Ekko defers queued bytes behind the mode action.
The settled comparison therefore sends `Ctrl-g`, `Ctrl-p b`, `Ctrl-g`, and `c`
as separate stages and records both raw streams and cell snapshots. This is an
observed routing/timing discrepancy, not evidence of parity.

Pinned source links below use `v0.43.1`; the Nix source content hash protects
the executable reference against tag changes. The primary source
files are [`data.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/data.rs),
[`actions.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/input/actions.rs),
[`cli.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/cli.rs),
[`options.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/input/options.rs),
and [`plugin_command.proto`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/plugin_api/plugin_command.proto).

## Modes and keybindings

Zellij defines these 14 modes: `Normal`, `Locked`, `Resize`, `Pane`, `Tab`,
`Scroll`, `EnterSearch`, `Search`, `RenameTab`, `RenamePane`, `Session`, `Move`,
`Prompt`, and `Tmux`. `Normal` writes input except mode shortcuts; `Locked`
writes input and disables shortcuts except the unlock binding. The enum and
mode descriptions are in [`data.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/data.rs#L1029-L1089).

The following is an exhaustive transcription of the bindings in the pinned
default config. `shared_except` rows are included because they contribute
bindings to several modes. Multiple keys separated by `/` are aliases from
one KDL `bind` declaration. An empty Normal block is intentional: ordinary
input is handled by the mode fallback.

| Mode | Default key → action | Ekko status |
| --- | --- | --- |
| Normal | no mode-local bindings; ordinary input is written | `Observed` only for ordinary `a` in the differential slice; full fallback and shared behavior `Inventory` |
| Locked | `C-g` → Normal | `Observed` (`C-g` unlock); profile has the same binding |
| Resize | `C-n` → Normal; `h/Left`, `j/Down`, `k/Up`, `l/Right` → Increase Left/Down/Up/Right; `H/J/K/L` → Decrease Left/Down/Up/Right; `=/+` → Increase; `-` → Decrease | `Unimplemented` as a Zellij mode; Ekko has a separate percentage `:resize` action |
| Pane | `C-p` → Normal; `h/Left`, `l/Right`, `j/Down`, `k/Up` → MoveFocus; `p` → SwitchFocus; `n` → NewPane; `d` → NewPane Down; `r` → NewPane Right; `s` → NewPane stacked; `x` → CloseFocus; `f` → ToggleFocusFullscreen; `z` → TogglePaneFrames; `w` → ToggleFloatingPanes; `e` → TogglePaneEmbedOrFloating; `c` → RenamePane input; `i` → TogglePanePinned | Partial: entry/exits/locking/fullscreen `Implemented`; directional/cyclic focus, n/d/r/x and failed-split frames are in integration. Other Pane controls remain `Unimplemented`; complete rendering and behavioral parity remain unproven |
| Move | `C-h` → Normal; `n/Tab` → MovePane; `p` → MovePaneBackwards; `h/Left`, `j/Down`, `k/Up`, `l/Right` → MovePane direction | `Partial`: tiled swaps and mode bindings through public `:set-layout`; regular/bare lifecycle and private Move screenshots verified. Fullscreen no-op and tiled PTY histories are paired; startup, small-terminal output, floating/stacked movement, and shared-mode coverage remain subject to the [paired evidence](../evidence/zellij/move-mode/README.md). |
| Tab | `C-t` → Normal; `r` → RenameTab input; `h/Left/Up/k` → previous tab; `l/Right/Down/j` → next tab; `n` → NewTab; `x` → CloseTab; `s` → ToggleActiveSyncTab; `b` → BreakPane; `]` → BreakPaneRight; `[` → BreakPaneLeft; `1`…`9` → GoToTab; `Tab` → ToggleTab | `Unimplemented` as a Zellij mode; Ekko has basic tabless session focus |
| Scroll | `C-s` → Normal; `e` → EditScrollback then Normal; `s` → EnterSearch plus SearchInput; `C-c` → ScrollToBottom then Normal; `j/Down` → ScrollDown; `k/Up` → ScrollUp; `C-f/PageDown/Right/l` → PageScrollDown; `C-b/PageUp/Left/h` → PageScrollUp; `d` → HalfPageScrollDown; `u` → HalfPageScrollUp; optional `Alt-c` → Copy | `Unimplemented` as a Zellij mode; Ekko copy mode has similar movement/search controls |
| Search | `C-s` → Normal; `C-c` → ScrollToBottom then Normal; `j/Down` → ScrollDown; `k/Up` → ScrollUp; `C-f/PageDown/Right/l` → PageScrollDown; `C-b/PageUp/Left/h` → PageScrollUp; `d/u` → half-page down/up; `n/p` → Search down/up; `c/w/o` → toggle CaseSensitivity/Wrap/WholeWord | `Unimplemented` as a Zellij mode |
| EnterSearch | `C-c/Esc` → Scroll; `Enter` → Search | `Unimplemented` as a Zellij mode |
| RenameTab | `C-c` → Normal; `Esc` → UndoRenameTab then Tab | `Unimplemented` |
| RenamePane | `C-c` → Normal; `Esc` → UndoRenamePane then Pane | `Partial`: entry, incremental input, DEL, filtered paste, commit/cancel and reload-safe undo through public fallback commands/state. ASCII and Unicode per-key paired/visual evidence; batched DEL also has matching settled evidence; mixed mode-switch reads, input limits, remaining shared transitions and errors are still open. |
| Session | `C-o` → Normal; `C-s` → Scroll; `d` → Detach; `w` → floating session-manager; `c` → floating configuration; `p` → floating plugin-manager; `a` → floating `zellij:about`; `s` → floating `zellij:share` | `Unimplemented` as a mode; Ekko has `:detach`, `:reload`, `:help` actions |
| Tmux | `[` → Scroll; `C-b` → Write 2 then Normal; `"` → NewPane Down; `%` → NewPane Right; `z` → fullscreen; `c` → NewTab; `,` → RenameTab; `p/n` → previous/next tab; arrows and `h/j/k/l` → MoveFocus; `o` → FocusNextPane; `d` → Detach; Space → NextSwapLayout; `x` → CloseFocus | `Unimplemented` |
| Prompt | no default block in the pinned config; prompt handling is an internal input mode | `Unimplemented` |

Bindings shared by all modes except `locked`: `C-g` → Locked, `C-q` → Quit,
`Alt-f` → ToggleFloatingPanes, `Alt-n` → NewPane, `Alt-i/o` → MoveTab
Left/Right, `Alt-h/l` (and Alt arrows) → MoveFocusOrTab Left/Right,
`Alt-j/k` (and Alt arrows) → MoveFocus Down/Up, `Alt-=/+` → Resize Increase,
`Alt--` → Resize Decrease, `Alt-[/]` → Previous/NextSwapLayout, `Alt-p` →
TogglePaneInGroup, and `Alt-Shift-p` → ToggleGroupMarking. Shared by all
modes except Normal and Locked: `Enter/Esc` → Normal. Shared mode entry
bindings, each excluded from Locked and its target mode, are `C-p` → Pane,
`C-n` → Resize, `C-s` → Scroll, `C-o` → Session, `C-t` → Tab, `C-h` → Move,
and `C-b` → Tmux. These rows are [`default.kdl`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/assets/config/default.kdl#L177-L218)
and the checked-in copy is [`config.kdl`](../../tests/zellij/reference/config.kdl#L177).

The keybind parser supports custom maps, `clear-defaults`, unbinds, multiple
actions per key, mode inclusion/exclusion, modifier aliases, mouse actions,
and Kitty keyboard protocol events; see [`keybinds.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/input/keybinds.rs)
and KDL parsing in [`kdl/mod.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/kdl/mod.rs#L3830-L4040). Ekko's current extension API exposes maps named `:prefix` and
`:copy` (plus profile-created names), a single configurable prefix, and
declared commands; it does not expose Zellij's KDL map inheritance or all
input modes. `Unimplemented`/`Inventory` here is deliberate.

## Configuration options

The `Options` struct is the authoritative option inventory. Values can come
from KDL or CLI flags, with CLI precedence; web server address/certificate
fields are configuration-only. See [`options.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/input/options.rs#L37-L234)
and the option section of [`default.kdl`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/assets/config/default.kdl#L246-L520).

| Option(s) | Pinned Zellij behavior | Ekko status |
| --- | --- | --- |
| `simplified_ui`, `theme`, `default_mode`, `default_shell`, `default_cwd`, `default_layout` | Plugin font/UI preference; theme; startup mode; pane shell/cwd; startup layout | `Unimplemented`; Ekko has shell argv but no theme/layout option equivalent |
| `layout_dir`, `theme_dir` | Search roots for layouts and themes | `Unimplemented` |
| `mouse_mode`, `advanced_mouse_actions` | Enable mouse input; hover and pane grouping behavior | `Unimplemented`/`Inventory`; Ekko has basic mouse routing but no Zellij option |
| `pane_frames`, `styled_underlines` | Pane borders; styled/colored underlines | Standard ASCII tiled frames observed through public Lisp decorations; small-terminal/default-layout differences remain; underline `Unimplemented` |
| `mirror_session` | Shared session with mirrored cursor versus per-client cursor | `Unimplemented`; Ekko currently supports one attached writer |
| `on_force_close` (`detach`/`quit`) | SIGTERM/SIGINT/SIGQUIT/SIGHUP policy | `Unimplemented`; Ekko has bounded shutdown but no option parity |
| `scroll_buffer_size` | Bounded FIFO scrollback, default 10,000 | `Implemented` with a different 10,000-row/8 MiB accounting policy |
| `copy_command`, `copy_clipboard` (`system`/`primary`), `copy_on_select` | External clipboard or OSC 52 destination and mouse-copy policy | Host clipboard/OSC 52 `Unimplemented`; daemon buffer is `Implemented` |
| `scrollback_editor` | Editor for pane scrollback, default `$EDITOR`/`$VISUAL` | `Unimplemented` |
| `session_name`, `attach_to_session` | Startup session selection and attach policy | Session naming/attach are `Implemented` with a different CLI |
| `auto_layout`, `stacked_resize`, `show_startup_tips`, `show_release_notes` | Automatic predefined layout and stacking on resize; startup tips and first-run release notes | `Unimplemented`; Ekko has mixed split trees and no release-note pane |
| `session_serialization`, `serialize_pane_viewport`, `scrollback_lines_to_serialize`, `serialization_interval`, `post_command_discovery_hook` | Disk session resurrection, optional viewport/scrollback, interval, command rewrite hook | `Unimplemented`; daemon state survives client loss only |
| `disable_session_metadata` | Suppress metadata writes | `Unimplemented` |
| `support_kitty_keyboard_protocol` | Enhanced Kitty keyboard protocol when host supports it | Basic keyboard support is `Implemented`; exact negotiation/coverage `Inventory` |
| `web_server`, `web_sharing`, `web_server_ip`, `web_server_port`, `web_server_cert`, `web_server_key`, `enforce_https_for_localhost` | Local web server, sharing policy, bind/TLS settings | `Unimplemented`; Ekko has no network listener |

Themes use the `Styling` model: selected/unselected text, ribbons, table cells,
and lists; table title; optional unselected frame, selected frame, highlighted
frame; success/error exit codes; and ten multiplayer-user colors. Each
`StyleDeclaration` carries `base`, `background`, and four emphasis colors.
Legacy `Palette` also includes `fg`, `bg`, the eight ANSI names, plus orange,
gray, purple, gold, silver, pink, and brown. Colors accept RGB or 8-bit values.
`Style`/`FrameConfig` add rounded corners and hidden session names. Sources:
[`data.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/data.rs#L1196-L1287)
and [`theme.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/input/theme.rs).
Ekko's `:status-style` is a small SGR list; the full role-based style model,
legacy-theme conversion, configuration parser behavior, defaults, and responsive
rendering remain `Unimplemented`/`Inventory`.

## Actions and CLI

The bound action enum contains: `Quit`; `Write`; `WriteChars`; `SwitchToMode`;
`SwitchModeForAllClients`; `Resize`; `FocusNextPane`; `FocusPreviousPane`;
`SwitchFocus`; `MoveFocus`; `MoveFocusOrTab`; `MovePane`; `MovePaneBackwards`;
`ClearScreen`; `DumpScreen`; `DumpLayout`; `EditScrollback`; `ScrollUp`;
`ScrollUpAt`; `ScrollDown`; `ScrollDownAt`; `ScrollToBottom`; `ScrollToTop`;
`PageScrollUp`; `PageScrollDown`; `HalfPageScrollUp`; `HalfPageScrollDown`;
`ToggleFocusFullscreen`; `TogglePaneFrames`; `ToggleActiveSyncTab`; `NewPane`;
`EditFile`; `NewFloatingPane`; `NewTiledPane`; `NewInPlacePane`;
`NewStackedPane`; `TogglePaneEmbedOrFloating`; `ToggleFloatingPanes`;
`CloseFocus`; `PaneNameInput`; `UndoRenamePane`; `NewTab`; `NoOp`;
`GoToNextTab`; `GoToPreviousTab`; `CloseTab`; `GoToTab`; `GoToTabName`;
`ToggleTab`; `TabNameInput`; `UndoRenameTab`; `MoveTab`; `Run`; `Detach`;
`LaunchOrFocusPlugin`; `LaunchPlugin`; `MouseEvent`; `Copy`; `Confirm`; `Deny`;
`SkipConfirm`; `SearchInput`; `Search`; `SearchToggleOption`; `ToggleMouseMode`;
`PreviousSwapLayout`; `NextSwapLayout`; `QueryTabNames`; `NewTiledPluginPane`;
`NewFloatingPluginPane`; `NewInPlacePluginPane`; `StartOrReloadPlugin`;
`CloseTerminalPane`; `ClosePluginPane`; `FocusTerminalPaneWithId`;
`FocusPluginPaneWithId`; `RenameTerminalPane`; `RenamePluginPane`; `RenameTab`;
`BreakPane`; `BreakPaneRight`; `BreakPaneLeft`; `RenameSession`; `CliPipe`;
`KeybindPipe`; `ListClients`; `TogglePanePinned`; `StackPanes`;
`ChangeFloatingPaneCoordinates`; `TogglePaneInGroup`; and `ToggleGroupMarking`.
This list is [`actions.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/input/actions.rs#L102-L304);
payload types and CLI conversion are in the same file. Ekko's validated public
actions are listed in [`docs/customization.md`](../customization.md#public-api-version-1)
and cover a small session/copy/status subset (`:split`, `:focus`, `:rename`,
`:resize`, `:status`, copy actions, `:zoom`, `:swap`, `:close`, `:detach`,
`:reload`, and `:help`). The rest are `Unimplemented`.

The top-level CLI is `options`, `setup`, `web`, session commands, `run`,
`plugin`, `edit`, `convert-config`, `convert-layout`, `convert-theme`, and
`pipe`; global flags include `--max-panes`, `--data-dir`, `--session`,
`--layout`, `--new-session-with-layout`, `--config`, `--config-dir`, and
`--debug`. Session commands are `list-sessions`/`ls`, `list-aliases`/`la`,
`attach`/`a`, `kill-session`/`k`, `delete-session`/`d`, `kill-all-sessions`/`ka`,
`delete-all-sessions`/`da`, and `action`/`ac`. Other top-level commands are
`run`/`r`, `plugin`/`p`, `edit`/`e`, and `pipe` (plus the conversion commands).
Their flags include floating/in-place/stacked panes, cwd, pane
coordinates and pinning, hold-on-start/close, plugin configuration/cache,
session creation/resurrection, and formatting controls. This CLI inventory is
[`cli.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/cli.rs#L37-L494).

`zellij action` exposes the action names above plus target-pane variants and
web/session controls: write, resize, focus/move, clear/dump, scroll, fullscreen,
pane/tab creation and naming, plugin launch/reload, pipe, client listing,
floating coordinates/pinning, pane stacking/grouping, layout dump, web server
start/stop/status, web token create/revoke/list/rename, config reconfigure, and
pane replacement. The complete `CliAction` payload inventory is
[`cli.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/cli.rs#L496-L906).
Ekko's `ekko command` and `ekko split` are a narrower, unrelated CLI; the
Zellij command names are not accepted.

## Panes, tabs, layouts, and sessions

Zellij has terminal PTY panes and WASM plugin panes. Panes may be tiled,
floating, in-place, stacked, suppressed, fullscreen, pinned, grouped, or
temporarily hidden; they have IDs, titles, geometry, focus/selectability,
exit/held state, and per-client focus. Terminal panes maintain a VT grid,
scrollback, selection/search state, cursor, alternate screen, hyperlinks, and
Sixel image state. Plugin panes maintain a separate per-client grid and plugin
worker state. See [`terminal_pane.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-server/src/panes/terminal_pane.rs),
[`plugin_pane.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-server/src/panes/plugin_pane.rs),
and [`screen.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-server/src/screen.rs).
Ekko has PTY panes, up to 16 mixed split-tree panes, focus/zoom/swap/resize,
scrollback, and a daemon buffer (`Implemented`); floating/plugin panes,
stacking, grouping, pane IDs with Zellij semantics, held panes, and multiple
tabs are `Unimplemented` or `Inventory`.

Layouts are KDL trees with named tabs, tiled and floating pane layouts, percent
or fixed sizes, commands/plugins, cwd, focus, pane names, pinned state, initial
contents, logical positions, and swap layouts. The data model and parser are
[`layout.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/input/layout.rs)
and [`kdl_layout_parser.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/kdl/kdl_layout_parser.rs).
The pinned default layout is tab-bar / terminal / status-bar. Ekko's Lisp
configuration has no KDL layout parser; its split tree is a separate API.

Sessions are daemon/server state attached by clients. Zellij supports attach,
detach, kill, delete, multiple clients, mirrored or independent cursors,
session metadata, periodic disk serialization, resurrectable sessions,
viewport/scrollback resurrection, command discovery and rerun. See
[`session_serialization.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/session_serialization.rs),
[`sessions.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/sessions.rs),
and [`zellij-client/src/lib.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-client/src/lib.rs).
Ekko preserves PTYs and logical state across client loss and reconnect
(`Implemented`), but does not resurrect a session after daemon death,
serialize layouts/commands/viewport, support multiple attached writers, or
provide Zellij's session CLI (`Unimplemented`).

## Terminal input and rendering

The server owns one PTY per terminal pane and parses child bytes incrementally
with `vte`; the client parses host stdin, including legacy ANSI and enhanced
Kitty keyboard protocol, then sends semantic keys and raw bytes to the server.
Mouse events include click, release, hold, hover, wheel, and modifiers. Paste
honors bracketed paste. Relevant code is [`pty.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-server/src/pty.rs),
[`terminal_bytes.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-server/src/terminal_bytes.rs),
[`stdin_ansi_parser.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-client/src/stdin_ansi_parser.rs),
[`keyboard_parser.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-client/src/keyboard_parser.rs),
and [`mouse.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/input/mouse.rs).
Ekko has incremental UTF-8/VT parsing, basic mouse, paste, and Kitty keyboard
support (`Implemented`), with incomplete VT/terminfo, grapheme, modifier,
clipboard, and child-query coverage (`Inventory`/`Unimplemented`).

Rendering emits terminal control sequences from the client. Zellij renders
styled Unicode cells, wide characters, ANSI colors, hyperlinks (OSC 8), pane
frames, status/tab UI, cursor, selection, scrollback, and Sixel images. Sixel
images are decoded into an image store, tracked by pane-local locations, clipped
to visible regions, and serialized after text with an implied image layer; see
[`output/mod.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-server/src/output/mod.rs)
and [`sixel.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-server/src/panes/sixel.rs).
Ekko's text and Kitty direct RGB/RGBA rendering, pane-local image IDs, native
placement, clipping, repeated static frames, and reconnect reconstruction are
`Implemented` for its documented subset. Sixel, PNG, Unicode placeholders,
relative/scaled placement, full image lifecycle, animation, explicit layers,
and complete scroll/erase semantics are `Unimplemented` or `Inventory`; see
[`docs/compatibility.md`](../compatibility.md).

## Bundled plugins and plugin interface

The pinned build includes these built-in WASM plugins and aliases: `tab-bar`,
`status-bar`, `compact-bar`, `strider`, `session-manager`, `configuration`,
`plugin-manager`, `about`, `share`, and `multiple-select`. `welcome-screen` is
an alias of `session-manager` with `welcome_screen true`; `filepicker` is an
alias of `strider` with cwd `/`. The alias table is in
[`default.kdl`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/assets/config/default.kdl#L221-L244)
and resolution/embedded asset behavior is [`plugins.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/input/plugins.rs).
Their source directories are [`default-plugins`](https://github.com/zellij-org/zellij/tree/v0.43.1/default-plugins).
The plugins render tab/status/compact bars, file navigation/search/filepicker,
session creation/attach/resurrection/kill, configuration and keybinding UI,
plugin discovery/install/configuration, about/help, web sharing/token UI, and
multi-pane selection/grouping. Ekko has status contributions and Lisp commands
but no WASM plugin panes, aliases, bundled plugin UI, or plugin manager.

The public plugin wire interface is protobuf over the plugin worker bridge:

* Commands include subscribe/unsubscribe, selectable state, plugin/version
  queries, open file/terminal/command panes (tiled/floating/in-place/near a
  plugin), tab/focus/resize/move/scroll/fullscreen/clear/write actions,
  detach, timers, host command execution, inter-plugin messages, hide/show,
  mode switching, layouts, plugin loading/reloading, pane/tab/session control,
  filesystem scans/watchers, config reconfigure, CLI pipes, client listing,
  web requests/server/token control, permissions, input interception, and pane
  grouping/floating/stacking. See the complete `CommandName` enum in
  [`plugin_command.proto`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/plugin_api/plugin_command.proto#L15-L162)
  and payloads in lines 164–705.
* Events include mode/tab/pane/key/mouse/timer, clipboard success/failure,
  input received, visibility, custom messages, filesystem create/read/update/
  delete, permission results, session and command results, web results/status,
  pane open/exit/close, config write, client list, host-folder changes,
  paste, before-close, and intercepted keys. See
  [`event.proto`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/plugin_api/event.proto#L10-L60).
* Permissions are `ReadApplicationState`, `ChangeApplicationState`,
  `OpenFiles`, `RunCommands`, `OpenTerminalsOrPlugins`, `WriteToStdin`,
  `WebAccess`, `ReadCliPipes`, `MessageAndLaunchOtherPlugins`, `Reconfigure`,
  `FullHdAccess`, `StartWebServer`, and `InterceptInput`; see
  [`plugin_permission.proto`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/plugin_api/plugin_permission.proto#L5-L19).
* Plugin event payloads expose pane/tab/session manifests, mode/keybind/style,
  geometry, cursor, focus, client identity, layouts, plugin URLs/configuration,
  web-sharing state, and resurrectable sessions. See the manifest messages in
  [`event.proto`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/plugin_api/event.proto#L261-L390).

Ekko's extension boundary is trusted Lisp in a worker with detached snapshots,
declared reads, commands/keymaps/options/status hooks, validated actions, and
reload/recovery (`Implemented` as its own API). It has no compatibility adapter
for Zellij's protobuf/WASM API, plugin permissions, plugin lifecycle, event
subscriptions, pipes, or host filesystem/web access (`Unimplemented`).

## Web client and web server

With the default `web_server_capability` feature, Zellij can start/stop/query a
local web server, bind localhost or another address with TLS requirements,
create/revoke/list/rename login tokens, opt sessions into web sharing, and
attach browser clients over HTTP/WebSocket. The CLI flags are in
[`cli.rs`](https://github.com/zellij-org/zellij/blob/v0.43.1/zellij-utils/src/cli.rs#L102-L188),
server/client implementation is under [`zellij-client/src/web_client`](https://github.com/zellij-org/zellij/tree/v0.43.1/zellij-client/src/web_client),
and the web capability is feature-gated in [`Cargo.toml`](https://github.com/zellij-org/zellij/blob/v0.43.1/Cargo.toml#L131-L139).
Ekko currently has no network listener, browser client, web sharing, or
web token model (`Unimplemented`).

## Rendering, persistence, and interface gaps still requiring investigation

The inventory above is source-derived but not a test matrix. Remaining work
before any compatibility statement would need focused tests for every row,
including all default key sequences and mode transitions, KDL merge/clear/
unbind behavior, every action payload and CLI flag, floating and stacked pane
geometry, tabs and swap layouts, multiple clients/mirroring, session
serialization/resurrection, Sixel and Unicode placeholder image behavior,
OSC/ANSI edge cases, plugin permissions/events/pipes, web authentication and
WebSocket reconnect, and host-specific rendering. The current differential
check covers only the startup and settled Normal/Locked stages in its harness and retains raw ANSI;
there is no physical-terminal screenshot parity result. No Zellij parity claim
is made by this document or by the profile.


## Frame-toggle measured slice (2026-09-06)

`TogglePaneFrames` is **Partial**: ordinary tiled on/off/on, focus changes,
fullscreen transitions, and exact child size histories are exercised at 80×24
and 20×8. Regular/bare lifecycle checks cover owner removal and reload without
restarting children. Fourteen settled graphical checkpoints match exactly in
the base runtime and in the isolated Finix patched runtime. The complete
input/cell gate remains false. Stacked/floating/borderless/grouped/multiplayer
frames, arbitrary layout junctions, and remaining tiny-viewport variants need
coverage; no screenshot result promotes those rows. Original startup differences
remain. [Evidence and failures](../evidence/zellij/frame-toggle/README.md).


## Session and exit measured slice (2026-09-06)

Session mode is **Partial**: entry/exits, locking, transitions to Pane/Move,
single-client Detach and Quit from the five supported unlocked modes are covered.
The same children survive explicit detach/reattach; quit terminates them and
restores host termios. The mode's bundled-plugin launchers and other shared mode
transitions are still unimplemented. Reattachment after involuntary viewer loss,
non-Normal configured defaults, and multi-client Quit behavior require separate
coverage. This does not establish session CLI or resurrection parity.

A generic Lisp `:viewer-exit-text` option supplies the farewell. Actual settled
Session screenshots match in 10 checkpoints. Post-quit screenshot and native
Kitty styled-text/wrap/cursor exports match exactly. Pyte's post-exit grids remain
in the evidence as model limitations; raw data is not changed. Startup and the
remaining surface are still required. [Evidence](../evidence/zellij/session-mode/README.md).

## Startup mechanism continuation (2026-09-06)

Public Lisp initialization is now implemented and transaction-tested before child
spawn/configuration commit. It supplies owned state, keymap, status and decoration
actions. This is a runtime capability, not a completed Zellij feature row. Release
notes, tips, durable marker semantics and floating/plugin input remain unimplemented.
[Mechanism evidence](../evidence/zellij/initialization/README.md).
