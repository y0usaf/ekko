"""Bounded pinned-Zellij explicit split failure oracle.

This probe records the red pane-frame flash at short intervals, then checks
that the failed compound action returns to Normal and that later fullscreen
still works.  A second case reaches the same failure by repeatedly splitting
at 80x24.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time

from differential import Terminal
from pane_differential import verify_reference
from pane_workflow_probe import (LABELS, child, env_for, read_bytes, snapshot,
                                  wait_for_apps, write_layout)


def run_case(binary, reference, cols, rows, name, sequence, output):
    import tempfile
    with tempfile.TemporaryDirectory(prefix="ekko-zellij-split-error-") as temp:
        work = Path(temp)
        apps = {label: (work / f"{label}.input", work / f"{label}.events")
                for label in LABELS}
        layout = write_layout(work, apps)
        config = work / "config.kdl"
        config.write_text((reference / "config.kdl").read_text()
                          + "\nshow_release_notes false\nshow_startup_tips false\n")
        env = env_for(work)
        argv = [binary, "--config", str(config), "--new-session-with-layout",
                str(layout), "--session", "pane-split-error"]
        cleanup = [binary, "kill-session", "pane-split-error"]
        terminal = Terminal(argv, env, cols, rows)
        stages = []
        result = None
        cleanup_record = {}
        try:
            wait_for_apps(terminal, apps)
            terminal.pump(1)
            stages.append(snapshot(terminal, "startup", b"", apps,
                                   {label: b"" for label in LABELS}))
            for step_name, keys, waits in sequence:
                before = {label: read_bytes(paths[0]) for label, paths in apps.items()}
                os.write(terminal.fd, keys)
                for suffix, duration in waits:
                    terminal.pump(duration)
                    stages.append(snapshot(terminal, step_name + suffix, keys,
                                           apps, before))
                    # Only the first snapshot after an input records a delta;
                    # subsequent timing samples are state-only observations.
                    before = {label: read_bytes(paths[0]) for label, paths in apps.items()}
            result = {"name": name, "dimensions": [cols, rows],
                      "stages": stages, "raw_hex": bytes(terminal.raw).hex()}
        finally:
            output.joinpath(name + ".ansi").write_bytes(terminal.raw)
            try:
                cleanup_result = subprocess.run(
                    cleanup, env=env, stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE, timeout=5, check=False)
                cleanup_record = {
                    "argv": cleanup,
                    "returncode": cleanup_result.returncode,
                    "stdout": cleanup_result.stdout.decode(errors="replace"),
                    "stderr": cleanup_result.stderr.decode(errors="replace"),
                }
            except Exception as error:
                cleanup_record = {"argv": cleanup, "error": repr(error)}
            finally:
                terminal.close()
            output.joinpath(name + ".cleanup.json").write_text(
                json.dumps(cleanup_record, indent=2) + "\n")
        if cleanup_record.get("returncode", 0) != 0 or "error" in cleanup_record:
            raise RuntimeError(f"session cleanup failed for {name}: {cleanup_record}")
        return result


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--zellij", default="/nix/store/wxjfyl2ksqkh263zwikp2igqh2a23y8r-zellij-0.43.1/bin/zellij")
    parser.add_argument("--reference", type=Path,
                        default=Path(__file__).resolve().parent / "reference")
    parser.add_argument("--output", type=Path,
                        default=Path("/tmp/ekko-zellij-pane-split-error"))
    parser.add_argument("--child", nargs=3, metavar=("LABEL", "EVENTS", "INPUT"))
    args = parser.parse_args()
    if args.child:
        child(args.child[0], args.child[1], args.child[2])
        return
    args.output.mkdir(parents=True, exist_ok=True)
    reference = args.reference.resolve()
    verify_reference(args.zellij, reference)
    # Each wait list is sampled after one input. The d failure flash is
    # expected to persist for approximately one second (source constant).
    small = [
        ("pane-enter", b"\x10", [("", .20)]),
        ("split-fail", b"d", [("+0.01", .01), ("+0.05", .04),
                                ("+0.20", .15), ("+0.50", .30),
                                ("+0.95", .45), ("+1.10", .15),
                                ("+1.50", .40)]),
        ("post-fail-q", b"q", [("", .20)]),
        ("pane-enter-again", b"\x10", [("", .20)]),
        ("fullscreen-after-fail", b"f", [("", .30)]),
        ("pane-enter-fullscreen-fail", b"\x10", [("", .10)]),
        ("split-fail-from-fullscreen", b"d", [("+0.05", .05),
                                                ("+1.10", 1.05)]),
        ("post-fullscreen-fail-q", b"q", [("", .20)]),
        ("pane-enter-fullscreen", b"\x10", [("", .10)]),
        ("fullscreen-off", b"f", [("", .30)]),
    ]
    repeated = [
        ("pane-enter", b"\x10", [("", .20)]),
        ("split-1", b"d", [("", .25)]),
        ("pane-enter-2", b"\x10", [("", .10)]),
        ("split-2", b"d", [("", .25)]),
        ("pane-enter-3", b"\x10", [("", .10)]),
        ("split-3-fail", b"d", [("+0.01", .01), ("+0.20", .19),
                                   ("+0.95", .75), ("+1.20", .25)]),
        ("post-fail-q", b"q", [("", .20)]),
    ]
    results = {
        "20x8": run_case(args.zellij, reference, 20, 8, "small-20x8",
                         small, args.output),
        "80x24": run_case(args.zellij, reference, 80, 24, "repeated-80x24",
                          repeated, args.output),
    }
    report = {"binary": args.zellij, "reference": str(reference),
              "normalizations": [], "cases": results}
    (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
