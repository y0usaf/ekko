# Performance measurements

Run the packaged benchmark with `nix run .#performance -- --seconds 5`.
It emits JSON and creates isolated temporary sessions that it stops on exit.
The existing `.#benchmark` app remains the interactive browser/Slack launcher.

The fixed viewport is 120 × 40 cells, 960 × 640 pixels. Four separate sessions
exercise idle state, scrolling text at 20 updates/second, replacement of a
512 × 512 RGBA image at 5 Hz, and a 16 KiB bracketed paste every half second.
Graphics pixels come from a fixed random seed; each frame carries a monotonic
timestamp. A half-second warmup precedes each measured window. Input probes run
at 10 Hz during text and graphics workloads. The benchmark uses one pane; the
correctness suite separately covers two-pane isolation and layout changes.

CPU is expressed as percent of one core from Linux process counters, independently
for the daemon, client, fixture, and extension worker (when exposed by the binary). RSS is sampled at the start and end, not
measured as a peak. Voluntary context switches help expose idle polling changes
when CPU ticks are too coarse. Daemon allocation and GC counters are cumulative
SBCL counters exposed through `status`; the report subtracts their initial values.
Older executables without these fields report null, rather than estimated values.
Client allocations and individual GC pause distributions are not measured.

Input latency runs from enqueueing a probe in the harness to the child reading
the complete probe. It includes input backpressure and child scheduling/work.
Frame latency runs from timestamping the raw pixels to receipt of the complete
Kitty upload at the synthetic terminal, including fixture compression. It does
not measure physical presentation, and a coalesced frame may never be uploaded.
Output bytes include all text and graphics received during the sampling window.
Percentiles use nearest rank; short runs have few samples. CPU sampling excludes
the final input drain; input latency includes that drain.

`nix run .#performance -- --seconds 5 --direct` runs the same fixture directly in
a PTY for transport comparison. It does not launch Kitty. Real browser/Slack and
GPU/display performance still need separate live measurements. No timing threshold
is a portable correctness gate.

