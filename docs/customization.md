# Lisp customization and daily-use controls

Ekko loads `$EKKO_CONFIG`, or `$XDG_CONFIG_HOME/ekko/init.lisp` (default
`~/.config/ekko/init.lisp`). It does not discover configuration in the working
directory. An absent default file uses builtins; an explicitly named missing
file is an error. See [a complete example](../examples/init.lisp).

```sh
ekko config check                  # validate the file in a disposable worker
ekko config reload workspace       # replace this session's active configuration
ekko inspect workspace             # JSON: owners, commands, keymaps, options, errors
ekko command --session workspace label-work
```

`ekko config reload SESSION` reloads the configuration. Editing a file takes effect after
reload, without rebuilding or restarting applications. A bad reload reports an
error and keeps the previous configuration. Each session remembers its creation
configuration path and launch environment; `config check` uses the caller's environment.

These are trusted Lisp files with your OS permissions, like an Emacs init file.
They run in a separate process. This boundary isolates host state and lets the
daemon terminate a runaway callback; it is not an arbitrary-Lisp security sandbox.
Configuration loading has a five-second deadline; each command and initialization
callback has a 50 ms wall-clock deadline, while a change hook gets 500 ms because
it paints UI chrome and a loaded machine may answer late. Messages are limited to
64 KiB and the init file to 32 KiB.
Load local helper files with `load` if needed. Reloading a worker also reloads
those files; recovery retains the accepted init text, not copies of its dependencies.

## Shared sessions and views

Sessions share one daemon, global pane IDs and the durable component store.
A session keeps its launch cwd, environment, worker, configuration and applications.
Each view has its own focus, mode, zoom, selection, geometry contributions,
component state, decorations and layout camera. The first attachment uses the
session's home view; a default reconnect can resume that detached view.

Callbacks receive an immutable snapshot of their originating view. Commands
capture that view's epoch and default pane target before dispatch. Completion
cannot use another view's focus. Detach or session switching cancels old-origin
actions and pending input. A shared structural edit refreshes affected views.
Use `ekko command --session NAME --view ID COMMAND` when several views are attached.
The public `:show-session :name NAME` action switches the originating view.

## Layout providers

A provider is an owned, isolated callback, not a host layout enum:

```lisp
(layout-api-version) ; => 1
(register-layout-provider
 :component :workspace :name "my-layout" :api-version 1
 :reads '(:panes :focus :viewport :geometry)
 :handler function)
(set-option :component :workspace :name :layout-provider :value "my-layout")
(set-option :component :workspace :name :workspace-scope :value :all-panes)
```

Register the component first. The provider's handler receives `(snapshot event)`
and returns exactly one action:

```lisp
(list (action :place-panes :version 1
              :placements '((:pane 42 :x -19 :y 1 :cols 60 :rows 20
                              :outer (-20 0 62 22)))
              :camera '(100 0)))
```

IDs must come from the supplied pane catalog. Content must fit its full outer
rectangle. Placements are ordered back to front; they may overlap, be offscreen,
or omit panes. `:visible nil` retains a logical size demand without displaying
that placement; visibility defaults to `t`. Omitted panes have no demand from
that view and keep their size if no other view participates. The host subtracts
the camera only for presentation and input coordinates. Panning never resizes a
PTY to its visible sliver.

`:panes`, `:focus`, `:viewport` and `:geometry` are required reads. Additional
reads use the same declared-dependency API as hooks. Provider pane metadata omits
derived geometry/output fields so a placement does not trigger its own feedback
loop. `:sessions` and `:all-panes` expose the daemon-wide catalog;
`:workspace-scope` chooses `:session` or `:all-panes` for the view's `:panes`.
`:workspace` includes scope and camera. There is no built-in-only placement API.
The default `"tiled"` and `"floating"` providers use this boundary too.
They live in `examples/profiles/layouts.lisp`; a bare config can load that file
and call `ekko/layout-providers:install-layouts`. `ekko/layout` supplies optional
public pure split-tree helpers.

The host validates the complete answer before committing it. It rejects duplicate
or unknown IDs, invalid containment, stale declared inputs, old view epochs and
old config generations. Callback execution has a 500 ms deadline. A failed answer
keeps the last valid placement. Removing/replacing an owner removes its placement
and camera contribution, not its applications. The bare host can display a focused
pane; a multi-pane policy is supplied through the same public registration API.

Bounds are explicit: at most 128 placements, content 1–500 columns by 1–300 rows,
outer frames at most 532 by 332 cells, and coordinates/camera within
−1,000,000 to 1,000,000. `:pane-budget` is 1–128, default 16. These are finite
resource limits, not a claim of unlimited terminals.

At most 128 sessions (including pending/retiring sessions), 16 concurrent
creations and 256 peers are admitted. The daemon keeps every home/live view and
the eight newest detached extra views per session. Older detached extra state
may be reclaimed; this never removes an application or the home view.

[scrolling.lisp](../examples/profiles/scrolling.lisp) demonstrates fixed-width
columns over all sessions, focus-following camera, explicit pan commands and
live column sizing. Load the same file in regular Ekko or `ekko-bare`.
To use project-separated workspaces, select `:workspace-scope :session` and a
provider per session. Reloading policy does not restart PTYs.

### Shared application sizing and native pixels

Each pane has one VT/PTY. Its size is the componentwise minimum of participating
views' full logical content rectangles, after insets, not their visible crops.
Minimized and frozen-copy views do not constrain it. The public tiled/floating
providers return hidden base-layout demands during zoom to preserve application
sizes. The host does not derive these demands from a built-in layout name.
Without participants,
a running pane keeps its last size. New headless panes use their creation viewport.

