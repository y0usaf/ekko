"""Bounded real-PTY workflow comparison for a three-pane Pane slice.

The runner keeps each side's raw ANSI stream, application input, child PTY
dimension events, Pyte cells and candidate state snapshots.  The comparison is
evidence for this named workflow only; it is not a parity gate for the whole
Zellij surface.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import select
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import termios

from differential import Terminal


COLS = 80
ROWS = 24
CELL_WIDTH = 8
CELL_HEIGHT = 16
STAGES = [
    ("startup", b""),
    ("dismiss-release-notes", b"\x1b"),
    ("pane-enter", b"\x10"),
    ("direction-right", b"l"),
    ("switch-focus", b"p"),
    ("new-pane", b"n"),
    ("pane-reenter-down", b"\x10"),
    ("new-pane-down", b"d"),
    ("pane-reenter-right", b"\x10"),
    ("new-pane-right", b"r"),
    ("pane-reenter-close", b"\x10"),
    ("close-focus", b"x"),
]
SCENARIOS = {
    "navigation": STAGES[:5],
    "new-n": STAGES[:3] + [("new-pane", b"n")],
    "new-d": STAGES[:3] + [("new-pane-down", b"d")],
    "new-r": STAGES[:3] + [("new-pane-right", b"r")],
    "close-x": STAGES[:3] + [("close-focus", b"x")],
    "fullscreen-navigation": STAGES[:3] + [
        ("fullscreen-on", b"f"), ("fullscreen-reenter", b"\x10"),
        ("fullscreen-focus-right", b"l"), ("fullscreen-off", b"f")],
}


class QueryTerminal(Terminal):
    """PTY viewer that answers only terminal queries emitted by the app."""

    _CSI = re.compile(rb"\x1b\[[0-9;?]*([@-~])")

    def __init__(self, argv, env, cols, rows, answer_queries=True):
        super().__init__(argv, env, cols, rows)
        self._query_pending = bytearray()
        self.query_log = []
        self.answer_queries = answer_queries

    def _answer_queries(self, data):
        self._query_pending.extend(data)
        while self._query_pending:
            start = self._query_pending.find(b"\x1b[")
            if start < 0:
                # Preserve a possible ESC prefix split across PTY reads.
                self._query_pending[:] = self._query_pending[-1:] \
                    if self._query_pending.endswith(b"\x1b") else b""
                return
            if start:
                del self._query_pending[:start]
            match = self._CSI.match(self._query_pending)
            if match is None:
                # Keep a valid numeric CSI prefix until its final byte arrives;
                # PTY reads may split ESC, [, parameters, and t separately.
                # Invalid or unbounded sequences are discarded one byte at a
                # time so output cannot stall the responder indefinitely.
                prefix = self._query_pending[2:]
                if (len(self._query_pending) <= 66 and
                        all(byte in b"0123456789;?" for byte in prefix)):
                    return
                if len(self._query_pending) > 2:
                    del self._query_pending[:1]
                    continue
                return
            sequence = bytes(match.group(0))
            final = bytes(match.group(1))
            del self._query_pending[:len(sequence)]
            if final != b"t":
                continue
            try:
                parameters = sequence[2:-1].decode("ascii")
                code = int(parameters or "0")
            except ValueError:
                continue
            if not self.answer_queries:
                self.query_log.append({"query_hex": sequence.hex(),
                                       "response_hex": None, "code": code})
                continue
            response = {
                14: f"\x1b[4;{self.rows * CELL_HEIGHT};{self.cols * CELL_WIDTH}t".encode(),
                16: f"\x1b[6;{CELL_HEIGHT};{CELL_WIDTH}t".encode(),
                18: f"\x1b[8;{self.rows};{self.cols}t".encode(),
                19: f"\x1b[9;{self.rows};{self.cols}t".encode(),
            }.get(code)
            if response is None:
                continue
            os.write(self.fd, response)
            self.query_log.append({"query_hex": sequence.hex(),
                                   "response_hex": response.hex(),
                                   "code": code})

    def pump(self, duration=.25):
        deadline = time.monotonic() + duration
        while time.monotonic() < deadline:
            if select.select([self.fd], [], [], .01)[0]:
                try:
                    data = os.read(self.fd, 65536)
                except OSError:
                    break
                if not data:
                    break
                self.raw.extend(data)
                self._answer_queries(data)
                self.stream.feed(self.decoder.decode(data))


def dimensions(fd):
    import fcntl
    import struct
    import termios

    rows, cols, xpixels, ypixels = struct.unpack(
        "HHHH", fcntl.ioctl(fd, termios.TIOCGWINSZ, bytes(8)))
    return [rows, cols, xpixels, ypixels]


def read_bytes(path):
    return path.read_bytes() if path.exists() else b""


def read_events(path):
    if not path.exists():
        return []
    return [json.loads(line) for line in path.read_text().splitlines()]


def env_for(work):
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(("ZELLIJ", "EKKO", "XDG_"))}
    env.update(HOME=str(work), XDG_RUNTIME_DIR=str(work),
               XDG_CONFIG_HOME=str(work), XDG_CACHE_HOME=str(work),
               XDG_DATA_HOME=str(work), TERM="xterm-256color",
               COLORTERM="truecolor", LANG="C.UTF-8", LC_ALL="C.UTF-8",
               SHELL="/bin/sh")
    return env


def wait_for(predicate, terminal, timeout=8):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        terminal.pump(.05)
        value = predicate()
        if value:
            return value
    raise AssertionError("condition did not become true")


def app_paths(work):
    return {label: (work / f"{label}.input", work / f"{label}.events")
            for label in ("A", "B", "C")}


def child_args(shell, paths, label):
    input_path, events_path = paths[label]
    return [str(shell), "--fixture", label,
            str(input_path), str(events_path)]


def write_zellij_layout(work, shell, paths):
    # This is the same 50/50 L tree supplied to Ekko's declarative startup
    # option.  Both sides therefore spawn the same three commands at startup.
    def pane(label, extra=""):
        args = child_args(shell, paths, label)
        return [f'pane {extra} command={json.dumps(str(shell))} {{',
                "    args " + " ".join(json.dumps(arg) for arg in args[1:]),
                "}"]

    lines = ["layout {", '    pane split_direction="vertical" {']
    lines.extend("        " + line for line in pane("A"))
    lines.append('        pane split_direction="horizontal" {')
    lines.extend("            " + line for line in pane("B"))
    lines.extend("            " + line for line in pane("C"))
    lines.extend(["        }", "    }", "}", ""])
    layout = work / "workflow.kdl"
    layout.write_text("\n".join(lines))
    return layout


def write_spawn_shell(root):
    """Create the one executable and argv contract used by both muxes."""
    path = root / "workflow-shell.py"
    path.write_text(inspect_spawn_source())
    path.chmod(path.stat().st_mode | stat.S_IXUSR)
    return path


def inspect_spawn_source():
    # This standalone wrapper avoids importing the test module from a child
    # PTY.  With --fixture it is the initial app; with no arguments it is the
    # shell used by every later NewPane action.
    return r'''#!/usr/bin/env python3
import errno
import fcntl
import json
import os
import signal
import struct
import sys
import termios
import tty
from pathlib import Path

def size():
    rows, cols, xp, yp = struct.unpack("HHHH", fcntl.ioctl(0, termios.TIOCGWINSZ, bytes(8)))
    return [rows, cols, xp, yp]

def run(label, input_path, events_path, message):
    tty.setraw(0, termios.TCSANOW)
    events = Path(events_path)
    inputs = Path(input_path)
    def event(kind, data=b""):
        with events.open("a", encoding="utf-8") as output:
            output.write(json.dumps({"event": kind, "input_hex": data.hex(),
                                     "pid": os.getpid(), "winsize": size()},
                                    sort_keys=True) + "\n")
            output.flush()
    def on_winch(_signum, _frame):
        event("WINCH")
    signal.signal(signal.SIGWINCH, on_winch)
    event("FIRST")
    os.write(1, message)
    while True:
        try:
            data = os.read(0, 4096)
        except OSError as error:
            if error.errno == errno.EINTR:
                continue
            raise
        if not data:
            return
        with inputs.open("ab", buffering=0) as output:
            output.write(data)
        event("READ", data)
        if message == b"WORKFLOW-" + label.encode() + b" READY\r\n":
            os.write(1, b"WORKFLOW-READY\r\n")

if len(sys.argv) == 5 and sys.argv[1] == "--fixture":
    run(sys.argv[2], sys.argv[3], sys.argv[4],
        b"WORKFLOW-" + sys.argv[2].encode() + b" READY\r\n")
else:
    root = Path(__file__).resolve().parent / "session"
    lock_path = root / "spawn-sequence.lock"
    with lock_path.open("a+") as lock:
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
        lock.seek(0)
        previous = lock.read().strip()
        ordinal = int(previous or "0") + 1
        lock.seek(0)
        lock.truncate()
        lock.write(str(ordinal))
        lock.flush()
        os.fsync(lock.fileno())
        fcntl.flock(lock.fileno(), fcntl.LOCK_UN)
    label = "spawn-%d" % ordinal
    run(label, root / (label + ".input"),
        root / (label + ".events"), b"WORKFLOW-SPAWNED READY\r\n")
'''


def lisp_string_list(values):
    return "(" + " ".join(json.dumps(value) for value in values) + ")"


def write_ekko_config(work, profile, shell):
    config = work / "workflow.lisp"
    config.write_text(
        "(load " + json.dumps(str(profile.resolve())) + ")\n"
        "(ekko/extensions:register-component :id :workflow-test)\n"
        "(ekko/extensions:set-option :component :workflow-test :name :shell :value "
        + "'" + lisp_string_list([str(shell)]) + ")\n"
        # The reference L layout has no status bar. Pane frames still consume
        # one cell at each content edge, yielding the same 38x22 / 38x10
        # application rectangles at an 80x24 outer size.
        "(ekko/extensions:set-option :component :workflow-test :name :viewport-insets :value "
        "'(0 0 0 0))\n"
        # Match the pinned KDL fixture's 50/50 L tree at startup. The one-based
        # leaves are the three command
        # slots supplied to `run`; no post-spawn resize is involved.
        "(ekko/extensions:set-option :component :workflow-test :name :initial-layout :value "
                "'(:columns 50 1 (:rows 50 2 3)))\n")
    return config


def verify_query_responder():
    """Exercise split CSI reads without launching another terminal process."""
    read_fd, write_fd = os.pipe()
    try:
        terminal = object.__new__(QueryTerminal)
        terminal._query_pending = bytearray()
        terminal.query_log = []
        terminal.fd = write_fd
        terminal.rows, terminal.cols = ROWS, COLS
        terminal.answer_queries = True
        queries = (b"\x1b[14t", b"\x1b[16t", b"\x1b[18t", b"\x1b[19t")
        for query in queries:
            for byte in query:
                terminal._answer_queries(bytes((byte,)))
        expected = [14, 16, 18, 19]
        assert [entry["code"] for entry in terminal.query_log] == expected
        responses = os.read(read_fd, 4096)
        assert responses == b"".join(bytes.fromhex(entry["response_hex"])
                                     for entry in terminal.query_log)
    finally:
        os.close(read_fd)
        os.close(write_fd)


def inspect_candidate(binary, env):
    result = subprocess.run([binary, "inspect", "workflow"], env=env,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            timeout=8)
    assert result.returncode == 0, result.stderr.decode(errors="replace")
    return json.loads(result.stdout)


def status_candidate(binary, env):
    result = subprocess.run([binary, "status", "workflow"], env=env,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            timeout=8)
    assert result.returncode == 0, result.stderr.decode(errors="replace")
    return json.loads(result.stdout)


def fixture_state(paths):
    inputs = {label: read_bytes(pair[0]).hex() for label, pair in paths.items()}
    events = {label: read_events(pair[1]) for label, pair in paths.items()}
    for path in sorted(next(iter(paths.values()))[0].parent.glob("spawn-*.events")):
        label = path.stem
        events[label] = read_events(path)
        input_path = path.with_suffix(".input")
        inputs[label] = read_bytes(input_path).hex()
    return {"inputs": inputs, "events": events}


def focus_probe(terminal, paths, kind, binary, env, resume, marker=b"Q",
                leave_pane=True):
    """Identify the focused child through a normal-mode printable marker."""
    before = fixture_state(paths)
    if leave_pane:
        os.write(terminal.fd, b"\x1b")
        terminal.pump(.25)
    if kind == "ekko" and leave_pane:
        def candidate_mode(mode):
            state = inspect_candidate(binary, env)
            return state if state["mode"] == mode else None
        normal = wait_for(lambda: candidate_mode("normal"), terminal)
    elif kind == "ekko":
        normal = inspect_candidate(binary, env)
    else:
        normal = None
    normal_inputs = fixture_state(paths)["inputs"]
    os.write(terminal.fd, marker)
    terminal.pump(.35)
    after_marker = fixture_state(paths)
    receivers = []
    for label, value in after_marker["inputs"].items():
        old = normal_inputs.get(label, "")
        delta = value[len(old):]
        if marker.hex() in delta:
            receivers.append(label)
    assert len(receivers) == 1, (kind, marker, before, after_marker)
    if resume:
        os.write(terminal.fd, b"\x10")
        terminal.pump(.25)
        if kind == "ekko":
            wait_for(lambda: candidate_mode("pane"), terminal)
    return {
        "escape_hex": "1b",
        "marker_hex": marker.hex(),
        "resume_hex": "10" if resume else None,
        "receiver": receivers[0],
        "input_before": before["inputs"],
        "input_after_marker": after_marker["inputs"],
        "resumed": resume,
        "normal_state": normal,
    }


def assert_initial_geometry(kind, paths, inspect, cols, rows):
    """Require the 50/50 L first ioctl to equal its final pane geometry."""
    left = cols // 2
    right = cols - left
    top = rows // 2
    bottom = rows - top
    expected = ((max(1, left - 2), max(1, rows - 2)),
                (max(1, right - 2), max(1, top - 2)),
                (max(1, right - 2), max(1, bottom - 2)))
    if kind == "ekko":
        panes = sorted(inspect["panes"], key=lambda pane: pane["id"])
        assert [(pane["cols"], pane["rows"]) for pane in panes] == list(expected), inspect
    fixtures = fixture_state(paths)
    for label, (pane_cols, pane_rows) in zip(("A", "B", "C"), expected):
        events = fixtures["events"].get(label, [])
        assert events and events[0]["event"] == "FIRST", (kind, label, events)
        assert len(events) == 1, (kind, label, events)
        assert events[0]["winsize"][:2] == [pane_rows, pane_cols], (kind, label, events)


def assert_candidate_geometry(state):
    """Validate every settled candidate rectangle and its PTY content size."""
    pane_insets = state["geometry"]["pane-insets"]
    top, right, bottom, left = pane_insets
    viewport = state["viewport"]
    viewport_insets = viewport["insets"]
    viewport_width = viewport["cols"] - viewport_insets[1] - viewport_insets[3]
    viewport_height = viewport["rows"] - viewport_insets[0] - viewport_insets[2]
    for pane in state["panes"]:
        outer = pane["outer_rect"]
        assert len(outer) == 4 and outer[2] > 0 and outer[3] > 0, state
        assert 0 <= outer[0] and 0 <= outer[1], state
        assert outer[0] + outer[2] <= viewport_width, state
        assert outer[1] + outer[3] <= viewport_height, state
        assert (pane["x"], pane["y"]) == (outer[0] + left, outer[1] + top), state
        assert (pane["cols"], pane["rows"]) == (
            max(1, outer[2] - left - right),
            max(1, outer[3] - top - bottom)), state


def expected_split_count(stage, cols, rows):
    """Return the pinned source's split result for the requested viewport."""
    if stage == "new-pane-down":
        succeeds = rows >= 10
    elif stage == "new-pane-right":
        succeeds = cols // 2 >= 10
    else:
        # The default split is accepted when one of the three initial outer
        # rectangles has a usable dimension above ten cells.
        left, right = cols // 2, cols - cols // 2
        top, bottom = rows // 2, rows - rows // 2
        succeeds = any(width >= 5 and height >= 5 and (width > 10 or height > 10)
                       for width, height in ((left, rows), (right, top), (right, bottom)))
    return 4 if succeeds else 3


