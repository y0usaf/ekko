"""Deterministic PTY benchmark; terminal receipt is not physical display latency."""
import argparse
import base64
import fcntl
import json
import math
import os
from pathlib import Path
import platform
import pty
import random
import select
import struct
import subprocess
import sys
import tempfile
import termios
import time
import tty
import zlib

ESC = b"\x1b"


def fixture(mode, log_path, width=512, height=512, fps=5, frame_path=None):
    tty.setraw(0)
    pixels = (Path(frame_path).read_bytes() if frame_path else
              random.Random(42).randbytes(width * height * 4))
    pending = b""
    frame = 0
    next_frame = time.monotonic()
    with open(log_path, "a", buffering=1) as log:
        log.write("ready\n")
        while True:
            now = time.monotonic()
            if mode in ("text", "graphics") and now >= next_frame:
                if mode == "text":
                    os.write(1, (f"line {frame:08d} " + "text workload " * 5 + "\r\n").encode())
                else:
                    raw = struct.pack(">Q", time.monotonic_ns()) + pixels[8:]
                    if os.environ.get("EKKO_PERF_TRANSPORT") == "shared":
                        name = f"/ekko-perf-{os.getpid()}-{frame % 4}"
                        Path("/dev/shm" + name).write_bytes(raw)
                        os.write(1, ESC + b"[H" + ESC + f"_Ga=T,t=s,f=32,s={width},v={height},i=7,C=1,q=2;".encode()
                                 + base64.b64encode(name.encode()) + ESC + b"\\")
                        payload = b""
                    else:
                        payload = base64.b64encode(zlib.compress(raw, 1))
                        os.write(1, ESC + b"[H")
                    for offset in range(0, len(payload), 4096):
                        more = int(offset + 4096 < len(payload))
                        header = f"a=T,f=32,o=z,s={width},v={height},i=7,C=1,q=2,".encode() if offset == 0 else b""
                        os.write(1, ESC + b"_G" + header + f"m={more};".encode()
                                 + payload[offset:offset + 4096] + ESC + b"\\")
                frame += 1
                next_frame = now + (0.05 if mode == "text" else 1 / fps)
            ready, _, _ = select.select([0], [], [], 0.005)
            if ready:
                pending += os.read(0, 65536)
                while b"\n" in pending:
                    line, pending = pending.split(b"\n", 1)
                    # Input probes begin with a monotonic timestamp; filler tests paste.
                    token = line.split(b":", 1)[0]
                    if token.isdigit():
                        log.write(f"{token.decode()} {time.monotonic_ns()}\n")


class Receiver:
    def __init__(self):
        self.replies = bytearray()
        self.buffer = b""
        self.payload = bytearray()
        self.bytes = 0
        self.frames = []

    def feed(self, data):
        self.bytes += len(data)
        self.buffer += data
        while True:
            start = self.buffer.find(ESC + b"_G")
            if start < 0:
                self.buffer = self.buffer[-2:]
                return
            self.buffer = self.buffer[start:]
            end = self.buffer.find(ESC + b"\\")
            if end < 0:
                return
            command, self.buffer = self.buffer[3:end], self.buffer[end + 2:]
            header, separator, payload = command.partition(b";")
            fields = dict(part.split(b"=", 1) for part in header.split(b","))
            if not separator or fields.get(b"a") in (b"d", b"p"):
                continue
            if fields.get(b"t") in (b"f", b"s"):
                name = base64.b64decode(payload, validate=True).decode()
                path = Path("/dev/shm" + name if fields[b"t"] == b"s" else name)
                raw = path.read_bytes()
                if fields[b"t"] == b"s":
                    path.unlink()
                assert len(raw) == int(fields[b"s"]) * int(fields[b"v"]) * 4
                if fields.get(b"a") != b"q":
                    self.frames.append((time.monotonic_ns() - struct.unpack(">Q", raw[:8])[0]) / 1e6)
                if fields.get(b"q", b"0") == b"0":
                    placement = b",p=" + fields[b"p"] if b"p" in fields else b""
                    self.replies.extend(ESC + b"_Gi=" + fields[b"i"] + placement + b";OK" + ESC + b"\\")
                continue
            self.payload.extend(payload)
            if fields.get(b"m", b"0") == b"0":
                received = time.monotonic_ns()
                raw = zlib.decompress(base64.b64decode(self.payload))
                sent = struct.unpack(">Q", raw[:8])[0]
                self.frames.append((received - sent) / 1e6)
                self.payload.clear()


