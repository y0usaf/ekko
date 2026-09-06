"""Generic keymap API contracts over the real daemon and extension worker."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

from daily import Attachment, eventually


def integration(binary, profile, bare=False):
    profile = Path(profile).resolve()
    profile_source = profile.read_text()
    copy_mode_command = '''
(ekko/extensions:register-component :id :keymap-test)
(ekko/extensions:register-command :component :keymap-test :name "copy-mode"
 :handler (lambda (s e) (declare (ignore s e))
            (list (ekko/extensions:action :copy-mode))))
'''
    unicode_commands = '''
(ekko/extensions:register-component :id :unicode-test)
(dolist (spec '(("unicode-233" "matched-233") ("unicode-195" "matched-195") ("unicode-reset" "reset")))
  (destructuring-bind (name label) spec
    (ekko/extensions:register-command :component :unicode-test :name name
      :handler (lambda (s e) (declare (ignore s e))
                 (list (ekko/extensions:action :rename :text label))))))
(ekko/extensions:bind-key :component :unicode-test :map :normal :key 233 :command "unicode-233")
(ekko/extensions:bind-key :component :unicode-test :map :normal :key 195 :command "unicode-195")
'''
    with tempfile.TemporaryDirectory(prefix="ekko-keymaps-") as directory:
        root = Path(directory)
        for helper in profile.parent.glob("zellij-*.lisp"):
            (root / helper.name).write_bytes(helper.read_bytes())
        config = root / "init.lisp"
        config.write_text(profile_source + copy_mode_command + unicode_commands)
        env = dict(os.environ, XDG_RUNTIME_DIR=directory, EKKO_CONFIG=str(config))
        log = root / "input"
        daemon_log = open(root / "daemon.log", "wb")
        daemon = subprocess.Popen(
            [binary, "--serve", "keymaps", sys.executable,
             str(Path(__file__).with_name("daily.py")), "--child", str(log)],
            env=env, stdout=daemon_log, stderr=daemon_log)
        attached = None

        def cli(*args, ok=True):
            result = subprocess.run([binary, *args], env=env, capture_output=True, timeout=8)
            assert (result.returncode == 0) == ok, (args, result.returncode, result.stderr.decode())
            if ok and args[0] not in ("status", "inspect", "buffer"):
                assert not result.stdout, (args, result.stdout)
            return result

        def status():
            return json.loads(cli("status", "keymaps").stdout)

        def inspect():
            return json.loads(cli("inspect", "keymaps").stdout)

        def command(name, *args, ok=True):
            return cli("command", "--session", "keymaps", name, *args, ok=ok)

        try:
            socket_path = root / "ekko-v2/keymaps.sock"
            eventually(lambda: socket_path.exists() or daemon.poll() is not None)
            assert daemon.poll() is None, (root / "daemon.log").read_text()
            cli("config", "check")
            eventually(lambda: status()["panes"][0]["history_rows"] > 50)
            original_pid = status()["panes"][0]["pid"]
            attached = Attachment(socket_path)

            state = inspect()
            assert state["mode"] == "normal"
            assert {(m["name"], m["unbound"], m["owner"])
                    for m in state["keymaps"]} == {
                        ("normal", "forward", "zellij-modes"),
                        ("locked", "forward", "zellij-modes"),
                        ("pane", "ignore", "zellij-modes")}
            assert {c["name"] for c in state["commands"]} >= {"lock", "unlock"}

            def label():
                return status()["panes"][0]["label"]

            def reset_label():
                command("unicode-reset")
                eventually(lambda: label() == "reset")

            def forwarded(data):
                before = log.read_bytes() if log.exists() else b""
                attached.send(2, data)
                eventually(lambda: log.read_bytes() == before + data)

            # A registered Unicode code point matches its complete UTF-8 key,
            # including when the client splits the character across packets.
            reset_label()
            attached.send(2, b"\xc3\xa9")
            eventually(lambda: label() == "matched-233")
            reset_label()
            attached.send(2, b"\xc3")
            time.sleep(.1)
            assert label() == "reset"
            attached.send(2, b"\xa9")
            eventually(lambda: label() == "matched-233")

            # A valid but unbound character forwards its original bytes, while
            # malformed UTF-8 never falls through to a lead-byte binding.
            reset_label()
            forwarded(b"\xc3\xa6")
            reset_label()
            forwarded(b"\xc3x")
            assert label() == "reset"
            reset_label()
            forwarded(b"\xe2x")
            forwarded(b"\xe2\x02")
            reset_label()
            before = log.read_bytes()
            attached.send(2, b"\xe2")
            time.sleep(.1)
            assert log.read_bytes() == before
            attached.send(2, b"x")
            eventually(lambda: log.read_bytes() == before + b"\xe2x")
            assert label() == "reset"

            command("lock")
            assert inspect()["mode"] == "locked"
            command("copy-mode")
            # A custom map is active even in copy mode; its unbound :forward
            # policy sends Ctrl-b to the pane instead of entering prefix mode.
            attached.send(2, b"\x02")
            eventually(lambda: b"\x02" in log.read_bytes())

            # Reloading the same registry preserves the selected map and PTY.
            cli("config", "reload", "keymaps")
            assert inspect()["mode"] == "locked"
            assert status()["panes"][0]["pid"] == original_pid

            # An invalid custom-map reference is rejected atomically.
            stable = inspect()
            invalid = (profile_source + copy_mode_command + unicode_commands
                       + '\n(ekko/extensions:bind-key :component :zellij-modes :map :missing :key "x")\n')
            config.write_text(invalid)
            cli("config", "reload", "keymaps", ok=False)
            rolled_back = inspect()
            assert rolled_back["mode"] == stable["mode"] == "locked"
            assert rolled_back["keymaps"] == stable["keymaps"]
            assert status()["panes"][0]["pid"] == original_pid

            ordinary = '''
(ekko/extensions:register-component :id :ordinary)
(ekko/extensions:set-option :component :ordinary :name :prefix :value "C-b")
'''
            config.write_text(ordinary)
            cli("config", "reload", "keymaps")
            restored = inspect()
            assert restored["mode"] is None
            assert restored["keymaps"] is None
            assert restored["options"]["prefix"] == 2
            if not bare:
                assert any(c["id"] == "defaults" for c in restored["components"])
            assert status()["panes"][0]["pid"] == original_pid

            cli("stop", "keymaps")
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
    print(json.dumps({"status": "pass", "suite": "keymaps-bare" if bare else "keymaps"}))


if __name__ == "__main__":
    if len(sys.argv) < 3:
        raise SystemExit("usage: keymaps.py BINARY ZELLIJ_PROFILE [bare]")
    integration(sys.argv[1], sys.argv[2], bare=len(sys.argv) > 3)
