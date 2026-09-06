"""Pinned-Zellij Pane workflow oracle.

Runs a deterministic three-pane L layout and records every settled action.
The A/B/C children log PTY input and resize events; pane-management actions
may create an uninstrumented shell pane, whose frame and geometry are still
captured in the outer terminal stream.
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
from pane_differential import verify_reference


PINNED_ZELLIJ = "/nix/store/wxjfyl2ksqkh263zwikp2igqh2a23y8r-zellij-0.43.1/bin/zellij"
LABELS = ("A", "B", "C")


def dimensions():
    try:
        rows, cols, _, _ = struct.unpack("HHHH", fcntl.ioctl(0, termios.TIOCGWINSZ, bytes(8)))
        return cols, rows
    except OSError:
        return 0, 0


def child(label, events_path, input_path):
    tty.setraw(0, termios.TCSANOW)

    def event(kind):
        cols, rows = dimensions()
        with open(events_path, "a", encoding="ascii") as out:
            out.write(f"{kind} {cols}x{rows}\n")

    signal.signal(signal.SIGWINCH, lambda _signum, _frame: event("WINCH"))
    event("READY")
    os.write(1, b"\x1b[2J\x1b[HAPP-" + label.encode("ascii") + b" READY")
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


def write_layout(work, apps):
    lines = ["layout {", '    pane split_direction="vertical" {']
    # A occupies the left half. B/C share the right half horizontally.
    label = "A"
    input_path, events_path = apps[label]
    args = [str(Path(__file__).resolve()), "--child", label,
            str(events_path), str(input_path)]
    lines.extend([
        f'        pane name="A" command={json.dumps(sys.executable)} {{',
        "            args " + " ".join(json.dumps(arg) for arg in args),
        "        }",
        '        pane split_direction="horizontal" {',
    ])
    for label in ("B", "C"):
        input_path, events_path = apps[label]
        args = [str(Path(__file__).resolve()), "--child", label,
                str(events_path), str(input_path)]
        lines.extend([
            f'            pane name="{label}" command={json.dumps(sys.executable)} {{',
            "                args " + " ".join(json.dumps(arg) for arg in args),
            "            }",
        ])
    lines.extend(["        }", "    }", "}", ""])
    layout = work / "layout.kdl"
    layout.write_text("\n".join(lines))
    return layout


def snapshot(terminal, name, sent, apps, before):
    inputs = {label: read_bytes(paths[0]) for label, paths in apps.items()}
    events = {label: read_text(paths[1]) for label, paths in apps.items()}
    return {
        "name": name,
        "sent_hex": sent.hex(),
        "pane_input_hex": {label: data.hex() for label, data in inputs.items()},
        "pane_input_delta_hex": {
            label: data[len(before[label]):].hex() for label, data in inputs.items()
        },
        "pane_events": events,
        "terminal_dimensions": [terminal.cols, terminal.rows],
        "cursor": [terminal.screen.cursor.x, terminal.screen.cursor.y,
                    terminal.screen.cursor.hidden],
        "display": list(terminal.screen.display),
        "cells": [[terminal.screen.buffer[y][x]._asdict()
                   for x in range(terminal.cols)] for y in range(terminal.rows)],
    }


def wait_for_apps(terminal, apps):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        if all(paths[1].exists() for paths in apps.values()):
            return
        terminal.pump(.1)
    raise RuntimeError(f"apps failed to start: {bytes(terminal.raw)!r}")


CASES = {
    # h at A and l/j/k/p exercise edge no-op, adjacency, vertical tie-break,
    # reverse movement, and cyclic focus order in one settled session.
    "navigation": [("pane-enter", b"\x10"), ("edge-left", b"h"),
                   ("move-right", b"l"), ("move-down", b"j"),
                   ("move-up", b"k"), ("switch-focus", b"p"),
                   ("edge-right", b"l")],
    "new-n": [("pane-enter", b"\x10"), ("new-default", b"n"),
              ("post-new-q", b"q")],
    "new-d": [("pane-enter", b"\x10"), ("new-down", b"d"),
              ("post-new-q", b"q")],
    "new-r": [("pane-enter", b"\x10"), ("new-right", b"r"),
              ("post-new-q", b"q")],
    "close-x": [("pane-enter", b"\x10"), ("close-focused", b"x"),
                ("post-close-q", b"q")],
    "fullscreen-navigation": [("pane-enter", b"\x10"),
                               ("fullscreen-on", b"f"),
                               ("pane-enter-fullscreen", b"\x10"),
                               ("fullscreen-move-right", b"l"),
                               ("fullscreen-off", b"f")],
}


def run_case(binary, reference, cols, rows, case_name, steps, output):
    with tempfile.TemporaryDirectory(prefix="ekko-zellij-workflow-") as temp:
        work = Path(temp)
        apps = {label: (work / f"{label}.input", work / f"{label}.events")
                for label in LABELS}
        layout = write_layout(work, apps)
        config = work / "config.kdl"
        config.write_text((reference / "config.kdl").read_text()
                          + "\nshow_release_notes false\nshow_startup_tips false\n")
        env = env_for(work)
        argv = [binary, "--config", str(config), "--new-session-with-layout",
                str(layout), "--session", "pane-workflow"]
        cleanup = [binary, "kill-session", "pane-workflow"]
        terminal = Terminal(argv, env, cols, rows)
        stages = []
        try:
            wait_for_apps(terminal, apps)
            terminal.pump(1)
            stages.append(snapshot(terminal, "startup", b"", apps,
                                   {label: b"" for label in LABELS}))
            for name, keys in steps:
                before = {label: read_bytes(paths[0]) for label, paths in apps.items()}
                os.write(terminal.fd, keys)
                terminal.pump(.75)
                stages.append(snapshot(terminal, name, keys, apps, before))
            return {"case": case_name, "dimensions": [cols, rows],
                    "stages": stages, "raw_hex": bytes(terminal.raw).hex()}
        finally:
            output.joinpath(case_name + ".ansi").write_bytes(terminal.raw)
            try:
                result = subprocess.run(cleanup, env=env, stdout=subprocess.PIPE,
                                        stderr=subprocess.PIPE, timeout=5)
                if result.returncode:
                    print(f"cleanup failed for {case_name}: {result.stderr!r}",
                          file=sys.stderr)
            finally:
                terminal.close()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--zellij", default=PINNED_ZELLIJ)
    parser.add_argument("--reference", type=Path,
                        default=Path(__file__).resolve().parent / "reference")
    parser.add_argument("--output", type=Path,
                        default=Path("/tmp/ekko-zellij-pane-workflow"))
    parser.add_argument("--cols", type=int, default=80)
    parser.add_argument("--rows", type=int, default=24)
    parser.add_argument("--only", action="append", choices=sorted(CASES))
    parser.add_argument("--child", nargs=3, metavar=("LABEL", "EVENTS", "INPUT"))
    args = parser.parse_args()
    if args.child:
        child(args.child[0], args.child[1], args.child[2])
        return
    args.output.mkdir(parents=True, exist_ok=True)
    reference = args.reference.resolve()
    verify_reference(args.zellij, reference)
    selected = args.only or list(CASES)
    results = {name: run_case(args.zellij, reference, args.cols, args.rows,
                              name, CASES[name], args.output)
               for name in selected}
    report = {"binary": args.zellij, "dimensions": [args.cols, args.rows],
              "layout": "three-pane-L (A left; B/C right stacked horizontally)",
              "cases": results}
    (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
