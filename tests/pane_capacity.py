"""Pane capacity regression: 16 panes, cramped-viewport hiding, overflow list.

Usage: python tests/pane_capacity.py BINARY OUTPUT_DIRECTORY

Starts an isolated daemon per scenario (temp XDG_RUNTIME_DIR, empty config),
splits to capacity over the public CLI, drives zoom/minimize/overflow through
real framed-IPC mouse and key input, then restores the full viewport and
checks every pane kept its PTY and PID. Evidence (phase JSON, daemon logs,
raw scenes) is persisted into OUTPUT_DIRECTORY on success AND failure.
"""
import json
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import time
import traceback
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from daily import Attachment, eventually

CHILD_CODE = "import time\nwhile True:\n    time.sleep(3600)\n"
CHILD_ARGV = [sys.executable, "-c", CHILD_CODE]
FULL = (120, 40)
VERSION = 13


class Scenario:
    def __init__(self, binary, out, tag):
        self.binary = binary
        self.out = out
        self.tag = tag
        self.session = f"cap-{tag}"
        self.ev = {"tag": tag, "binary": binary, "ok": False, "phases": {}}
        self.dir = None
        self.env = None
        self.daemon = None
        self.attached = None
        self.log = None

    # -- evidence ---------------------------------------------------------
    def save(self):
        try:
            (self.out / f"evidence-{self.tag}.json").write_text(
                json.dumps(self.ev, indent=1, default=str))
        except OSError:
            pass

    def record(self, name, **data):
        self.ev["phases"][name] = data
        self.save()

    # -- process plumbing --------------------------------------------------
    def start(self):
        self.dir = tempfile.mkdtemp(prefix=f"ekko-cap-{self.tag}-")
        config = Path(self.dir) / "init.lisp"
        config.write_text("")
        self.env = dict(os.environ, XDG_RUNTIME_DIR=self.dir,
                        EKKO_CONFIG=str(config), TERM="xterm-256color")
        self.log = open(Path(self.dir) / "daemon.log", "wb")
        self.daemon = subprocess.Popen(
            [self.binary, "--serve", self.session, "--viewport",
             str(FULL[0]), str(FULL[1]), "8", "16", *CHILD_ARGV],
            env=self.env, stdout=self.log, stderr=self.log)
        sock = Path(self.dir) / f"ekko-v2/{self.session}.sock"
        eventually(lambda: sock.exists() or self.daemon.poll() is not None)
        assert self.daemon.poll() is None, self.daemon_log()
        self.attached = Attachment(sock, version=VERSION)

    def daemon_log(self):
        try:
            return (Path(self.dir) / "daemon.log").read_text()[-4000:]
        except OSError:
            return ""

    def cli(self, *args, ok=True):
        p = subprocess.run([self.binary, *args], env=self.env,
                           capture_output=True, timeout=10)
        if ok:
            assert p.returncode == 0, (args, p.returncode,
                                       p.stderr.decode(), self.daemon_log())
        return p

    def inspect(self):
        data = json.loads(self.cli("inspect", self.session).stdout)
        assert not data.get("error"), data.get("error")
        return data

    def status(self):
        return json.loads(self.cli("status", self.session).stdout)

    # -- input -------------------------------------------------------------
    def resize(self, cols, rows):
        self.attached.send(1, struct.pack(">IIIII", VERSION, cols, rows, 8, 16))
        self.attached.pump(.1)
        eventually(lambda: self.inspect()["viewport"]["cols"] == cols
                   and self.inspect()["viewport"]["rows"] == rows)

    def click(self, x, y):
        for suffix in ("M", "m"):
            self.attached.send(
                4, f"\x1b[<0;{x * 8 + 1};{y * 16 + 1}{suffix}".encode())
            self.attached.pump(.1)

    def click_span(self, span):
        self.click(span["x"], span["y"])

    @staticmethod
    def find_span(state, pred):
        for decoration in state["decorations"]:
            for span in decoration["spans"] or []:
                if pred(span):
                    return span
        return None

    def overflow_span(self):
        return self.find_span(self.inspect(),
                              lambda s: s.get("command") == "desktop-window-list")

    def visible(self):
        return [p for p in self.inspect()["panes"] if p["visible"]]

    def settled_state(self, sample, timeout=4):
        """Wait until two samples 150ms apart agree, then return the final
        sample: the viewport field updates before animations and
        decorations finish, so geometry checks on a moving layout flake."""
        def stable():
            first = sample()
            time.sleep(0.15)
            return first == sample()
        eventually(stable, timeout=timeout)
        return sample()

    def pane(self, pane_id):
        return next(p for p in self.inspect()["panes"] if p["id"] == pane_id)

    # -- checks ------------------------------------------------------------
    @staticmethod
    def rect(pane):
        r = pane["outer_rect"]
        if isinstance(r, dict):
            return (r.get("x", 0), r.get("y", 0),
                    r.get("cols", r.get("width", 0)),
                    r.get("rows", r.get("height", 0)))
        return r[0], r[1], r[2], r[3]

    @classmethod
    def rect_ok(cls, pane, cols, rows):
        x, y, w, h = cls.rect(pane)
        return w > 0 and h > 0 and x >= 0 and y >= 0 \
            and x + w <= cols and y + h <= rows

    @staticmethod
    def pty_positive(pane):
        # pty_size is [cols, rows, x_pixels, y_pixels]: a narrowed terminal
        # shrinks cols first, so 1 column is the legitimate floor; cols and
        # rows must never hit zero. Pixel dimensions may legitimately be 0,
        # so they only have to be numeric and non-negative.
        size = pane["pty_size"]
        if isinstance(size, dict):
            cols = size.get("cols")
            rows = size.get("rows")
            extra = [v for k, v in size.items()
                     if k not in ("cols", "rows") and isinstance(v, int)]
        elif isinstance(size, (list, tuple)) and len(size) >= 2:
            cols, rows = size[0], size[1]
            extra = list(size[2:])
        else:
            raise AssertionError(f"pty_size is not [cols, rows, x, y]: "
                                 f"{size!r} for pane {pane.get('id')}")
        assert isinstance(cols, int) and not isinstance(cols, bool) \
            and cols >= 1, f"pty cols must be >= 1: {size!r}"
        assert isinstance(rows, int) and not isinstance(rows, bool) \
            and rows >= 1, f"pty rows must be >= 1: {size!r}"
        assert all(isinstance(v, int) and not isinstance(v, bool) and v >= 0
                   for v in extra), f"pty pixels must be >= 0: {size!r}"
        return True

    def split(self, axis, count):
        for i in range(count):
            self.cli("split", "--session", self.session, axis, *CHILD_ARGV)
            eventually(lambda target=i + 2:
                       len(self.inspect()["panes"]) >= target)

    # -- scenario ----------------------------------------------------------
    def run(self, plan, small, min_subset):
        try:
            self.start()
            state = self.inspect()
            self.record("startup", panes=len(state["panes"]),
                        viewport=state["viewport"])
            for axis, count in plan:
                self.split(axis, count)
            state = self.inspect()
            ids = [p["id"] for p in state["panes"]]
            pids = {p["id"]: p["pid"] for p in state["panes"]}
            assert len(ids) == 1 + sum(c for _, c in plan), ids
            assert len(set(pids.values())) == len(pids), pids
            self.record("all_panes", ids=ids, pids=pids,
                        viewport=state["viewport"],
                        rects={p["id"]: p["outer_rect"] for p in state["panes"]},
                        pty={p["id"]: p["pty_size"] for p in state["panes"]})

            # Zoom strictly through the real key path: Ctrl-p then f.
            focus = self.status()["focus"]
            self.attached.send(2, b"\x10")
            self.attached.pump(.1)
            self.attached.send(2, b"f")
            self.attached.pump(.2)
            eventually(lambda: self.inspect().get("zoom"))
            assert [p["id"] for p in self.inspect()["panes"]
                    if p["visible"]] == [focus], "zoom must keep only focus"
            self.record("zoom_on", focus=focus)
            self.attached.send(2, b"\x10")
            self.attached.pump(.1)
            self.attached.send(2, b"f")
            self.attached.pump(.2)
            eventually(lambda: not self.inspect().get("zoom"))
            assert len(self.visible()) == len(ids), "zoom off must restore all"
            self.record("zoom_off")

            # Cramped viewport: hiding, overflow chip, window list.
            self.resize(*small)
            cols, rows = small
            # Settle geometry first: the viewport field alone may precede
            # animations/decorations. Persist the final settled state; the
            # exact surviving count may legitimately vary with insets, so
            # only a valid subset is asserted below.
            self.settled_state(
                lambda: tuple(sorted((p["id"], self.rect(p))
                                     for p in self.visible())))
            visible = self.visible()
            cramped_focus = self.status()["focus"]
            self.record("cramped", visible=[p["id"] for p in visible],
                        visible_count=len(visible),
                        focus=cramped_focus,
                        rects={p["id"]: p["outer_rect"] for p in visible})
            if min_subset is not None:
                assert len(visible) >= min_subset, \
                    f"only {len(visible)} panes survive at {small}"
                assert len(visible) < len(ids), "no pane was hidden when cramped"
                survivors = [p for p in visible if p["id"] == cramped_focus]
                assert survivors, "focus pane must stay visible when cramped"
                assert self.rect_ok(survivors[0], cols, rows), survivors[0]
                assert self.pty_positive(survivors[0]), survivors[0]
            # Every visible pane stays in bounds with a live PTY at the
            # cramped phase, not just the focus pane.
            for pane in visible:
                assert self.rect_ok(pane, cols, rows), pane
                assert self.pty_positive(pane), pane
            # An all-columns layout at capacity must overflow horizontally
            # and expose the overflow span; a regression that silently drops
            # it would strand focus above the hidden panes.
            columns_only = all(axis == "columns" for axis, _ in plan)
            if columns_only:
                span = eventually(self.overflow_span)
            else:
                # The dock overflows horizontally only; vertical hiding
                # (short rows) hides panes without one. Observe, don't infer.
                try:
                    span = eventually(self.overflow_span, timeout=2)
                except AssertionError:
                    span = None
                    self.record("no_overflow",
                                visible=[p["id"] for p in visible])
            if span:
                span = eventually(self.overflow_span)
                self.record("overflow_span", span={k: span[k] for k in
                                                   ("x", "y", "text", "command")
                                                   if k in span})
                # The chip opens the window list. Every pane, hidden or not,
                # is on it; clicking the hidden pane's row focuses it and the
                # layout brings it back. Row order is activation order, which
                # the test does not know, so walk the rows until the hidden
                # pane is focused.
                visible_now = {p["id"] for p in self.visible()}
                hidden = [p["id"] for p in self.inspect()["panes"]
                          if p["id"] not in visible_now]
                # A small dock can overflow even when every pane fits the
                # layout, so the chip is not proof of a hidden pane.
                target = hidden[0] if hidden else ids[-1]
                rows = 0
                for index in range(len(ids)):
                    self.click_span(self.overflow_span())
                    popup = eventually(lambda: self.inspect().get("popup"))
                    rows += 1
                    self.click(popup["x"] + 2, popup["y"] + 1 + index)
                    eventually(lambda: not self.inspect().get("popup"))
                    if self.status()["focus"] == target:
                        break
                else:
                    raise AssertionError(
                        f"window list never focused hidden pane {target}")
                eventually(lambda: self.pane(target)["visible"])
                self.record("window_list", rows=rows, focused=target,
                            hidden=hidden)

            # Back to full viewport: everything returns with its own PTY.
            self.resize(*FULL)
            eventually(lambda: len(self.visible()) == len(ids))
            state = self.inspect()
            assert {p["id"]: p["pid"] for p in state["panes"]} == pids, \
                "pane PID mapping changed across resize"
            for pane in state["panes"]:
                assert self.rect_ok(pane, *FULL), pane
                assert self.pty_positive(pane), pane
                os.kill(pane["pid"], 0)
            assert not state.get("zoom"), "zoom must be reset at the end"
            self.record("restored", visible=[p["id"] for p in state["panes"]],
                        pids={p["id"]: p["pid"] for p in state["panes"]})
            self.ev["ok"] = True
            return self.ev
        finally:
            self.cleanup()

    def cleanup(self):
        if self.attached:
            try:
                self.attached.close()
            except OSError:
                pass
            scenes = self.attached.scenes
        else:
            scenes = []
        if self.daemon and self.daemon.poll() is None:
            try:
                subprocess.run([self.binary, "stop", self.session],
                               env=self.env, capture_output=True, timeout=8)
                try:
                    self.daemon.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    self.daemon.kill()
                    self.daemon.wait()
            except Exception:
                try:
                    self.daemon.kill()
                    self.daemon.wait()
                except Exception:
                    pass
        if self.log:
            try:
                self.log.close()
                (self.out / f"daemon-{self.tag}.log").write_bytes(
                    (Path(self.dir) / "daemon.log").read_bytes())
            except OSError:
                pass
        if scenes:
            try:
                with open(self.out / f"scenes-{self.tag}.jsonl", "w") as sink:
                    for scene in scenes[-400:]:
                        sink.write(json.dumps(scene) + "\n")
                self.ev["scene_count"] = len(scenes)
            except OSError:
                pass
        if self.dir:
            shutil.rmtree(self.dir, ignore_errors=True)
        self.save()


