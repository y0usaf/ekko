"""Fallback commands receive ordered semantic keys and original UTF-8 octets."""
import json
import os
from pathlib import Path
import subprocess
import signal
import sys
import tempfile

from daily import Attachment, eventually

CONFIG = r'''
(in-package :cl-user)
(ekko/extensions:register-component :id :input-test :reads '(:panes :focus :component-state))
(ekko/extensions:register-keymap :component :input-test :name :capture :unbound "capture")
(ekko/extensions:register-keymap :component :input-test :name :normal :unbound :forward)
(ekko/extensions:set-option :component :input-test :name :initial-keymap :value :capture)
(ekko/extensions:register-command :component :input-test :name "capture"
 :handler (lambda (snapshot event)
            (let ((pane (find (getf snapshot :focus) (getf snapshot :panes)
                              :key (lambda (p) (getf p :id)))))
              (list (ekko/extensions:action :rename :text
                      (format nil "~A~A=~{~D~^,~}|" (or (getf pane :name) "")
                              (getf event :key) (getf event :bytes)))
                    (ekko/extensions:action :set-state :value
                      (list :last-key (getf event :key) :bytes (getf event :bytes)
                            :count (1+ (getf (cdr (assoc "input-test"
                                               (getf snapshot :component-state) :test #'equal))
                                            :count 0))))))))
(ekko/extensions:register-command :component :input-test :name "finish"
 :handler (lambda (snapshot event) (declare (ignore snapshot event))
            (list (ekko/extensions:action :set-keymap :name :normal))))
(ekko/extensions:bind-key :component :input-test :map :capture :key "Enter" :command "finish")
'''


def integration(binary, bare=False):
    with tempfile.TemporaryDirectory(prefix="ekko-keymap-input-") as directory:
        root = Path(directory)
        config = root / "init.lisp"
        config.write_text(CONFIG)
        log = root / "input"
        env = dict(os.environ, XDG_RUNTIME_DIR=directory, EKKO_CONFIG=str(config))
        daemon = subprocess.Popen([binary, "--serve", "input", sys.executable,
                                   str(Path(__file__).with_name("daily.py")), "--child", str(log)],
                                  env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        attached = None

        def cli(*args, ok=True):
            p = subprocess.run([binary, *args], env=env, capture_output=True, timeout=8)
            assert (p.returncode == 0) == ok, (args, p.stderr.decode(errors="replace"))
            return p

        def inspect():
            return json.loads(cli("inspect", "input").stdout)

        try:
            socket = root / "ekko-v2/input.sock"
            eventually(lambda: log.exists() or daemon.poll() is not None)
            assert daemon.poll() is None, daemon.stderr.read().decode(errors="replace")
            attached = Attachment(socket)
            pid = inspect()["panes"][0]["pid"]
            baseline = log.read_bytes()
            # One packet starts several asynchronous commands. Each handler
            # must see the preceding result, not the same stale snapshot.
            attached.send(2, b"ab" + "λ".encode())
            expected = "97=97|98=98|955=206,187|"
            eventually(lambda: inspect()["panes"][0]["name"] == expected)
            # Incomplete UTF-8 spans wire packets and is delivered once.
            attached.send(2, b"\xf0\x9f")
            attached.pump(.1)
            assert inspect()["panes"][0]["name"] == expected
            attached.send(2, b"\x98\x80")
            expected += "128512=240,159,152,128|"
            eventually(lambda: inspect()["panes"][0]["name"] == expected)
            attached.send(2, b"\x1b[D")
            expected += "LEFT=27,91,68|"
            eventually(lambda: inspect()["panes"][0]["name"] == expected)
            # A bracketed paste is one callback, bypasses bound keys, and
            # retains raw UTF-8 across packet boundaries.
            attached.send(5, b"\x1b[200~")
            attached.send(5, b"x\r\xce")
            attached.send(5, b"\xbb\x7f")
            attached.send(5, b"\x1b[201~")
            expected += "NIL=120,13,206,187,127|"
            eventually(lambda: inspect()["panes"][0]["name"] == expected)
            assert inspect()["mode"] == "capture"
            assert log.read_bytes() == baseline
            # Oversized callback pastes fail without being delivered to the
            # child or partially changing component state.
            attached.send(5, b"\x1b[200~")
            attached.send(5, b"x" * 4097)
            attached.send(5, b"\x1b[201~")
            eventually(lambda: "4096" in (inspect()["error"] or ""))
            assert inspect()["panes"][0]["name"] == expected
            assert log.read_bytes() == baseline
            # The bound Enter wins over fallback. Remaining packet bytes are
            # routed according to the committed new map.
            attached.send(2, b"\rPASSTHROUGH")
            eventually(lambda: inspect()["mode"] == "normal")
            eventually(lambda: log.read_bytes() == baseline + b"PASSTHROUGH")
            assert inspect()["panes"][0]["name"] == expected
            assert inspect()["panes"][0]["pid"] == pid
            state = inspect()["component-state"]
            assert state == [{"owner": "input-test", "value":
                              ["last-key", None, "bytes", [120, 13, 206, 187, 127], "count", 6]}], state
            attached.close()
            attached = None
            cli("config", "reload", "input")
            assert inspect()["component-state"] == state
            attached = Attachment(socket)
            worker = json.loads(cli("status", "input").stdout)["extension_pid"]
            os.kill(worker, signal.SIGKILL)
            eventually(lambda: json.loads(cli("status", "input").stdout)["extension_pid"]
                       not in (None, worker))
            assert inspect()["component-state"] == state
            cli("command", "--session", "input", "capture")
            expected += "NIL=|"
            assert inspect()["panes"][0]["name"] == expected
            state = inspect()["component-state"]
            assert state[0]["value"][-1] == 7
            config.write_text(CONFIG.replace(':unbound "capture"', ':unbound "missing"'))
            cli("config", "reload", "input", ok=False)
            assert inspect()["mode"] == "normal"
            assert inspect()["component-state"] == state
            assert inspect()["panes"][0]["pid"] == pid
            attached.send(2, b"STILL-LIVE")
            eventually(lambda: log.read_bytes() == baseline + b"PASSTHROUGHSTILL-LIVE")
            config.write_text("")
            cli("config", "reload", "input")
            assert not inspect()["component-state"]
            assert inspect()["panes"][0]["pid"] == pid
            cli("stop", "input")
            daemon.wait(timeout=3)
            assert daemon.returncode == 0
        finally:
            if attached:
                attached.close()
            if daemon.poll() is None:
                daemon.terminate()
                daemon.wait(timeout=3)
    print(json.dumps({"status": "pass", "suite": "keymap-input-bare" if bare else "keymap-input"}))


if __name__ == "__main__":
    integration(sys.argv[1], len(sys.argv) > 2 and sys.argv[2] == "bare")
