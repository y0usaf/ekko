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
COLOURS = {b"10": b"rgb:fefe/fafa/f1f1", b"11": b"rgb:2020/1f1f/2626",
           **{b"4;%d" % i: b"rgb:%04x/0000/ffff" % i for i in range(16)}}


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
        for target in re.findall(rb"\x1b\]((?:10|11|4;\d+));\?", seen):
            os.write(self.master, b"\x1b]%s;%s\x1b\\" % (target, COLOURS[target]))
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


def pane_colours(binary):
    asked = [b"10", b"11", b"4;9"]
    with Instance(binary, "colours") as ekko:
        log = f"{ekko.root}/replies"
        queries = "".join(f"\\033]{t.decode()};?\\a" for t in asked)
        ekko.ekko("run", "--detached", "sh", "-c", f"stty raw -echo; sleep 2; printf '{queries}'; exec cat > {log}")
        term = ekko.attach()
        term.run_for(4.0)
        term.close()
        with open(log, "rb") as f:
            replies = f.read()
        return {"answered": replies == b"".join(b"\x1b]%s;%s\x1b\\" % (t, COLOURS[t]) for t in asked),
                "replies": replies.decode(errors="replace")}


def mouse_flood(binary):
    with Instance(binary, "flood") as ekko:
        log = f"{ekko.root}/app.log"
        ekko.ekko("run", "--detached", "sh", "-c", f"stty raw -echo; printf '\\033[?1003h\\033[?1006h'; exec cat > {log}")
        term = ekko.attach()
        term.run_for(2.0)
        x, y = 20 * CW + 3, 20 * CH + 5
        term.mouse(x, y, 0)
        for i in range(1, 600):
            term.mouse(x + i, y, 32)
            term.pump(0.001)
        term.mouse(x + 599, y, 0, release=True)
        term.run_for(1.0)
        term.close()
        with open(log, "rb") as f:
            motion = re.findall(rb"\x1b\[<32;\d+;\d+M", f.read())
        return {"host_motion": 599, "app_motion": len(motion), "cells": len(set(motion)),
                "repeats": sum(a == b for a, b in zip(motion, motion[1:]))}


LONG_LINE = "https://claude.ai/oauth/authorize?code=true&client_id=9d1c250a-e61b-47e5-9d4f-8b6c2f1a7e03&response_type=code&redirect_uri=https%3A%2F%2Fconsole.anthropic.com%2Foauth%2Fcode%2Fcallback&scope=org%3Acreate_api_key+user%3Aprofile+user%3Ainference&code_challenge=abcdefghijklmnopqrstuvwxyz0123456789ABCD&state=gzuwBfMt"


def pane_text(term, cols, rows):
    return "".join(line[:cols].strip("\u2502 ") for line in term.screen.display[1:rows - 2])


def reflow(binary):
    with Instance(binary, "reflow") as ekko:
        path = f"{ekko.root}/long-line.txt"
        with open(path, "w") as f:
            f.write(LONG_LINE + "\n")
        ekko.ekko("run", "--detached", "sh", "-c", f"cat {path}; exec sleep 600")
        term = ekko.attach(160, 30)
        intact = []
        for cols, rows, required in [(160, 30, True), (60, 30, True), (40, 12, False), (160, 30, True)]:
            term.resize(cols, rows)
            term.run_for(2.0)
            if required:
                intact.append(LONG_LINE in pane_text(term, cols, rows))
        term.close()
        return {"intact": intact}


def no_autowrap(binary):
    with Instance(binary, "no-autowrap") as ekko:
        line = "".join(chr(65 + i % 26) for i in range(300))
        ekko.ekko("run", "--detached", "sh", "-c",
                  f"sleep 1.5; printf '\\033[?1049h\\033[H\\033[?7l{line}\\033[?7h\\r\\nnext'; exec sleep 600")
        term = ekko.attach(120, 30)
        term.run_for(3.0)
        pane = json.loads(ekko.ekko("status").stdout)["panes"][0]
        x, y, cols = pane["x"], pane["y"], pane["cols"]
        rows = [term.screen.display[y + i][x:x + cols] for i in range(2)]
        term.close()
        return {"rows": rows, "expected": [line[:cols - 1] + line[-1], "next"]}


REFUSED = """(ekko/extensions:register-component :id ':probe :reads '(:panes)
  :handler (lambda (snapshot event)
             (declare (ignore event))
             (let* ((count (length (ekko/extensions:value snapshot :panes))) (sgr (list 1)))
               (when (= count 2)
                 (dotimes (i 48) (setf sgr (list sgr))))
               (list (ekko/extensions:action :decorate ':spans
                       (list (list :x 0 :y 0 :text (format nil "probe ~D" count) :sgr sgr)))))))
"""


def spans(state, owner):
    return [s for d in state["decorations"] or [] if d["owner"] == owner for s in d["spans"] or []]


