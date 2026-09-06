"""Real-worker Pane workflow contract using a public profile and layout.

This covers the generic daemon path for both the regular and bare executables.
The paired Zellij workflow oracle owns byte-for-byte comparison; this test
keeps the worker-side mode, process, geometry and action invariants focused.
"""
import argparse
import errno
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
import tty

from daily import eventually


CELL_WIDTH = 8
CELL_HEIGHT = 16
INITIAL_LAYOUT = ["columns", 50, 1, ["rows", 50, 2, 3]]


def dimensions(fd):
    rows, cols, xpixels, ypixels = struct.unpack(
        "HHHH", fcntl.ioctl(fd, termios.TIOCGWINSZ, bytes(8)))
    return {"rows": rows, "cols": cols,
            "xpixels": xpixels, "ypixels": ypixels}


def append_event(path, event, fd=0, data=b""):
    record = {"event": event, "input_hex": data.hex(), "winsize": dimensions(fd)}
    with Path(path).open("a", encoding="utf-8") as output:
        output.write(json.dumps(record, sort_keys=True) + "\n")
        output.flush()


def child(label, events_path, input_path):
    tty.setraw(0, termios.TCSANOW)

    def on_winch(_signum, _frame):
        append_event(events_path, "WINCH")

    signal.signal(signal.SIGWINCH, on_winch)
    append_event(events_path, "FIRST")
    os.write(1, b"WORKFLOW-" + label.encode("ascii") + b" READY\r\n")
    while True:
        try:
            data = os.read(0, 4096)
        except OSError as error:
            if error.errno == errno.EINTR:
                continue
            raise
        if not data:
            return
        with Path(input_path).open("ab", buffering=0) as output:
            output.write(data)
        append_event(events_path, "READ", data=data)


class Viewer:
    def __init__(self, argv, env, cols, rows):
        self.fd, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ,
                    struct.pack("HHHH", rows, cols,
                                cols * CELL_WIDTH, rows * CELL_HEIGHT))
        self.process = subprocess.Popen(argv, stdin=slave, stdout=slave,
                                        stderr=slave, env=env,
                                        start_new_session=True)
        os.close(slave)
        os.set_blocking(self.fd, False)
        self.raw = bytearray()

    def pump(self, seconds=.05):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if select.select([self.fd], [], [], .01)[0]:
                try:
                    data = os.read(self.fd, 65536)
                except OSError as error:
                    if error.errno in (errno.EIO, errno.EBADF):
                        return
                    raise
                if not data:
                    return
                self.raw.extend(data)

    def close(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=3)
        try:
            os.close(self.fd)
        except OSError:
            pass


def read_events(path):
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines()]


def read_input(path):
    return path.read_bytes() if path.exists() else b""


def env_for(root, config):
    return dict(os.environ, XDG_RUNTIME_DIR=str(root), EKKO_CONFIG=str(config),
                TERM="xterm-256color", COLORTERM="truecolor",
                LANG="C.UTF-8", LC_ALL="C.UTF-8")


def cli(binary, env, *args, ok=True):
    result = subprocess.run([binary, *args], env=env, capture_output=True,
                            timeout=8)
    assert (result.returncode == 0) == ok, (
        args, result.returncode, result.stderr.decode(errors="replace"))
    return result


def inspect(binary, env, name):
    return json.loads(cli(binary, env, "inspect", name).stdout)


def status(binary, env, name):
    return json.loads(cli(binary, env, "status", name).stdout)


def wait_for(predicate, viewer, timeout=8):
    deadline = time.monotonic() + timeout

    def check():
        viewer.pump(.02)
        return predicate()

    return eventually(check, timeout=max(.1, deadline - time.monotonic()))


def mode_is(binary, env, name, mode):
    return inspect(binary, env, name)["mode"] == mode


def panes_by_id(state):
    return {pane["id"]: pane for pane in state["panes"]}


