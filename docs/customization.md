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

`Ctrl-b r` reloads with the default bindings. Editing a file takes effect after
reload, without rebuilding or restarting applications. A bad reload reports an
error and keeps the previous configuration. Each daemon remembers its startup
configuration path; `config check` uses the caller's environment.

These are trusted Lisp files with your OS permissions, like an Emacs init file.
They run in a separate process. This boundary isolates host state and lets the
daemon terminate a runaway callback; it is not an arbitrary-Lisp security sandbox.
Configuration loading has a five-second deadline; each callback has a 50 ms
wall-clock deadline. Messages are limited to 64 KiB and the init file to 32 KiB.
Load local helper files with `load` if needed. Reloading a worker also reloads
those files; recovery retains the accepted init text, not copies of its dependencies.

## Public API, version 1

The public package is `ekko/extensions`, supplied by the ASDF system of the same
name. Its exports are:

```lisp
(api-version) ; => 1
(register-component :id :name :api-version 1 :reads '(:focus) :handler function)
(unregister-component :name)
(register-command :component :name :name "command" :handler function)
(bind-key :component :name :key "v" :command "command" :map :prefix)
(set-option :component :name :name :prefix :value "C-a")
(value snapshot :focus)
(action :rename :text "work")
```

Registration calls belong in init loading. Handlers receive `(snapshot event)`
and return a list of actions, or `nil`. Commands receive `(:arguments (...))`;
change hooks receive `(:type :change)`. The snapshot contains only the component's
declared keys. `value` rejects undeclared reads. Mutating the detached snapshot
cannot change daemon state. No host objects, file descriptors, or image buffers
cross this boundary.

| Snapshot key | Value |
| --- | --- |
| `:session` | Session name |
| `:focus` | Stable focused pane ID |
| `:panes` | Plists with `:id`, `:label`, `:pid`, `:cols`, `:rows`, `:exit` |
| `:layout` | Pane ID leaves; branches `(axis percentage first second)` |

A change hook runs initially and when one of its declared keys changes; changes
can coalesce while a handler runs. Stale hook results are discarded. Hooks may
only return `:status` contributions, preventing reactive action loops. A timed-out
hook is disabled until explicit reload. After worker failure, Ekko reconstructs
registrations from the accepted init source; worker-local variables reset.

Components own their commands, bindings, options, and status contributions.
Later components shadow earlier ones. Removal or reload reconstructs those
contributions, restoring underlying defaults. The preserved state is the session's
PTYs, labels, layout, history, copy selections, and buffer. User-invoked commands
change that session state; removing their component does not undo past user actions.
`inspect` lists registrations with owners, contributions, disabled hooks, and the
last error. Builtins use the same API in `ekko/builtins`; `ekko-bare` is packaged
and tested with no builtins and an externally loaded command.

| Option | Value |
| --- | --- |
| `:prefix` | `"C-a"` through `"C-z"`, or integer 1–26 |
| `:shell` | Executable argument list for new panes; defaults to `$SHELL -i` |
| `:status-text` | Up to 512 characters |
| `:status-style` | SGR integer list, e.g. `'(0 37 44)` |

Bindings use maps `:prefix` or `:copy`. Keys are single-character strings, integer
code points, control names, or `Tab`, `Enter`, `Escape`, `Up`, `Down`, `PageUp`,
`PageDown`, `Home`, `End`. A `nil` command unbinds a key in that component. Prefix
followed by itself sends the literal control byte to the application.

Actions are plists prefixed by their kind. A batch accepts up to 32 actions,
with at most **one session action** plus status contributions. Validation happens
before application; unsupported actions or arguments reject the whole batch.

| Action | Arguments |
| --- | --- |
| `:split` | `:axis :columns` or `:rows`; optional `:argv` list |
| `:focus` | `:pane` stable ID |
| `:rename` | `:text`; optional `:pane` |
| `:resize` | `:delta` percentage points on the nearest split |
| `:status` | `:text`, owned by the returning component |
| `:copy-move` | `:delta` rows |
| `:copy-edge` | `:edge :start` or `:end` |
| `:focus-next`, `:zoom`, `:swap`, `:close`, `:detach`, `:stop`, `:reload`, `:help` | None |
| `:copy-mode`, `:copy-mark`, `:copy-selection`, `:copy-exit`, `:copy-search`, `:copy-search-next`, `:paste-buffer` | None |

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

Default prefix bindings: `%` splits columns, `/` splits rows, `x` closes, Tab
cycles focus, `1`–`9` select by pane order, `z` zooms, `s` swaps with the next pane,
`<`/`>` resize, `[` enters copy mode, `]` pastes, `?` lists prefix bindings.

Copy mode freezes a plain-text snapshot while the application continues running.
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