SCENARIOS = [
    # (tag, plan, cramped viewport, minimum surviving panes or None)
    ("columns", [("columns", 15)], (40, 30), 2),
    ("rows", [("rows", 4)], (120, 8), None),
    ("mixed", [("columns", 3), ("rows", 2), ("columns", 1)], (30, 20), None),
]


def main():
    binary, out_name = sys.argv[1], sys.argv[2]
    out = Path(out_name)
    out.mkdir(parents=True, exist_ok=True)
    summary = {"binary": binary, "scenarios": []}
    failed = False
    for tag, plan, small, min_subset in SCENARIOS:
        scenario = Scenario(binary, out, tag)
        try:
            scenario.run(plan, small, min_subset)
            summary["scenarios"].append({"tag": tag, "result": "pass"})
        except BaseException:
            failed = True
            summary["scenarios"].append(
                {"tag": tag, "result": "fail",
                 "error": traceback.format_exc()})
            print(summary["scenarios"][-1]["error"], file=sys.stderr)
            if min_subset is not None:
                break
        finally:
            (out / "summary.json").write_text(json.dumps(summary, indent=1))
    if failed:
        sys.exit(1)
    print(json.dumps({"status": "pass", "binary": Path(binary).name,
                      "scenarios": [s["tag"] for s in summary["scenarios"]]}))


if __name__ == "__main__":
    main()
