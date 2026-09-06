"""Real-daemon contracts for owned chrome decorations and geometry snapshots."""
import json
import os
from pathlib import Path
import subprocess
import struct
import sys
import tempfile

from daily import Attachment, eventually


def field(mapping, *names):
    for name in names:
        if name in mapping:
            return mapping[name]
    raise AssertionError(f"none of {names!r} found in {mapping!r}")


def chrome_text(snapshot):
    value = field(snapshot, "chrome-status", "chrome_status")
    if isinstance(value, str):
        return value
    return field(value, "text", "status")


def owned_state(snapshot):
    """Return only extension-owned entries, excluding diagnostic notices."""
    contributions = tuple(sorted(
        (item["owner"], item.get("text"))
        for item in (snapshot.get("contributions") or [])))
    marker = object()
    decorations = snapshot.get("decorations", marker)
    if decorations is marker:
        decorations = snapshot.get("decoration-owners", marker)
    assert decorations is not marker, "inspect must expose owned decorations"
    decorations = decorations or []
    # Hook reevaluation can move an owner to the front of the host registry;
    # compare each owner’s actual spans while retaining span order within it.
    decorations = sorted(decorations, key=lambda entry: entry["owner"])
    return contributions, json.dumps(decorations, sort_keys=True, separators=(",", ":"))


def decoration_owners(snapshot):
    decorations = snapshot.get("decorations", snapshot.get("decoration-owners")) or []
    assert isinstance(decorations, list), decorations
    return [entry["owner"] for entry in decorations]


DECORATION_CONFIG = r'''
(in-package :cl-user)
(defun decoration-spans (snapshot &optional event)
  (declare (ignore event))
  (let* ((focus (ekko/extensions:value snapshot :focus))
         (viewport (ekko/extensions:value snapshot :viewport))
         (panes (ekko/extensions:value snapshot :panes))
         (zoom (ekko/extensions:value snapshot :zoom))
         (chrome (ekko/extensions:value snapshot :chrome-status))
         (text (format nil "HOOK-~D" focus)))
    (unless (and (integerp (getf viewport :cols))
                 (integerp (getf viewport :rows))
                 (= (length (getf viewport :insets)) 4)
                 (= (length (getf viewport :gaps)) 2)
                 (member zoom '(t nil))
                 (listp chrome)
                 (= (length panes) 2)
                 (every (lambda (pane)
                         (and (integerp (getf pane :x))
                              (integerp (getf pane :y))
                              (= (length (getf pane :outer-rect)) 4)
                              (member (getf pane :visible) '(t nil))
                              (integerp (getf pane :history-rows)))) panes))
      (error "Malformed extension snapshot geometry"))
    (list
     (ekko/extensions:action :status :text (format nil "chrome-~D" focus))
     (ekko/extensions:action :decorate :spans
       (list (list :x 0 :y 0 :text text :sgr '(0 31))
             ;; This span must be clipped at the private viewport boundary.
             (list :x 499 :y 0 :text "OFFSCREEN" :sgr '(0 32)))))))
(ekko/extensions:register-component :id :decorator
 :reads '(:focus :viewport :panes :zoom :chrome-status) :handler #'decoration-spans)
(ekko/extensions:register-command :component :decorator :name "decorate"
 :handler (lambda (snapshot event) (declare (ignore event)) (decoration-spans snapshot)))
(ekko/extensions:register-command :component :decorator :name "focus-second"
 :handler (lambda (snapshot event) (declare (ignore snapshot event))
            (list (ekko/extensions:action :focus :pane 2))))
(ekko/extensions:register-command :component :decorator :name "focus-first"
 :handler (lambda (snapshot event) (declare (ignore snapshot event))
            (list (ekko/extensions:action :focus :pane 1))))
(ekko/extensions:register-command :component :decorator :name "bad-decoration"
 :handler (lambda (snapshot event) (declare (ignore snapshot event))
            (list (ekko/extensions:action :decorate :spans
                    (list (list :x "bad" :y 0 :text "rejected" :sgr '(0 31)))))))
(ekko/extensions:register-command :component :decorator :name "control-decoration"
 :handler (lambda (snapshot event) (declare (ignore snapshot event))
            (list (ekko/extensions:action :decorate :spans
                    (list (list :x 0 :y 0
                                :text (concatenate 'string "bad" (string (code-char 1)))
                                :sgr '(0 31)))))))
(ekko/extensions:register-command :component :decorator :name "large-decoration"
 :handler (lambda (snapshot event) (declare (ignore snapshot event))
            (list (ekko/extensions:action :decorate :spans
                    (list (list :x 0 :y 0 :text (make-string 16001 :initial-element #\x)
                                :sgr '(0 31)))))))
'''