def full_desktop(binary):
    with Instance(binary, "desktop") as ekko:
        ekko.ekko("run", "--detached", "sh", "-c", "exec sleep 600")
        term = ekko.attach()
        for _ in range(15):
            ekko.ekko("split", "columns", "sh", "-c", "exec sleep 600")
            term.run_for(0.4)
        term.run_for(1.0)
        state = json.loads(ekko.ekko("inspect").stdout)
        term.close()
        return {"windows": len(state["panes"]), "chrome_spans": len(spans(state, "defaults")),
                "hook_failures": state.get("hook-failures")}


def refused_answer(binary):
    with Instance(binary, "refused") as ekko:
        os.makedirs(f"{ekko.root}/config/ekko")
        with open(f"{ekko.root}/config/ekko/init.lisp", "w") as f:
            f.write(REFUSED)
        ekko.ekko("run", "--detached", "sh", "-c", "exec sleep 600")
        term = ekko.attach()
        term.run_for(1.0)
        ekko.ekko("split", "columns", "sh", "-c", "exec sleep 600")
        term.run_for(2.0)
        ekko.ekko("split", "columns", "sh", "-c", "exec sleep 600")
        term.run_for(1.0)
        state = json.loads(ekko.ekko("inspect").stdout)
        term.close()
        return {"probe": [s["text"] for s in spans(state, "probe")], "hook_failures": state.get("hook-failures")}


SLOW = """(ekko/extensions:register-component :id ':slow :reads '(:panes)
  :handler (lambda (snapshot event)
             (declare (ignore event))
             (when (probe-file "{flag}") (sleep 2))
             (list (ekko/extensions:action :decorate ':spans
                     (list (list :x 0 :y 0 :text (format nil "slow ~D" (length (ekko/extensions:value snapshot :panes)))))))))
"""


def slow_hook(binary):
    with Instance(binary, "slow") as ekko:
        flag = f"{ekko.root}/slow"
        os.makedirs(f"{ekko.root}/config/ekko")
        with open(f"{ekko.root}/config/ekko/init.lisp", "w") as f:
            f.write(SLOW.replace("{flag}", flag))
        ekko.ekko("run", "--detached", "sh", "-c", "exec sleep 600")
        term = ekko.attach()
        term.run_for(1.0)
        open(flag, "w").close()
        ekko.ekko("split", "columns", "sh", "-c", "exec sleep 600")
        paused = banner = False
        for _ in range(30):
            term.run_for(0.5)
            failures = json.loads(ekko.ekko("inspect").stdout).get("hook-failures") or []
            paused = any(f["paused"] and f["misses"] >= 3 for f in failures)
            banner = "missed" in "\n".join(term.screen.display)
            if paused and banner:
                break
        os.remove(flag)
        for _ in range(30):
            term.run_for(0.5)
            state = json.loads(ekko.ekko("inspect").stdout)
            if [s["text"] for s in spans(state, "slow")] == ["slow 2"]:
                break
        term.run_for(0.5)
        state = json.loads(ekko.ekko("inspect").stdout)
        after = "missed" in "\n".join(term.screen.display)
        term.close()
        return {"paused": paused, "banner": banner, "probe": [s["text"] for s in spans(state, "slow")],
                "hook_failures": state.get("hook-failures"), "banner_after": after}


PAINT = ("sleep 1; printf '\\033[2J\\033[H'; i=1; while [ $i -le 30 ]; do "
         "printf '\\033[%d;1H\\033[3%dm%02d \u4e2d\u6587 e\\314\\201 ' $i $((i % 8)) $i; sleep 0.02; i=$((i+1)); done; "
         "printf '\\033[0m\\033[5;5Hx\\033[6;4H\\033[44m wide-cut \\033[0m'; sleep 0.05; "
         "printf '\\033[2;1H\\033[35m02\\033[0m\\033[3;1H\\033[2K\\033[7;3H\\033[5P\\033[8;3H\\033[4@'; sleep 0.05; "
         "printf '\\033[30;1H\\n\\n\\n\\033[1;7mdone\\033[0m'; exec sleep 600")


def cells(term, cols, rows):
    return [[(c.data, c.fg, c.bg, c.bold, c.reverse, c.underscore)
             for c in (term.screen.buffer[y][x] for x in range(cols))] for y in range(rows)]


def incremental(binary):
    with Instance(binary, "incremental") as ekko:
        ekko.ekko("run", "--detached", "sh", "-c", PAINT)
        first = ekko.attach(120, 40)
        first.run_for(3.0)
        painted = cells(first, 120, 39)
        first.close()
        second = ekko.attach(120, 40)
        second.run_for(1.5)
        fresh = cells(second, 120, 39)
        second.close()
        differences = [[x, y, painted[y][x], fresh[y][x]] for y in range(39) for x in range(120)
                       if painted[y][x] != fresh[y][x]]
        return {"frames": len(first.frames), "differences": len(differences), "first": differences[:3]}


