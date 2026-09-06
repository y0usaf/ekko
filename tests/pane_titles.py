"""Real-worker title policy, rendering, and profile replacement contracts."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from daily import Attachment, eventually

COMMANDS = r'''
(ekko/extensions:register-component :id :title-probe)
(ekko/extensions:register-command :component :title-probe :name "close-focused"
 :handler (lambda (s e) (declare (ignore s e))
            (list (ekko/extensions:action :close :focus 1))))
(ekko/extensions:register-command :component :title-probe :name "split-shell"
 :handler (lambda (s e) (declare (ignore s e))
            (list (ekko/extensions:action :split :axis :rows))))
(ekko/extensions:register-command :component :title-probe :name "rename-first"
 :handler (lambda (s e) (declare (ignore s))
            (list (ekko/extensions:action :rename :pane 1
                    :text (first (getf e :arguments))))))
(ekko/extensions:register-command :component :title-probe :name "note"
 :handler (lambda (s e) (declare (ignore s e))
            (list (ekko/extensions:action :pane-note :pane 1 :text "NOTE-TITLE"
                    :sgr '(0 31) :duration 10000))))
'''


def child(directory):
    import termios
    import tty
    tty.setraw(0, termios.TCSANOW)
    Path(directory, f"ready-{os.getpid()}").touch()
    os.write(1, b"TITLE-WORKER-READY\r\n")
    pending = b""
    while True:
        data = os.read(0, 4096)
        if not data:
            return
        pending += data
        while b"\n" in pending:
            command, pending = pending.split(b"\n", 1)
            if command.startswith((b"OSC0:", b"OSC2:")):
                os.write(1, b"\x1b]" + command[3:4] + b";" + command[5:] + b"\x07")


def integration(binary, profile, bare=False):
    profile = Path(profile).resolve()
    with tempfile.TemporaryDirectory(prefix="ekko-titles-") as directory:
        root = Path(directory)
        config = root / "init.lisp"
        for helper in profile.parent.glob("zellij-*.lisp"):
            (root / helper.name).write_bytes(helper.read_bytes())
        shell = root / "app"
        argv = [sys.executable, str(Path(__file__).resolve()), "--child", directory]
        shell.write_text(f"#!{sys.executable}\nimport os\nos.execv({sys.executable!r}, {argv!r})\n")
        shell.chmod(0o700)
        probe = COMMANDS + (
            '\n(ekko/extensions:set-option :component :title-probe :name :shell :value '
            f"'({json.dumps(str(shell))}))\n")
        source = profile.read_text() + probe
        config.write_text(source)
        env = dict(os.environ, XDG_RUNTIME_DIR=directory, EKKO_CONFIG=str(config))
        attached = None
        with open(root / "daemon.log", "wb") as daemon_log:
            daemon = subprocess.Popen([binary, "--serve", "titles", str(shell)],
                                      env=env, stdout=daemon_log, stderr=daemon_log)

            def cli(*args):
                result = subprocess.run([binary, *args], env=env, capture_output=True, timeout=8)
                assert result.returncode == 0, (args, result.stderr.decode(errors="replace"))
                return result

            def inspect():
                return json.loads(cli("inspect", "titles").stdout)

            def command(name, *args):
                cli("command", "--session", "titles", name, *args)

            def frame_has(text):
                state = inspect()
                owners = state.get("decorations") or []
                spans = next((d["spans"] for d in owners
                              if d["owner"] == "zellij-decoration"), [])
                attached.pump(.02)
                return (any(text in s["text"] for s in spans)
                        and bool(attached.scenes and text in attached.scenes[-1]))

            def metadata():
                return [{k: p[k] for k in ("id", "pid", "argv", "launch_kind",
                                           "creation_position", "name", "terminal_title")}
                        for p in inspect()["panes"]]

            def reload(text):
                generation = inspect()["generation"]
                config.write_text(text)
                cli("config", "reload", "titles")
                eventually(lambda: inspect()["generation"] > generation)

            try:
                socket = root / "ekko-v2/titles.sock"
                eventually(lambda: socket.exists() or daemon.poll() is not None)
                assert daemon.poll() is None, (root / "daemon.log").read_text()
                attached = Attachment(socket)
                initial = metadata()[0]
                assert initial["argv"] == [str(shell)]
                assert initial["launch_kind"] == "command" and initial["creation_position"] == 1
                assert initial["name"] is None and initial["terminal_title"] is None
                eventually(lambda: (root / f"ready-{initial['pid']}").exists())
                eventually(lambda: frame_has(str(shell)))
                attached.send(2, b"OSC0:  title with spaces  \n")
                eventually(lambda: metadata()[0]["terminal_title"] == "  title with spaces  ")
                eventually(lambda: frame_has("title with spaces"))
                attached.send(2, b"OSC2:\n")
                eventually(lambda: metadata()[0]["terminal_title"] == "")
                eventually(lambda: not frame_has("title with spaces") and not frame_has(str(shell)))
                attached.send(2, b"OSC2:" + b"x" * 240 + b"\n")
                eventually(lambda: metadata()[0]["terminal_title"] == "x" * 240)
                eventually(lambda: frame_has("x" * 20))
                command("rename-first", "RENAMED")
                eventually(lambda: frame_has("RENAMED"))
                for unicode_name in ("界面", "Cafe\u0301", "界面-Cafe\u0301-" + "mixed" * 20):
                    command("rename-first", unicode_name)
                    eventually(lambda name=unicode_name: metadata()[0]["name"] == name)
                    # Long titles are clipped by the published frame span;
                    # verify the visible prefix while metadata retains all data.
                    visible = unicode_name if len(unicode_name) < 20 else unicode_name[:12]
                    eventually(lambda name=visible: frame_has(name))
                command("rename-first", "RENAMED")
                eventually(lambda: frame_has("RENAMED"))
                attached.send(2, b"OSC2:changed under rename\n")
                eventually(lambda: metadata()[0]["terminal_title"] == "changed under rename")
                assert frame_has("RENAMED")
                command("rename-first", "")
                eventually(lambda: frame_has("changed under rename"))
                for osc_title in ("界面", "Cafe\u0301"):
                    attached.send(2, ("OSC2:" + osc_title + "\n").encode("utf-8"))
                    eventually(lambda title=osc_title: metadata()[0]["terminal_title"] == title)
                    eventually(lambda title=osc_title: frame_has(title))
                command("split-shell")
                eventually(lambda: len(metadata()) == 2)
                second = metadata()[1]
                assert second["launch_kind"] == "shell" and second["creation_position"] == 2
                eventually(lambda: frame_has("Pane #2"))
                command("close-focused")
                eventually(lambda: len(metadata()) == 1)
                command("split-shell")
                eventually(lambda: len(metadata()) == 2)
                reopened = metadata()[1]
                assert reopened["id"] != second["id"] and reopened["pid"] != second["pid"]
                assert reopened["creation_position"] == 2
                eventually(lambda: frame_has("Pane #2"))
                before = metadata()
                reload('(ekko/extensions:unregister-component :defaults)\n' + probe)
                assert metadata() == before
                assert not any(d["owner"] == "zellij-decoration"
                               for d in inspect().get("decorations") or [])
                reload(source)
                assert metadata() == before
                eventually(lambda: frame_has("Pane #2"))
                command("note")
                eventually(lambda: frame_has("NOTE-TITLE"))
                cli("stop", "titles")
                daemon.wait(timeout=3)
                assert daemon.returncode == 0, (root / "daemon.log").read_text()
            finally:
                if attached:
                    attached.close()
                if daemon.poll() is None:
                    daemon.terminate()
                    try:
                        daemon.wait(timeout=3)
                    except subprocess.TimeoutExpired:
                        daemon.kill()
                        daemon.wait(timeout=3)
    print(json.dumps({"status": "pass", "suite": "pane-titles-bare" if bare else "pane-titles"}))


if __name__ == "__main__":
    if sys.argv[1:2] == ["--child"]:
        child(sys.argv[2])
    else:
        integration(sys.argv[1], sys.argv[2], bare=len(sys.argv) > 3)
