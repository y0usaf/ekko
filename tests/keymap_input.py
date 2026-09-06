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

READ_BYTES_CONFIG = r'''
(in-package :cl-user)
(ekko/extensions:register-component :id :read-test :reads '(:panes :focus :component-state))
(ekko/extensions:register-keymap :component :read-test :name :capture :unbound "capture")
(ekko/extensions:register-keymap :component :read-test :name :normal :unbound :forward)
(ekko/extensions:set-option :component :read-test :name :initial-keymap :value :capture)
(ekko/extensions:register-command :component :read-test :name "capture"
 :handler (lambda (snapshot event)
            (let* ((old (cdr (assoc "read-test" (getf snapshot :component-state)
                   :test #'equal)))
                   (count (1+ (getf old :count 0)))
                   (present (member :read-bytes event))
                   (record (list (getf event :bytes) (if present t nil)
                                 (getf event :read-bytes))))
              (list (ekko/extensions:action :rename :pane (getf snapshot :focus)
                                             :text (write-to-string count))
                    (ekko/extensions:action :set-state :value
                      (list :count count :bytes (getf event :bytes)
                            :read-present (if present t nil)
                            :read-bytes (getf event :read-bytes)
                            :history (append (getf old :history) (list record))))))))
(ekko/extensions:register-command :component :read-test :name "finish"
 :handler (lambda (snapshot event) (declare (ignore snapshot event))
            (list (ekko/extensions:action :set-keymap :name :normal))))
(ekko/extensions:register-command :component :read-test :name "capture-mode"
 :handler (lambda (snapshot event) (declare (ignore snapshot event))
            (list (ekko/extensions:action :set-keymap :name :capture))))
(ekko/extensions:register-command :component :read-test :name "bound"
 :handler (lambda (snapshot event) (declare (ignore snapshot event)) nil))
(ekko/extensions:bind-key :component :read-test :map :capture :key "Enter" :command "finish")
(ekko/extensions:bind-key :component :read-test :map :normal :key "c" :command "capture-mode")
(ekko/extensions:bind-key :component :read-test :map :capture :key "C-g" :command "bound")
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


def read_bytes_integration(binary):
    """Verify framed stdin read metadata without changing legacy packet 2."""
    with tempfile.TemporaryDirectory(prefix="ekko-keymap-read-bytes-") as directory:
        root = Path(directory)
        config = root / "init.lisp"
        config.write_text(READ_BYTES_CONFIG)
        log = root / "input"
        env = dict(os.environ, XDG_RUNTIME_DIR=directory, EKKO_CONFIG=str(config))
        daemon = subprocess.Popen([binary, "--serve", "read", sys.executable,
                                   str(Path(__file__).with_name("daily.py")), "--child", str(log)],
                                  env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)

        def cli(*args):
            p = subprocess.run([binary, *args], env=env, capture_output=True, timeout=8)
            assert p.returncode == 0, (args, p.stderr.decode(errors="replace"))
            return p

        def inspect():
            return json.loads(cli("inspect", "read").stdout)

        def state():
            entries = inspect()["component-state"]
            if not entries:
                return {}
            value = entries[0]["value"]
            return dict(zip(value[::2], value[1::2]))

        try:
            socket = root / "ekko-v2/read.sock"
            eventually(lambda: socket.exists() or daemon.poll() is not None)
            assert daemon.poll() is None, daemon.stderr.read().decode(errors="replace")
            attached = Attachment(socket, version=7)
            # Two unbound DEL keys share one framed read; only the first
            # semantic callback receives the whole raw read.
            attached.send(16, b"\x7f\x7f")
            attached.send(2, b"\x7f\x7f")
            eventually(lambda: state().get("count") == 2)
            assert state()["history"] == [
                [[127], True, [127, 127]], [[127], True, None]]
            # A bound Enter consumes the same read; the trailing normal-mode
            # byte is forwarded unchanged to the child.
            attached.send(16, b"b\rQ")
            attached.send(2, b"b\rQ")
            eventually(lambda: inspect()["mode"] == "normal")
            eventually(lambda: log.read_bytes().endswith(b"Q"))
            assert state()["read-present"] is True
            assert state()["read-bytes"] == [98, 13, 81]
            assert state()["history"][2] == [[98], True, [98, 13, 81]]
            attached.send(2, b"c")
            eventually(lambda: inspect()["mode"] == "capture")
            attached.send(16, b"\x07x")
            attached.send(2, b"\x07x")
            eventually(lambda: state().get("count") == 4)
            assert state()["history"][3] == [[120], True, None]
            # UTF-8 framing spans reads, but the completing callback sees the
            # accumulated scalar bytes rather than only the continuation.
            attached.send(16, b"\xc3")
            attached.send(2, b"\xc3")
            attached.send(16, b"\xa9")
            attached.send(2, b"\xa9")
            eventually(lambda: state().get("count") == 5)
            assert state()["bytes"] == [195, 169]
            assert state()["read-bytes"] == [195, 169]
            assert state()["history"][4] == [[195, 169], True, [195, 169]]
            # Legacy direct packet 2 has no read metadata field.
            attached.close()
            attached = Attachment(socket, version=7)
            cli("config", "reload", "read")
            attached.send(2, b"x")
            eventually(lambda: state().get("count") == 6)
            assert state()["bytes"] == [120] and state()["read-present"] is None
            # A new framed viewer starts a fresh first-read marker.
            attached.close()
            attached = Attachment(socket, version=7)
            attached.send(16, b"z")
            attached.send(2, b"z")
            eventually(lambda: state().get("count") == 7)
            assert state()["read-present"] is True and state()["read-bytes"] == [122]
            attached.close()
            attached = None
            cli("stop", "read")
            daemon.wait(timeout=3)
            assert daemon.returncode == 0
        finally:
            if 'attached' in locals() and attached:
                attached.close()
            if daemon.poll() is None:
                daemon.terminate()
                daemon.wait(timeout=3)
    print(json.dumps({"status": "pass", "suite": "keymap-input-read-bytes"}))


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--read-bytes":
        read_bytes_integration(sys.argv[2])
    else:
        integration(sys.argv[1], len(sys.argv) > 2 and sys.argv[2] == "bare")
