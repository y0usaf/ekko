# ADR-003: owned local frame snapshots and acknowledged presentation

Status: implemented, 2026-09-05

CookUnity-sized inline frames spent most of their transport budget compressing,
Base64 encoding/decoding, and traversing PTYs. A fresh 1274 × 1368 replay measured
97.4 ms p95 frame receipt before this change. The existing browser already has
POSIX shared-memory transport; the launcher forced inline because Ekko rejected
local transfers. Removing another array copy would leave that expensive path.

Use the [Kitty local transport protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/#the-transmission-medium).
The browser negotiates `t=s`. Ekko takes a bounded, immutable snapshot owned by
the daemon; IPC carries identity, geometry, and the snapshot filename. The viewer
probes whether the host can read that file, then presents with `t=f`, otherwise
falling back to the existing compressed inline renderer. Child `t=f` is still
rejected: a reusable producer file is not retained scene state. The browser's
shared-memory implementation creates distinct objects when reusing names.

Do not forward producer filenames or retain their mutable buffers. The daemon
owns current image references and leases for a single outstanding scene. The
viewer acknowledges the scene only after draining output and receiving all
expected local upload replies. Later revisions coalesce while it waits. Reply
IDs belong to the attachment and never route to children. Lost acknowledgements
terminate that attachment after a bounded interval. Deletion/replacement and
client death release the corresponding references. A crashed daemon's files are
reclaimed on the next startup of that session under its exclusive lock.

The snapshots have per-pane and global byte quotas, including outstanding leases.
They avoid allocating whole raw frames on the Lisp heap. They still require a
copy and filesystem storage (normally runtime tmpfs); this is not GPU zero-copy.
Raw snapshot files are larger than compressed assets. Full frames still pass
through the browser's compositor and Kitty's image upload path. Damage-region
rendering and GPU texture sharing would be separate, measured changes.

IPC version 2 adds local asset metadata and mandatory scene acknowledgements.
This is a breaking change; old attachments receive an explicit version error.
The old server can still be stopped with its control command. New workspaces need
new session names; attaching does not replace a daemon's executable.

The pinned browser has a separate environment ownership bug: transport selection
reads process-global environment even when a request supplies session settings.
The checked-in dependency patch reads its existing `SessionEnv` instead. Nix
applies that patch to pinned upstream source; the launcher passes the resulting
source to terminal-slack's browser build wrapper without changing that checkout
or its lock. The source hash selects a separate browser runtime/daemon. Live
verification opens an explicitly inline session against the new auto-negotiating
daemon and confirms that override remains local to the requesting session.

Evidence and limits: [performance report](../performance.md). Packaged tests
exercise immutable ownership, quotas, invalid shared objects, reply isolation,
slow-host leases, normal and missing-probe fallback, reconnect, and shutdown.