The oldest participating view supplies canonical cell and reported pixel metrics.
Other views retain their own host metrics for rendering. Images stay at native
pixel size; they are not resampled to match another terminal's font. Cell mouse
input uses local host cells. Pixel-mode mouse input uses native local pixels,
clamped to the application's canonical pixel bounds. Offscreen text, images and
cursors are clipped to the host content viewport.

## Public API, version 1

The public package is `ekko/extensions`, supplied by the ASDF system of the same
name. Its exports are:

```lisp
(api-version) ; => 1
(layout-api-version) ; => 1
(display-width "work") ; => 4
(clip-decorations spans cols rows)
(register-layout-provider :component :name :name "layout" :api-version 1
                          :reads '(:panes :focus :viewport :geometry)
                          :handler function)
(register-component :id :name :api-version 1 :reads '(:focus)
                    :handler function :initialize initialization-function)
(unregister-component :name)
(register-command :component :name :name "command" :handler function)
(register-keymap :component :name :name :normal :unbound :forward)
(bind-key :component :name :key "v" :command "command" :map :prefix)
(set-option :component :name :name :initial-keymap :value :normal)
(set-option :component :name :name :prefix :value "C-a")
(value snapshot :focus)
(action :rename :text "work")
(action :set-keymap :name :normal)
```

Registration calls belong in init loading. Handlers receive `(snapshot event)`
and return a list of actions, or `nil`. Commands receive `(:arguments (...))`;
change hooks receive `(:type :change)`. The snapshot contains only the component's
declared keys. `value` rejects undeclared reads. Mutating the detached snapshot
cannot change daemon state. No host objects, file descriptors, or image buffers
cross this boundary.

| Snapshot key | Value |
| --- | --- |
| `:session` | Originating session name |
| `:view` | Origin view `:id` and `:epoch` |
| `:sessions` | Daemon-global session catalog |
| `:all-panes` | Daemon-global pane metadata |
| `:workspace` | Scope, session and camera metadata |
| `:focus` | Origin view's daemon-global focused pane ID |
| `:mode` | Active custom keymap keyword, or `nil` for built-in routing |
| `:panes` | Plists with `:id`, raw `:label`, explicit `:name` (nil until rename), detached `:argv`, `:launch-kind` (`:command` or `:shell`), immutable `:creation-position`, `:terminal-title` (nil until OSC title, empty when cleared), `:display-label`, `:pid`, `:cols`, `:rows`, `:exit`, content `:x`/`:y`, `:outer-rect` `(x y width height)`, `:layout-rect`, `:activation-order`, `:visible`, `:pty-size`, `:history-rows`, and `:activity` |
| `:viewport` | Plist with `:cols`, `:rows`, effective pixel `:cell-width`/`:cell-height`, nullable `:reported-cell-width`/`:reported-cell-height`, resolved `:insets` and `:gaps` |
| `:zoom` | Lisp boolean (`t` or `nil`) |
| `:chrome-status` | Resolved status `:text` and SGR `:style` |
| `:component-state` | Per-view detached alist of component ID strings to daemon-owned plain values; other components can read it when declared |
| `:pane-notes` | Active temporary contributions, each with `:owner`, `:pane`, `:text`, and `:sgr`, ordered by component registration |
| `:layout` | Pane ID leaves; branches `(axis percentage first second)` |
| `:time` | Wall-clock seconds (CL universal time); a component that reads it re-runs its hook when the second changes |

Pane launch metadata is daemon-owned and survives component removal, reload,
and viewer replacement. `:launch-kind` is `:command` for startup commands and
splits with explicit `:argv`, or `:shell` for splits using the configured shell.
`:creation-position` records the live pane count including the new pane when
it is created; it is not the stable pane ID and is not renumbered after closes.
`:name` records the last rename, including an empty string; `:label` retains
its existing builtin behavior. `:terminal-title` preserves surrounding spaces
and distinguishes no OSC title (`nil`) from an explicitly empty title (`""`).
The OSC parser remains bounded to 16 KiB and extension packets to 64 KiB;
oversized snapshots are not silently truncated. Formatting and title precedence
belong to ordinary components.

`:layout-rect` is the pane's outer `(x y width height)` rectangle with zoom
ignored; it is `nil` for panes hidden by a layout too small to fit. It allows
navigation policy to inspect the tiled arrangement while one pane is fullscreen.
`:activation-order` is a daemon-owned increasing integer updated when focus
moves to a pane. Reload and reattachment preserve it. Cell pixel dimensions are
the daemon's effective values, including fallback values when host dimensions
are unavailable. Reported values remain `nil` until a host query reply arrives;
they are distinct from effective rendering metrics and survive detach/reload.
The client accepts both text-area and cell-size replies, with direct cell-size
reports taking precedence. Merely reporting metrics does not resize applications.
`:pty-pixel-source` selects `:effective` (default) or `:reported` for application
PTY pixels, applied at startup and subsequent layout operations. Unknown reported
metrics produce zero pixel fields. Pane `:pty-size` records `(cols rows xpixels
ypixels)` last successfully applied to the kernel; inspect calls it `pty_size`.
Pixel fields follow the platform's unsigned 16-bit winsize representation.
Physical VT/rendering geometry remains independent of this option. Removing the
owning component restores the effective source on reload without replacing panes.

