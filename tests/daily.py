"""Daemon/worker contracts over real processes, PTYs and framed IPC."""
import json
import os
from pathlib import Path
import select
import socket
import struct
import subprocess
import sys
import tempfile
import time
import termios
import tty


def child(log):
    # Preserve bytes already queued while the new process was starting.
    tty.setraw(0, termios.TCSANOW)
    Path(log + ".session").write_text(os.environ.get("EKKO_SESSION_NAME", ""))
    os.write(1, b"".join(f"old{i}\r\n".encode() for i in range(100)))
    with open(log, "ab", buffering=0) as out:
        while True:
            data = os.read(0, 65536)
            out.write(data)
            os.write(1, b"received\r\n")


def eventually(fn, timeout=4):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = fn()
        if value:
            return value
        time.sleep(.01)
    raise AssertionError(f"condition did not become true: {fn}")


class Attachment:
    def __init__(self, path):
        self.sock = socket.socket(socket.AF_UNIX)
        self.sock.connect(str(path))
        self.buffer = b""
        self.scenes = []
        self.send(1, struct.pack(">IIIII", 3, 120, 40, 8, 16))
        self.pump(.05)

    def send(self, kind, data=b""):
        self.sock.sendall(struct.pack(">I", len(data) + 1) + bytes([kind]) + data)

    def pump(self, seconds=.05):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if select.select([self.sock], [], [], .01)[0]:
                data = self.sock.recv(65536)
                if not data:
                    return
                self.buffer += data
                while len(self.buffer) >= 4:
                    size = struct.unpack(">I", self.buffer[:4])[0]
                    if len(self.buffer) < size + 4:
                        break
                    body, self.buffer = self.buffer[4:4+size], self.buffer[4+size:]
                    if body[0] == 12:
                        self.scenes.append(body[1:].decode())
                        self.send(14)
                    elif body[0] == 21:
                        raise AssertionError(body[1:])

    def close(self):
        self.sock.close()


CONFIG = '''
(in-package :cl-user)
(defvar *calls* 0)
(ekko/extensions:register-component :id :user :reads '(:focus)
 :handler (lambda (snapshot event) (declare (ignore event))
            (list (ekko/extensions:action :status :text
              (format nil "focus=~D calls=~D" (ekko/extensions:value snapshot :focus) (incf *calls*))))))
(ekko/extensions:set-option :component :user :name :prefix :value "C-a")
(ekko/extensions:set-option :component :user :name :status-text :value "user status")
(ekko/extensions:set-option :component :user :name :status-style :value '(0 32))
(ekko/extensions:bind-key :component :user :key "v" :command "split-rows")
(ekko/extensions:register-command :component :user :name "hang"
 :handler (lambda (s e) (declare (ignore s e)) (loop)))
(ekko/extensions:register-command :component :user :name "bad"
 :handler (lambda (s e) (declare (ignore s e))
            '((:rename :text "must-not-commit") (:focus :pane 999))))
(ekko/extensions:register-command :component :user :name "not-actions"
 :handler (lambda (s e) (declare (ignore s e)) 42))
(ekko/extensions:register-command :component :user :name "undeclared"
 :handler (lambda (s e) (declare (ignore e)) (ekko/extensions:value s :panes)))
(ekko/extensions:register-component :id :snapshot-test :reads '(:panes :layout))
(ekko/extensions:register-command :component :snapshot-test :name "mutate"
 :handler (lambda (s e) (declare (ignore e))
            (setf (getf (first (ekko/extensions:value s :panes)) :label) "MUTATED") nil))
'''


