"""Durable namespaced component state: atomic writes, restart reads, failures.

Phases: a fresh namespace is written during startup initialization; a second
daemon reads it back without rewriting; an unwritable store directory is
reported without killing the daemon; a rejected candidate reload writes
nothing; removing the owner keeps the durable value.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from daily import Attachment, eventually


BASE = '''
(ekko/extensions:register-component :id :base)
'''

MARKER = '''
(ekko/extensions:register-component :id :marker :reads '(:store :component-state)
 :initialize
 (lambda (s e)
   (declare (ignore e))
   (let ((entries (cdr (assoc "marker" (ekko/extensions:value s :store) :test #'equal))))
     (if (assoc "seen" entries :test #'equal)
         (list (ekko/extensions:action :status :text
                 (format nil "seen=~A" (cdr (assoc "seen" entries :test #'equal)))))
         (list (ekko/extensions:action :store-set :key "seen" :value "v1")
               (ekko/extensions:action :status :text "fresh"))))))
'''

BLOCKED = '''
(ekko/extensions:register-component :id :marker
 :initialize
 (lambda (s e) (declare (ignore s e))
   (list (ekko/extensions:action :store-set :key "blocked" :value "x"))))
'''

CANDIDATE = '''
(ekko/extensions:register-component :id :marker
 :initialize
 (lambda (s e) (declare (ignore s e))
   (list (ekko/extensions:action :store-set :key "candidate" :value "x"))))
(ekko/extensions:register-component :id :bad
 :initialize (lambda (s e) (declare (ignore s e)) (error "candidate refused")))
'''


def integration(binary):
    with tempfile.TemporaryDirectory(prefix='ekko-store-') as directory:
        root = Path(directory)
        store = root / 'store'
        source = root / 'init.lisp'
        env = dict(os.environ, XDG_RUNTIME_DIR=directory, EKKO_CONFIG=str(source),
                   EKKO_STORE_DIR=str(store), TERM='xterm-256color')
        daemon = None
        attachment = None

        def cli(*args, ok=True):
            result = subprocess.run([binary, *args], env=env, capture_output=True, timeout=8)
            assert (result.returncode == 0) == ok, (args, result.stderr.decode())
            return result

        def inspect():
            return json.loads(cli('inspect', 'store').stdout)

        def contribution(owner):
            return next((c['text'] for c in inspect()['contributions'] or [] if c['owner'] == owner), None)

        def start():
            nonlocal daemon, attachment
            daemon = subprocess.Popen([binary, '--serve', 'store', '--viewport', '120', '40', '8', '16',
                                       sys.executable, '-c', 'import time\nwhile True: time.sleep(3600)\n'],
                                      env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
            sock = root / 'ekko-v2/store.sock'
            eventually(lambda: sock.exists() or daemon.poll() is not None)
            assert daemon.poll() is None, daemon.stderr.read().decode()
            attachment = Attachment(sock, version=13)

        def stop():
            nonlocal daemon, attachment
            if attachment:
                attachment.close()
                attachment = None
            subprocess.run([binary, 'stop', 'store'], env=env, capture_output=True, timeout=8)
            daemon.wait(timeout=8)
            daemon = None

        def marker_file():
            return store / 'marker.lisp'

        try:
            # Fresh namespace: the initializer writes one key during startup.
            source.write_text(BASE + MARKER)
            start()
            assert contribution('marker') == 'fresh', inspect()['contributions']
            eventually(marker_file().exists)
            first_bytes = marker_file().read_bytes()
            assert b'EKKO-STORE 1 "marker"' in first_bytes, first_bytes
            assert b'"seen"' in first_bytes and b'"v1"' in first_bytes, first_bytes
            stop()

            # Restart: the value is read back and not rewritten.
            start()
            assert contribution('marker') == 'seen=v1', inspect()['contributions']
            assert marker_file().read_bytes() == first_bytes, 'restart rewrote the store'
            stop()

            # Unwritable directory: the write fails loudly, the daemon survives.
            source.write_text(BASE + BLOCKED)
            readonly = root / 'readonly'
            readonly.mkdir(exist_ok=True)
            os.chmod(readonly, 0o500)
            env['EKKO_STORE_DIR'] = str(readonly / 'store')
            try:
                start()
                eventually(lambda: 'store write failed' in (inspect()['error'] or ''))
                assert daemon.poll() is None, 'daemon died on a failed store write'
                assert cli('status', 'store').returncode == 0
                assert not (readonly / 'store' / 'blocked.lisp').exists(), 'wrote while unwritable'
            finally:
                os.chmod(readonly, 0o700)
                env['EKKO_STORE_DIR'] = str(store)
            stop()

            # Rejected candidate: staged store actions must not reach the disk.
            source.write_text(BASE + MARKER)
            start()
            before = marker_file().read_bytes()
            source.write_text(BASE + MARKER + CANDIDATE)
            cli('config', 'reload', 'store', ok=False)
            eventually(lambda: 'candidate refused' in (inspect()['error'] or ''))
            assert marker_file().read_bytes() == before, 'rejected candidate wrote the store'
            assert daemon.poll() is None
            stop()

            # Owner removal: durable state outlives its component.
            source.write_text(BASE + MARKER)
            start()
            source.write_text(BASE)
            cli('config', 'reload', 'store')
            assert contribution('marker') is None, inspect()['contributions']
            assert marker_file().read_bytes() == before, 'owner removal erased the store'
            stop()
            print(json.dumps({'status': 'pass', 'binary': Path(binary).name,
                              'phases': ['fresh', 'restart', 'unwritable', 'rejected', 'owner-removal']}))
        finally:
            if attachment:
                attachment.close()
            if daemon and daemon.poll() is None:
                subprocess.run([binary, 'stop', 'store'], env=env, capture_output=True, timeout=8)
                try:
                    daemon.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    daemon.kill()


if __name__ == '__main__':
    integration(sys.argv[1])
