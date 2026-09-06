"""Public layout replacement preserves real children and daemon-owned state."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from daily import Attachment, eventually
from pane_pixels import child, read_events

CONFIG = r'''
(in-package :cl-user)
(ekko/extensions:register-component :id :layout-test)
(dolist (entry '((:pane-insets 0 0 0 0) (:viewport-insets 0 0 0 0)
                 (:split-gaps 0 0)))
  (ekko/extensions:set-option :component :layout-test :name (first entry)
                             :value (rest entry)))
(ekko/extensions:set-option :component :layout-test :name :initial-layout
 :value '(:columns 50 1 (:rows 50 2 3)))
(ekko/extensions:register-command :component :layout-test :name "arrange"
 :handler (lambda (snapshot event) (declare (ignore snapshot event))
            (list (ekko/extensions:action :set-layout
                    :tree '(:rows 40 3 (:columns 30 1 2))))))
(ekko/extensions:register-command :component :layout-test :name "invalid"
 :handler (lambda (snapshot event) (declare (ignore snapshot event))
            (list (ekko/extensions:action :set-layout
                    :tree '(:rows 40 3 (:columns 30 1 1))))))
(ekko/extensions:register-command :component :layout-test :name "focus-two"
 :handler (lambda (snapshot event) (declare (ignore snapshot event))
            (list (ekko/extensions:action :focus :pane 2))))
(ekko/extensions:register-command :component :layout-test :name "zoom"
 :handler (lambda (snapshot event) (declare (ignore snapshot event))
            (list (ekko/extensions:action :zoom))))
'''


def integration(binary, bare=False):
    with tempfile.TemporaryDirectory(prefix="ekko-pane-layouts-") as directory:
        root = Path(directory)
        config = root / "init.lisp"
        config.write_text(CONFIG)
        env = dict(os.environ, XDG_RUNTIME_DIR=directory, EKKO_CONFIG=str(config))
        logs = [root / f"{i}.events" for i in range(3)]
        argv = [binary, "--serve", "layouts", "--viewport", "120", "40", "8", "16"]
        for i, log in enumerate(logs):
            if i:
                argv.append(":::")
            argv.extend([sys.executable, str(Path(__file__).resolve()), "--child", str(log)])
        daemon = subprocess.Popen(argv, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        attached = None

        def cli(*args, ok=True):
            result = subprocess.run([binary, *args], env=env, capture_output=True, timeout=8)
            assert (result.returncode == 0) == ok, (args, result.stderr.decode(errors="replace"))
            return result

        def inspect():
            return json.loads(cli("inspect", "layouts").stdout)

        def command(name, ok=True):
            return cli("command", "--session", "layouts", name, ok=ok)

        def state():
            data = inspect()
            return {key: data[key] for key in ("layout", "zoom", "panes")}

        try:
            socket = root / "ekko-v2/layouts.sock"
            eventually(lambda: socket.exists() or daemon.poll() is not None)
            assert daemon.poll() is None, daemon.stderr.read().decode(errors="replace")
            eventually(lambda: all(read_events(log) for log in logs))
            attached = Attachment(socket)
            pids = [p["pid"] for p in inspect()["panes"]]
            command("focus-two")
            command("zoom")
            command("arrange")
            arranged = inspect()
            assert arranged["zoom"] is True
            assert arranged["layout"] == ["rows", 40, 3, ["columns", 30, 1, 2]], arranged["layout"]
            assert [p["pid"] for p in arranged["panes"]] == pids
            visible = [p["id"] for p in arranged["panes"] if p["visible"]]
            assert visible == [2], visible
            command("zoom")
            expected = [[0, 16, 36, 24], [36, 16, 84, 24], [0, 0, 120, 16]]
            panes = inspect()["panes"]
            assert [p["outer_rect"] for p in panes] == expected
            for log, pane in zip(logs, panes):
                eventually(lambda: read_events(log)[-1]["pty_size"] == pane["pty_size"])
            before = state()
            command("invalid", ok=False)
            assert state() == before
            attached.close()
            attached = None
            config.write_text("")
            cli("config", "reload", "layouts")
            restored = inspect()
            assert restored["layout"] == arranged["layout"]
            assert [p["pid"] for p in restored["panes"]] == pids
            attached = Attachment(socket)
            config.write_text(CONFIG)
            cli("config", "reload", "layouts")
            restored = inspect()
            assert restored["layout"] == arranged["layout"]
            assert [p["outer_rect"] for p in restored["panes"]] == expected
            assert [p["pid"] for p in restored["panes"]] == pids
            cli("stop", "layouts")
            daemon.wait(timeout=3)
            assert daemon.returncode == 0
        finally:
            if attached:
                attached.close()
            if daemon.poll() is None:
                daemon.terminate()
                daemon.wait(timeout=3)
    return {"status": "pass", "suite": "pane-layouts-bare" if bare else "pane-layouts"}


if __name__ == "__main__":
    if sys.argv[1:2] == ["--child"]:
        child(sys.argv[2])
    else:
        print(json.dumps(integration(sys.argv[1], len(sys.argv) > 2 and sys.argv[2] == "bare")))