def adjacent(candidate, current, direction):
    cx, cy, cw, ch = candidate["layout_rect"]
    x, y, w, h = current["layout_rect"]
    overlap_x = cx < x + w and x < cx + cw
    overlap_y = cy < y + h and y < cy + ch
    return {
        "left": cx + cw == x and overlap_y,
        "right": x + w == cx and overlap_y,
        "up": cy + ch == y and overlap_x,
        "down": y + h == cy and overlap_x,
    }[direction]


def expected_direction_target(state, direction):
    panes = panes_by_id(state)
    current = panes[state["focus"]]
    candidates = [pane for pane in panes.values()
                  if pane["id"] != current["id"] and adjacent(pane, current, direction)]
    assert candidates, (direction, state)
    return max(candidates, key=lambda pane: (pane["activation_order"], pane["id"]))[
        "id"]


def assert_first_events(event_paths, state):
    panes = panes_by_id(state)
    for label, path in event_paths.items():
        events = read_events(path)
        assert len(events) == 1 and events[0]["event"] == "FIRST", (label, events)
        pane = panes[int(label)]
        first = events[0]["winsize"]
        assert (first["cols"], first["rows"]) == (pane["cols"], pane["rows"]), (
            label, first, pane)
        assert (first["xpixels"], first["ypixels"]) == (
            first["cols"] * CELL_WIDTH, first["rows"] * CELL_HEIGHT), first


def write_config(root, profile):
    for helper in profile.parent.glob("zellij-*.lisp"):
        (root / helper.name).write_bytes(helper.read_bytes())
    config = root / "workflow.lisp"
    config.write_text(profile.read_text() + """
(in-package :cl-user)
(ekko/extensions:register-component :id :workflow-layout)
(ekko/extensions:set-option :component :workflow-layout :name :initial-layout
 :value '(:columns 50 1 (:rows 50 2 3)))
""")
    return config


