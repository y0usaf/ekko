"""Paired settled Pane-mode differential run.

The Zellij side intentionally uses the custom two-terminal, no-bar layout
from pane_probe.py.  Its geometry is an explicit scenario and is not the
default-bar differential harness.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

from differential import Terminal
from pane_probe import env_for, read_bytes, read_text, wait_for_apps


SCENARIO = "custom-two-pane-no-bar"
SESSION = "pane-differential"
SETUP_STAGES = {"startup", "dismiss-release-notes"}
STEPS = [
    ("pane-enter", b"\x10", "pane", False),
    ("pane-fullscreen-on", b"f", "normal", True),
    ("normal-return-pane", b"\x10", "pane", True),
    ("pane-fullscreen-off", b"f", "normal", False),
    ("pane-enter-for-escape", b"\x10", "pane", False),
    ("pane-escape-return", b"\x1b", "normal", False),
    ("pane-enter-for-enter", b"\x10", "pane", False),
    ("pane-enter-return", b"\r", "normal", False),
    ("pane-enter-for-ctrl-p", b"\x10", "pane", False),
    ("pane-ctrl-p-return", b"\x10", "normal", False),
    ("pane-enter-for-unbound", b"\x10", "pane", False),
    ("pane-unbound-q", b"q", "pane", False),
    ("pane-lock", b"\x07", "locked", False),
    ("locked-write-q", b"q", "locked", False),
    ("locked-return", b"\x07", "normal", False),
]
EXPECTED_STAGES = ["startup", "dismiss-release-notes"] + [step[0] for step in STEPS]


def child_args(label, input_path, events_path):
    return [str(Path(__file__).with_name("pane_probe.py")), "--child", label,
            str(input_path), str(events_path)]


def make_apps(work):
    apps = {}
    for label in ("A", "B"):
        input_path = work / f"{label}.input"
        events_path = work / f"{label}.events"
        apps[label] = (input_path, events_path)
    return apps


def write_zellij_layout(work, apps):
    lines = ["layout {", '    pane split_direction="vertical" {']
    for label, (input_path, events_path) in apps.items():
        args = child_args(label, input_path, events_path)
        lines.append(f"        pane name={json.dumps(label)} command={json.dumps(sys.executable)} {{")
        lines.append("            args " + " ".join(json.dumps(arg) for arg in args))
        lines.append("        }")
    lines.extend(["    }", "}", ""])
    layout = work / "layout.kdl"
    layout.write_text("\n".join(lines))
    return layout


def inspect_candidate(binary, env):
    result = subprocess.run([binary, "inspect", SESSION], env=env,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=8)
    if result.returncode:
        return {"error": result.stderr.decode(errors="replace"), "returncode": result.returncode}
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as error:
        return {"error": f"invalid inspect JSON: {error}",
                "stdout": result.stdout.decode(errors="replace")}


def status_candidate(binary, env):
    result = subprocess.run([binary, "status", SESSION], env=env,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=8)
    if result.returncode:
        return {"error": result.stderr.decode(errors="replace"), "returncode": result.returncode}
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as error:
        return {"error": f"invalid status JSON: {error}",
                "stdout": result.stdout.decode(errors="replace")}


def event_dimensions(events):
    dimensions = []
    for line in events.splitlines():
        fields = line.split()
        if len(fields) == 2 and fields[0] in ("READY", "WINCH") and "x" in fields[1]:
            dimensions.append({"event": fields[0], "size": fields[1]})
    return dimensions


def capture(terminal, name, sent, apps, before, side, inspect=None, status=None):
    inputs = {label: read_bytes(paths[0]) for label, paths in apps.items()}
    events = {label: read_text(paths[1]) for label, paths in apps.items()}
    state = {
        "name": name,
        "sent_hex": sent.hex(),
        "pane_input_hex": {label: data.hex() for label, data in inputs.items()},
        "pane_input_delta_hex": {
            label: data[len(before[label]):].hex() for label, data in inputs.items()
        },
        "pane_events": events,
        "pane_dimensions": {label: event_dimensions(data) for label, data in events.items()},
        "terminal_dimensions": [terminal.cols, terminal.rows],
        "cursor": [terminal.screen.cursor.x, terminal.screen.cursor.y,
                   terminal.screen.cursor.hidden],
        "display": list(terminal.screen.display),
        "cells": [[terminal.screen.buffer[y][x]._asdict() for x in range(terminal.cols)]
                  for y in range(terminal.rows)],
        "side": side,
    }
    if inspect is not None:
        state["inspect"] = inspect
    if status is not None:
        state["status"] = status
    return state


def run_side(kind, binary, profile, reference, output, root, cols, rows):
    work = root / "session"
    if work.exists():
        shutil.rmtree(work)
    work.mkdir()
    apps = make_apps(work)
    env = env_for(work)
    if kind == "ekko":
        # Match this scenario's explicit no-bar layout and A/B names using
        # ordinary public configuration/actions. Keep setup output in raw ANSI.
        config = work / "profile.lisp"
        config.write_text("(load " + json.dumps(str(profile), ensure_ascii=False) + ")\n" + '''
(ekko/extensions:register-component :id :oracle-layout :reads '(:panes))
(ekko/extensions:set-option :component :oracle-layout :name :viewport-insets :value '(0 0 0 0))
(ekko/extensions:register-command :component :oracle-layout :name "fixture-label"
 :handler (lambda (snapshot event)
   (let* ((args (getf event :arguments))
          (pane (nth (parse-integer (first args)) (ekko/extensions:value snapshot :panes))))
     (list (ekko/extensions:action :rename :pane (getf pane :id) :text (second args))))))
''')
        env["EKKO_CONFIG"] = str(config)
        argv = [binary, "run", "--session", SESSION]
        for index, label in enumerate(("A", "B")):
            argv.extend([sys.executable, *child_args(label, *apps[label])])
            if index == 0:
                argv.append(":::")
        cleanup = [binary, "stop", SESSION]
    else:
        layout = write_zellij_layout(work, apps)
        argv = [binary, "--config", str(reference / "config.kdl"),
                "--new-session-with-layout", str(layout), "--session", SESSION]
        cleanup = [binary, "kill-session", SESSION]
    terminal = Terminal(argv, env, cols, rows)
    stages = []
    failure = None
    cleanup_error = None
    try:
        wait_for_apps(terminal, {label: paths[0] for label, paths in apps.items()})
        if kind == "ekko":
            for index, label in enumerate(("A", "B")):
                subprocess.run([binary, "command", "--session", SESSION,
                                "fixture-label", str(index), label], env=env,
                               capture_output=True, timeout=8, check=True)
        terminal.pump(1)
        stages.append(capture(terminal, "startup", b"", apps,
                              {label: b"" for label in apps}, kind,
                              inspect_candidate(binary, env) if kind == "ekko" else None,
                              status_candidate(binary, env) if kind == "ekko" else None))
        before = {label: read_bytes(paths[0]) for label, paths in apps.items()}
        os.write(terminal.fd, b"\x1b")
        terminal.pump(.4)
        stages.append(capture(terminal, "dismiss-release-notes", b"\x1b", apps, before,
                              kind, inspect_candidate(binary, env) if kind == "ekko" else None,
                              status_candidate(binary, env) if kind == "ekko" else None))
        for name, keys, _mode, _zoom in STEPS:
            before = {label: read_bytes(paths[0]) for label, paths in apps.items()}
            os.write(terminal.fd, keys)
            terminal.pump(.5)
            stages.append(capture(terminal, name, keys, apps, before, kind,
                                  inspect_candidate(binary, env) if kind == "ekko" else None,
                                  status_candidate(binary, env) if kind == "ekko" else None))
        return stages
    except BaseException as error:
        failure = error
        raise
    finally:
        (output / f"{kind}.ansi").write_bytes(terminal.raw)
        (output / f"{kind}.json").write_text(json.dumps(stages, indent=2) + "\n")
        try:
            result = subprocess.run(cleanup, env=env, stdout=subprocess.PIPE,
                                    stderr=subprocess.PIPE, timeout=8)
            if result.returncode:
                cleanup_error = RuntimeError(
                    f"{kind} cleanup failed ({result.returncode}): "
                    f"{result.stderr.decode(errors='replace')}")
        except BaseException as error:
            cleanup_error = error
        finally:
            terminal.close()
        if cleanup_error is not None and failure is None:
            raise cleanup_error


def verify_reference(binary, reference):
    pin = json.loads((reference / "pin.json").read_text())
    expected_version = "zellij " + pin["release"]
    actual_version = subprocess.check_output([binary, "--version"]).decode().strip()
    if actual_version != expected_version:
        raise RuntimeError(f"expected {expected_version!r}, got {actual_version!r}")
    for name, digest in pin["files"].items():
        actual_digest = hashlib.sha256((reference / name).read_bytes()).hexdigest()
        if actual_digest != digest:
            raise RuntimeError(f"{name} hash changed: expected {digest}, got {actual_digest}")


def compare(reference_stages, candidate_stages):
    differences = []
    expected = {name: (mode, zoom) for name, _keys, mode, zoom in STEPS}
    for ref, candidate in zip(reference_stages, candidate_stages):
        cells = [{"x": x, "y": y, "zellij": zc, "ekko": ec}
                 for y, (zr, er) in enumerate(zip(ref["cells"], candidate["cells"]))
                 for x, (zc, ec) in enumerate(zip(zr, er)) if zc != ec]
        delta_equal = ref["pane_input_delta_hex"] == candidate["pane_input_delta_hex"]
        entry = {
            "stage": ref["name"],
            "input_equal": ref["pane_input_hex"] == candidate["pane_input_hex"],
            "input_delta_equal": delta_equal,
            "zellij_pane_input_hex": ref["pane_input_hex"],
            "ekko_pane_input_hex": candidate["pane_input_hex"],
            "zellij_pane_input_delta_hex": ref["pane_input_delta_hex"],
            "ekko_pane_input_delta_hex": candidate["pane_input_delta_hex"],
            "events_equal": ref["pane_events"] == candidate["pane_events"],
            "zellij_pane_dimensions": ref["pane_dimensions"],
            "ekko_pane_dimensions": candidate["pane_dimensions"],
            "dimensions_equal": ref["pane_dimensions"] == candidate["pane_dimensions"],
            "terminal_dimensions_equal": ref["terminal_dimensions"] == candidate["terminal_dimensions"],
            "zellij_terminal_dimensions": ref["terminal_dimensions"],
            "ekko_terminal_dimensions": candidate["terminal_dimensions"],
            "cursor_equal": ref["cursor"] == candidate["cursor"],
            "zellij_cursor": ref["cursor"],
            "ekko_cursor": candidate["cursor"],
            "cells": cells,
            "zellij_pane_events": ref["pane_events"],
            "ekko_pane_events": candidate["pane_events"],
        }
        if ref["name"] in expected:
            mode, zoom = expected[ref["name"]]
            inspect = candidate.get("inspect", {})
            entry["expected_ekko_mode"] = mode
            entry["expected_ekko_zoom"] = zoom
            entry["ekko_mode_equal"] = inspect.get("mode") == mode
            entry["ekko_zoom_equal"] = inspect.get("zoom") == zoom
        differences.append(entry)
    if len(reference_stages) != len(candidate_stages):
        for index in range(min(len(reference_stages), len(candidate_stages)),
                           max(len(reference_stages), len(candidate_stages))):
            ref = reference_stages[index] if index < len(reference_stages) else None
            candidate = candidate_stages[index] if index < len(candidate_stages) else None
            differences.append({
                "stage": (ref or candidate)["name"],
                "stage_index": index,
                "missing_side": "ekko" if candidate is None else "zellij",
                "input_equal": False,
                "input_delta_equal": False,
                "events_equal": False,
                "cursor_equal": False,
                "cells": [],
            })
    return differences


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--zellij", required=True)
    parser.add_argument("--ekko", required=True)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--reference", type=Path,
                        default=Path(__file__).resolve().parent / "reference")
    parser.add_argument("--cols", type=int, default=80)
    parser.add_argument("--rows", type=int, default=24)
    parser.add_argument("--require-parity", action="store_true")
    args = parser.parse_args()
    if args.cols <= 0 or args.rows <= 0:
        raise SystemExit("terminal dimensions must be positive")
    args.output.mkdir(parents=True, exist_ok=True)
    verify_reference(args.zellij, args.reference.resolve())
    with tempfile.TemporaryDirectory(prefix="ekko-zellij-pane-pair-") as pair:
        pair_root = Path(pair)
        reference_stages = run_side("zellij", args.zellij, args.profile.resolve(),
                                    args.reference.resolve(), args.output, pair_root,
                                    args.cols, args.rows)
        candidate_stages = run_side("ekko", args.ekko, args.profile.resolve(),
                                    args.reference.resolve(), args.output, pair_root,
                                    args.cols, args.rows)
    differences = compare(reference_stages, candidate_stages)
    settled = [d for d in differences if d["stage"] not in SETUP_STAGES]
    report = {
        "scenario": SCENARIO,
        "dimensions": [args.cols, args.rows],
        "settled_input_slice_passed": all(d["input_delta_equal"] for d in settled),
        "stage_count_equal": len(reference_stages) == len(candidate_stages),
        "stage_names_equal": [stage["name"] for stage in reference_stages]
        == [stage["name"] for stage in candidate_stages],
        "expected_zellij_stage_sequence": [stage["name"] for stage in reference_stages] == EXPECTED_STAGES,
        "expected_ekko_stage_sequence": [stage["name"] for stage in candidate_stages] == EXPECTED_STAGES,
        "zellij_stage_names": [stage["name"] for stage in reference_stages],
        "ekko_stage_names": [stage["name"] for stage in candidate_stages],
        "application_input_parity": all(d["input_equal"] for d in differences),
        "application_geometry_slice_passed": bool(differences) and all(
            d.get("dimensions_equal", False) for d in differences),
        "modeled_cell_parity": all(
            not d["cells"] and d["cursor_equal"] and d.get("terminal_dimensions_equal", False)
            and d.get("dimensions_equal", False)
            for d in differences),
        "internal_ekko_mode_zoom_assertions": all(
            d.get("ekko_mode_equal", True) and d.get("ekko_zoom_equal", True)
            for d in differences),
        "full_parity": False,
        "coverage_complete": False,
        "default_bar_geometry_claimed": False,
        "layout_setup": "Two named panes A/B, no bars; Ekko public viewport option and rename actions",
        "limitations": [
            "custom two-pane no-bar Zellij layout; default-bar harness remains separate",
            "pyte cell model is incomplete; raw streams retained",
            "rapid batching is intentionally excluded and remains nondeterministic",
            "no physical-terminal screenshot",
        ],
        "normalizations": [],
        "differences": differences,
    }
    (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"scenario": SCENARIO,
                      "settled_input_slice_passed": report["settled_input_slice_passed"],
                      "application_geometry_slice_passed": report["application_geometry_slice_passed"],
                      "output": str(args.output)}))
    if (not report["stage_count_equal"] or not report["stage_names_equal"]
            or not report["expected_zellij_stage_sequence"]
            or not report["expected_ekko_stage_sequence"]
            or not report["settled_input_slice_passed"]
            or not report["application_geometry_slice_passed"]
            or not report["internal_ekko_mode_zoom_assertions"]):
        raise SystemExit("Pane input/geometry differential failed; see report.json")
    if args.require_parity and not (report["full_parity"] and report["coverage_complete"]
                                    and report["modeled_cell_parity"]
                                    and report["application_input_parity"]):
        raise SystemExit("Parity gate incomplete or differences remain; see report.json")


if __name__ == "__main__":
    main()