def snapshot(terminal, stage, sent, paths, before, side, inspect=None, status=None):
    fixtures = fixture_state(paths)
    current_inputs = {label: bytes.fromhex(data)
                      for label, data in fixtures["inputs"].items()}
    before_inputs = {label: before.get(label, b"") for label in current_inputs}
    return {
        "name": stage, "sent_hex": sent.hex(), "side": side,
        "terminal_dimensions": [terminal.cols, terminal.rows],
        "terminal_size_ioctl": dimensions(terminal.fd),
        "terminal_queries": list(terminal.query_log),
        "cursor": [terminal.screen.cursor.x, terminal.screen.cursor.y,
                    terminal.screen.cursor.hidden],
        "display": list(terminal.screen.display),
        "cells": [[terminal.screen.buffer[y][x]._asdict()
                   for x in range(terminal.cols)] for y in range(terminal.rows)],
        "fixture_input_hex": fixtures["inputs"],
        "fixture_input_delta_hex": {
            label: current_inputs[label][len(before_inputs.get(label, b"")):].hex()
            for label in current_inputs},
        "fixture_events": fixtures["events"],
        "inspect": inspect,
        "status": status,
    }


def run_side(kind, binary, profile, reference, root, output, shell,
             stage_plan, cols, rows, answer_queries=True):
    # Recreate exactly one workdir for each side. This keeps the generated
    # shell path and all non-EKKO environment values identical.
    work = root / "session"
    if work.exists():
        shutil.rmtree(work)
    work.mkdir()
    paths = app_paths(work)
    env = env_for(work)
    if kind == "zellij":
        config = work / "config.kdl"
        config.write_text((reference / "config.kdl").read_text()
                          + f'\ndefault_shell {json.dumps(str(shell))}\n')
        layout = write_zellij_layout(work, shell, paths)
        argv = [binary, "--config", str(config), "--new-session-with-layout",
                str(layout), "--session", "workflow"]
        cleanup = [binary, "kill-session", "workflow"]
    else:
        config = write_ekko_config(work, profile, shell)
        env["EKKO_CONFIG"] = str(config)
        argv = [binary, "run", "--session", "workflow"]
        for index, label in enumerate(("A", "B", "C")):
            argv.extend(child_args(shell, paths, label))
            if index != 2:
                argv.append(":::")
        cleanup = [binary, "stop", "workflow"]
    terminal = QueryTerminal(argv, env, cols, rows, answer_queries)
    stages = []
    fixture_contract = {
        "initial_argv": {label: child_args(shell, paths, label)
                         for label in ("A", "B", "C")},
        "spawn_argv": [str(shell)],
        "env": {key: value for key, value in env.items()
                if key != "EKKO_CONFIG"},
    }

    def record(name, sent, before, inspect=None, status=None):
        value = snapshot(terminal, name, sent, paths, before, kind,
                         inspect, status)
        value["fixture_contract"] = fixture_contract
        stages.append(value)

    primary_error = None
    cleanup_error = None
    try:
        wait_for(lambda: all(pair[1].exists() for pair in paths.values()), terminal)
        terminal.pump(1)
        before = {label: read_bytes(pair[0]) for label, pair in paths.items()}
        record("startup", b"", before,
               inspect_candidate(binary, env) if kind == "ekko" else None,
               status_candidate(binary, env) if kind == "ekko" else None)
        assert_initial_geometry(kind, paths,
                                stages[-1]["inspect"] if kind == "ekko" else None,
                                cols, rows)
        for stage, keys in stage_plan[1:]:
            before = {label: read_bytes(pair[0]) for label, pair in paths.items()}
            os.write(terminal.fd, keys)
            expected_count = None
            probe_stage = stage in {
                "direction-right", "switch-focus", "fullscreen-focus-right",
                "new-pane", "new-pane-down", "new-pane-right", "close-focus",
            }
            if kind == "ekko":
                expected_count = (expected_split_count(stage, cols, rows)
                                  if stage in {"new-pane", "new-pane-down",
                                                "new-pane-right"} else
                                  {"close-focus": 2}.get(stage))
                expected_focus = {
                    "direction-right": 3,
                    "switch-focus": 1,
                    "fullscreen-on": 1,
                    "fullscreen-reenter": 1,
                    "fullscreen-focus-right": 3,
                    "fullscreen-off": 3,
                    "close-focus": 3,
                }.get(stage)
                if stage in ("new-pane", "new-pane-down", "new-pane-right"):
                    # A rejected split leaves pane 1 focused; each successful
                    # split focuses its new pane 4.
                    expected_focus = 4 if expected_count == 4 else 1
                expected_zoom = {
                    "fullscreen-on": True,
                    "fullscreen-reenter": True,
                    "fullscreen-focus-right": True,
                    "fullscreen-off": False,
                }.get(stage, False)

                def candidate_ready():
                    state = inspect_candidate(binary, env)
                    status = status_candidate(binary, env)
                    mode = state["mode"]
                    if stage in ("pane-enter", "direction-right", "switch-focus",
                                 "pane-reenter-down", "pane-reenter-right",
                                 "pane-reenter-close", "fullscreen-reenter",
                                 "fullscreen-focus-right") and mode != "pane":
                        return None
                    if stage in ("new-pane", "new-pane-down", "new-pane-right",
                                 "close-focus", "fullscreen-on", "fullscreen-off") and mode != "normal":
                        return None
                    if expected_count is not None and len(state["panes"]) != expected_count:
                        return None
                    if expected_focus is not None and status["focus"] != expected_focus:
                        return None
                    if state["zoom"] is not expected_zoom:
                        return None
                    return state, status

                state, status = wait_for(candidate_ready, terminal)
                # Directional focus and SwitchFocus are required to remain in
                # Pane mode and move selection; creation/close return Normal.
                if stage in ("direction-right", "switch-focus", "fullscreen-focus-right"):
                    prior = stages[-1]["status"]["focus"]
                    assert status["focus"] != prior, (stage, prior, status)
                if stage == "fullscreen-on":
                    assert state["zoom"] is True, state
                if stage == "fullscreen-off":
                    assert state["zoom"] is False, state
                assert_candidate_geometry(state)
                expected_focus = {
                    # Zellij's directional tie break selects C from A because
                    # the two right candidates have the same edge and C is
                    # the more recently activated pane.
                    "direction-right": 3,
                    "switch-focus": 1,
                    "new-pane": 4 if expected_count == 4 else 1,
                    "new-pane-down": 4 if expected_count == 4 else 1,
                    "new-pane-right": 4 if expected_count == 4 else 1,
                    "close-focus": 3,
                    "fullscreen-focus-right": 3,
                    "fullscreen-off": 3,
                }.get(stage)
                if expected_focus is not None:
                    assert status["focus"] == expected_focus, (stage, status, state)
                inspect = state
            else:
                terminal.pump(.5)
                inspect = status = None
            record(stage, keys, before, inspect, status)
            # Keep this stage's terminal grid and fixture state before the
            # marker; the probe's post-marker raw writes are retained beside
            # it and become the next stage's input delta.
            if probe_stage:
                stages[-1]["focus_probe"] = focus_probe(
                    terminal, paths, kind, binary, env,
                    resume=stage in {"direction-right", "switch-focus",
                                     "fullscreen-focus-right"},
                    marker=b"Q",
                    leave_pane=stage in {"direction-right", "switch-focus",
                                         "fullscreen-focus-right"})
        return stages
    except BaseException as error:
        primary_error = error
        raise
    finally:
        result = None
        try:
            result = subprocess.run(cleanup, env=env, stdout=subprocess.PIPE,
                                    stderr=subprocess.PIPE, timeout=8)
            if result.returncode:
                cleanup_error = AssertionError(
                    f"{kind} cleanup failed: {result.stderr.decode(errors='replace')}")
        except BaseException as error:
            cleanup_error = error
        try:
            terminal.close()
        except BaseException as error:
            cleanup_error = cleanup_error or error
        (output / f"{kind}-cleanup.json").write_text(json.dumps({
            "command": cleanup, "exit_code": result.returncode if result else None,
            "stdout": result.stdout.decode(errors="replace") if result else None,
            "stderr": result.stderr.decode(errors="replace") if result else None,
            "error": repr(cleanup_error) if cleanup_error else None,
            "primary_error": repr(primary_error) if primary_error else None,
        }, indent=2) + "\n")
        (output / f"{kind}.ansi").write_bytes(bytes(terminal.raw))
        (output / f"{kind}.json").write_text(json.dumps(stages, indent=2) + "\n")
        if primary_error is None and cleanup_error is not None:
            raise cleanup_error