The optimizations keep the existing bounded queues and synchronized graphics
transactions. Text rows are compared before terminal output, queues append in
constant time, input uses byte runs with daemon-owned prefix interpretation,
and idle polls use maintenance deadlines. Placement-only updates reuse a Kitty
image/placement pair, following the [Kitty graphics protocol](https://github.com/kovidgoyal/kitty/blob/d56721e64dd02e9250800f37aa6a98f2d095da75/docs/graphics-protocol.rst).

The smaller alternatives were to keep full redraws and per-byte input. The
measured paste overhead and unnecessary terminal writes justify the small caches
and queue tail; a new binary scene protocol or general rendering framework was
not needed. Complete scene snapshots remain the reconnect mechanism.

## Recorded comparison — 2026-09-04

One five-second sample per workload on an AMD Ryzen 9 7950X, using the pinned
Nix environment. These short samples indicate where work was removed; they are
not broad performance guarantees. The baseline is the existing packaged preview,
and the optimized executable is identified by its store path in the report.

| Measurement | Before | After |
| --- | ---: | ---: |
| 16 KiB paste, p95 child-read latency | 240.68 ms | 5.20 ms |
| Text workload terminal bytes | 1,036,200 | 717,468 |
| Idle client voluntary context switches | 312 | 30 |
| Idle daemon voluntary context switches | 311 | 5 |
| Text input p95 | 0.12 ms | 0.24 ms |
| Graphics input p95 | 44.92 ms | 42.80 ms |
| Graphics frame receipt p95 | 126.85 ms | 132.59 ms |

Text output fell about 31%, and paste latency approached the direct-PTY sample's
5.14 ms p95. Idle CPU rounded to zero in both samples, so context switches are
the useful comparison. Graphics did not show a throughput/latency improvement:
both runs delivered 25 frames, and frame p95 was slightly higher afterward.
The new counters measured about 923 MB allocated by the daemon during the
five-second graphics workload; parsing/encoding remains a candidate for future
profiling. Text client CPU rose from about 0.2% to 0.6% of one core in these short
samples, while daemon CPU fell from 0.4% to 0.2%. Row comparison saves output but
still constructs rows; it does not eliminate scene serialization or comparison.

Raw reports: [before](evidence/performance-before.json),
[after](evidence/performance-after.json), [direct PTY](evidence/performance-direct.json).
The direct report's binary path identifies the supplied comparison executable;
direct mode does not execute it.

Validation: `nix build` and `nix flake check -L` exited zero. Regression coverage
includes focus changes within input batches, raw and Kitty prefix framing,
queue drain/reuse, graphics-only text suppression, shorter-line clearing, cell
size/viewport invalidation, static image zoom/swap without reupload, standalone
ESC delivery, and upload expiration while idle. Existing reconnect, isolation,
shell job control, and terminal restoration checks continue to pass.

## Graphics follow-up — 2026-09-04

An SBCL CPU profile of 30 repeated 1 MiB image uploads found about 32% of samples
inclusive in `POSITION`, primarily from searching the Base64 alphabet. Validation
also re-encoded every decoded chunk, and ingress converted each payload to a
wide-character string. The profiler fed a fixed, seeded, zlib-compressed RGBA
frame through the VT parser and graphics store; it excludes IPC and client output.

The follow-up replaces alphabet searching with a lookup table, validates unused
padding bits directly, and decodes directly from octet ranges. Graphics control
strings now copy runs in bulk while preserving the 16 KiB limit and incremental
ESC/ST state. Outgoing Base64 uses ASCII storage. Bounds checks, canonical Base64
validation, complete zlib validation, and pane/image quotas remain in place.

| Five-second graphics workload | Previous optimization | Graphics follow-up |
| --- | ---: | ---: |
| Frame receipt p95 | 132.59 ms | 60.92 ms |
| Input-to-child-read p95 | 42.80 ms | 3.60 ms |
| Daemon CPU, percent of one core | 45.35% | 13.19% |
| Daemon allocated bytes | 923,341,792 | 219,169,120 |
| Completed frames | 25 | 25 |

The ingress microprofile's allocation count fell from 1,049,170,528 to
157,617,472 bytes across 30 frames. Sampling times were about 2.46 and 0.66
seconds respectively; use the packaged end-to-end benchmark for latency claims.
These remain single synthetic runs, not measurements of a particular website or
physical display. Real website visual defects have not been assessed by this test.

Evidence: [packaged benchmark](evidence/performance-graphics.json),
[ingress profile before](evidence/graphics-profile-before.txt),
[ingress profile after](evidence/graphics-profile-after.txt).
`nix build -L` and `nix flake check -L` exited zero. Added tests cover known Base64
vectors, byte/string ranges, malformed input and padding, all byte values,
fragmented control strings, quota boundaries, and recovery after invalid escapes.

## CookUnity-sized frames — 2026-09-04

A frame captured from the running CookUnity browser was 1274 × 1368 RGBA
(6,971,328 decoded bytes; 2,052,062 compressed bytes). The earlier 512 × 512,
5 Hz fixture did not exercise this load. Profiling a replay of the larger frame
found generic array access and integer operations in Base64 decoding, generic
sequence concatenation when assembling uploads, and untyped searches in the VT
parser. The client also queued each 4096-character Kitty chunk separately.

The codec and VT loops now provide SBCL with byte/integer types and local speed
optimization while retaining safety checks. Adjustable/displaced codec inputs
are normalized at the API boundary. Frame assembly allocates the known total
size and copies each chunk into it. Egress queues a complete ASCII upload,
retaining Kitty chunk boundaries and synchronized presentation. No wire version,
image format, browser dependency, or graphics quota changed. A native rewrite
or a new transport would add substantially more code than these measured fixes.

The benchmark now accepts `--graphics-size WIDTH HEIGHT`, `--graphics-fps FPS`,
and optional `--graphics-frame FILE` (raw RGBA). Defaults preserve the original
workloads. Without a file it generates seeded random pixels. Replay overwrites
the first eight bytes with a timestamp; it includes fixture-side compression
and input can wait while the fixture is compressing or writing a frame.

```sh
nix run .#performance -- --seconds 5 --graphics-size 1274 1368 --graphics-fps 30
# To replay a saved frame instead of seeded noise, also pass:
# --graphics-frame /path/to/frame.rgba
```

Five-second replay samples of the captured pixels, targeting 30 Hz:

| Measurement | Previous packaged build | Updated build |
| --- | ---: | ---: |
| Completed frames | 61 | 89 |
| Frame receipt p50 | 139.62 ms | 88.67 ms |
| Frame receipt p95 | 147.81 ms | 97.58 ms |
| Input-to-child-read p50 | 45.33 ms | 7.46 ms |
| Input-to-child-read p95 | 83.31 ms | 56.09 ms |
| Daemon CPU, percent of one core | 75.99% | 49.56% |
| Client CPU, percent of one core | 33.00% | 19.39% |

These are transport replays, not website-loading or physical display timings.
They delivered about 46% more frames, with about 34% lower p95 frame latency.
Total daemon allocation increased from 1.30 GB to 1.86 GB while processing more
frames; allocation per received frame was approximately unchanged. The fixture
used about 84% of one core after the change and did not sustain the requested
30 Hz. This is not a claim of smooth 30/60 Hz browser presentation.

The [direct PTY replay](evidence/cookunity-direct.json) delivered 92 frames with
54.54 ms p95 receipt, showing that the fixture itself limits throughput here.
The [unchanged default workload](evidence/cookunity-default.json) delivered all
25 frames with 36.23 ms p95 receipt and 0.16 ms p95 input-to-child-read latency.

Evidence: [before](evidence/cookunity-before.json),
[after](evidence/cookunity-after.json), and
[live session counters](evidence/cookunity-live.json). Raw captured pixels stay
outside the repository; their SHA-256 is
`ee05860aed643c1090122cbc57e0a9882cd5acd642669dcf09496206a78db625`.
The live `cookunity-fast` session uses the updated executable and continues
receiving CookUnity frames without browser graphics errors. Visual smoothness
was not measured. Existing sessions keep their original daemon executable;
reattaching alone does not upgrade them.

Validation: `nix build -L` and `nix flake check -L` exited zero. Added regression
coverage includes non-simple codec inputs, unequal upload chunks, and real PTY
backpressure with a multi-chunk image whose decoded pixels are checked in full,
including existing clipping, deletion, and reconnect scenarios.

## Local frame transport — 2026-09-05

This change removes inline pixel transport from the local browser path. Raw
RGB/RGBA frames arrive through POSIX shared memory, are copied into immutable
owned snapshots, and travel to the viewer as filenames. A successful host probe
allows Kitty to read the snapshot directly. No compression/Base64 or bulk pixel
IPC occurs on that path. Inline input/output remains available. No frame-rate or
resolution cap was added. See [ADR-003](adr/003-local-frame-transport.md).

Fresh five-second samples on the same Ryzen 9 7950X replay the same captured
1274 × 1368 RGBA pixels, targeting 30 Hz. Inline and shared below use the updated
executable; a separate three-second pre-change inline sample measured 97.35 ms
p95 frame receipt and 56.86 ms input latency, consistent with the new inline run.

| Measurement | Inline | Local shared snapshots | Direct shared PTY |
| --- | ---: | ---: | ---: |
| Frame receipt p95 | 97.66 ms | 8.84 ms | 4.55 ms |
| Input-to-child-read p95 | 58.27 ms | 1.44 ms | 1.47 ms |
| Completed frames | 89 | 136 | 137 |
| Daemon CPU, percent of one core | 49.79% | 8.19% | — |
| Client CPU, percent of one core | 20.00% | 0.80% | — |
| Producer CPU, percent of one core | 84.19% | 6.79% | 6.80% |
| Daemon allocated bytes | 1,860,959,088 | 18,776,064 | — |
| Terminal stream bytes | 229,690,560 | 29,580 | 9,727 |

This is about 11× lower p95 frame receipt latency, 87% less combined Ekko CPU,
and 99% fewer daemon heap allocations in this sample. The shared producer also
avoids compression. It still copies raw pixels: terminal stream bytes exclude
shared-memory/file I/O, and raw snapshots occupy storage outside the Lisp heap.
Do not interpret these numbers as eliminating memory bandwidth or total memory
use. The fixture's scheduling delivers about 27 FPS on both shared paths; this
is not evidence of a 30 FPS ceiling in Ekko.

The receiver reads and validates the named frame's byte length before recording
local receipt, then sends the protocol acknowledgement. Direct inline receipt
keeps the earlier complete-upload timestamp before receiver decompression. All
modes include producer work. These remain synthetic transport measurements,
excluding website execution, Kitty GPU work, and physical display latency.

```sh
nix run .#performance -- --seconds 5 --workload graphics --transport shared \
  --graphics-size 1274 1368 --graphics-fps 30 \
  --graphics-frame /tmp/ekko-cookunity.rgba
# Compare --transport inline and --transport shared --direct with the same file.
```

The captured file is optional and remains outside the repository. Without it,
the fixture uses seeded random RGBA; that has a different compression workload.

Evidence: [pre-change](evidence/local-transport-before.json),
[updated inline](evidence/local-transport-inline.json),
[shared snapshots](evidence/local-transport-shared.json),
[direct shared](evidence/local-transport-direct.json).

Live verification: the `cookunity-shared` workspace visibly presents CookUnity
in a Kitty window beside an interactive shell. Its status reports a raw 6,971,328
byte local image, with only small control messages arriving through the browser
PTY. The initial unsupported file probe increments the browser pane's graphics
error count once; shared-memory negotiation then succeeds. A window-only screenshot
was inspected in /tmp and was not committed. This confirms the real application
uses the path, not its input-to-display latency or subjective smoothness.

Validation: `nix build -L` and `nix flake check -L` exited zero. The packaged
runtime suite covers the original inline path plus shared ingress with successful
file egress, explicit host rejection, and an unanswered probe. It includes delayed
read acknowledgements, bounded retained snapshots, scoped deletion, client death,
detached updates, reconnect and final cleanup. Unit tests cover immutable snapshots,
query ownership, invalid dimensions/names/lengths/modes, quota rejection, and
fragmented/unrelated host replies. [Live counters](evidence/local-transport-live.json)
and the [session-preference check](evidence/browser-session-transport.json) record
the real browser path and an inline override against the same browser daemon.

## Daily-use foundation — 2026-09-05

The final 1274 × 1368 CookUnity-frame replay (shared transport, 30 Hz target,
five seconds) delivered 138 frames with 6.82 ms p95 frame receipt and 1.22 ms
p95 input latency. Combined daemon/client/extension CPU was about 7.0% of one
core. These short synthetic samples retain the local-transport speedup; they
exclude Kitty rendering and display latency and do not establish a new speedup.

The extension worker used no measurable CPU in this rendering workload and
had five voluntary context switches. Its sampled RSS was 123.8 MiB. This is
an additional SBCL process per session; RSS is not peak or proportional memory.
Reducing worker memory is a remaining efficiency task. History is allocated
lazily; ordinary key input and graphics do not invoke Lisp extension callbacks.

[Raw replay report](evidence/daily-performance.json) and
[Nix verification](evidence/daily-checks.json) identify the executable and checks.
The checks include failed/stuck reloads, callback termination and recovery,
exact declared dependencies, action validation, split/input ordering, copy/search,
client replacement, image restoration, and an external command in the bare build.
