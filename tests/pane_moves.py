"""Real PTY Move-mode lifecycle and public layout replacement contract."""
import json
import os
from pathlib import Path
import subprocess
import struct
import sys
import tempfile
import termios
import tty

from daily import Attachment, eventually


CONFIG = r'''
(in-package :cl-user)
(ekko/extensions:register-component :id :move-test)
(ekko/extensions:set-option :component :move-test :name :initial-layout
 :value '(:columns 50 1 (:rows 50 2 3)))
'''


def child(path):
    tty.setraw(0, termios.TCSANOW)
    Path(path).write_bytes(b"")
    os.write(1, b"MOVE-READY\r\n")
    while True:
        data = os.read(0, 4096)
        if not data:
            return
        with Path(path).open("ab", buffering=0) as output:
            output.write(data)


def integration(binary, profile, bare=False):
    profile = Path(profile).resolve()
    with tempfile.TemporaryDirectory(prefix="ekko-pane-moves-") as directory:
        root = Path(directory)
        config = root / "moves.lisp"
        for helper in profile.parent.glob("zellij-*.lisp"):
            (root / helper.name).write_bytes(helper.read_bytes())
        config.write_text(profile.read_text() + CONFIG)
        logs = [root / f"{i}.input" for i in range(1, 4)]
        env = dict(os.environ, XDG_RUNTIME_DIR=directory, EKKO_CONFIG=str(config))
        child_argv = [sys.executable, str(Path(__file__).resolve()), "--child"]
        argv = [binary, "--serve", "moves", "--viewport", "120", "40", "8", "16"]
        for index, log in enumerate(logs):
            if index:
                argv.append(":::")
            argv.extend(child_argv + [str(log)])
        daemon = subprocess.Popen(argv, env=env, stdout=subprocess.DEVNULL,
                                  stderr=subprocess.PIPE)
        attached = None

        def cli(*args, ok=True):
            result = subprocess.run([binary, *args], env=env, capture_output=True,
                                    timeout=8)
            assert (result.returncode == 0) == ok, (args, result.stderr.decode())
            return result

        def inspect():
            return json.loads(cli("inspect", "moves").stdout)

        def command(name):
            cli("command", "--session", "moves", name)

        try:
            socket = root / "ekko-v2/moves.sock"
            eventually(lambda: socket.exists() or daemon.poll() is not None)
            assert daemon.poll() is None, daemon.stderr.read().decode(errors="replace")
            eventually(lambda: all(path.exists() for path in logs))
            attached = Attachment(socket)
            initial = inspect()
            assert initial["layout"] == ["columns", 50, 1, ["rows", 50, 2, 3]]
            pids = [pane["pid"] for pane in initial["panes"]]
            rects = [pane["outer_rect"] for pane in initial["panes"]]
            assert all(len(rect) == 4 for rect in rects)
            focus = cli("status", "moves").stdout
            focus = json.loads(focus)["focus"]

            # Reporting dimensions does not resize children. The subsequent
            # layout reconciles pixels, including the unchanged third pane.
            attached.send(15, struct.pack(">II", 8, 16))
            eventually(lambda: inspect()["viewport"]["reported-cell-width"] == 8)
            assert all(p["pty_size"][2:] == [0, 0] for p in inspect()["panes"])

            # Move mode uses the public set-layout action. n swaps the focused
            # pane with the next row-major pane; p applies the inverse.
            attached.send(2, b"\x08")
            eventually(lambda: inspect()["mode"] == "move")
            attached.send(2, b"n")
            eventually(lambda: inspect()["layout"] == ["columns", 50, 2, ["rows", 50, 1, 3]])
            swapped = inspect()
            assert swapped["layout"] == ["columns", 50, 2, ["rows", 50, 1, 3]]
            assert json.loads(cli("status", "moves").stdout)["focus"] == focus
            swapped_rects = [pane["outer_rect"] for pane in swapped["panes"]]
            assert swapped_rects == [rects[1], rects[0], rects[2]]
            for pane in swapped["panes"]:
                assert pane["pty_size"][2:] == [8 * pane["cols"], 16 * pane["rows"]]
            assert [pane["pid"] for pane in swapped["panes"]] == pids
            attached.send(2, b"p")
            eventually(lambda: inspect()["layout"] == initial["layout"])
            restored = inspect()
            assert restored["layout"] == initial["layout"]
            assert [pane["outer_rect"] for pane in restored["panes"]] == rects
            assert [pane["pid"] for pane in restored["panes"]] == pids

            # Movement stays in Move mode. q is ignored without PTY leakage.
            assert inspect()["mode"] == "move"
            before = [path.read_bytes() for path in logs]
            attached.send(2, b"q")
            attached.pump(.15)
            assert [path.read_bytes() for path in logs] == before

            # Navigation and both tab directions are consumed by Move mode.
            attached.send(2, b"n\tp" + b"hjkl" + b"\x1b[D\x1b[B\x1b[A\x1b[C")
            attached.pump(.3)
            assert [path.read_bytes() for path in logs] == before
            assert json.loads(cli("status", "moves").stdout)["focus"] == focus
            attached.send(2, b"\x07")
            eventually(lambda: inspect()["mode"] == "locked")
            attached.send(2, b"\x07")
            eventually(lambda: inspect()["mode"] == "normal")

            # All documented exits leave no mode-key bytes in child input.
            for key in (b"\r", b"\x08"):
                if inspect()["mode"] != "move":
                    attached.send(2, b"\x08")
                    eventually(lambda: inspect()["mode"] == "move")
                attached.send(2, key)
                eventually(lambda: inspect()["mode"] == "normal")
            attached.send(2, b"\x08")
            eventually(lambda: inspect()["mode"] == "move")
            attached.send(2, b"\x1b")
            eventually(lambda: inspect()["mode"] == "normal")

            assert [path.read_bytes() for path in logs] == before
            attached.send(2, b"INPUT-CHECK")
            eventually(lambda: logs[focus - 1].read_bytes() == before[focus - 1] + b"INPUT-CHECK")

            # Reload and reattach preserve the current tree and all processes.
            before_reload = inspect()
            attached.close()
            attached = None
            cli("config", "reload", "moves")
            after_reload = inspect()
            assert after_reload["layout"] == before_reload["layout"]
            assert [pane["pid"] for pane in after_reload["panes"]] == pids
            attached = Attachment(socket)
            cli("stop", "moves")
            daemon.wait(timeout=3)
            assert daemon.returncode == 0, daemon.stderr.read().decode(errors="replace")
        finally:
            if attached:
                attached.close()
            if daemon.poll() is None:
                daemon.terminate()
                daemon.wait(timeout=3)
    print(json.dumps({"status": "pass", "suite": "pane-moves-bare" if bare else "pane-moves"}))


if __name__ == "__main__":
    if sys.argv[1:2] == ["--child"]:
        child(sys.argv[2])
    else:
        integration(sys.argv[1], sys.argv[2], len(sys.argv) > 3 and sys.argv[3] == "bare")
