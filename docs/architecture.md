# Current module contracts

One daemon owns all sessions, application PTYs, terminal state, images, and the
durable component store. Many clients can attach concurrently. Sessions group
applications; they are not separate daemon processes. Each client renders an
independent daemon-owned view. Applications never write to the outer terminal.
The default socket is `$XDG_RUNTIME_DIR/ekko/ekko.sock`; `--instance NAME` provides
explicit isolation. Pane IDs are daemon-global and are not reused during its life.

| Module | Responsibility |
| --- | --- |
| `platform.c`, `platform.lisp` | Controlling PTYs, argv execution, nonblocking I/O, poll, terminal modes, local sockets, process groups, bounded zlib calls |
| `vt.lisp` | Incremental byte parser, basic text cells/rendition, cursor, alternate screen, modes, virtual terminal replies |
| `history.lisp`, `layout.lisp` | Bounded main-screen row history and pure mixed split-tree geometry |
| `extensions.lisp`, `builtins.lisp` | Public declaration/snapshot/action API and default commands/keymaps |
| `worker.lisp`, `commands.lisp` | Worker transport and deadlines, atomic config replacement, validated actions, copy mode |
| `graphics.lisp` | Pane-owned RGB/RGBA uploads, validation, compressed assets, native placements, deletion, quotas |
| `assets.lisp` | Daemon-owned raw frame snapshots, byte quota, reference ownership and crash reclamation |
| `wire.lisp` | Version-15 routed IPC, input binding generations, bounded peer queues, scene acknowledgements, private daemon directory |
| `daemon.lisp` | Global session registry, reactor, asynchronous creation, routing, session switching and shutdown |
| `views.lisp` | Real per-view focus, interaction, geometry, selection state and captured command origins |
| `layout-policy.lisp` | Owned layout-provider requests, detached dependencies, validated placements and stale-result rejection |
| `server.lisp` | View projection, canonical PTY sizing, input routing, scenes and status |
| `client.lisp` | Host input decoding, dimensions, text/Kitty rendering, per-client image IDs, terminal restoration |
| `geometry.lisp`, `presentation.lisp` | Rational clipping and attachment identity/transaction contracts, also exercised by synthetic experiments |

The IPC sends binary inline assets or local snapshot filenames followed by a complete scene snapshot. The client
stages assets until the associated snapshot arrives. Only one scene is in flight per client. A visible pane inside an application-synchronized update
frame (DECSET 2026) holds publication until the application closes the frame or a one-second deadline expires, so a batched
repaint presents once instead of frame by frame. The client acknowledges after its output
drains and all local-file uploads receive host read acknowledgements. Later scene
revisions coalesce in daemon state while it waits. Each client has an independent
8 MiB queue limit; an outstanding frame does not block other clients. Session
switching waits for the old frame's terminal acknowledgement before releasing
its image leases. Graphics frames use
bounded 16 KiB control strings, a 32 MiB upload/decoded-image limit, up to 64
images and 128 MiB of retained asset data per pane, and a ten-second incomplete
upload timeout. Providers return full logical rectangles and a camera separately.
Panning clips text, images and cursors without resizing applications to visible
slivers. PTY dimensions are the componentwise minimum of participating views'
full content dimensions, including explicit hidden provider placements.
With no participating view, a pane retains its size.
The oldest participating view supplies canonical cell metrics. Images retain
native pixels, not arbitrary scaling; each client clips them using its host metrics.

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

Detach drops client caches and cancels transient input, not applications. A
session's home view retains focus and copy state for default reconnect; other
views have separate identities. Stopping one session leaves other sessions and
the empty daemon alive. Shutdown closes sockets/PTYs and signals owned process
groups with a bounded escalation period. Socket startup uses an exclusive lock;
peers must have the same OS UID. There is no network listener or eval RPC.

Session creation is staged in the reactor. Loading or initializing one session
must not stop existing PTY and client service. Each creation captures its cwd,
environment and config path. Workers and subsequent panes use that launch
context; only child processes change cwd/environment, never the daemon.

Configuration is trusted Lisp in a separate worker process. The daemon sends
only declared metadata snapshots; callbacks return validated actions. Registration
replacement reconstructs owned contributions while preserving session state.
Hooks react only to declared context changes and may contribute status. Candidate
loads run alongside the reactor; failed loads leave the active worker in place.
A callback deadline kills the worker and reconstructs it from the accepted init
text; a change hook has a longer deadline than a command, and only a hook that
misses three consecutive deadlines is disabled, so one late chrome repaint does
not take the decorations down. Details and current restrictions are in
[customization](customization.md).

Command dispatch captures the origin view, epoch, config generation and default
target. A late callback cannot resolve another client's focus. Input following a
command waits in that view's bounded queue and resumes through its committed
state. Detach or switching invalidates pending input and old-origin actions.
Each connection also has an acknowledged input binding generation. The client
cancels old parser and paste fragments at cutover; every input packet retains
its generation, so a late old-view event cannot enter the new session.
PTYs/history are shared; focus, modes, camera, copy, selection and component UI
contributions are per-view. Copy mode hides graphics only in that view without
releasing the application's image ownership.

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
Files live in a mode-0700 directory beside the daemon socket, normally on the
runtime directory's tmpfs. A daemon crash can leave snapshots there; startup
under that daemon's exclusive lock reclaims them. Normal replacement, deletion,
client death, and shutdown release references and unlink files. This is a bounded
copy path, not zero-copy GPU sharing. Filesystem traffic and Kitty's pixel upload
remain. Every peer begins with a version-15 route naming its session and optional
view. Old versions and unversioned controls are rejected explicitly.
