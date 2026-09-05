# Current module contracts

The live preview has one daemon per named session, one attached interactive
client, and up to 16 independently owned application PTYs. The daemon stores
text screens, compressed inline image assets, and immutable local frame snapshots; a client reconstructs presentation on
attach. Applications never write to the outer terminal directly.

| Module | Responsibility |
| --- | --- |
| `platform.c`, `platform.lisp` | Controlling PTYs, argv execution, nonblocking I/O, poll, terminal modes, local sockets, process groups, bounded zlib calls |
| `vt.lisp` | Incremental byte parser, basic text cells/rendition, cursor, alternate screen, modes, virtual terminal replies |
| `history.lisp`, `layout.lisp` | Bounded main-screen row history and pure mixed split-tree geometry |
| `extensions.lisp`, `builtins.lisp` | Public declaration/snapshot/action API and default commands/keymaps |
| `worker.lisp`, `commands.lisp` | Worker transport and deadlines, atomic config replacement, validated actions, copy mode |
| `graphics.lisp` | Pane-owned RGB/RGBA uploads, validation, compressed assets, native placements, deletion, quotas |
| `assets.lisp` | Daemon-owned raw frame snapshots, byte quota, reference ownership and crash reclamation |
| `wire.lisp` | Length-prefixed local IPC, wire version 3, bounded buffers, scene acknowledgements, private session directory |
| `server.lisp` | Reactor, pane processes, layout application, focus, input routing, snapshots, status, shutdown |
| `client.lisp` | Host input decoding, dimensions, text/Kitty rendering, per-client image IDs, terminal restoration |
| `geometry.lisp`, `presentation.lisp` | Rational clipping and attachment identity/transaction contracts, also exercised by synthetic experiments |

The IPC sends binary inline assets or local snapshot filenames followed by a complete scene snapshot. The client
stages assets until the associated snapshot arrives. Only one scene is in flight per client. The client acknowledges after its output
drains and all local-file uploads receive host read acknowledgements. Later scene
revisions coalesce in daemon state while it waits. Slow clients have an 8 MiB
queue limit and a ten-second presentation timeout. Graphics frames use
bounded 16 KiB control strings, a 32 MiB upload/decoded-image limit, up to 64
images and 128 MiB of retained asset data per pane, and a ten-second incomplete
upload timeout. Layout changes resize the PTYs and clip old frames until the
applications replace them.

The renderer serializes complete Kitty uploads and uses synchronized updates.
It caches complete text rows and writes only changed rows, resetting rendition
before clearing each changed row. It avoids an outer clear-screen command,
because Kitty's clear screen also deletes graphics belonging to unchanged panes.
Unchanged image generations reuse their host image and placement IDs when their
crop, position, or cell dimensions change; only new generations upload pixels.

Ordinary input travels in byte runs, with escape sequences kept as individual
events. The daemon owns prefix interpretation, including commands embedded in
input batches. Output queues maintain a tail pointer for constant-time insertion.
The client polls until its next 200 ms size check or 40 ms ESC deadline; the daemon
uses upload/peer deadlines and a one-second child-reaping interval. Ready file
descriptors wake either loop immediately. Status includes cumulative daemon
allocation and GC time counters for performance measurement.

Detach drops client caches and leaves the daemon, PTYs, and images alive.
Shutdown closes sockets/PTYS and signals owned process groups with a bounded
escalation period. Socket startup uses an exclusive lock; peers must have the
same OS UID. There is no network listener or eval RPC.

Configuration is trusted Lisp in a separate worker process. The daemon sends
only declared metadata snapshots; callbacks return validated actions. Registration
replacement reconstructs owned contributions while preserving session state.
Hooks react only to declared context changes and may contribute status. Candidate
loads run alongside the reactor; failed loads leave the active worker in place.
A callback deadline kills the worker and reconstructs it from the accepted init
text. Details and current restrictions are in [customization](customization.md).

Command dispatch is asynchronous. Input following a command is deferred in a
bounded queue, then replayed after the action commits, so focus/split commands
route trailing bytes to the resulting pane. Main-screen history and frozen copy
snapshots live in the daemon. Copy mode suppresses graphics presentation without
releasing the application's image ownership; exit or reattachment reconstructs it.

Local transport uses the existing Kitty `t=s` ingress and `t=f` egress protocols.
Ingress currently accepts complete uncompressed RGB/RGBA shared objects only;
compressed shared objects and `S/O` subranges are rejected. The OS adapter opens
a same-UID regular shared object, checks exact size, unlinks its name, and copies
through a fixed 64 KiB buffer into an exclusively created mode-0600 file. No
producer-owned path is forwarded to a client. The daemon never compresses or
Base64-encodes those pixels. IPC sends only dimensions, identity and filename.

Each current image owns one snapshot reference. Each published scene retains
its visible local snapshots until acknowledgement or peer teardown. Replacement
and pane-scoped deletion release the image reference; they cannot invalidate an
in-flight frame. The client probes file access using a query against its first
snapshot; explicit rejection or a one-second timeout chooses inline fallback.
Host upload replies are matched to outstanding image/placement IDs and never
forwarded to children. A failed upload terminates the attachment explicitly.

All current and leased raw snapshots share a 256 MiB daemon quota, in addition
to the per-pane quota. `status` exposes `snapshot_bytes` and pane `local_images`.
Files live in a mode-0700 directory beside the session socket, normally on the
runtime directory's tmpfs. A daemon crash can leave snapshots there; startup
under that session's exclusive lock reclaims them. Normal replacement, deletion,
client death, and shutdown release references and unlink files. This is a bounded
copy path, not zero-copy GPU sharing. Filesystem traffic and Kitty's pixel upload
remain. Version 3 adds configurable status metadata and command IPC; older attachments are rejected.