OVERLAP_CONFIG = r'''
(defun overlap-spans (snapshot &optional event)
  (declare (ignore snapshot event))
  (list (ekko/extensions:action :decorate :spans
          (list (list :x 0 :y 0 :text "OVERLAP" :sgr '(0 33))))))
(ekko/extensions:register-component :id :overlap
 :reads '(:focus) :handler #'overlap-spans)
'''


def child(log):
    """Keep two real PTYs alive and leave enough history for snapshot checks."""
    import tty
    import termios

    tty.setraw(0, termios.TCSANOW)
    path = Path(log)
    path.with_name(path.name + ".session").write_text(os.environ.get("EKKO_SESSION_NAME", ""))
    os.write(1, b"".join(f"old{i}\r\n".encode() for i in range(100)))
    with path.open("ab", buffering=0) as output:
        while True:
            data = os.read(0, 65536)
            output.write(data)
            os.write(1, b"received\r\n")


def integration(binary, profile, bare=False):
    profile = Path(profile).resolve()
    profile_source = "" if bare else profile.read_text()
    source = profile_source + DECORATION_CONFIG + ("" if bare else OVERLAP_CONFIG)
    with tempfile.TemporaryDirectory(prefix="ekko-decorations-") as directory:
        root = Path(directory)
        if not bare:
            for helper in profile.parent.glob("zellij-*.lisp"):
                (root / helper.name).write_bytes(helper.read_bytes())
        config = root / "init.lisp"
        config.write_text(source)
        first_log = root / "first-input"
        second_log = root / "second-input"
        child_argv = [sys.executable, str(Path(__file__).resolve()), "--child"]
        env = dict(os.environ, XDG_RUNTIME_DIR=directory, EKKO_CONFIG=str(config))
        daemon_log = open(root / "daemon.log", "wb")
        daemon = subprocess.Popen(
            [binary, "--serve", "decorations", *child_argv, str(first_log), ":::" ,
             *child_argv, str(second_log)],
            env=env, stdout=daemon_log, stderr=daemon_log)
        attached = None

        def cli(*args, ok=True):
            result = subprocess.run([binary, *args], env=env, capture_output=True, timeout=8)
            assert (result.returncode == 0) == ok, (args, result.returncode, result.stderr.decode())
            if ok and args[0] not in ("status", "inspect", "buffer"):
                assert not result.stdout, (args, result.stdout)
            return result

        def status():
            return json.loads(cli("status", "decorations").stdout)

        def inspect():
            return json.loads(cli("inspect", "decorations").stdout)

        def command(name, *args, ok=True):
            return cli("command", "--session", "decorations", name, *args, ok=ok)

        def settle(predicate, timeout=4):
            def check():
                attached.pump(.01)
                return predicate()
            return eventually(check, timeout=timeout)

        def snapshot_contract():
            state = inspect()
            viewport = field(state, "viewport")
            assert field(viewport, "cols") >= 5
            assert field(viewport, "rows") >= 4
            assert len(field(viewport, "insets")) == 4
            assert len(field(viewport, "gaps")) == 2
            assert isinstance(field(state, "zoom"), bool)
            panes = field(state, "panes")
            assert len(panes) == 2
            for pane in panes:
                assert isinstance(field(pane, "x"), int)
                assert isinstance(field(pane, "y"), int)
                assert len(field(pane, "outer-rect", "outer_rect")) == 4
                assert isinstance(field(pane, "visible"), bool)
                assert isinstance(field(pane, "history-rows", "history_rows"), int)
            assert isinstance(chrome_text(state), str)
            return state

        def latest_scene_has(text):
            attached.pump(.01)
            return bool(attached.scenes and text in attached.scenes[-1])

        def clear_scenes():
            attached.scenes.clear()

        try:
            socket_path = root / "ekko-v2/decorations.sock"
            eventually(lambda: (first_log.exists() and second_log.exists()) or daemon.poll() is not None)
            assert daemon.poll() is None, (root / "daemon.log").read_text()
            attached = Attachment(socket_path)
            eventually(lambda: status()["panes"][0]["history_rows"] > 50)
            original = status()
            pids = [pane["pid"] for pane in original["panes"]]
            assert len(pids) == 2 and len(set(pids)) == 2
            settle(lambda: latest_scene_has("HOOK-1"))
            initial = snapshot_contract()
            initial_owned = owned_state(initial)
            initial_owners = decoration_owners(initial)
            assert chrome_text(initial).startswith("chrome-")
            if not bare:
                assert initial_owners.count("overlap") == 1
                assert latest_scene_has("OVERLAP")
            assert not latest_scene_has("OFFSCREEN")

            # An explicit owner command replaces its owned spans/status while
            # preserving both real PTY identities.
            clear_scenes()
            command("decorate")
            settle(lambda: latest_scene_has("HOOK-1"))
            replaced_owners = decoration_owners(inspect())
            assert set(replaced_owners) == set(initial_owners)
            assert len(replaced_owners) == len(set(replaced_owners))
            assert [pane["pid"] for pane in status()["panes"]] == pids

            # Focus causes the hook to replace the old owner contribution; the
            # old marker must not survive in a later scene.
            clear_scenes()
            command("focus-second")
            settle(lambda: status()["focus"] == 2 and latest_scene_has("HOOK-2"))
            assert "HOOK-1" not in attached.scenes[-1]
            focused = snapshot_contract()
            assert chrome_text(focused) == "chrome-2"
            assert [pane["pid"] for pane in status()["panes"]] == pids

            # The same decoration remains clipped after a tiny resize and the
            # focused content cannot expand beyond the outer viewport.
            attached.send(1, struct.pack(">IIIII", 5, 5, 4, 8, 16))
            settle(lambda: field(inspect(), "viewport")["cols"] == 5
                   and field(inspect(), "viewport")["rows"] == 4)
            tiny = snapshot_contract()
            assert [pane["pid"] for pane in status()["panes"]] == pids
            clear_scenes()
            command("focus-first")
            settle(lambda: status()["focus"] == 1 and latest_scene_has("HOOK-"))
            # The five-column viewport clips the sixth character; no text
            # beyond the reserved chrome row may spill into PTY cells.
            assert "HOOK-1" not in attached.scenes[-1]
            assert "OFFSCREEN" not in attached.scenes[-1]
            attached.send(1, struct.pack(">IIIII", 5, 120, 40, 8, 16))
            settle(lambda: field(inspect(), "viewport")["cols"] == 120)
            assert [pane["pid"] for pane in status()["panes"]] == pids

            # Malformed and oversized spans fail before commit and leave the
            # visible owner contribution untouched.
            stable = inspect()
            stable_owned = owned_state(stable)
            command("bad-decoration", ok=False)
            command("control-decoration", ok=False)
            command("large-decoration", ok=False)
            after_invalid = inspect()
            assert owned_state(after_invalid) == stable_owned, (
                stable_owned, owned_state(after_invalid))
            assert after_invalid["generation"] == stable["generation"]
            assert [pane["pid"] for pane in status()["panes"]] == pids

            # Removing and replacing the owner reconstructs its status/spans;
            # a failed reload keeps the replacement active.
            config.write_text(profile_source)
            cli("config", "reload", "decorations")
            clear_scenes()
            assert attached.scenes == []
            settle(lambda: bool(attached.scenes) and not latest_scene_has("HOOK-1"))
            assert not latest_scene_has("OFFSCREEN")
            removed = inspect()
            assert owned_state(removed) != initial_owned
            assert "overlap" not in decoration_owners(removed)
            assert "OVERLAP" not in attached.scenes[-1]
            assert [pane["pid"] for pane in status()["panes"]] == pids

            replacement = profile_source + DECORATION_CONFIG.replace("HOOK-", "REPLACED-") + ("" if bare else OVERLAP_CONFIG)
            config.write_text(replacement)
            cli("config", "reload", "decorations")
            clear_scenes()
            settle(lambda: latest_scene_has("REPLACED-1"))
            assert "HOOK-1" not in attached.scenes[-1]
            stable = inspect()
            stable_owned = owned_state(stable)
            config.write_text('(error "broken decorations")')
            cli("config", "reload", "decorations", ok=False)
            rolled_back = inspect()
            assert rolled_back["generation"] == stable["generation"]
            assert owned_state(rolled_back) == stable_owned
            assert [pane["pid"] for pane in status()["panes"]] == pids

            cli("stop", "decorations")
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
    print(json.dumps({"status": "pass", "suite": "decorations-bare" if bare else "decorations"}))


if __name__ == "__main__":
    if len(sys.argv) < 3:
        raise SystemExit("usage: decorations.py BINARY ZELLIJ_PROFILE [bare]")
    if sys.argv[1] == "--child":
        child(sys.argv[2])
    else:
        integration(sys.argv[1], sys.argv[2], bare=len(sys.argv) > 3)
