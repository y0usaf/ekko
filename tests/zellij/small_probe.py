"""Bounded investigation of pinned Zellij's 20x8 startup redraw.

The fixture is deliberately deterministic.  Each case varies only the bytes
written by the child and/or their delay after the PTY reports READY.  Raw ANSI,
child dimensions/events, and pyte displays are retained so a blank initial
pane can be distinguished from a fixture or timing race.
"""

import argparse
import json
import os
from pathlib import Path
import shutil
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


def dimensions():
    try:
        rows, cols, _, _ = struct.unpack(
            "HHHH", fcntl.ioctl(0, termios.TIOCGWINSZ, bytes(8)))
        return cols, rows
    except OSError:
        return 0, 0


def child(label, events_path, output_hex, delay):
    tty.setraw(0, termios.TCSANOW)

    def event(kind):
        cols, rows = dimensions()
        with open(events_path, "a", encoding="ascii") as out:
            out.write(f"{kind} {cols}x{rows}\n")

    signal.signal(signal.SIGWINCH, lambda _signum, _frame: event("WINCH"))
    event("READY")
    if delay:
        time.sleep(delay)
    os.write(1, bytes.fromhex(output_hex))
    event("OUTPUT")
    while True:
        data = os.read(0, 4096)
        if not data:
            return
        event("READ-" + data.hex())


def env_for(work):
    env = {key: value for key, value in os.environ.items()
           if not key.startswith(("ZELLIJ", "EKKO", "XDG_"))}
    env.update(HOME=str(work), XDG_RUNTIME_DIR=str(work),
               XDG_CONFIG_HOME=str(work), XDG_CACHE_HOME=str(work),
               XDG_DATA_HOME=str(work), TERM="xterm-256color",
               COLORTERM="truecolor", LANG="C.UTF-8", LC_ALL="C.UTF-8",
               SHELL="/bin/sh")
    return env


def read_text(path):
    return path.read_text() if path.exists() else ""


def snapshot(terminal, events_path, name, elapsed):
    return {
        "name": name,
        "elapsed": elapsed,
        "display": list(terminal.screen.display),
        "cursor": [terminal.screen.cursor.x, terminal.screen.cursor.y,
                    terminal.screen.cursor.hidden],
        "events": read_text(events_path),
    }


