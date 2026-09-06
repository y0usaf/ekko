"""Real-daemon contract for the generic temporary pane-note API.

The test component deliberately renders notes through its ordinary decoration
hook.  This keeps the command API independent from any particular profile and
checks that note state, expiry, ownership and geometry are observable through
the public snapshot.
"""
import json
import os
from pathlib import Path
import subprocess
import struct
import sys
import tempfile
import time

from daily import Attachment, eventually


NOTE_CONFIG = r'''
(in-package :cl-user)
(defun note-pane (snapshot id)
  (find id (ekko/extensions:value snapshot :panes)
        :key (lambda (pane) (getf pane :id))))
(defun note-decoration-hook (snapshot event)
  (declare (ignore event))
  (list (ekko/extensions:action :decorate :spans
          (loop for note in (ekko/extensions:value snapshot :pane-notes)
                for pane = (note-pane snapshot (getf note :pane))
                for rect = (and pane (getf pane :outer-rect))
                when pane
                  collect (list :x (+ (first rect) 2) :y (second rect)
                                :text (getf note :text) :sgr (getf note :sgr))))))
(ekko/extensions:register-component
 :id :pane-note-test :reads '(:pane-notes :panes)
 :handler #'note-decoration-hook)
(ekko/extensions:register-command :component :pane-note-test :name "note-one"
 :handler (lambda (snapshot event) (declare (ignore snapshot event))
            (list (ekko/extensions:action :pane-note :pane 1
                    :text "NOTE-ONE" :sgr '(0 31) :duration 10000))))
(ekko/extensions:register-command :component :pane-note-test :name "note-two"
 :handler (lambda (snapshot event) (declare (ignore snapshot event))
            (list (ekko/extensions:action :pane-note :pane 2
                    :text "NOTE-TWO" :sgr '(0 32) :duration 10000))))
(ekko/extensions:register-command :component :pane-note-test :name "note-short"
 :handler (lambda (snapshot event) (declare (ignore snapshot event))
            (list (ekko/extensions:action :pane-note :pane 1
                    :text "NOTE-SHORT" :sgr '(0 33) :duration 1000))))
(ekko/extensions:register-command :component :pane-note-test :name "note-changed"
 :handler (lambda (snapshot event) (declare (ignore snapshot event))
            (list (ekko/extensions:action :pane-note :pane 1
                    :text "NOTE-CHANGED" :sgr '(0 34) :duration 10000))))
(ekko/extensions:register-command :component :pane-note-test :name "bad-note"
 :handler (lambda (snapshot event) (declare (ignore snapshot event))
            (list (ekko/extensions:action :pane-note :pane 1
                    :text "SHOULD-NOT-COMMIT" :sgr '(0 35) :duration 1000)
                  (ekko/extensions:action :pane-note :pane 1
                    :text "INVALID" :sgr '(0 35) :duration 0))))
(ekko/extensions:register-command :component :pane-note-test :name "close-two"
 :handler (lambda (snapshot event) (declare (ignore snapshot event))
            (list (ekko/extensions:action :close :focus 1))))
(ekko/extensions:register-command :component :pane-note-test :name "focus-two"
 :handler (lambda (snapshot event) (declare (ignore snapshot event))
            (list (ekko/extensions:action :focus :pane 2))))
'''


def child(log):
    # Reuse the ordinary persistent PTY fixture so note rendering is checked
    # over real application output and after a resize.
    from daily import child as daily_child
    daily_child(log)


