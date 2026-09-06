"""Bounded Zellij Pane-mode controls over two real daemon PTYs."""
import json
import os
from pathlib import Path
import subprocess
import struct
import sys
import tempfile

from daily import Attachment, eventually


def integration(binary, profile, bare=False):
    profile = Path(profile).resolve()
    with tempfile.TemporaryDirectory(prefix="ekko-pane-modes-") as directory:
        root = Path(directory)
        for helper in profile.parent.glob("zellij-*.lisp"):
            (root / helper.name).write_bytes(helper.read_bytes())
        config = root / "init.lisp"
        config.write_text(profile.read_text())
        first_log = root / "first-input"
        second_log = root / "second-input"
        child = [sys.executable, str(Path(__file__).with_name("daily.py")), "--child"]
        env = dict(os.environ, XDG_RUNTIME_DIR=directory, EKKO_CONFIG=str(config))
        daemon_log = open(root / "daemon.log", "wb")
        daemon = subprocess.Popen(
            [binary, "--serve", "pane-modes"]
            + child + [str(first_log), ":::"] + child + [str(second_log)],
            env=env, stdout=daemon_log, stderr=daemon_log)
        attached = None

        def cli(*args, ok=True):
            result = subprocess.run([binary, *args], env=env, capture_output=True, timeout=8)
            assert (result.returncode == 0) == ok, (args, result.returncode, result.stderr.decode())
            if ok and args[0] not in ("status", "inspect", "buffer"):
                assert not result.stdout, (args, result.stdout)
            return result

        def status():
            return json.loads(cli("status", "pane-modes").stdout)

        def inspect():
            return json.loads(cli("inspect", "pane-modes").stdout)

        def settle(predicate):
            def check():
                attached.pump(.01)
                return predicate()
            eventually(check)

        def input_settled(log, before, data):
            attached.pump(.01)
            return log.read_bytes() == before + data

        try:
            eventually(lambda: (first_log.exists() and second_log.exists()) or daemon.poll() is not None)
            assert daemon.poll() is None, (root / "daemon.log").read_text()
            cli("config", "check")
            eventually(lambda: len(status()["panes"]) == 2)
            attached = Attachment(root / "ekko-v2/pane-modes.sock")
            # Attachment negotiates 120x40, so record the baseline only after
            # the daemon has applied that viewport to both PTYs.
            eventually(lambda: status()["panes"][0]["rows"] > 34)
            initial = status()
            pids = [pane["pid"] for pane in initial["panes"]]
            initial_geometry = [(pane["cols"], pane["rows"]) for pane in initial["panes"]]
            assert not inspect()["zoom"]
            assert inspect()["mode"] == "normal"
            before_input = first_log.read_bytes()
            second_before_input = second_log.read_bytes()

            # Pane's unbound q is suppressed, while each reference exit key is
            # consumed and returns to Normal.
            attached.send(2, b"\x10")
            settle(lambda: inspect()["mode"] == "pane")
            attached.send(2, b"q")
            attached.pump(.15)
            assert inspect()["mode"] == "pane"
            assert first_log.read_bytes() == before_input
            attached.send(2, b"\r")
            settle(lambda: inspect()["mode"] == "normal")
            assert first_log.read_bytes() == before_input
            for key in (b"\x1b", b"\x10"):
                attached.send(2, b"\x10")
                settle(lambda: inspect()["mode"] == "pane")
                attached.send(2, key)
                settle(lambda: inspect()["mode"] == "normal")
                assert first_log.read_bytes() == before_input
            assert second_log.read_bytes() == second_before_input

            # Pane Ctrl-g enters Locked. Its q is forwarded according to the
            # locked map's policy, then Ctrl-g returns to Normal.
            attached.send(2, b"\x10")
            settle(lambda: inspect()["mode"] == "pane")
            attached.send(2, b"\x07")
            settle(lambda: inspect()["mode"] == "locked")
            locked_input = first_log.read_bytes()
            attached.send(2, b"q")
            eventually(lambda: input_settled(first_log, locked_input, b"q"))
            attached.send(2, b"\x07")
            settle(lambda: inspect()["mode"] == "normal")
            assert second_log.read_bytes() == second_before_input

            # Normal Ctrl-p enters Pane mode, and Pane f zooms then returns to
            # Normal. Both mode keys are consumed by Ekko.
            before_input = first_log.read_bytes()
            attached.send(2, b"\x10")
            settle(lambda: inspect()["mode"] == "pane")
            attached.send(2, b"f")
            settle(lambda: inspect()["mode"] == "normal" and inspect()["zoom"])
            zoomed = status()
            zoomed_geometry = [(pane["cols"], pane["rows"]) for pane in zoomed["panes"]]
            assert zoomed["panes"][0]["cols"] > initial_geometry[0][0]
            assert zoomed["panes"][0]["rows"] == initial_geometry[0][1]
            assert [pane["pid"] for pane in zoomed["panes"]] == pids
            assert first_log.read_bytes() == before_input

            # Enter Pane again and repeat f; the original split geometry and
            # Normal mode return after the fullscreen toggle.
            attached.send(2, b"\x10")
            settle(lambda: inspect()["mode"] == "pane")
            attached.send(2, b"f")
            settle(lambda: inspect()["mode"] == "normal" and not inspect()["zoom"])
            restored = status()
            assert [(pane["cols"], pane["rows"]) for pane in restored["panes"]] == initial_geometry
            assert [pane["pid"] for pane in restored["panes"]] == pids

            # Reload while zoomed preserves both durable pane processes and the
            # selected session state because the profile still registers Pane.
            attached.send(2, b"\x10")
            settle(lambda: inspect()["mode"] == "pane")
            attached.send(2, b"f")
            settle(lambda: inspect()["mode"] == "normal" and inspect()["zoom"])
            # Reload geometry options through the public extension API. The
            # first reload keeps fullscreen active, so the focused pane uses
            # the full outer viewport before we toggle back to the split.
            custom_geometry = """
(in-package :cl-user)
(ekko/extensions:register-component :id :geometry-test)
(ekko/extensions:set-option :component :geometry-test :name :pane-insets :value '(1 1 1 1))
(ekko/extensions:set-option :component :geometry-test :name :viewport-insets :value '(0 0 0 0))
(ekko/extensions:set-option :component :geometry-test :name :split-gaps :value '(0 0))
"""
            config.write_text(profile.read_text() + custom_geometry)
            cli("config", "reload", "pane-modes")
            reloaded = inspect()
            assert reloaded["mode"] == "normal" and reloaded["zoom"]
            assert reloaded["geometry"] == {
                "pane-insets": [1, 1, 1, 1],
                "boundary-insets": None,
                "contributions": None,
                "viewport-insets": [0, 0, 0, 0],
                "split-gaps": [0, 0],
            }
            reloaded_status = status()
            # Reload applies the new viewport size to every durable PTY,
            # including the hidden sibling retained during fullscreen.
            assert [(pane["cols"], pane["rows"]) for pane in reloaded_status["panes"]] == [(118, 38), (58, 38)]
            assert [pane["pid"] for pane in reloaded_status["panes"]] == pids

            # Leave fullscreen under the custom geometry and check both
            # split content rectangles after the transactional reload.
            attached.send(2, b"\x10")
            settle(lambda: inspect()["mode"] == "pane")
            attached.send(2, b"f")
            settle(lambda: inspect()["mode"] == "normal" and not inspect()["zoom"])
            split_custom = status()
            assert [(pane["cols"], pane["rows"]) for pane in split_custom["panes"]] == [(58, 38), (58, 38)]
            assert [pane["pid"] for pane in split_custom["panes"]] == pids

            # Insets larger than a 5x4 viewport collapse the tree first and
            # then clamp top/left, leaving a one-cell PTY in the focused pane.
            tiny_geometry = custom_geometry.replace("'(1 1 1 1)", "'(16 16 16 16)")
            config.write_text(profile.read_text() + tiny_geometry)
            cli("config", "reload", "pane-modes")
            attached.send(1, struct.pack(">IIIII", 6, 5, 4, 8, 16))
            eventually(lambda: status()["panes"][0]["cols"] == 1 and status()["panes"][0]["rows"] == 1)
            tiny = status()
            assert (tiny["panes"][0]["x"], tiny["panes"][0]["y"]) == (4, 3)
            attached.send(1, struct.pack(">IIIII", 6, 120, 40, 8, 16))
            eventually(lambda: status()["panes"][0]["cols"] == 28 and status()["panes"][0]["rows"] == 8)

            # Restore the source profile and verify the original split while
            # preserving both PTY identities and the durable mode state.
            config.write_text(profile.read_text())
            cli("config", "reload", "pane-modes")
            restored_profile = inspect()
            assert restored_profile["mode"] == "normal" and not restored_profile["zoom"]
            restored_status = status()
            assert [(pane["cols"], pane["rows"]) for pane in restored_status["panes"]] == initial_geometry
            assert [pane["pid"] for pane in restored_status["panes"]] == pids

            cli("stop", "pane-modes")
            daemon.wait(timeout=3)
            assert daemon.returncode == 0, (root / "daemon.log").read_text()
        except BaseException:
            print((root / "daemon.log").read_text(), file=sys.stderr)
            raise
        finally:
            if attached:
                attached.close()
            if daemon.poll() is None:
                daemon.terminate()
                daemon.wait(timeout=3)
            daemon_log.close()
    print(json.dumps({"status": "pass", "suite": "pane-modes-bare" if bare else "pane-modes"}))


if __name__ == "__main__":
    if len(sys.argv) < 3:
        raise SystemExit("usage: pane_modes.py BINARY ZELLIJ_PROFILE [bare]")
    integration(sys.argv[1], sys.argv[2], bare=len(sys.argv) > 3)