def run_case(binary, reference, cols, rows, case, output_dir, dismiss=0):
    name = case["name"]
    with tempfile.TemporaryDirectory(prefix="ekko-zellij-small-") as temp:
        work = Path(temp)
        events = work / "events"
        layout = work / "layout.kdl"
        script = str(Path(__file__).resolve())
        panes = []
        for label in ("A", "B"):
            args = [script, "--child", label, str(events),
                    case["output_hex"], str(case["delay"])]
            panes.append(
                f'        pane name="{label}" command={json.dumps(sys.executable)} {{\n'
                f'            args ' + " ".join(json.dumps(arg) for arg in args) + "\n"
                "        }"
            )
        layout.write_text("\n".join(
            ["layout {", '    pane split_direction="vertical" {', *panes,
             "    }", "}", ""]))
        config = reference / "config.kdl"
        if case.get("show_release_notes") is False:
            config = work / "config.kdl"
            config.write_text((reference / "config.kdl").read_text()
                              + "\nshow_release_notes false\n")
        if case.get("seen_release_notes"):
            # ProjectDirs uses $XDG_CACHE_HOME/zellij on Linux.  Supplying the
            # marker tests the cache branch without changing the pinned config.
            marker = work / "zellij" / "0.43.1" / "seen_release_notes"
            marker.parent.mkdir(parents=True)
            marker.write_bytes(b"")
        env = env_for(work)
        argv = [binary, "--config", str(config),
                "--new-session-with-layout", str(layout),
                "--session", "small-probe"]
        cleanup = [binary, "kill-session", "small-probe"]
        terminal = Terminal(argv, env, cols, rows)
        started = time.monotonic()
        snapshots = []
        try:
            deadline = started + 15
            while time.monotonic() < deadline and not events.exists():
                terminal.pump(.05)
            if not events.exists():
                raise RuntimeError("fixture did not report READY")
            # Capture before and after the Zellij pane resize settles.  The
            # child writes at a fixed offset from READY, so this isolates
            # whether the resize itself discards output.
            timings = [("ready+0.05", .05), ("ready+0.25", .20),
                       ("ready+0.75", .50), ("ready+1.75", 1.00)]
            if case["delay"] > 2:
                timings.extend([("ready+4.75", 3.00),
                                 ("ready+5.75", 1.00)])
            for label, duration in timings:
                terminal.pump(duration)
                snapshots.append(snapshot(terminal, events, label,
                                           time.monotonic() - started))
            if dismiss:
                os.write(terminal.fd, b"\x1b" * dismiss)
                terminal.pump(.75)
                snapshots.append(snapshot(terminal, events, "after-escape",
                                           time.monotonic() - started))
            return {"case": case, "snapshots": snapshots,
                    "raw_hex": bytes(terminal.raw).hex()}
        finally:
            out = output_dir / name
            out.with_suffix(".ansi").write_bytes(terminal.raw)
            cleanup_record = {"case": name, "argv": cleanup,
                              "returncode": None, "stdout_hex": "",
                              "stderr_hex": "", "error": None}
            cleanup_error = None
            try:
                result = subprocess.run(cleanup, env=env, stdout=subprocess.PIPE,
                                        stderr=subprocess.PIPE, timeout=5)
                cleanup_record.update(returncode=result.returncode,
                                      stdout_hex=result.stdout.hex(),
                                      stderr_hex=result.stderr.hex())
                if result.returncode:
                    cleanup_error = RuntimeError(
                        f"cleanup failed for {name}: {result.stderr!r}")
            except BaseException as error:
                cleanup_record["error"] = repr(error)
                cleanup_error = error
            finally:
                terminal.close()
            out.with_suffix(".cleanup.json").write_text(
                json.dumps(cleanup_record, indent=2) + "\n")
            if cleanup_error is not None:
                raise cleanup_error


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--zellij", default="/nix/store/wxjfyl2ksqkh263zwikp2igqh2a23y8r-zellij-0.43.1/bin/zellij")
    parser.add_argument("--reference", type=Path,
                        default=Path(__file__).resolve().parent / "reference")
    parser.add_argument("--output", type=Path,
                        default=Path("/tmp/ekko-zellij-small-probe"))
    parser.add_argument("--cols", type=int, default=20)
    parser.add_argument("--rows", type=int, default=8)
    parser.add_argument("--dismiss", type=int, default=0, metavar="COUNT",
                        help="send COUNT Escape bytes after startup captures")
    parser.add_argument("--only", action="append", metavar="CASE",
                        help="run only the named case (repeatable)")
    parser.add_argument("--child", nargs=4,
                        metavar=("LABEL", "EVENTS", "OUTPUT_HEX", "DELAY"))
    args = parser.parse_args()
    if args.child:
        child(args.child[0], args.child[1], args.child[2], float(args.child[3]))
        return
    verify_reference(args.zellij, args.reference.resolve())
    args.output.mkdir(parents=True, exist_ok=True)
    # All output strings begin with an explicit UTF-8 clear/home where noted.
    # The short text case also checks whether startup output is lost without
    # an application erase sequence.
    cases = [
        {"name": "clear-home-text", "delay": 0,
         "output_hex": "1b5b324a1b5b4850414e452d41205245414459"},
        {"name": "text-only", "delay": 0,
         "output_hex": "50414e452d41205245414459"},
        {"name": "clear-home-text-delayed", "delay": .50,
         "output_hex": "1b5b324a1b5b4850414e452d41205245414459"},
        {"name": "clear-home-only", "delay": 0,
         "output_hex": "1b5b324a1b5b48"},
        {"name": "text-then-clear-home", "delay": 0,
         "output_hex": "50414e452d412052454144591b5b324a1b5b48"},
        {"name": "clear-home-text-late", "delay": 1.25,
         "output_hex": "1b5b324a1b5b4850414e452d41205245414459"},
        {"name": "clear-home-text-after-layout", "delay": 4.00,
         "output_hex": "1b5b324a1b5b4850414e452d41205245414459"},
        {"name": "text-only-no-release-option", "delay": 0,
         "output_hex": "50414e452d41205245414459",
         "show_release_notes": False},
        {"name": "text-only-seen-release-cache", "delay": 0,
         "output_hex": "50414e452d41205245414459",
         "seen_release_notes": True},
    ]
    if args.only:
        cases = [case for case in cases if case["name"] in args.only]
        missing = set(args.only) - {case["name"] for case in cases}
        if missing:
            parser.error("unknown case(s): " + ", ".join(sorted(missing)))
    results = {case["name"]: run_case(args.zellij, args.reference,
                                        args.cols, args.rows, case, args.output,
                                        args.dismiss)
               for case in cases}
    report = {"binary": args.zellij, "dimensions": [args.cols, args.rows],
              "cases": results}
    (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
