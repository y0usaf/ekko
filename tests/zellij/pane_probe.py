"""Probe pinned Zellij pane-mode routing and pane resize behavior.

This intentionally runs only the reference binary.  It imports the existing
real-PTY Terminal model from differential.py and keeps the fixture input logs
separate for the two deterministic panes.
"""
import argparse
import json
import os
from pathlib import Path
import signal
import struct
import subprocess
import sys
import tempfile
import time
import fcntl
import termios
import tty

from differential import Terminal


def child(label, input_path, events_path):
    """Deterministic app fixture used by each reference pane."""
    tty.setraw(0, termios.TCSANOW)

    def dimensions():
        try:
            rows, cols, _, _ = struct.unpack("HHHH", fcntl.ioctl(0, termios.TIOCGWINSZ, bytes(8)))
            return cols, rows
        except OSError:
            return 0, 0

    def event(kind):
        cols, rows = dimensions()
        with open(events_path, "a", encoding="ascii") as out:
            out.write(f"{kind} {cols}x{rows}\n")

    signal.signal(signal.SIGWINCH, lambda _signum, _frame: event("WINCH"))
    event("READY")
    os.write(1, b"\x1b[2J\x1b[H" + f"PANE-{label} READY".encode("ascii"))
    while True:
        data = os.read(0, 4096)
        if not data:
            return
        event("READ-" + data.hex())
        with open(input_path, "ab", buffering=0) as out:
            out.write(data)


def env_for(work):
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(("ZELLIJ", "EKKO", "XDG_"))}
    env.update(HOME=str(work), XDG_RUNTIME_DIR=str(work), XDG_CONFIG_HOME=str(work),
               XDG_CACHE_HOME=str(work), XDG_DATA_HOME=str(work), TERM="xterm-256color",
               COLORTERM="truecolor", LANG="C.UTF-8", LC_ALL="C.UTF-8", SHELL="/bin/sh")
    return env


def read_bytes(path):
    return path.read_bytes() if path.exists() else b""


def read_text(path):
    return path.read_text() if path.exists() else ""


def checkpoint(terminal, name, sent, logs, before_inputs):
    inputs = {label: read_bytes(path) for label, path in logs.items()}
    return {
        "name": name,
        "sent_hex": sent.hex(),
        "pane_input_hex": {label: data.hex() for label, data in inputs.items()},
        "pane_input_delta_hex": {
            label: data[len(before_inputs[label]):].hex() for label, data in inputs.items()
        },
        "pane_events": {label: read_text(path.with_suffix(".events"))
                        for label, path in logs.items()},
        "cursor": [terminal.screen.cursor.x, terminal.screen.cursor.y,
                   terminal.screen.cursor.hidden],
        "display": list(terminal.screen.display),
        "cells": [[terminal.screen.buffer[y][x]._asdict() for x in range(terminal.cols)]
                  for y in range(terminal.rows)],
    }


def wait_for_apps(terminal, logs, timeout=15):
    deadline = time.monotonic() + timeout
    ready = lambda: all(path.with_suffix(".events").exists() for path in logs.values())
    while time.monotonic() < deadline and not ready():
        terminal.pump(.1)
    if not ready():
        raise RuntimeError(f"apps failed to start: {terminal.raw!r}")


