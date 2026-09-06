# Startup UI requirements from pinned Zellij 0.43.1

This source investigation uses the pinned store checkout
`/nix/store/2q437kxp07ki50dkh6a4nmmcc4nlylqw-source`. It does not claim that Ekko
implements release notes, startup tips or floating plugin panes.

In `zellij-server/src/lib.rs:745-787`, explicit tab layouts follow a separate
startup path. For the template/no-tab path, the setup wizard takes precedence,
then release notes, then startup tips. The release-notes predicate
(`lib.rs:1719-1746`) skips welcome-screen layouts, explicit false configuration,
an existing seen marker, and failed writes. Otherwise it writes the seen marker
before scheduling the About plugin. The version-specific marker is
`ZELLIJ_CACHE_DIR / VERSION / seen_release_notes`
(`zellij-utils/src/consts.rs:74-75`). Closing the popup is not what creates it.

The plugin alias is `about`, with ordinary configuration
`is_release_notes=true` (`lib.rs:1697-1707`). Placement first tries a half-size
rectangle centered using rounded quarter offsets, then corner placements with
two-cell offsets (`floating_pane_grid.rs:796-870`). Both dimensions must be at
least five cells (`:936`, `tab/mod.rs:140-141`). This accounts for the 20×8
reference's missing popup: half-height four cannot fit. Importantly, its seen
marker was already written. This is a source-derived explanation, not permission
to drop startup input from comparisons.

`default-plugins/about/src/main.rs` subscribes to key, mouse, mode, command,
tab and configuration-write events. Its title is `Zellij VERSION Release Notes`.
Escape closes the main page; Escape in a subpage returns to the main page.
Enter activates selection, mouse clicks navigate/open links, hover changes
selection styling, and startup tips have additional navigation/reconfiguration
behavior. The main release-notes page includes five topics, changelog and
sponsor links, and context-sensitive help (`pages.rs:19-114`). A static image
plus Escape would not cover this plugin behavior.

## Public mechanism requirements

The shared runtime now has the existing Finix opaque-decoration, literal-input,
and copy-fallback capabilities. They are ordinary public APIs, with no Zellij
names or behavior embedded in the renderer. They permit testing an owned opaque
Lisp UI over text/graphics, but do not provide floating panes, pointer interception,
startup state initialization, or durable plugin state.

The next bounded mechanism must establish initialization before input routing,
with immutable viewport/state inputs and validated owner-scoped effects. It must
also preserve transactional reload: callbacks from a candidate configuration
cannot change live PTYs, keymaps, decorations or persistent state before commit.
The current reload path validates registration data on a detached session, then
swaps the worker and schedules ordinary decoration hooks. A startup callback
cannot be added as an unchecked post-commit side effect.

Persistent “seen” state needs a public storage contract, including namespaces,
atomic writes, failure reporting, reload/owner-removal semantics and the preserved
state set. Direct profile file writes would evade those guarantees. Pointer
routing and floating/selectable UI geometry need separate public contracts too.
Tests must cover fresh cache, existing marker, unwritable cache, tiny viewports,
explicit-tab and welcome layouts, dismissal, subpage navigation, failed reload,
unchanged child PIDs/geometry, and later startup-tip selection. Keep all raw
startup input, cells, native terminal exports and screenshots in the gate.
