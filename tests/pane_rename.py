"""Real-worker RenamePane editing, undo state, paste, and reload lifecycle."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from daily import Attachment, eventually


def integration(binary, profile, bare=False):
    profile = Path(profile).resolve()
    with tempfile.TemporaryDirectory(prefix="ekko-pane-rename-") as directory:
        root = Path(directory)
        for helper in profile.parent.glob("zellij-*.lisp"):
            (root / helper.name).write_bytes(helper.read_bytes())
        config = root / "init.lisp"
        config.write_text(profile.read_text())
        log = root / "input"
        env = dict(os.environ, XDG_RUNTIME_DIR=directory, EKKO_CONFIG=str(config))
        daemon = subprocess.Popen([binary, "--serve", "rename", sys.executable,
                                   str(Path(__file__).with_name("daily.py")), "--child", str(log)],
                                  env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        attached = None

        def cli(*args):
            p = subprocess.run([binary, *args], env=env, capture_output=True, timeout=8)
            assert p.returncode == 0, (args, p.stderr.decode(errors="replace"))
            return p

        def inspect():
            return json.loads(cli("inspect", "rename").stdout)

        def send(data, mode, name):
            attached.send(2, data)
            def ready():
                attached.pump(.01)
                s = inspect()
                return s["mode"] == mode and s["panes"][0]["name"] == name
            eventually(ready)

        try:
            eventually(lambda: log.exists() or daemon.poll() is not None)
            assert daemon.poll() is None, daemon.stderr.read().decode(errors="replace")
            socket = root / "ekko-v2/rename.sock"
            attached = Attachment(socket)
            pid = inspect()["panes"][0]["pid"]
            baseline = log.read_bytes()
            send(b"\x10c", "rename", None)
            send(b"X", "rename", "X")
            send(b"\x7f", "rename", "")
            def empty_placeholder():
                attached.pump(.02)
                return bool(attached.scenes and "Enter name..." in attached.scenes[-1])
            eventually(empty_placeholder)
            send(b"ABC\r", "normal", "ABC")
            send(b"\x10cX", "rename", "ABCX")
            old_state = inspect()["component-state"]
            attached.close()
            attached = None
            cli("config", "reload", "rename")
            assert inspect()["mode"] == "rename"
            assert inspect()["component-state"] == old_state
            attached = Attachment(socket)
            send(b"\x1b", "pane", "ABC")
            # Undo retains its saved name until the next rename entry.
            assert inspect()["component-state"] == old_state
            send(b"c\x7f", "rename", "AB")
            attached.send(5, b"\x1b[200~")
            attached.send(5, b"Z\nQ")
            attached.send(5, b"\x1b[201~")
            eventually(lambda: inspect()["panes"][0]["name"] == "ABZQ")
            assert inspect()["mode"] == "rename"
            send(b"\x03", "normal", "ABZQ")
            assert log.read_bytes() == baseline
            assert inspect()["panes"][0]["pid"] == pid
            config.write_text("")
            cli("config", "reload", "rename")
            assert not inspect()["component-state"]
            assert inspect()["panes"][0]["name"] == "ABZQ"
            config.write_text(profile.read_text())
            cli("config", "reload", "rename")
            assert inspect()["panes"][0]["pid"] == pid
            attached.send(2, b"NORMAL-INPUT")
            eventually(lambda: log.read_bytes() == baseline + b"NORMAL-INPUT")
            cli("stop", "rename")
            daemon.wait(timeout=3)
            assert daemon.returncode == 0
        finally:
            if attached:
                attached.close()
            if daemon.poll() is None:
                daemon.terminate()
                daemon.wait(timeout=3)
    print(json.dumps({"status": "pass", "suite": "pane-rename-bare" if bare else "pane-rename"}))


if __name__ == "__main__":
    integration(sys.argv[1], sys.argv[2], len(sys.argv) > 3 and sys.argv[3] == "bare")