def run_case(binary, reference, output, cols, rows, case_name, steps):
    with tempfile.TemporaryDirectory(prefix="ekko-zellij-pane-") as temp:
        work = Path(temp)
        logs = {}
        app_args = []
        for label in ("A", "B"):
            input_path = work / f"{label}.input"
            events_path = work / f"{label}.events"
            logs[label] = input_path
            app_args.append((label, input_path, events_path))
        layout_lines = ["layout {", '    pane split_direction="vertical" {']
        for label, input_path, events_path in app_args:
            args = [str(Path(__file__).resolve()), "--child", label,
                    str(input_path), str(events_path)]
            layout_lines.append(f"        pane name={json.dumps(label)} command={json.dumps(sys.executable)} {{")
            layout_lines.append("            args " + " ".join(json.dumps(arg) for arg in args))
            layout_lines.append("        }")
        layout_lines.extend(["    }", "}", ""])
        layout = work / "layout.kdl"
        layout.write_text("\n".join(layout_lines))
        env = env_for(work)
        argv = [binary, "--config", str(reference / "config.kdl"),
                "--new-session-with-layout", str(layout), "--session", "pane-probe"]
        cleanup = [binary, "kill-session", "pane-probe"]
        terminal = Terminal(argv, env, cols, rows)
        stages = []
        try:
            wait_for_apps(terminal, logs)
            terminal.pump(1)
            stages.append(checkpoint(terminal, "startup", b"", logs,
                                     {label: b"" for label in logs}))

            before = {label: read_bytes(path) for label, path in logs.items()}
            os.write(terminal.fd, b"\x1b")
            terminal.pump(.4)
            stages.append(checkpoint(terminal, "dismiss-release-notes", b"\x1b", logs, before))

            for name, keys in steps:
                before = {label: read_bytes(path) for label, path in logs.items()}
                os.write(terminal.fd, keys)
                # Actions that switch modes/fullscreen are synchronous on the
                # reference command path; this also captures resulting redraws.
                terminal.pump(.5)
                stages.append(checkpoint(terminal, name, keys, logs, before))
            return stages
        finally:
            (output / f"{case_name}.ansi").write_bytes(terminal.raw)
            (output / f"{case_name}.json").write_text(json.dumps(stages, indent=2) + "\n")
            try:
                subprocess.run(cleanup, env=env, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, timeout=5)
            finally:
                terminal.close()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--zellij", default="/nix/store/wxjfyl2ksqkh263zwikp2igqh2a23y8r-zellij-0.43.1/bin/zellij")
    parser.add_argument("--output", type=Path, default=Path("/tmp/ekko-zellij-pane-probe"))
    parser.add_argument("--cols", type=int, default=80)
    parser.add_argument("--rows", type=int, default=24)
    parser.add_argument("--child", nargs=3, metavar=("LABEL", "INPUT", "EVENTS"))
    args = parser.parse_args()
    if args.child:
        child(args.child[0], args.child[1], args.child[2])
        return
    args.output.mkdir(parents=True, exist_ok=True)
    reference = Path(__file__).resolve().parent / "reference"
    settled = [
        ("pane-enter", b"\x10"),
        ("pane-fullscreen-on", b"f"),
        ("normal-return-pane", b"\x10"),
        ("pane-fullscreen-off", b"f"),
        ("pane-enter-for-escape", b"\x10"),
        ("pane-escape-return", b"\x1b"),
        ("pane-enter-for-enter", b"\x10"),
        ("pane-enter-return", b"\r"),
        ("pane-enter-for-ctrl-p", b"\x10"),
        ("pane-ctrl-p-return", b"\x10"),
        ("pane-enter-for-unbound", b"\x10"),
        ("pane-unbound-q", b"q"),
        ("pane-lock", b"\x07"),
        ("locked-write-q", b"q"),
        ("locked-return", b"\x07"),
    ]
    rapid = [
        ("rapid-ctrl-g-ctrl-p-b", b"\x07\x10b"),
        # A following f distinguishes the Pane branch (fullscreen, no app
        # input) from the forwarding branch. The following Ctrl-p then
        # distinguishes the forwarding branch as Locked: it reaches A as 10;
        # Normal would consume Ctrl-p as Pane entry.
        ("rapid-followup-f", b"f"),
        ("rapid-pane-reentry", b"\x10"),
        ("rapid-fullscreen-off", b"f"),
    ]
    all_results = {
        "dimensions": [args.cols, args.rows],
        "settled": run_case(args.zellij, reference, args.output, args.cols, args.rows,
                             "settled", settled),
        "rapid": run_case(args.zellij, reference, args.output, args.cols, args.rows,
                           "rapid", rapid),
    }
    (args.output / "report.json").write_text(json.dumps(all_results, indent=2) + "\n")


if __name__ == "__main__":
    main()
