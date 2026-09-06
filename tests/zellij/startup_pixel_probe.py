"""Record startup PTY pixel/query ordering for Zellij and Ekko.

This is deliberately separate from the acceptance harness.  It reuses its
deterministic three child fixture and runs only the startup checkpoint under
immediate, gated, and absent query responses.
"""
import argparse
import json
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import pane_workflow_differential as workflow


class ProbeTerminal(workflow.QueryTerminal):
    capture_path = None
    schedule = "immediate"
    event_dir = None
    last_instance = None

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        type(self).last_instance = self
        self.probe_times = []
        self._gated = bytearray()
        self.gate_complete_monotonic = None

    def _gate_open(self):
        paths = [self.event_dir / f"{label}.events" for label in ("A", "B", "C")]
        if not all(path.exists() for path in paths):
            return False
        if not all(any(json.loads(line).get("event") == "FIRST"
                       for line in path.read_text().splitlines()) for path in paths):
            return False
        if self.gate_complete_monotonic is None:
            self.gate_complete_monotonic = time.monotonic()
        return True

    def _answer_queries(self, data):
        seen = time.monotonic()
        if self.schedule == "gated":
            # Hold query bytes until all children have recorded FIRST.  This
            # changes metric availability without inserting a child delay.
            if not self._gate_open():
                self._gated.extend(data)
                return
            data = bytes(self._gated) + data
            self._gated.clear()
            seen = time.monotonic()
        before = len(self.query_log)
        super()._answer_queries(data)
        if len(self.query_log) != before:
            if self.schedule == "gated" and self.gate_complete_monotonic is not None:
                assert seen >= self.gate_complete_monotonic
            self.probe_times.append({"parser_release_monotonic": seen,
                                     "queries": self.query_log[before:],
                                     "gate_complete_monotonic": self.gate_complete_monotonic})

    def pump(self, duration=.25):
        result = super().pump(duration)
        if self.schedule == "gated" and self._gated and self._gate_open():
            self._answer_queries(b"")
        if self.capture_path is not None:
            self.capture_path.write_bytes(bytes(self.raw))
        return result


def run_one(kind, binary, profile, reference, root, out, schedule, cols, rows):
    old = workflow.QueryTerminal
    ProbeTerminal.schedule = schedule
    ProbeTerminal.capture_path = out / "raw.ansi"
    workflow.QueryTerminal = ProbeTerminal
    try:
        ProbeTerminal.event_dir = root / "session"
        # run_side creates the child fixture and session directory itself.
        stages = workflow.run_side(kind, binary, profile, reference, root, out,
                                    workflow.write_spawn_shell(root),
                                    [("startup", b"")], cols, rows,
                                    answer_queries=(schedule != "none"))
        startup = stages[0]
        startup["probe_schedule"] = schedule
        startup["captured_at_monotonic"] = time.monotonic()
        startup["probe_times"] = ProbeTerminal.last_instance.probe_times
        startup["gate_complete_monotonic"] = ProbeTerminal.last_instance.gate_complete_monotonic
        if schedule == "gated":
            assert startup["gate_complete_monotonic"] is not None, "FIRST gate never opened"
            assert any(q.get("response_hex") for entry in startup["probe_times"]
                       for q in entry["queries"]), "no gated query responses were sent"
        return startup
    finally:
        workflow.QueryTerminal = old


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--zellij", required=True)
    parser.add_argument("--ekko", required=True)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--reference", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    results = {}
    for schedule in ("immediate", "gated", "none"):
        for kind, binary in (("zellij", args.zellij), ("ekko", args.ekko)):
            with tempfile.TemporaryDirectory(prefix="ekko-startup-probe-") as temp:
                root = Path(temp)
                target = args.output / schedule / kind
                target.mkdir(parents=True, exist_ok=True)
                results[f"{schedule}:{kind}"] = run_one(
                    kind, binary, args.profile.resolve(), args.reference.resolve(),
                    root, target, schedule, 80, 24)
    report = {
        "dimensions": [80, 24],
        "schedules": ["immediate", "gated", "none"],
        "results": results,
        "normalizations": [],
        "limitations": [
            "child fixture events have harness observation timestamps only; raw event order and sizes are retained",
            "parser-release timestamps bound response processing; gated query bytes may have arrived earlier",
        ],
    }
    (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"output": str(args.output), "schedules": report["schedules"]}))


if __name__ == "__main__":
    main()