def integration(binary, profile, bare, cols, rows):
    profile = Path(profile).resolve()
    with tempfile.TemporaryDirectory(prefix="ekko-pane-workflow-") as directory:
        root = Path(directory)
        config = write_config(root, profile)
        env = env_for(root, config)
        name = "pane-workflow"
        event_paths = {label: root / f"{label}.events" for label in (1, 2, 3)}
        input_paths = {label: root / f"{label}.input" for label in (1, 2, 3)}
        command = [sys.executable, str(Path(__file__).resolve()), "--child"]
        argv = [binary, "run", "--session", name]
        for label in (1, 2, 3):
            argv.extend(command + [str(label), str(event_paths[label]), str(input_paths[label])])
            if label != 3:
                argv.append(":::")
        viewer = None
        try:
            viewer = Viewer(argv, env, cols, rows)
            wait_for(lambda: all(path.exists() for path in event_paths.values()), viewer)
            initial = inspect(binary, env, name)
            assert initial["layout"] == INITIAL_LAYOUT, initial
            assert (initial["viewport"]["cols"], initial["viewport"]["rows"]) == (cols, rows)
            assert len(initial["panes"]) == 3, initial
            assert_first_events(event_paths, initial)
            initial_pids = {pane["id"]: pane["pid"] for pane in initial["panes"]}
            initial_activation = {pane["id"]: pane["activation_order"]
                                  for pane in initial["panes"]}
            initial_inputs = {label: read_input(path) for label, path in input_paths.items()}

            # Ctrl-p enters Pane mode.  Letter and arrow directional bindings
            # use the same adjacency and activation-recency policy.
            os.write(viewer.fd, b"\x10")
            wait_for(lambda: mode_is(binary, env, name, "pane"), viewer)
            pane_state = inspect(binary, env, name)
            pane_state["focus"] = status(binary, env, name)["focus"]
            expected = expected_direction_target(pane_state, "right")
            os.write(viewer.fd, b"l")
            wait_for(lambda: status(binary, env, name)["focus"] == expected, viewer)
            os.write(viewer.fd, b"\x1b[D")
            wait_for(lambda: status(binary, env, name)["focus"] != expected, viewer)
            after_arrow = status(binary, env, name)
            assert after_arrow["focus"] == 1, after_arrow
            os.write(viewer.fd, b"p")
            wait_for(lambda: status(binary, env, name)["focus"] == 2, viewer)
            assert {label: read_input(path) for label, path in input_paths.items()} == initial_inputs

            # Fullscreen keeps every original PTY alive and retains tiled
            # rectangles for navigation while the focused pane is zoomed.
            os.write(viewer.fd, b"\x1b")
            wait_for(lambda: mode_is(binary, env, name, "normal"), viewer)
            os.write(viewer.fd, b"\x10")
            wait_for(lambda: mode_is(binary, env, name, "pane"), viewer)
            os.write(viewer.fd, b"f")
            wait_for(lambda: inspect(binary, env, name)["zoom"] is True and
                     mode_is(binary, env, name, "normal"), viewer)
            zoomed = inspect(binary, env, name)
            assert {pane["id"]: pane["pid"] for pane in zoomed["panes"]} == initial_pids
            os.write(viewer.fd, b"\x10")
            wait_for(lambda: mode_is(binary, env, name, "pane"), viewer)
            fullscreen_state = inspect(binary, env, name)
            fullscreen_state["focus"] = status(binary, env, name)["focus"]
            fullscreen_expected = expected_direction_target(fullscreen_state, "down")
            os.write(viewer.fd, b"j")
            wait_for(lambda: status(binary, env, name)["focus"] == fullscreen_expected, viewer)
            os.write(viewer.fd, b"\x1b")
            wait_for(lambda: mode_is(binary, env, name, "normal"), viewer)
            os.write(viewer.fd, b"\x10")
            wait_for(lambda: mode_is(binary, env, name, "pane"), viewer)
            os.write(viewer.fd, b"f")
            wait_for(lambda: inspect(binary, env, name)["zoom"] is False and
                     mode_is(binary, env, name, "normal"), viewer)
            restored = status(binary, env, name)
            assert {pane["id"]: pane["pid"] for pane in restored["panes"]} == initial_pids

            # At small dimensions d has insufficient height and reports the
            # bounded red failure note. n has no eligible pane and silently
            # returns to Normal; r still succeeds at the width threshold.
            if cols <= 20 or rows <= 8:
                os.write(viewer.fd, b"\x10")
                wait_for(lambda: mode_is(binary, env, name, "pane"), viewer)
                os.write(viewer.fd, b"d")
                wait_for(lambda: mode_is(binary, env, name, "normal"), viewer)
                failed = inspect(binary, env, name)
                assert failed.get("pane-notes") and failed["pane-notes"][0]["text"] == "CAN'T SPLIT!", failed
                note_count = len(failed["pane-notes"])
                os.write(viewer.fd, b"\x10")
                wait_for(lambda: mode_is(binary, env, name, "pane"), viewer)
                os.write(viewer.fd, b"n")
                wait_for(lambda: mode_is(binary, env, name, "normal"), viewer)
                silent = inspect(binary, env, name)
                assert len(silent.get("pane-notes") or []) == note_count, silent
                assert len(silent["panes"]) == 3, silent
                os.write(viewer.fd, b"\x10")
                wait_for(lambda: mode_is(binary, env, name, "pane"), viewer)
                os.write(viewer.fd, b"r")
                wait_for(lambda: mode_is(binary, env, name, "normal") and
                         len(inspect(binary, env, name)["panes"]) == 4, viewer)
                after_r = inspect(binary, env, name)
                focused = status(binary, env, name)["focus"]
                focused_pane = next(p for p in after_r["panes"] if p["id"] == focused)
                assert focused_pane["cols"] == 3, after_r
                survivors = [pane for pane in after_r["panes"] if pane["id"] != focused]
                expected_survivor = max(
                    survivors,
                    key=lambda pane: (pane["activation_order"], pane["id"]))["id"]
                os.write(viewer.fd, b"\x10")
                wait_for(lambda: mode_is(binary, env, name, "pane"), viewer)
                os.write(viewer.fd, b"x")
                wait_for(lambda: mode_is(binary, env, name, "normal") and
                         len(inspect(binary, env, name)["panes"]) == 3, viewer)
                assert status(binary, env, name)["focus"] == expected_survivor
            else:
                # d, n and r each create a pane with the requested axis and
                # return to Normal.  The worker exposes positive final PTY geometry.
                os.write(viewer.fd, b"\x10")
                wait_for(lambda: mode_is(binary, env, name, "pane"), viewer)
                os.write(viewer.fd, b"d")
                wait_for(lambda: mode_is(binary, env, name, "normal") and
                         len(inspect(binary, env, name)["panes"]) == 4, viewer)
                after_d = inspect(binary, env, name)
                assert all(pane["cols"] >= 1 and pane["rows"] >= 1
                           for pane in after_d["panes"]), after_d
                os.write(viewer.fd, b"\x10")
                wait_for(lambda: mode_is(binary, env, name, "pane"), viewer)
                os.write(viewer.fd, b"n")
                wait_for(lambda: mode_is(binary, env, name, "normal") and
                         len(inspect(binary, env, name)["panes"]) == 5, viewer)
                os.write(viewer.fd, b"\x10")
                wait_for(lambda: mode_is(binary, env, name, "pane"), viewer)
                os.write(viewer.fd, b"r")
                wait_for(lambda: mode_is(binary, env, name, "normal") and
                         len(inspect(binary, env, name)["panes"]) == 6, viewer)
                before_close = inspect(binary, env, name)
                focused = status(binary, env, name)["focus"]
                survivors = [pane for pane in before_close["panes"] if pane["id"] != focused]
                expected_survivor = max(survivors,
                                        key=lambda pane: (pane["activation_order"], pane["id"]))[
                                            "id"]
                os.write(viewer.fd, b"\x10")
                wait_for(lambda: mode_is(binary, env, name, "pane"), viewer)
                os.write(viewer.fd, b"x")
                wait_for(lambda: mode_is(binary, env, name, "normal") and
                         len(inspect(binary, env, name)["panes"]) == 5, viewer)
                closed = inspect(binary, env, name)
                assert status(binary, env, name)["focus"] == expected_survivor, (
                    closed, expected_survivor)

            # Reloading the worker source must preserve the live tree, PTYs and
            # activation recency even when the initializer is present again.
            before_reload = inspect(binary, env, name)
            generation = before_reload["generation"]
            config.write_text(config.read_text() + "\n")
            cli(binary, env, "config", "reload", name)
            def reloaded_state():
                candidate = inspect(binary, env, name)
                return candidate if candidate["generation"] > generation else None

            reloaded = wait_for(reloaded_state, viewer)
            assert reloaded["layout"] == before_reload["layout"], reloaded
            assert {pane["id"]: pane["pid"] for pane in reloaded["panes"]} == {
                pane["id"]: pane["pid"] for pane in before_reload["panes"]}
            assert {pane["id"]: pane["activation_order"] for pane in reloaded["panes"]} == {
                pane["id"]: pane["activation_order"] for pane in before_reload["panes"]}

            cli(binary, env, "stop", name)
            viewer.process.wait(timeout=3)
            assert viewer.process.returncode == 0, bytes(viewer.raw)
        except BaseException:
            try:
                print(bytes(viewer.raw).decode(errors="replace"), file=sys.stderr)
            except Exception:
                pass
            raise
        finally:
            if viewer:
                viewer.close()
            try:
                cli(binary, env, "stop", name)
            except AssertionError:
                pass
    print(json.dumps({"status": "pass", "suite": "pane-workflow-bare" if bare else "pane-workflow",
                      "dimensions": [cols, rows]}))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary")
    parser.add_argument("profile")
    parser.add_argument("mode", choices=("regular", "bare"))
    parser.add_argument("cols", type=int, nargs="?", default=80)
    parser.add_argument("rows", type=int, nargs="?", default=24)
    args = parser.parse_args()
    integration(args.binary, args.profile, args.mode == "bare", args.cols, args.rows)


if __name__ == "__main__":
    if len(sys.argv) >= 2 and sys.argv[1] == "--child":
        if len(sys.argv) != 5:
            raise SystemExit("usage: pane_workflow.py --child LABEL EVENTS INPUT")
        child(sys.argv[2], sys.argv[3], sys.argv[4])
    else:
        main()