def geometry_history(stage, pixels=False):
    # Keep every observed event, including the FIRST of an attempted child
    # subsequently rejected by the reference. Only process IDs are omitted.
    return {label: [(event["event"], event["winsize"] if pixels else event["winsize"][:2])
                    for event in events if event["event"] in {"FIRST", "WINCH"}]
            for label, events in stage["fixture_events"].items()}


def probe_delta(stage):
    probe = stage.get("focus_probe")
    if probe is None:
        return None
    return {label: value[len(probe["input_before"].get(label, "")):]
            for label, value in probe["input_after_marker"].items()}


def compare(reference_stages, candidate_stages):
    if [s["name"] for s in reference_stages] != [s["name"] for s in candidate_stages]:
        raise AssertionError("stage sequences differ")
    differences = []
    for reference, candidate in zip(reference_stages, candidate_stages):
        zgrid, egrid = reference["cells"], candidate["cells"]
        cells = []
        for y in range(max(len(zgrid), len(egrid))):
            zrow = zgrid[y] if y < len(zgrid) else []
            erow = egrid[y] if y < len(egrid) else []
            for x in range(max(len(zrow), len(erow))):
                zcell = zrow[x] if x < len(zrow) else None
                ecell = erow[x] if x < len(erow) else None
                if zcell != ecell:
                    cells.append({"x": x, "y": y, "zellij": zcell, "ekko": ecell})
        zp, ep = reference.get("focus_probe"), candidate.get("focus_probe")
        differences.append({
            "stage": reference["name"],
            "input_equal": reference["fixture_input_hex"] == candidate["fixture_input_hex"],
            "input_delta_equal": reference["fixture_input_delta_hex"] == candidate["fixture_input_delta_hex"],
            "application_cell_geometry_equal": geometry_history(reference) == geometry_history(candidate),
            "pixel_geometry_equal": geometry_history(reference, True) == geometry_history(candidate, True),
            "events_equal_ignoring_pid":
                {label: [{k: v for k, v in e.items() if k != "pid"} for e in events]
                 for label, events in reference["fixture_events"].items()} ==
                {label: [{k: v for k, v in e.items() if k != "pid"} for e in events]
                 for label, events in candidate["fixture_events"].items()},
            "dimensions_equal": reference["terminal_dimensions"] == candidate["terminal_dimensions"],
            "grid_shape_equal": [len(row) for row in zgrid] == [len(row) for row in egrid],
            "outer_ioctl_equal": reference["terminal_size_ioctl"] == candidate["terminal_size_ioctl"],
            "terminal_queries_equal": reference["terminal_queries"] == candidate["terminal_queries"],
            "cursor_equal": reference["cursor"] == candidate["cursor"],
            "focus_receiver_equal": (zp["receiver"] == ep["receiver"] if zp and ep else zp is ep),
            "probe_input_equal": probe_delta(reference) == probe_delta(candidate),
            "differing_cells": len(cells), "cells": cells,
        })
    return differences


