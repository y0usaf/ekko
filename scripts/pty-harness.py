import base64
import fcntl
import json
import os
import pty
import re
import select
import shutil
import struct
import subprocess
import sys
import tempfile
import termios
import time

import pyte

CW, CH = 10, 24
END = b"\x1b[?2026l"
TICKS = "i=0; while :; do i=$((i+1)); printf 'tick %d lorem ipsum dolor sit amet\\n' $i; sleep 0.1; done"
FILL = "i=0; while [ $i -lt 60 ]; do printf '\\033[1;34mline %03d\\033[0m lorem ipsum dolor sit amet\\n' $i; i=$((i+1)); done; exec sleep 600"


class Term:
    def __init__(self, argv, env, cols, rows):
        self.screen = pyte.Screen(252, 58)
        self.stream = pyte.ByteStream(self.screen)
        self.log, self.frames, self.tail = [], [], b""
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, cols * CW, rows * CH))

        def controlling():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)

        self.proc = subprocess.Popen(argv, stdin=slave, stdout=slave, stderr=slave, env=env, preexec_fn=controlling)
        os.close(slave)
        os.set_blocking(master, False)
        self.master, self.t0 = master, time.monotonic()

    def now(self):
        return time.monotonic() - self.t0

    def resize(self, cols, rows):
        fcntl.ioctl(self.master, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, cols * CW, rows * CH))

    def extent(self):
        cells = [(x, y) for y in range(self.screen.lines) for x in range(self.screen.columns)
                 if (c := self.screen.buffer[y][x]).data != " " or c.bg != "default" or c.reverse]
        return (max(x for x, _ in cells) + 1, max(y for _, y in cells) + 1) if cells else (0, 0)

    def pump(self, timeout):
        if not select.select([self.master], [], [], max(0, timeout))[0]:
            return False
        try:
            data = os.read(self.master, 1 << 16)
        except OSError:
            return False
        self.log.append(data)
        seen = self.tail + data
        if b"\x1b[16t" in seen:
            os.write(self.master, f"\x1b[6;{CH};{CW}t".encode())
        self.stream.feed(data)
        if END in seen:
            self.frames.append((self.now(), self.extent()))
        self.tail = seen[-16:]
        return True

    def run_for(self, seconds):
        until = time.monotonic() + seconds
        while time.monotonic() < until:
            self.pump(until - time.monotonic())

    def mouse(self, x, y, button, release=False):
        os.write(self.master, f"\x1b[<{button};{x};{y}{'m' if release else 'M'}".encode())

    def ticks(self):
        found = [int(n) for n in re.findall(r"tick (\d+)", "\n".join(self.screen.display))]
        return max(found) if found else None

    def osc52(self):
        return [base64.b64decode(p).decode(errors="replace")
                for p in re.findall(rb"\x1b\]52;[^;]*;([A-Za-z0-9+/=]*)", b"".join(self.log))]

    def close(self):
        self.proc.terminate()
        try:
            self.proc.wait(5)
        except subprocess.TimeoutExpired:
            self.proc.kill()


class Instance:
    def __init__(self, binary, name):
        self.binary, self.root = binary, tempfile.mkdtemp(prefix=f"{name}-")
        os.mkdir(f"{self.root}/run", 0o700)
        self.env = {"PATH": os.environ["PATH"], "HOME": self.root, "XDG_RUNTIME_DIR": f"{self.root}/run",
                    "XDG_CONFIG_HOME": f"{self.root}/config", "EKKO_INSTANCE": name, "SHELL": "/bin/sh",
                    "TERM": "xterm-256color"}

    def ekko(self, *arguments):
        return subprocess.run([self.binary, *arguments], env=self.env, check=True, capture_output=True, timeout=30)

    def attach(self, cols=252, rows=58):
        return Term([self.binary, "attach"], self.env, cols, rows)

    def __enter__(self):
        return self

    def __exit__(self, *_):
        subprocess.run([self.binary, "stop", "--force"], env=self.env, capture_output=True, timeout=30)
        shutil.rmtree(self.root, ignore_errors=True)


def startup(binary):
    with Instance(binary, "startup") as ekko:
        ekko.ekko("run", "--detached", "sh", "-c", FILL)
        ekko.ekko("split", "columns", "sh", "-c", FILL)
        time.sleep(1)
        term, resized = ekko.attach(120, 40), None
        while term.now() < 10:
            term.pump(0.005)
            if resized is None and term.now() >= 0.15:
                term.resize(252, 58)
                resized = term.now()
            full = next((t for t, (x, y) in term.frames if x > 200 and y > 50), None)
            if full is not None:
                break
        term.close()
        return {"first_full_frame_after_resize": None if full is None else round(full - resized, 3)}


def click(binary):
    with Instance(binary, "click") as ekko:
        ekko.ekko("run", "--detached", "sh", "-c", TICKS)
        term = ekko.attach()
        term.run_for(2.0)
        x, y = 40 * CW + 3, 20 * CH + 5
        term.mouse(x, y, 35)
        term.mouse(x, y, 0)
        term.run_for(0.05)
        term.mouse(x, y, 0, release=True)
        term.run_for(0.5)
        before = term.ticks()
        term.run_for(2.0)
        after = term.ticks()
        term.close()
        return {"ticks_after_click": [before, after]}


def copy(binary):
    with Instance(binary, "copy") as ekko:
        ekko.ekko("run", "--detached", "sh", "-c", TICKS)
        term = ekko.attach()
        term.run_for(2.0)
        x, y = 3 * CW + 3, 20 * CH + 5
        term.mouse(x, y, 35)
        term.mouse(x, y, 0)
        for i in range(1, 300):
            term.mouse(x + i, y + i // 60 * CH, 32)
            term.pump(0.001)
        term.run_for(0.1)
        term.mouse(x + 300, y + 4 * CH, 0, release=True)
        term.run_for(0.7)
        before = term.ticks()
        term.run_for(1.5)
        after = term.ticks()
        copied = term.osc52()
        term.close()
        return {"ticks_after_copy": [before, after], "copied": [c[:20] for c in copied]}


def pane_osc52(binary):
    payload = "pane osc52 probe"
    with Instance(binary, "osc52") as ekko:
        encoded = base64.b64encode(payload.encode()).decode()
        ekko.ekko("run", "--detached", "sh", "-c", f"sleep 2; printf '\\033]52;c;{encoded}\\a'; exec sleep 600")
        term = ekko.attach()
        term.run_for(4.0)
        term.close()
        return {"forwarded": payload in term.osc52()}


def advanced(pair):
    return None not in pair and pair[1] > pair[0]


CHECKS = {
    "startup": (startup, lambda r: r["first_full_frame_after_resize"] is not None and r["first_full_frame_after_resize"] < 0.6),
    "click": (click, lambda r: advanced(r["ticks_after_click"])),
    "copy": (copy, lambda r: advanced(r["ticks_after_copy"]) and r["copied"] != []),
    "pane-osc52": (pane_osc52, lambda r: r["forwarded"]),
}

failed = []
for name, (scenario, passes) in CHECKS.items():
    result = scenario(sys.argv[1])
    ok = passes(result)
    print(json.dumps({"scenario": name, "pass": ok, **result}))
    if not ok:
        failed.append(name)
sys.exit(1 if failed else 0)