def integration(binary, bare=False):
    with tempfile.TemporaryDirectory(prefix="ekko-daily-") as directory:
        root = Path(directory)
        config = root / "init.lisp"
        config.write_text("" if bare else CONFIG)
        env = dict(os.environ, XDG_RUNTIME_DIR=directory, EKKO_CONFIG=str(config))
        log = root / "input"
        daemon_log = open(root / "daemon.log", "wb")
        daemon = subprocess.Popen([binary, "--serve", "daily", sys.executable, __file__, "--child", str(log)],
                                  env=env, stdout=daemon_log, stderr=daemon_log)
        attached = None

        def cli(*args, ok=True):
            p = subprocess.run([binary, *args], env=env, capture_output=True, timeout=8)
            assert (p.returncode == 0) == ok, (args, p.returncode, p.stderr.decode())
            if ok and args[0] not in ("status", "inspect", "buffer"):
                assert not p.stdout, (args, p.stdout)
            return p

        def status():
            return json.loads(cli("status", "daily").stdout)

        def inspect():
            return json.loads(cli("inspect", "daily").stdout)

        def command(name, *args, ok=True):
            return cli("command", "--session", "daily", name, *args, ok=ok)

        def contribution():
            return next((c["text"] for c in inspect()["contributions"] or [] if c["owner"] == "user"), None)

        try:
            eventually(lambda: (root / "ekko-v2/daily.sock").exists() or daemon.poll() is not None)
            assert daemon.poll() is None, (root / "daemon.log").read_text()
            cli("config", "check")
            eventually(lambda: status()["panes"][0]["history_rows"] > 50)
            original_pid = status()["panes"][0]["pid"]
            assert Path(str(log) + ".session").read_text() == "daily"
            attached = Attachment(root / "ekko-v2/daily.sock")
            if bare:
                assert not inspect()["components"], "bare executable loaded builtins"
                attached.send(2, b"BARE")
                eventually(lambda: log.exists() and b"BARE" in log.read_bytes())
                config.write_text('''
(ekko/extensions:register-component :id :external)
(ekko/extensions:register-command :component :external :name "name"
 :handler (lambda (s e) (declare (ignore s e)) '((:rename :text "bare-extension"))))
''')
                cli("config", "reload", "daily")
                command("name")
                assert status()["panes"][0]["label"] == "bare-extension"
            else:
                before = eventually(contribution)
                command("rename", "renamed")
                command("mutate")
                assert status()["panes"][0]["label"] == "renamed"
                assert contribution() == before, "hook ran for undeclared :panes change"
                for name in ("bad", "not-actions", "undeclared", "missing"):
                    command(name, ok=False)
                    assert status()["panes"][0]["label"] == "renamed"
                # A single input packet contains text, a custom prefix, a split,
                # and trailing text. The latter must reach the newly focused PTY.
                second_log = root / "second-input"
                config.write_text(CONFIG + f'''
(ekko/extensions:set-option :component :user :name :shell
 :value '({json.dumps(sys.executable)} {json.dumps(__file__)} "--child" {json.dumps(str(second_log))}))
''')
                cli("config", "reload", "daily")
                attached.send(2, b"FIRST\x01vSECOND")
                eventually(lambda: len(status()["panes"]) == 2)
                eventually(lambda: second_log.exists() and b"SECOND" in second_log.read_bytes())
                assert Path(str(second_log) + ".session").read_text() == "daily"
                assert b"FIRST" in log.read_bytes() and b"SECOND" not in log.read_bytes()
                assert status()["panes"][1]["y"] > 1
                eventually(lambda: "focus=2" in (contribution() or ""))
                command("focus-1")
                eventually(lambda: "focus=1" in (contribution() or ""))
                # Frozen scrollback, line selection and session-owned paste.
                command("copy-mode")
                attached.send(2, b"g j\r")  # home, mark, down, copy
                eventually(lambda: not status()["panes"][0]["copy_mode"])
                assert cli("buffer", "daily").stdout == b"old0\nold1"
                command("paste-buffer")
                eventually(lambda: b"old0\nold1" in log.read_bytes())
                command("copy-mode")
                command("copy-home")
                attached.send(2, b"/old42")
                attached.send(2, b"\x1b[13u")  # Kitty Enter also submits search
                attached.pump(.1)
                command("copy-selection")
                assert cli("buffer", "daily").stdout == b"old42"
                # Failed reload is atomic and old source can recover a timeout.
                stable = inspect()
                config.write_text('(error "broken init")')
                cli("config", "reload", "daily", ok=False)
                assert inspect()["generation"] == stable["generation"]
                assert inspect()["commands"] == stable["commands"]
                started = time.monotonic()
                result = command("hang", ok=False)
                assert b"timed out" in result.stderr and time.monotonic() - started < 2
                eventually(lambda: inspect()["generation"] > stable["generation"])
                command("rename", "still-alive")
                attached.send(2, b"AFTER-HANG")
                eventually(lambda: b"AFTER-HANG" in log.read_bytes())
                # A control client may disappear before its callback completes.
                generation = inspect()["generation"]
                abandoned = socket.socket(socket.AF_UNIX)
                abandoned.connect(str(root / "ekko-v2/daily.sock"))
                abandoned.sendall(struct.pack(">I", 5) + b"\x07hang")
                abandoned.close()
                eventually(lambda: inspect()["generation"] > generation)
                command("rename", "still-alive")
                # Candidate initialization has its own deadline and does not
                # block the existing worker, input, or PTY reactor.
                config.write_text('(loop)')
                loading = subprocess.Popen([binary, "config", "reload", "daily"], env=env,
                                           stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                time.sleep(.05)
                attached.send(2, b"DURING-RELOAD")
                eventually(lambda: b"DURING-RELOAD" in log.read_bytes(), timeout=1)
                _, error = loading.communicate(timeout=7)
                assert loading.returncode == 2 and b"timed out" in error
                # A runaway change hook is disabled after worker recovery until
                # an explicit reload, instead of creating a restart loop.
                config.write_text(CONFIG + '''
(ekko/extensions:register-component :id :bad-hook :reads '(:focus)
 :handler (lambda (s e) (declare (ignore s e)) (loop)))
''')
                cli("config", "reload", "daily")
                eventually(lambda: "bad-hook" in (inspect()["disabled-hooks"] or []))
                command("rename", "still-alive")
                # If a separately loaded helper changes and recovery fails,
                # ordinary input still works and administrative reload can repair it.
                helper = root / "helper.lisp"
                helper.write_text("")
                config.write_text(CONFIG + f"\n(load {json.dumps(str(helper))})")
                cli("config", "reload", "daily")
                helper.write_text('(error "helper changed")')
                command("hang", ok=False)
                eventually(lambda: status()["extension_pid"] is None)
                attached.send(2, b"AFTER-FAILED-RECOVERY")
                eventually(lambda: b"AFTER-FAILED-RECOVERY" in log.read_bytes())
                # Reload reconstruction removes owned registrations and status,
                # restores shadowed options, and preserves panes/history/buffer.
                config.write_text("")
                cli("config", "reload", "daily")
                clean = inspect()
                assert [c["id"] for c in clean["components"]] == ["defaults"]
                assert clean["options"]["prefix"] == 2 and not clean["contributions"]
                assert not any(c["name"] == "hang" for c in clean["commands"])
                assert status()["panes"][0]["pid"] == original_pid
                assert status()["panes"][0]["label"] == "still-alive"
                assert cli("buffer", "daily").stdout == b"old42"
                cli("split", "--session", "daily", "columns", sys.executable, "-c", "import time; time.sleep(100)")
                assert len(status()["panes"]) == 3
                command("close")
                assert len(status()["panes"]) == 2
                # Client death and replacement preserve copy mode and history.
                command("copy-mode")
                attached.close()
                attached = None
                eventually(lambda: not status()["attached"])
                attached = Attachment(root / "ekko-v2/daily.sock")
                assert status()["panes"][0]["copy_mode"]
                assert status()["panes"][0]["pid"] == original_pid
            cli("stop", "daily")
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
                try:
                    daemon.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    daemon.kill()
                    daemon.wait()
            daemon_log.close()
    print(json.dumps({"status": "pass", "suite": "bare-external-extension" if bare else "daily-customization"}))


if __name__ == "__main__":
    if sys.argv[1] == "--child":
        child(sys.argv[2])
    else:
        integration(sys.argv[1], bare=len(sys.argv) > 2)