VT = ("\\033[41m\\033cR"
      "\\033[2;1H\\033(0lqqk\\033(B"
      "\\033[3;1H\\033)0\\016lqk\\017x"
      "\\033[4;1HAB\\033[3b"
      "\\033[5;1HABCD\\r\\033[4hXY\\033[4l"
      "\\033[3g\\033[1;5H\\033H\\033[1;9H\\033H\\033[6;1H\\tT\\tU\\033[7;12H\\033[2ZV\\033[8;1H\\033[2IW"
      "\\033[9;1Habc\\033[4m\\033[K\\033[0m"
      "\\033[10;1H\\033[38:2::255:0:0mX\\033[0m"
      "\\033[11;1H\\033[1;31m\\0337\\033[0mplain\\0338RED\\033[0m"
      "\\033[12;20r\\033[?6h\\033[1;1HO\\033[?6l\\033[r"
      "\\033[13;1HMAIN\\033[?1049h\\033[2;2H\\0337alt\\0338\\033[?1049lAFTER")


def vt(binary):
    with Instance(binary, "vt") as ekko:
        ekko.ekko("run", "--detached", "sh", "-c", f"sleep 1.5; printf '{VT}'; exec sleep 600")
        term = ekko.attach(120, 30)
        term.run_for(3.0)
        pane = json.loads(ekko.ekko("status").stdout)["panes"][0]
        x, y = pane["x"], pane["y"]

        def row(k):
            return term.screen.display[y + k][x:x + 16].rstrip()

        def cell(k, c):
            return term.screen.buffer[y + k][x + c]

        cases = {
            "ris": [[row(0), cell(0, 0).bg, cell(0, 5).bg], ["R", "default", "default"]],
            "dec-graphics": [row(1), "\u250c\u2500\u2500\u2510"],
            "shift-out": [row(2), "\u250c\u2500\u2510x"],
            "rep": [row(3), "ABBBB"],
            "insert": [row(4), "XYABCD"],
            "tab-stops": [[row(5), row(6), row(7)], ["    T   U", "    V", "        W"]],
            "erase-attributes": [[row(8), cell(8, 3).underscore], ["abc", False]],
            "colon-sgr": [cell(9, 0).fg, "ff0000"],
            "decsc-rendition": [[row(10), cell(10, 0).fg, cell(10, 0).bold], ["REDin", "red", True]],
            "origin": [row(11), "O"],
            "decsc-alternate": [row(12), "MAINAFTER"],
        }
        term.close()
        return {"failed": [name for name, (got, want) in cases.items() if got != want], "cases": cases}


CLUSTERS = ("sleep 1; printf 'a \\342\\230\\200\\357\\270\\217 b \\360\\237\\221\\250\\342\\200\\215\\360\\237\\221\\251 c END'; "
            "exec sleep 600")


def clusters(binary):
    with Instance(binary, "clusters") as ekko:
        ekko.ekko("run", "--detached", "sh", "-c", CLUSTERS)
        term = ekko.attach(120, 30)
        term.run_for(2.5)
        pane = json.loads(ekko.ekko("status").stdout)["panes"][0]
        line = term.screen.buffer[pane["y"]]
        row = [line[x].data for x in range(term.screen.columns)]
        term.close()
        found = [x - pane["x"] for x in range(len(row) - 2) if row[x:x + 3] == ["E", "N", "D"]]
        return {"end": found, "row": "".join(row[pane["x"]:pane["x"] + 16])}


def advanced(pair):
    return None not in pair and pair[1] > pair[0]


CHECKS = {
    "startup": (startup, lambda r: r["first_full_frame_after_resize"] is not None and r["first_full_frame_after_resize"] < 0.6),
    "click": (click, lambda r: advanced(r["ticks_after_click"])),
    "copy": (copy, lambda r: advanced(r["ticks_after_copy"]) and r["copied"] != []),
    "pane-osc52": (pane_osc52, lambda r: r["forwarded"]),
    "pane-colours": (pane_colours, lambda r: r["answered"]),
    "mouse-flood": (mouse_flood, lambda r: r["repeats"] == 0 and r["app_motion"] == r["cells"] >= 60),
    "reflow": (reflow, lambda r: all(r["intact"])),
    "incremental": (incremental, lambda r: r["frames"] >= 10 and r["differences"] == 0),
    "no-autowrap": (no_autowrap, lambda r: r["rows"][0] == r["expected"][0] and r["rows"][1].rstrip() == r["expected"][1]),
    "vt": (vt, lambda r: not r["failed"]),
    "clusters": (clusters, lambda r: r["end"] == [13]),
    "full-desktop": (full_desktop, lambda r: r["windows"] == 16 and r["chrome_spans"] > 0 and not r["hook_failures"]),
    "refused-answer": (refused_answer, lambda r: r["probe"] == ["probe 3"] and not r["hook_failures"]),
    "slow-hook": (slow_hook, lambda r: r["paused"] and r["banner"] and r["probe"] == ["slow 2"]
                  and not r["hook_failures"] and not r["banner_after"]),
}

failed = []
for name, (scenario, passes) in CHECKS.items():
    result = scenario(sys.argv[1])
    ok = passes(result)
    print(json.dumps({"scenario": name, "pass": ok, **result}))
    if not ok:
        failed.append(name)
sys.exit(1 if failed else 0)