A change hook runs initially and when one of its declared keys changes; changes
can coalesce while a handler runs. Hook results are discarded when a declared
dependency changes before completion; unrelated snapshot changes do not discard
them. Hooks with no declared reads run once after installation. Hooks may
only return `:status` or `:decorate` contributions, preventing reactive action loops. Each answer
clears the owner's timeout count; a hook that misses three consecutive deadlines
is disabled until explicit reload, so one scheduling delay cannot permanently
remove an owner's chrome. After worker failure, Ekko reconstructs
registrations from the accepted init source; worker-local variables reset.

A component may also supply `:initialize`, a `(snapshot event)` function returning
only `:set-state`, `:store-set`, `:set-keymap`, `:status` and `:decorate` actions.
Initialization runs once per view in an accepted configuration generation,
including startup, a new attachment and worker recovery. The event is
`(:type :initialize :reason :startup)`, `:attach`, `:reload` or `:recovery`.
The entire initializer group has the ordinary 50 ms callback deadline.
Registrations remain load-only. Commands, PTY writes, geometry changes, process
creation and other session actions are rejected in initialization.

At session creation, the home view is initialized before its application PTYs
start; pane PIDs are `nil`. The shared daemon socket and other sessions may
already exist and remain responsive.
During reload, callbacks see a detached snapshot of the last committed geometry,
mode and pane state, with removed owners filtered out of `:component-state`.
They do not see candidate option geometry or other initializers' proposed state.
All callbacks for one view use the same snapshot and run in registration order;
later owners' mode choices win. Candidate mode and focus are normalized to the
candidate keymaps and workspace scope. Provider validation follows initialization
for every affected view and sees the aggregate staged store writes. State survives successful owner-preserving reload, so an
initializer can distinguish first installation from later generations:

```lisp
(ekko/extensions:register-component
 :id :welcome :reads '(:component-state)
 :initialize
 (lambda (snapshot event)
   (declare (ignore event))
   (unless (assoc "welcome" (ekko/extensions:value snapshot :component-state)
                  :test #'equal)
     (list (ekko/extensions:action :set-state :value '(:initialized t))))))
```

Every returned batch and aggregate ownership limit is checked on a detached
session before the candidate replaces the live registry. An invalid later owner,
callback error or timeout rejects the entire candidate. Child output continues;
viewer input arriving during initialization is queued and replayed through the
accepted map on success, or the previous map on rejection. If a declared snapshot
dependency, retained component state or mode changed during the callback, reload
fails with a retry message instead of applying stale results. No PTY is restarted.
Ordinary change hooks run after commit and may replace their owner's initialized
status/decorations. Removing an owner removes its state and decorations; removing
the active keymap restores normal registry fallback rules. Initialization itself
does not provide durable storage; the durable component state below survives
daemon restarts. Like configuration loading itself, trusted Lisp can
perform arbitrary external I/O; the transaction covers returned public actions,
not direct filesystem/process side effects in user code. `config check` validates
registration data; it does not execute these session-dependent callbacks.

Components own their commands, bindings, keymaps, options, status contributions, and decorations.
Later components shadow earlier ones. Removal or reload reconstructs those
contributions, restoring underlying defaults. A keymap name is a keyword other
than the built-in `:prefix` and `:copy`; its `:unbound` policy is
`:forward` (send an unbound key to the focused pane), `:ignore` (discard it),
`:copy` (use the existing copy search editor and `:copy` bindings while the
focused pane has copy state), or a registered command-name string (receive the input event as described below).
`register-keymap` records the component as the map's owner. A `bind-key` map may
be `:prefix`, `:copy`, or a custom registered map; custom map references are
validated when the complete registry is installed, so an unknown map rejects the
reload and leaves the previous configuration active.