def process_stats(pid):
    stat = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()
    status = dict(line.split(":", 1) for line in Path(f"/proc/{pid}/status").read_text().splitlines())
    return {"cpu_seconds": (int(stat[11]) + int(stat[12])) / os.sysconf("SC_CLK_TCK"),
            "voluntary_context_switches": int(status["voluntary_ctxt_switches"]),
            "rss_bytes": int(stat[21]) * os.sysconf("SC_PAGE_SIZE")}


def distribution(samples):
    ordered = sorted(samples)
    if not ordered:
        return {"count": 0, "p50_ms": None, "p95_ms": None}
    return {"count": len(samples), **{f"p{p}_ms": ordered[max(0, math.ceil(len(ordered) * p / 100) - 1)]
                                     for p in (50, 95)}}


def benchmark(binary, mode, seconds, direct=False, width=512, height=512, fps=5, frame_path=None):
    with tempfile.TemporaryDirectory(prefix="ekko-perf-") as directory:
        log = Path(directory) / "input.log"
        env = dict(os.environ, XDG_RUNTIME_DIR=directory)
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 960, 640))
        command = [sys.executable, str(Path(__file__).resolve()), "--fixture", mode, str(log),
                   str(width), str(height), str(fps)]
        if frame_path:
            command.append(str(Path(frame_path).resolve()))
        if not direct:
            command = [binary, "run", "--session", "perf", *command]
        process = subprocess.Popen(command, stdin=slave, stdout=slave, stderr=slave,
                                   env=env, start_new_session=True)
        os.close(slave)
        os.set_blocking(master, False)
        receiver = Receiver()

        def pump(timeout):
            readable, _, _ = select.select([master], [], [], max(0, timeout))
            if readable:
                receiver.feed(os.read(master, 65536))
                if receiver.replies:
                    del receiver.replies[:os.write(master, receiver.replies)]

        def status():
            return json.loads(subprocess.check_output([binary, "status", "perf"], env=env, timeout=10))

        try:
            deadline = time.monotonic() + 15
            while not log.exists() or "ready" not in log.read_text():
                if process.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError("Benchmark fixture failed to start")
                pump(.01)
            # Drain startup and its first scene before sampling idle CPU/output.
            warmup = time.monotonic() + .5
            while time.monotonic() < warmup:
                pump(.01)
            initial_status = {} if direct else status()
            pids = {"fixture": process.pid} if direct else {
                "client": process.pid, "daemon": initial_status["daemon_pid"],
                "fixture": initial_status["panes"][0]["pid"]}
            before = {name: process_stats(pid) for name, pid in pids.items()}
            byte_start = receiver.bytes
            receiver.frames.clear()
            started = time.monotonic()
            next_probe = started
            pending = b""
            probes = 0
            while time.monotonic() - started < seconds or pending:
                now = time.monotonic()
                if now - started > seconds + 15:
                    raise RuntimeError("Input did not drain")
                if mode != "idle" and now >= next_probe and now - started < seconds and not pending:
                    filler = b"x" * 16384 if mode == "paste" else b""
                    pending = str(time.monotonic_ns()).encode() + b":" + filler + b"\n"
                    if mode == "paste" and not direct:
                        pending = ESC + b"[200~" + pending + ESC + b"[201~"
                    probes += 1
                    next_probe = now + (0.5 if mode == "paste" else 0.1)
                if pending:
                    try:
                        pending = pending[os.write(master, pending):]
                    except BlockingIOError:
                        pass
                pump(.005)
            elapsed = time.monotonic() - started
            after = {name: process_stats(pid) for name, pid in pids.items()}
            output_bytes = receiver.bytes - byte_start
            final_status = {} if direct else status()
            # Wait for outstanding input probes, without adding drain CPU to the window.
            deadline = time.monotonic() + 15
            while len(log.read_text().splitlines()) - 1 < probes:
                if time.monotonic() > deadline:
                    raise RuntimeError("Input probes lost or delayed more than 15 seconds")
                pump(.01)
            latency = [(int(end) - int(start)) / 1e6
                       for start, end in (line.split() for line in log.read_text().splitlines()[1:])]
            result = {"mode": mode, "elapsed_seconds": elapsed, "terminal_bytes": output_bytes,
                      "input_to_pty": distribution(latency),
                      "frame_to_terminal_receipt": distribution(receiver.frames),
                      "processes": {name: {"cpu_percent_one_core": 100 * (after[name]["cpu_seconds"] - values["cpu_seconds"]) / elapsed,
                                            "voluntary_context_switches": after[name]["voluntary_context_switches"] - values["voluntary_context_switches"],
                                            "rss_bytes_start": values["rss_bytes"], "rss_bytes_end": after[name]["rss_bytes"]}
                                    for name, values in before.items()}}
            for key in ("allocated_bytes", "gc_seconds"):
                result[f"daemon_{key}_delta"] = (final_status[key] - initial_status[key]
                                                  if key in initial_status and key in final_status else None)
            return result
        finally:
            if not direct:
                subprocess.run([binary, "stop", "perf"], env=env, stdout=subprocess.DEVNULL,
                               stderr=subprocess.DEVNULL, timeout=10)
            if process.poll() is None:
                process.terminate()
            process.wait(timeout=5)
            os.close(master)
            for path in Path("/dev/shm").glob(f"ekko-perf-{pids.get('fixture', process.pid) if 'pids' in locals() else process.pid}-*"):
                path.unlink(missing_ok=True)


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "--fixture":
        fixture(sys.argv[2], sys.argv[3], int(sys.argv[4]), int(sys.argv[5]),
                float(sys.argv[6]), sys.argv[7] if len(sys.argv) > 7 else None)
        return
    parser = argparse.ArgumentParser(prog="ekko-performance", description=__doc__)
    parser.add_argument("binary", help=argparse.SUPPRESS)
    parser.add_argument("--seconds", type=float, default=3)
    parser.add_argument("--workload", choices=["idle", "text", "graphics", "paste"],
                        help="Measure one workload instead of all four")
    parser.add_argument("--transport", choices=["inline", "shared"], default="inline",
                        help="Graphics producer transport; shared uses local raw snapshots")
    parser.add_argument("--direct", action="store_true", help="Run fixture directly in a PTY as transport baseline")
    parser.add_argument("--graphics-size", type=int, nargs=2, default=[512, 512], metavar=("WIDTH", "HEIGHT"))
    parser.add_argument("--graphics-fps", type=float, default=5)
    parser.add_argument("--graphics-frame", type=Path, help="Replay raw RGBA pixels matching --graphics-size")
    args = parser.parse_args()
    os.environ["EKKO_PERF_TRANSPORT"] = args.transport
    width, height = args.graphics_size
    if not (1 <= width <= 8192 and 1 <= height <= 8192 and 8 <= width * height * 4 <= 32 * 1024 * 1024):
        parser.error("graphics dimensions must fit an 8-byte timestamp and the 32 MiB image limit")
    if not math.isfinite(args.graphics_fps) or args.graphics_fps <= 0:
        parser.error("--graphics-fps must be finite and positive")
    if args.graphics_frame and (not args.graphics_frame.is_file() or args.graphics_frame.stat().st_size != width * height * 4):
        parser.error("--graphics-frame must contain exactly WIDTH * HEIGHT * 4 raw RGBA bytes")
    if not math.isfinite(args.seconds) or args.seconds <= 0:
        parser.error("--seconds must be finite and positive")
    print(json.dumps({"schema_version": 1, "binary": os.path.realpath(args.binary),
                      "platform": platform.platform(),
                      "cpu": next((line.split(":", 1)[1].strip() for line in Path("/proc/cpuinfo").read_text().splitlines()
                                   if line.startswith("model name")), platform.processor()),
                      "direct_pty": args.direct, "viewport": [120, 40, 960, 640],
                      "graphics": {"width": width, "height": height, "target_fps": args.graphics_fps, "transport": args.transport,
                                   "frame_file": str(args.graphics_frame) if args.graphics_frame else None},
                      "notes": "Synthetic PTY receiver; excludes Kitty rendering/display. RSS is sampled, not peak. CPU window excludes final drain. Allocation/GC are daemon-only when available.",
                      "workloads": [benchmark(args.binary, mode, args.seconds, args.direct, width, height,
                                               args.graphics_fps, args.graphics_frame)
                                    for mode in ([args.workload] if args.workload else ["idle", "text", "graphics", "paste"])]}, indent=2))


if __name__ == "__main__":
    main()