def integration(binary, profile, bare=False):
    profile = Path(profile).resolve()
    profile_source = "" if bare else profile.read_text()
    source = profile_source + NOTE_CONFIG
    with tempfile.TemporaryDirectory(prefix="ekko-pane-notes-") as directory:
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
            [binary, "--serve", "pane-notes", *child_argv, str(first_log), ":::",
             *child_argv, str(second_log)],
            env=env, stdout=daemon_log, stderr=daemon_log)
        attached = None

        def cli(*args, ok=True):
            result = subprocess.run([binary, *args], env=env, capture_output=True,
                                     timeout=8)
            assert (result.returncode == 0) == ok, (
                args, result.returncode, result.stderr.decode(errors="replace"))
            if ok and args[0] not in ("status", "inspect", "buffer"):
                assert not result.stdout, (args, result.stdout)
            return result

        def status():
            return json.loads(cli("status", "pane-notes").stdout)

        def inspect():
            return json.loads(cli("inspect", "pane-notes").stdout)

        def command(name, *args, ok=True):
            return cli("command", "--session", "pane-notes", name, *args, ok=ok)

        def notes():
            return inspect().get("pane-notes") or []

        def owner_decorations():
            entries = inspect().get("decorations") or []
            return next((e for e in entries if e.get("owner") == "pane-note-test"), None)

        def scene_has(text):
            attached.pump(.01)
            return any(text in scene for scene in attached.scenes)

        def latest_scene_has(text):
            attached.pump(.01)
            return bool(attached.scenes and text in attached.scenes[-1])

        def decoration_has(text):
            decoration = owner_decorations()
            return bool(decoration and any(span.get("text") == text
                                           for span in decoration.get("spans", [])))

        try:
            socket_path = root / "ekko-v2/pane-notes.sock"
            eventually(lambda: socket_path.exists() or daemon.poll() is not None)
            assert daemon.poll() is None, (root / "daemon.log").read_text()
            cli("config", "check")
            eventually(lambda: status()["panes"][0]["history_rows"] > 50)
            pids = [pane["pid"] for pane in status()["panes"]]
            assert len(pids) == 2 and len(set(pids)) == 2
            attached = Attachment(socket_path)
            assert notes() == []

            # A note is state in the public snapshot and is displayed through
            # the ordinary decoration contribution.
            command("note-one")
            eventually(lambda: len(notes()) == 1 and notes()[0]["text"] == "NOTE-ONE")
            eventually(lambda: scene_has("NOTE-ONE"))
            first_note = notes()[0]
            assert first_note == {
                "owner": "pane-note-test", "pane": 1, "text": "NOTE-ONE",
                "sgr": [0, 31]}
            eventually(lambda: decoration_has("NOTE-ONE"))

            # Changed text/style replaces the same owner's note for that pane.
            command("note-changed")
            eventually(lambda: len(notes()) == 1 and notes()[0]["text"] == "NOTE-CHANGED")
            eventually(lambda: decoration_has("NOTE-CHANGED"))
            assert notes()[0]["sgr"] == [0, 34]

            # Repeating an identical active note does not extend its deadline.
            command("note-short")
            eventually(lambda: len(notes()) == 1 and notes()[0]["text"] == "NOTE-SHORT")
            started = time.monotonic()
            time.sleep(.55)
            command("note-short")
            eventually(lambda: len(notes()) == 1 and notes()[0]["text"] == "NOTE-SHORT")
            eventually(lambda: notes() == [] and not latest_scene_has("NOTE-SHORT"), timeout=1.4)
            assert time.monotonic() - started < 1.3, "duplicate note extended deadline"

            # Expiry is daemon-owned: it must complete while no viewer is
            # attached, and a new viewer must receive the restored scene
            # without the expired decoration.
            command("note-short")
            eventually(lambda: notes() and notes()[0]["text"] == "NOTE-SHORT")
            attached.close()
            attached = None
            eventually(lambda: not status()["attached"])
            eventually(lambda: notes() == [], timeout=1.4)
            attached = Attachment(socket_path)
            eventually(lambda: bool(attached.scenes) and not latest_scene_has("NOTE-SHORT"))

            # A second pane note follows the target pane's current content
            # origin after a client resize.
            command("note-two")
            eventually(lambda: len(notes()) == 1 and notes()[0]["pane"] == 2)
            before_resize = inspect()
            pane2_before = next(p for p in before_resize["panes"] if p["id"] == 2)
            rect_before = pane2_before["outer_rect"]
            eventually(lambda: decoration_has("NOTE-TWO"))
            span_before = next(s for s in owner_decorations()["spans"]
                               if s["text"] == "NOTE-TWO")
            assert (span_before["x"], span_before["y"]) == (rect_before[0] + 2, rect_before[1])
            # Use the same minimum valid attachment size as the other real
            # daemon contracts; this exercises clipping as well as movement.
            attached.send(1, struct.pack(">IIIII", 5, 5, 4, 8, 16))
            eventually(lambda: inspect()["viewport"]["cols"] == 5
                       and inspect()["viewport"]["rows"] == 4)
            after_resize = inspect()
            pane2_after = next(p for p in after_resize["panes"] if p["id"] == 2)
            rect_after = pane2_after["outer_rect"]
            eventually(lambda: decoration_has("NOTE-TWO"))
            span_after = next(s for s in owner_decorations()["spans"]
                              if s["text"] == "NOTE-TWO")
            assert (span_after["x"], span_after["y"]) == (rect_after[0] + 2, rect_after[1])
            assert [pane["pid"] for pane in status()["panes"]] == pids

            # All actions in an invalid batch are rejected before the valid
            # note can commit.
            command("note-one")
            eventually(lambda: notes() and notes()[0]["pane"] == 1)
            stable_notes = notes()
            stable_generation = inspect()["generation"]
            command("bad-note", ok=False)
            assert notes() == stable_notes
            assert inspect()["generation"] == stable_generation

            # Closing a pane prunes notes targeting the removed pane.
            command("note-two")
            eventually(lambda: any(note["pane"] == 2 for note in notes()))
            command("focus-two")
            eventually(lambda: status()["focus"] == 2)
            command("close-two")
            eventually(lambda: len(status()["panes"]) == 1)
            eventually(lambda: notes() and all(note["pane"] == 1 for note in notes()))
            assert len(status()["panes"]) == 1

            # Reload resets the note registry while retaining the running PTY.
            command("note-one")
            eventually(lambda: notes())
            generation = inspect()["generation"]
            cli("config", "reload", "pane-notes")
            eventually(lambda: inspect()["generation"] > generation)
            assert notes() == []
            assert status()["panes"][0]["pid"] == pids[0]

            cli("stop", "pane-notes")
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
    print(json.dumps({"status": "pass", "suite": "pane-notes-bare" if bare else "pane-notes"}))


if __name__ == "__main__":
    if len(sys.argv) < 3:
        raise SystemExit("usage: pane_notes.py BINARY PROFILE [bare]")
    if sys.argv[1] == "--child":
        child(sys.argv[2])
    else:
        integration(sys.argv[1], sys.argv[2], bare=len(sys.argv) > 3)