The preserved state is the session's
PTYs, labels, layout, history, copy selections, and buffer. User-invoked commands
change that session state; removing their component does not undo past user actions.
`inspect` reports the active `:mode`, the `:zoom` state as a JSON boolean,
registered keymaps (including each map's owner and unbound policy), bindings,
contributions, disabled hooks, and the last error. Builtins use the same API in
`ekko/builtins`; `ekko-bare` is packaged without builtins and can load external
commands and layout providers.

Public extension API version 1 remains distinct from wire version **15**.
All clients, including controls, must first send packet 0 with bounded data
`(15 SESSION-OR-NIL VIEW-ID-OR-NIL)`. Older versions and unversioned traffic are
rejected; there is no legacy tiled-only attachment path.

Packet 1 attaches once with five big-endian u32 values: version, columns, rows,
cell width and cell height. Packet 17 resizes an attached view with four u32
viewport values. Packet 15 carries two reported cell metrics (width 1–128,
height 1–256). Scene and asset framing keep their existing layout, with
daemon-global pane identities. Packet 14 remains the graphics acknowledgement;
its packet number is not a wire version.

On attachment and each session switch, packet 24 announces one big-endian u32
connection binding generation. Packet 25 acknowledges that generation. The
client cancels old parser and original-read state at cutover. It retains only
cancellation knowledge for a split paste delimiter and discards the old paste
through its terminator before acknowledging. An ambiguous interrupted delimiter
closes that client rather than forwarding uncertain bytes; applications survive.
Input packets 2, 4, 5, 6 and 16 prepend the binding generation to their payload;
packet 16 retains the original bytes from a host input read. Old-generation
input is discarded, including a late packet after switching away and back to
the same view. No input is accepted before the new binding acknowledgement.
This boundary is separate from graphics acknowledgement and lease cleanup.

| Option | Value |
| --- | --- |
| `:pty-pixel-source` | `:effective` (default) or `:reported`; selects application PTY pixel metrics independently of rendering |
| `:pane-insets` | `(top right bottom left)`, each integer 0–16; default `(1 0 0 0)` |
| `:boundary-insets` | Optional `(top right bottom left)` override applied only where a pane meets the content viewport; each integer 0–16, default `nil` |
| `:viewport-insets` | Same order/range; default `(0 0 1 0)` |
| `:split-gaps` | `(column-gap row-gap)`, each integer 0–16; default `(1 0)` |
| `:erase-display-history` | Lisp boolean, default `nil`; ED2 transfers materialized main-screen rows into history when true |
| `:initial-layout` | Optional startup tree, e.g. `(:columns 50 1 (:rows 50 2 3))`; leaves name one-based startup command slots, each exactly once; mapped to global pane IDs at creation; percentages 1–99, bounded by `:pane-budget` |
| `:pane-budget` | Integer 1–128, default 16 |
| `:layout-provider` | Registered provider name, or `nil` for the focused-pane bootstrap |
| `:workspace-scope` | `:session` (default) or `:all-panes` |
| `:initial-keymap` | Registered custom keymap keyword, or `nil` |
| `:prefix` | `"C-a"` through `"C-z"`, or integer 1–26 |
| `:shell` | Executable argument list for new panes; defaults to `$SHELL -i` |
| `:viewer-exit-text` | `nil` (default) or at most 512 printable Unicode characters, excluding C0/DEL/C1 controls; appended with CRLF after returning to the host main screen |
| `:status-text` | Up to 512 characters |
| `:status-style` | SGR integer list, e.g. `'(0 37 44)` |

The initial layout is consumed before spawning startup applications, so their
first PTY size reflects the configured tree. Its leaf count must match the
startup command count. Reloading or removing this option preserves the live tree.

Geometry options belong to their component, just like other options. Reload or
removal recomputes pane rectangles and PTY dimensions without restarting the
applications. The layout reserves each pane's requested insets plus at least
one content cell; when the tree cannot fit, it shows the focused pane. In a tiny
viewport, top and left insets take priority, followed by bottom and right, with
all sides capped to leave content within the outer rectangle. Hidden panes keep
their state. `inspect` exposes resolved geometry options. These primitives reserve
space; ordinary decoration components supply frame text and style policy.

A command can replace its owner's runtime geometry contribution:

```lisp
(action :set-geometry
        :value '(:pane-insets (0 1 1 0)
                 :boundary-insets (0 0 0 0)
                 :split-gaps (0 0)))
(action :set-geometry :value nil) ; remove only this owner's override
```

The value is a property list with unique keys from `:pane-insets`,
`:boundary-insets`, `:viewport-insets`, and `:split-gaps`, using the ranges
above. An empty value removes the contribution. Each action replaces the whole
contribution for its registered owner. For each field the last registered
contributing component wins; runtime contributions override static options.
Fields omitted by an owner continue resolving through other owners and then
static options. `:boundary-insets` substitutes only viewport-facing edges,
after the viewport inset is applied. Outer split rectangles remain distinct
from application content rectangles. Minimum-size calculation respects the
boundary edges of each tree leaf.

Geometry is a primary action (at most one primary per batch), and can be
combined with `:set-state` and `:set-keymap`. Change hooks cannot change geometry.
Validation precedes every effect, and rejected batches preserve the previous
contributions and state. The host recomputes content geometry and resizes only
PTYs whose effective dimensions changed; pane processes and graphics ownership
remain intact. Owner geometry survives worker replacement, reload retaining the
owner, and viewer detach. Removing the owner reveals the next contribution or
static option; it also removes that owner's component state. The detached
`:geometry` snapshot exposes resolved fields, and `inspect` additionally lists
owner contributions. Declare `:geometry` when a hook reads it.

The public profile is exercised on regular and bare runtimes
with real child size histories, inverse toggles, unchanged PIDs, successful and
failed reload, detach/reattach, and owner removal. The profile chooses its
frame state, edge offsets, keybindings, glyphs, and colors in ordinary Lisp;
the runtime contains no Zellij frame-toggle action or renderer branch.

A command or hook can replace its owner's decoration contribution:

```lisp
(action :decorate :spans
        (list (list :x 0 :y 0 :text " work " :sgr '(0 1 38 5 154))
              (list :x 0 :y 1 :text "│" :sgr '(0) :rows 10)))
```

Coordinates are zero-based terminal cells: `:x` is 0–499 and `:y` is 0–299.
Optional `:rows` repeats the horizontal span on 1–300 consecutive rows; it
is one by default. Text excludes control characters, DEL, and C1 controls.
SGR lists contain at most 16 integers from 0 through 255. Retained contributions
across all owners allow at most 1,024 declared spans and 16,000 text characters,
counting repetitions. Worker message limits also apply. An empty span list
clears that owner's contribution. Invalid batches preserve previous contributions.

For camera-projected chrome, call `(clip-decorations spans cols rows)` before
returning the action. This pure helper accepts negative logical positions, clips
to the viewport, and preserves each span's style, actions and other attributes.
It drops wide glyphs that cross a boundary and keeps combining marks with their
accepted base glyph. Returned spans still pass ordinary host validation; this
helper does not grant an unchecked decoration path.

Later registered components paint above earlier components, independent of
callback completion order. By default the daemon clips spans to the terminal
and excludes visible application content before publication. An explicit
`:overlay t` marks opaque UI cells allowed over application content; normal
decorations paint first, then overlays in component registration order. Literal
spaces are opaque too. The viewer subtracts those cell rectangles from visible
Kitty image crops and preserves the source offsets of each fragment. Removing
or replacing the contribution reveals current application text and graphics;
applications continue running behind it. Overlays hide the application cursor.
The same span count, text size, validation, and owner teardown rules apply.
This flag controls appearance only; profiles must implement their own input
routing through keymaps/actions. It is not a floating-pane or plugin runtime. Wide glyphs crossing a
boundary are dropped; combining marks stay with an accepted base glyph.
Reload clears accepted contributions and schedules the new hooks. The default
titles, split dividers, and status line use this same API in `ekko/builtins`.
`:history-rows` is the bounded main-screen history count (zero on alternate
screen); it does not expose a scroll position or history reflow.

When `run` creates a session from a terminal, its initial cell and pixel size
is handed to the daemon before applications spawn. The loaded geometry options
therefore determine each application's first PTY size. New split panes likewise
start at their planned content size; an unsuccessful spawn leaves the current
layout and applications intact. A server started without a terminal viewport
uses the default 120×36 session and 8×16 cell size. A pane initially hidden by
layout collapse starts at 1×1 until it is shown. Later attachments and reloads
continue to resize existing PTYs without restarting their processes.

Bindings use maps `:prefix`, `:copy`, or a custom map registered with
`register-keymap`. Keys are single-character strings, integer
code points, control names, dash chords built from `C-`, `M-`/`Alt-` and
`W-`/`Super-`, or `Tab`, `Enter`, `Escape`, `Left`, `Right`, `Up`, `Down`, `PageUp`,
`PageDown`, `Home`, `End`. A control-only chord like `C-g` folds into its legacy
control code; `M-Tab` and `Super-1` stay distinct. `bind-key` accepts an optional
`:arguments` list of up to eight strings, delivered to the bound command as its
event arguments. A `nil` command unbinds a key in that component. Prefix
followed by itself sends the literal control byte to the application.

When a custom map is active, its bindings and unbound policy receive keys before
ordinary pane input, and the configured prefix and built-in copy/search key
handlers are inactive for that mode. Copy/history state is preserved. A
command can switch maps by returning `(action :set-keymap :name :normal)`; the
target must be registered in the active registry. Components can therefore keep
mode switching commands beside the maps they own:

```lisp
(ekko/extensions:register-component :id :modes :reads '(:mode))
(dolist (map '(:normal :locked))
  (ekko/extensions:register-keymap :component :modes :name map :unbound :forward))
(ekko/extensions:set-option :component :modes :name :initial-keymap :value :normal)
(ekko/extensions:register-command :component :modes :name "lock"
  :handler (lambda (snapshot event)
             (declare (ignore snapshot event))
             (list (ekko/extensions:action :set-keymap :name :locked))))
(ekko/extensions:bind-key :component :modes :map :normal :key "C-g" :command "lock")
```

The active map is available to a component that declares `:mode` in its snapshot
dependencies. On reload, Ekko keeps the selected mode if the new registry still
registers it. If it does not, Ekko selects `:initial-keymap` (or no custom mode
when that option is absent). A failed reload keeps both the old registry and its
current mode. Map names and bindings are configuration contributions: removing
the owning component removes them, while the session's prior user actions remain.

The checked-in [Zellij profile](../examples/profiles/zellij.lisp) demonstrates
Normal/Locked routing plus Pane entry, exits, locking, and fullscreen. It is still
in progress and provides a partial proof of the
generic API, not full Zellij keymap parity. Custom maps decode ordinary UTF-8
scalar keys across client packets and forward unbound keys with their original
bytes. Incomplete UTF-8 keys are transient input: changing modes or losing the
writer discards them. Grapheme and modifier handling is not complete Zellij parity.

Actions are plists prefixed by their kind. A batch accepts up to 32 actions,
with at most **one primary session action**, optionally followed by one
`:set-keymap` transition, plus contributions (including at most one `:set-state`). A map transition must
follow the primary action when both are present; a map transition by itself is
also valid. Status contributions may accompany either form. The complete batch
is validated before application, so an invalid action, map target, order, or
count produces no effects. Ekko applies the primary action first, then commits
the trailing mode transition and status contributions. If the primary action
raises, those trailing mode and status changes are not committed; effects the
primary action made before raising are not rolled back.

| Action | Arguments |
| --- | --- |
| `:set-state` | `:value` replaces the invoking component’s state; `nil` deletes it |
| `:set-layout` | `:tree` of `(:columns percentage first second)` / `(:rows percentage first second)` branches and existing stable pane-ID leaves; every pane exactly once |
| `:split` | `:axis :columns` or `:rows`; optional `:pane` target ID and `:argv` list |
| `:focus` | `:pane` stable ID |
| `:close` | Optional `:focus` surviving pane ID; otherwise retains the existing fallback policy |
| `:rename` | `:text`; optional `:pane` |
| `:resize` | `:delta` percentage points on the nearest split |
| `:set-keymap` | `:name` registered custom keymap |
| `:status` | `:text`, owned by the returning component |
| `:pane-note` | `:pane` ID, printable `:text` up to 512 characters, `:sgr` up to 16 integers 0–255, `:duration` 1–60000 milliseconds |
| `:send-input` | `:bytes` list of at most 4,096 octets, sent literally to the focused PTY through its bounded input queue; a primary session action |
| `:copy-move` | `:delta` rows |
| `:copy-edge` | `:edge :start` or `:end` |
| `:focus-next`, `:zoom`, `:swap`, `:detach`, `:stop`, `:reload`, `:help` | None |
| `:copy-mode`, `:copy-mark`, `:copy-selection`, `:copy-exit`, `:copy-search`, `:copy-search-next`, `:paste-buffer` | None |

Pane notes carry temporary presentation data; a decoration hook decides how to
render them. Each component can retain one note per pane. A new value replaces
that owner's note, while repeating identical active text and style preserves its
original deadline. Expiry changes `:pane-notes` and notifies its declared readers;
notes also disappear on pane close or registry replacement. Snapshot data omits
timers, and rendering remains ordinary Lisp policy. Commands may return notes
alongside a primary action and mode transition. Hooks cannot emit notes, avoiding
self-renewing timer loops. Later registered owners appear later in the snapshot,
so a renderer can choose its own precedence rule.

For example, a command can zoom the focused pane and return to Normal mode in
one validated batch:

```lisp
(ekko/extensions:register-command :component :modes :name "zoom-and-normal"
  :handler (lambda (snapshot event)
             (declare (ignore snapshot event))
             (list (ekko/extensions:action :zoom)
                   (ekko/extensions:action :set-keymap :name :normal)
                   (ekko/extensions:action :status :text "Normal mode"))))
```

The mode and status actions complete only after `:zoom` succeeds. This
guarantees the transition's observable state without making the primary action
itself a general transaction.

## Durable component state

A component may persist small values across daemon restarts. The snapshot key
`:store` holds `(namespace . entries)` pairs loaded at daemon start; a
component's namespace is its lowercased owner id and its entries are an alist
of `(key . value)`. The owner writes with
`(:store-set :key KEY :value VALUE)`; a nil value removes the key, values are
validated like component state, and one namespace is limited to 16 KiB.
Declare `:store` in the component's `:reads`.

```lisp
(let ((entries (cdr (assoc "welcome" (ekko/extensions:value snapshot :store)
                          :test #'equal))))
  (unless (assoc "seen" entries :test #'equal)
    (list (ekko/extensions:action :store-set :key "seen" :value "0.1.0"))))
```

Writes are applied with the rest of an accepted action batch and replace the
namespace file atomically through a temporary file and rename. They are never
staged: a rejected candidate reload and a failed startup write nothing, and
removing the owner keeps its durable values. The directory is
`$EKKO_STORE_DIR` when set, otherwise `$XDG_STATE_HOME/ekko/store`, otherwise
`~/.local/state/ekko/store`. A nondefault daemon instance uses `instances/NAME/`
under the default store directory. An explicit `EKKO_STORE_DIR` always wins;
pointing two instances at that same override deliberately shares its files.
The store directory is created private to the current user. Files
are read back with `*read-eval*` disabled and an unreadable file is skipped
with a warning. A failed write is reported through the session's last error
(visible in `inspect`) and never stops the daemon, so storage stays best-effort.

## Panes and copy mode

A session supports up to 16 panes in mixed row/column split trees. IDs remain
stable when panes close. If a terminal becomes too small for the tree, Ekko shows
only the focused pane and restores the layout when space returns.

```sh
ekko split --session workspace columns          # default shell
ekko split --session workspace rows htop         # explicit executable
ekko rename --session workspace 'build output'
ekko command --session workspace close
ekko buffer workspace > selection.txt
```

Default bindings use the desktop modes: Ctrl-p opens pane controls (r/d/n new,
Tab cycle/restore, h/j/k/l focus, m minimize, f maximize, c rename, x close).
Ctrl-h opens move controls, Ctrl-o opens session controls, Ctrl-g locks/unlocks,
and Ctrl-q stops the session. Commands such as `copy-mode`, `paste-buffer`,
`resize-left`, `resize-right`, `swap` and `reload` also remain callable through
`ekko command` or custom bindings.

Copy mode freezes a styled snapshot while the application continues running.
Use `j`/`k` or arrows to move, `b`/`f` or PageUp/PageDown to move 20 rows, `g`/`G`
for the first/last row, Space to mark, Enter or `y` to copy whole lines, `/` to
search, `n` for the next match, and `q` or Escape to exit. Mouse-wheel movement
scrolls the copy cursor. Graphics are hidden during copy mode and restored on exit.

History retains main-screen full-width scrolling rows, bounded by 10,000 rows
and an 8 MiB cell-accounting budget per pane. Alternate-screen output is not
added to history. Copy snapshots contain text only; they do not reflow on resize.
The daemon buffer holds up to 1 MiB and survives client replacement. Pasting
honors the application's bracketed-paste mode and currently accepts up to 60,000
UTF-8 bytes, within the pane's bounded input queue. Export larger selections with
`ekko buffer`; host clipboard integration and character-level selection remain future work.

The `:set-layout` action replaces the current tiled tree without spawning or
closing applications. Branch percentages are integers 1–99; every existing
pane ID must appear exactly once. Invalid trees reject the complete action
batch. Focus and fullscreen state are preserved: changing the tiled tree while
fullscreen takes effect behind the focused pane until fullscreen is cleared.
The tree is daemon-owned session state, so it survives detach, component
removal, and reload; `:initial-layout` is only a startup default. Profiles can
compute layout choices from their detached `:layout` and `:panes` snapshots
and return this ordinary action. The core performs geometry and PTY resizing.

A custom keymap's `:unbound` may also be a registered command-name string.
Unbound keys invoke that command through the ordinary isolated worker and
validated action path. The event includes `:key` (Unicode code point, semantic
key keyword, or `nil`) and `:bytes` (the original octet list). Bound commands
win first. Subsequent input waits for the current command result, so a mode
transition affects remaining bytes in the same input packet. Referencing a
missing fallback command rejects configuration reload before installation.

For a named fallback, bracketed paste delivers one event with `:key nil`,
`:paste t`, and the original payload `:bytes`; wrapper bytes are excluded and
bound-key lookup is bypassed. Payloads are bounded to 4096 raw bytes. Oversized
pastes report an error and are discarded through the closing wrapper, without
sending them to pane applications. `:ignore` consumes paste, while `:forward`
retains the normal application paste path.

`:set-state` belongs to the command's registered component; it cannot write
another owner's entry. Values may be nil/t, keywords, integers, strings, and
proper lists of those values. Validation bounds each value to 1024 visited
values and 16 nesting levels and all retained state to 16 KiB when printed and
UTF-8 encoded. Cycles, dotted lists, and host objects are rejected. Snapshots
copy nested strings as well as list structure. Inspect exposes entries as
`component-state` objects with `owner` and `value` fields.

Component state survives detach, worker restart, and reload while that owner
remains registered. Removing the owner discards its state. Invalid batches or
failed registry validation leave the prior state intact. State updates may
accompany one primary action and one mode transition, and commit only after the
primary succeeds. Change hooks cannot emit state updates. This state is visible
to other components that declare the snapshot key; it is not private storage.

`(ekko/extensions:display-width value)` measures a character or string in terminal
cells. The pure public function is available in isolated workers as well as
regular and bare builds. It uses the ordinary Unicode scalar widths from the
MIT-licensed `unicode-width` 0.1.10 tables: ambiguous scalars occupy one cell,
wide scalars two, combining marks zero, and controls zero. String measurement
adds scalar widths; it does not perform grapheme or emoji-ZWJ shaping. A zero
width does not make a control character valid decoration text.

The VT, copy clipping, and decoration clipping use the same measurements.
Profiles retain their own fitting/truncation policy. `checks.x86_64-linux.text-width`
compares every Unicode scalar with the pinned Rust crate as a build-time oracle;
Ekko itself has no Rust or Zellij runtime dependency for text measurement.

For viewers that announce original stdin reads, named fallback events also have
`:read-bytes`. The first completed semantic event consumes all accumulated raw
read bytes; later events from that read carry explicit `nil`. This field is
independent of `:bytes`, which always describes the decoded key. Bound or
ignored keys, mouse events, focus events, and paste routing also consume read
context. Paste callbacks retain their dedicated `:paste` and payload `:bytes`
contract. Legacy direct key packets omit `:read-bytes` entirely.

The client retains incomplete escape input across reads; the daemon retains
incomplete modal UTF-8 keys. Completely filtered terminal replies do not become
later key context. Original read buffers are bounded to 64 KiB, and pending
input retains the existing 64 KiB transport bound, including read metadata.
Framing follows the attached writer, survives ordinary viewport resize, and
resets on replacement or disconnect. Deferred input from a replaced writer is
discarded. Profile removal can clear an unfinished key but does not change the
writer's framing capability.

Wire 15 prefixes packet 16 original-read context with the connection binding generation. Framing belongs to the
originating view and cannot cross a session switch. The optional profile uses
`(getf event :read-bytes (getf event :bytes))` for complete-read rename batching
and explicit key-event fallback.


Viewer exit text is a static, owner-scoped option. Successful configuration
reload publishes its replacement to the attached viewer; removal publishes
`nil`, and rejected reload preserves the previous text. The viewer retains the
latest accepted scene value so it can display it after the daemon disconnects.
It prints the literal text after normal terminal restoration; it does not
interpret markup or embedded control sequences. Both registration and the viewer
validate it. The ordinary Zellij profile supplies its farewell wording; the
runtime contains no profile-specific exit message. `checks.viewer-exit` verifies
regular/bare replacement, rejected control characters, owner removal, unchanged
child PIDs and restored host termios. Failure-specific reference messages and
multi-client termination behavior remain separate parity work.


### Pointer selection actions

`:copy-point :pane ID :x COLUMN :y ROW :start BOOLEAN` selects in the named
pane's frozen styled viewport (zero-based cells). The first point captures history;
`:start t` replaces the anchor. Coordinates are validated and capped to the
runtime's maximum viewport. `:copy-scroll :pane ID :delta ROWS` moves the frozen
viewport; delta is an integer between -300 and 300. Pointer input translates to
these same validated actions. Selection is daemon-owned pane state, cleared by
`:copy-exit` or pane teardown; it does not resize PTYs or alter live VT contents.

`:copy-selection` copies either the character range or the existing keyboard
line selection. Both use the session buffer and a bounded clipboard event.
Wire packet 23 contains at most 1 MiB of UTF-8 text and is sent only to the
originating attached view. Its client base64-encodes it into OSC 52 for the
system clipboard. No clipboard reads or shell subprocesses are involved.
Other attached clients do not receive the selection export.


Selection shares the live cell-to-run renderer. Captured VT/history rows retain
immutable text and SGR cells; the highlight changes the selected background only,
leaving foreground colour, bold and other formatting intact. Plain text is used
for clipboard export and the search index, not for repainting captured output.

### Desktop controls

Desktop styling is installed by default, under the removable `:defaults` owner.
`examples/profiles/desktop-style.lisp` holds the shared style functions, and
`examples/profiles/desktop.lisp` demonstrates installing them in `ekko-bare`.
It uses centered window headers and a fixed bottom taskbar. The yellow `_` minimizes without
stopping the process; green `□` maximizes/restores; red `×` closes the pane.
Click an active dock entry to minimize it, a minimized entry to restore it,
or another window’s entry to focus it. Each window keeps a stable accent color
in its border, dark titlebar and taskbar entry. The dock reserves a terminal row, so
it never covers application content. Existing Ctrl-p pane and Ctrl-o session
bindings remain available; Ctrl-p then m minimizes, and Ctrl-p then Tab
cycles through all panes, restoring minimized ones. Very narrow headers omit
the buttons.

The dock shows a wall clock at its right edge while Normal mode is active, and
mode hints instead while another mode is active. A `●` marks a window that
produced output while unfocused; focusing it clears the mark. When every window
is minimized, a centered backdrop shows the session name, the clock and restore
hints. When the entries do not fit the viewport, a `+N` chip opens a most-recent
window list (the same list **Alt-Tab** opens). Wheel over a taskbar entry cycles
focus; middle-click closes that window.
Double-click a titlebar to maximize or restore its window.

Decoration spans accept an optional `:action`, one of `(:focus :pane ID)`,
`(:minimize :pane ID)`, `(:restore :pane ID)`, `(:zoom :pane ID)` or
`(:close :pane ID)`. These use the normal validated action path. A left press
and release on the same control activates it; dragging away cancels it.
Only reserved chrome cells accept these actions. Removing the decoration owner
removes its hit targets too. No viewer protocol changes are needed. Any
reserved span may declare `:hover-sgr` to repaint itself while the pointer is
over it. A span may instead declare `:wheel-command`, called with `:direction`
(-1 for wheel-up, 1 for wheel-down) and the span's `:arguments`, or
`:middle-command`, called with the span's `:arguments`.

`:minimize` and `:restore` are public actions; `:zoom` and `:close` now also accept
`:pane`. The pane snapshot and `ekko inspect` expose `:minimized`. Minimization
survives detach/reattach and preserves the original split tree and PTY size.
Explicit focus restores a minimized pane, including keyboard focus cycling.

### Context menus and motion

In the UI, a **window** is a terminal pane, and its button in the bottom
**taskbar** is a **taskbar entry**. API names continue to use `pane` and pane IDs.
These are tiled windows, not tabs or floating native operating-system windows.

Right-click a titlebar, border or taskbar entry for window actions. Right-click
empty taskbar space for new-window and session actions. Applications keep their
own right-click handling inside their content. Menus support hover, Up/Down,
Tab, Enter and Escape; clicking outside dismisses them without sending that
click to the application.

Decoration spans can declare `:command` for left-click or `:context-command`
for right-click, plus a shared `:arguments` list of strings. A context command
receives `:x` and `:y` in viewport cells and returns `(:show-menu :x X :y Y
:spans SPANS)`. Menu spans use local coordinates and may specify `:hover-sgr`;
reserved chrome spans use the same field for pointer hover. They use the same
validated actions/commands as other controls. The host clamps
the popup to the viewport and owns only its transient input state. Reload,
layout changes and detach discard it. Nested popups are not supported.

The desktop enables a 160 ms outline transition for minimize, restore, maximize and window placement. It is a decorative outline, not a scaled image of the application:
logical layout and PTY sizing commit once. Transient strokes are clipped away
from graphics content to preserve image placement and avoid retransmission.
Set `:window-animation-ms` to `0` on a later configuration owner to disable
motion; permitted durations are 0–250 ms.


### Window placement

Drag a titlebar to a tiled window's center to exchange positions, or to its
edge to split beside it. Dropping elsewhere floats the window. A floating
window moves freely away from snap edges; its sides, bottom, all four corners,
and the short top-edge handle beside its upper-left corner resize it.
Shared tiled borders resize their split while keeping both sides tiled; the
bottom edge of the upper window or the top corners of the lower window resize
a row split. Outside tiled edges and maximized tiles do not resize.
Hover highlights resize handles, and `:hover-sgr` on a menu entry or other
chrome span repaints it under the pointer. Escape cancels the gesture. The preview
changes while held; application dimensions commit once on release. Dragging to
the outermost content cell snaps there instead of onto a window below: the top
cell maximizes, the left or right cell takes half the content area.

Window menus expose **Float window** and **Tile window**, with **Ctrl-p, then t**
shown as the keyboard toggle. Floating placement
and stacking survive viewer replacement. Minimize keeps the process and its
placement; restoring raises the window. The taskbar remains reserved space.

Public actions are `(:float :pane ID :rect (X Y WIDTH HEIGHT))` (rectangle
optional), `(:tile :pane ID)`, and `(:place-window :pane ID :target ID :edge EDGE)`.
Edges are `:left`, `:right`, `:top`, `:bottom`, or `:center` (exchange).
Decorations declare `:pane ID` for stacking and optional `:drag` handles:
`:move`, `:left`, `:right`, `:top`, `:bottom`, `:top-left`, `:top-right`,
`:bottom-left`, `:bottom-right`. Cardinal handles on tiled panes resize the
visible split at that edge, committing through the public `:set-layout` action.
Nested and hidden-pane splits retain their other ratios. Geometry changes,
profile replacement, and viewer detachment clear active gestures and hover.
Pane snapshots and `inspect` expose `:floating` as a rectangle or nil.
