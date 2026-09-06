"""Real-PTY contract for reported pixel metrics and resize propagation."""
import json
import os
from pathlib import Path
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import fcntl

from daily import Attachment, eventually


CONFIG = r'''
(in-package :cl-user)
(ekko/extensions:register-component :id :pixel-test)
(ekko/extensions:set-option :component :pixel-test :name :pty-pixel-source
 :value :reported)
(ekko/extensions:register-command :component :pixel-test :name "split-pixel"
 :handler (lambda (snapshot event) (declare (ignore snapshot))
            (list (ekko/extensions:action :split :axis :rows
                    :argv (getf event :arguments)))))
'''


def child(log):
    path = Path(log)
    def size():
        rows, cols, xp, yp = struct.unpack("HHHH", fcntl.ioctl(0, termios.TIOCGWINSZ, bytes(8)))
        return [cols, rows, xp, yp]
    def record(event):
        with path.open("a", encoding="utf-8") as out:
            json.dump({"event": event, "pty_size": size()}, out)
            out.write("\n")
    import tty
    tty.setraw(0, termios.TCSANOW)
    signal.signal(signal.SIGWINCH, lambda _s, _f: record("WINCH"))
    record("FIRST")
    os.write(1, b"PIXEL-READY\r\n")
    while os.read(0, 4096):
        pass


def read_events(path):
    return [json.loads(line) for line in Path(path).read_text().splitlines()] if Path(path).exists() else []


def integration(binary, bare=False):
    with tempfile.TemporaryDirectory(prefix="ekko-pane-pixels-") as directory:
        root = Path(directory)
        config = root / "init.lisp"
        config.write_text(CONFIG)
        first = root / "first.events"
        second = root / "second.events"
        env = dict(os.environ, XDG_RUNTIME_DIR=directory, EKKO_CONFIG=str(config))
        child_argv = [sys.executable, str(Path(__file__).resolve()), "--child"]
        daemon = subprocess.Popen(
            [binary, "--serve", "pixels", "--viewport", "120", "40", "8", "16",
             *child_argv, str(first), ":::" , *child_argv, str(second)], env=env,
            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        attached = None

        def cli(*args, ok=True):
            result = subprocess.run([binary, *args], env=env, capture_output=True, timeout=8)
            assert (result.returncode == 0) == ok, (args, result.stderr.decode(errors="replace"))
            return result

        def inspect():
            return json.loads(cli("inspect", "pixels").stdout)

        def status():
            return json.loads(cli("status", "pixels").stdout)

        try:
            socket = root / "ekko-v2/pixels.sock"
            eventually(lambda: socket.exists() or daemon.poll() is not None)
            assert daemon.poll() is None, daemon.stderr.read().decode(errors="replace")
            eventually(lambda: len(read_events(first)) == 1 and len(read_events(second)) == 1)
            attached = Attachment(socket)
            eventually(lambda: len(status()["panes"]) == 2)
            initial = inspect()
            viewport = initial["viewport"]
            assert viewport["reported-cell-width"] is None
            assert viewport["reported-cell-height"] is None
            assert viewport["cell-width"] == 8
            assert viewport["cell-height"] == 16
            panes = initial["panes"]
            pids = [pane["pid"] for pane in panes]
            assert all(pane["pty_size"][2:] == [0, 0]
                       for pane in panes)
            assert read_events(first)[0]["pty_size"][2:] == [0, 0]
            assert read_events(second)[0]["pty_size"][2:] == [0, 0]
            sibling_cells = read_events(second)[0]["pty_size"][:2]

            # IPC kind 15 reports the cell size. It updates the durable
            # viewport fact but does not resize or signal existing PTYs.
            before_events = [len(read_events(path)) for path in (first, second)]
            attached.send(15, struct.pack(">II", 9, 17))
            eventually(lambda: inspect()["viewport"]["reported-cell-width"] == 9)
            assert inspect()["viewport"]["reported-cell-height"] == 17
            attached.pump(.15)
            assert [len(read_events(path)) for path in (first, second)] == before_events
            assert [pane["pid"] for pane in status()["panes"]] == pids

            # A public split applies the now-known physical metrics to every
            # surviving PTY, including the unchanged sibling pane.
            cli("command", "--session", "pixels", "split-pixel", sys.executable,
                str(Path(__file__).resolve()), "--child", str(root / "split.events"))
            split_log = root / "split.events"
            eventually(lambda: len(status()["panes"]) == 3)
            eventually(lambda: len(read_events(split_log)) >= 1)
            eventually(lambda: any(e["event"] == "WINCH" for e in read_events(first)))
            eventually(lambda: any(e["event"] == "WINCH" for e in read_events(second)))
            assert all(pane["pty_size"][2:] == [9 * pane["cols"], 17 * pane["rows"]]
                       for pane in inspect()["panes"])
            for path, pane in zip((first, second, split_log), inspect()["panes"]):
                eventually(lambda: read_events(path)[-1]["pty_size"] == pane["pty_size"])
            assert read_events(second)[-1]["pty_size"][:2] == sibling_cells
            assert [pane["pid"] for pane in status()["panes"]][:2] == pids
            assert inspect()["viewport"]["cell-width"] == 8

            # Removing the policy returns to effective default metrics while
            # retaining the reported facts and durable child processes.
            config.write_text("")
            cli("config", "reload", "pixels")
            eventually(lambda: inspect()["viewport"]["reported-cell-width"] == 9)
            assert inspect()["viewport"]["reported-cell-height"] == 17
            assert all(pane["pty_size"][2:] == [8 * pane["cols"], 16 * pane["rows"]]
                       for pane in inspect()["panes"])
            for path, pane in zip((first, second, split_log), inspect()["panes"]):
                eventually(lambda: read_events(path)[-1]["pty_size"] == pane["pty_size"])
            assert [pane["pid"] for pane in status()["panes"]][:2] == pids
            config.write_text(CONFIG)
            cli("config", "reload", "pixels")
            assert inspect()["viewport"]["reported-cell-width"] == 9
            assert all(pane["pty_size"][2:] == [9 * pane["cols"], 17 * pane["rows"]]
                       for pane in inspect()["panes"])
            for path, pane in zip((first, second, split_log), inspect()["panes"]):
                eventually(lambda: read_events(path)[-1]["pty_size"] == pane["pty_size"])
            assert [pane["pid"] for pane in status()["panes"]][:2] == pids
            cli("stop", "pixels")
            daemon.wait(timeout=3)
            assert daemon.returncode == 0
        finally:
            if attached:
                attached.close()
            if daemon.poll() is None:
                daemon.terminate()
                daemon.wait(timeout=3)
    return {"status": "pass", "suite": "pane-pixels-bare" if bare else "pane-pixels"}


if __name__ == "__main__":
    if sys.argv[1:2] == ["--child"]:
        child(sys.argv[2])
    else:
        if len(sys.argv) < 2:
            raise SystemExit("usage: pane_pixels.py BINARY [bare]")
        print(json.dumps(integration(sys.argv[1], bare=len(sys.argv) > 2)))