def verify_reference(binary, reference):
    pin = json.loads((reference / "pin.json").read_text())
    assert subprocess.check_output([binary, "--version"]).decode().strip() == \
        "zellij " + pin["release"]
    for name, digest in pin["files"].items():
        assert hashlib.sha256((reference / name).read_bytes()).hexdigest() == digest


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--zellij", required=True)
    parser.add_argument("--ekko", required=True)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--reference", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cols", type=int, default=80)
    parser.add_argument("--rows", type=int, default=24)
    parser.add_argument("--only", action="append", choices=sorted(SCENARIOS))
    parser.add_argument("--require-parity", action="store_true")
    parser.add_argument("--no-query-response", action="store_true",
                        help="record terminal queries without answering them")
    args = parser.parse_args()
    if args.cols < 5 or args.rows < 4:
        parser.error("terminal dimensions must be at least 5x4")
    args.output.mkdir(parents=True, exist_ok=True)
    # These are outer-pane limits from the pinned reference, including the
    # tall left pane omitted by a max-of-half-dimensions approximation.
    for stage, cols, rows, count in (
            ("new-pane", 20, 8, 3), ("new-pane-down", 20, 8, 3),
            ("new-pane-right", 20, 8, 4), ("new-pane", 80, 24, 4),
            ("new-pane", 10, 12, 4), ("new-pane", 8, 30, 3)):
        assert expected_split_count(stage, cols, rows) == count
    verify_query_responder()
    verify_reference(args.zellij, args.reference.resolve())
    selected = args.only or list(SCENARIOS)
    results = {}
    with tempfile.TemporaryDirectory(prefix="ekko-zellij-workflow-") as temp:
        root = Path(temp)
        shell = write_spawn_shell(root)
        shell_sha256 = hashlib.sha256(shell.read_bytes()).hexdigest()
        for scenario in selected:
            scenario_output = args.output / scenario
            scenario_output.mkdir(parents=True, exist_ok=True)
            reference_stages = run_side(
                "zellij", args.zellij, args.profile.resolve(),
                args.reference.resolve(), root, scenario_output, shell,
                SCENARIOS[scenario], args.cols, args.rows,
                not args.no_query_response)
            candidate_stages = run_side(
                "ekko", args.ekko, args.profile.resolve(),
                args.reference.resolve(), root, scenario_output, shell,
                SCENARIOS[scenario], args.cols, args.rows,
                not args.no_query_response)
            assert [stage["name"] for stage in reference_stages] == [stage["name"] for stage in candidate_stages]
            assert reference_stages[0]["fixture_contract"] == candidate_stages[0]["fixture_contract"]
            differences = compare(reference_stages, candidate_stages)
            expected_stages = [name for name, _ in SCENARIOS[scenario]]
            assert [stage["name"] for stage in reference_stages] == expected_stages
            settled = [item for item in differences
                       if item["stage"] not in {"startup", "dismiss-release-notes"}]
            results[scenario] = {
                "stage_names": [stage["name"] for stage in reference_stages],
                "differences": differences,
                "input_parity": all(item["input_equal"] for item in differences),
                "settled_input_slice_passed": all(item["input_delta_equal"] and item["probe_input_equal"]
                                                   for item in settled),
                "focus_slice_passed": all(item["focus_receiver_equal"] for item in differences),
                "application_cell_geometry_slice_passed": all(
                    item["application_cell_geometry_equal"] and item["dimensions_equal"]
                    and item["outer_ioctl_equal"] for item in differences),
                "cell_parity": all(item["differing_cells"] == 0 and item["cursor_equal"]
                                    for item in differences),
                "pixel_geometry_equal": all(item["pixel_geometry_equal"]
                                             for item in differences),
            }
    report = {
        "scenario": "three-pane-pane-workflow-matrix",
        "dimensions": [args.cols, args.rows],
        "query_responses": not args.no_query_response,
        "fixture_shell_sha256": shell_sha256,
        "scenarios": results,
        "full_parity": False,
        "coverage_complete": False,
        "settled_input_slice_passed": all(data["settled_input_slice_passed"] for data in results.values()),
        "focus_slice_passed": all(data["focus_slice_passed"] for data in results.values()),
        "application_cell_geometry_slice_passed": all(
            data["application_cell_geometry_slice_passed"] for data in results.values()),
        "pixel_geometry_comparison": {
            "per_scenario": {name: data["pixel_geometry_equal"]
                             for name, data in results.items()},
            "all_equal": all(data["pixel_geometry_equal"] for data in results.values()),
            "note": "Pixel fields are retained and compared without normalization; this is not a parity gate.",
        },
        "normalizations": ["Process IDs are retained in raw events and omitted from event equality; spawned fixtures use creation ordinals."],
        "limitations": ["three-pane workflow slice only", "raw and complete cell grids retained",
                        "settled input gate excludes startup/release-note Escape; full input differences remain reported",
                        "cell geometry gate compares every FIRST/WINCH cols/rows record; READ samples and child pixel fields remain in full event comparison",
                        "launcher environments match; multiplexer-injected child environment is not compared"],
    }
    (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"scenario": report["scenario"], "scenarios": selected,
                      "dimensions": report["dimensions"],
                      "output": str(args.output)}))
    if not all(report[key] for key in ("settled_input_slice_passed", "focus_slice_passed",
                                       "application_cell_geometry_slice_passed")):
        raise SystemExit("Pane workflow functional slice differs; see retained report")
    if args.require_parity:
        raise SystemExit("Full parity coverage remains incomplete")


if __name__ == "__main__":
    main()
