"""Replaceable exit text over a real viewer, with reload and owner reversal."""
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import subprocess
import struct
import sys
import tempfile
import termios
import time

from daily import eventually


def config(text, marker):
    exit_owner = '' if text is None else f'''
(ekko/extensions:register-component :id :exit-text)
(ekko/extensions:set-option :component :exit-text :name :viewer-exit-text :value {json.dumps(text)})
'''
    if text is not None:
        value = '(coerce (list ' + ' '.join(f'(code-char {ord(c)})' for c in text) + ") 'string)"
        exit_owner = exit_owner.replace(json.dumps(text), value)
    return exit_owner + f'''
(ekko/extensions:register-component :id :controller :reads nil
 :handler (lambda (s e) (declare (ignore s e))
  (list (ekko/extensions:action :decorate
   :spans '((:x 0 :y 0 :text "{marker}" :sgr (0 39 49)))))))
(ekko/extensions:register-command :component :controller :name "leave"
 :handler (lambda (s e) (declare (ignore s e)) (list (ekko/extensions:action :detach))))
'''


def integration(binary):
    with tempfile.TemporaryDirectory(prefix='ekko-viewer-exit-') as directory:
        root = Path(directory)
        source = root / 'init.lisp'
        source.write_text(config('FIRST', 'READY-FIRST'))
        env = dict(os.environ, XDG_RUNTIME_DIR=directory, EKKO_CONFIG=str(source))
        daemon = subprocess.Popen([binary, '--serve', 'exit-text', '--viewport', '80', '24', '8', '16',
                                   sys.executable, '-c', 'import time; time.sleep(120)'], env=env,
                                  stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        viewer = None
        master = None
        raw = bytearray()
        original_flags = None

        def cli(*args, ok=True):
            result = subprocess.run([binary, *args], env=env, capture_output=True, timeout=8)
            assert (result.returncode == 0) == ok, (args, result.stderr)
            return result

        def inspect():
            return json.loads(cli('inspect', 'exit-text').stdout)

        def attach():
            nonlocal master, viewer, raw, original_flags
            master, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 80, 640, 384))
            original_flags = termios.tcgetattr(slave)
            viewer = subprocess.Popen([binary, 'attach', 'exit-text'], env=env,
                                      stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
            os.close(slave)
            raw = bytearray()

        def pump():
            if select.select([master], [], [], .02)[0]:
                try:
                    raw.extend(os.read(master, 65536))
                except OSError:
                    pass

        def seen(marker):
            pump()
            return marker in raw

        def leave(text):
            cli('command', '--session', 'exit-text', 'leave')
            eventually(lambda: (pump(), viewer.poll() is not None)[1])
            pump()
            assert viewer.returncode == 0
            assert termios.tcgetattr(master) == original_flags
            ending = bytes(raw).split(b'\x1b[?1049l')[-1]
            assert ending == text, ending
            assert inspect()['panes'][0]['pid'] == pid
            os.close(master)

        try:
            eventually(lambda: (root / 'ekko-v2/exit-text.sock').exists() or daemon.poll() is not None)
            assert daemon.poll() is None, daemon.stderr.read().decode()
            pid = inspect()['panes'][0]['pid']
            attach()
            eventually(lambda: seen(b'READY-FIRST'))
            before = inspect()['generation']
            # Actual ESC, CR, LF and C1 control characters must fail validation.
            for character in ('\x1b', '\r', '\n', '\x85'):
                source.write_text(config('BAD' + character, 'INVALID'))
                cli('config', 'reload', 'exit-text', ok=False)
                assert inspect()['generation'] == before
            source.write_text(config('SECOND', 'READY-SECOND'))
            cli('config', 'reload', 'exit-text')
            eventually(lambda: seen(b'READY-SECOND'))
            leave(b'SECOND\r\n')
            master = None
            attach()
            eventually(lambda: seen(b'READY-SECOND'))
            source.write_text(config(None, 'READY-REMOVED'))
            cli('config', 'reload', 'exit-text')
            eventually(lambda: seen(b'READY-REMOVED'))
            leave(b'')
            master = None
            cli('stop', 'exit-text')
            daemon.wait(timeout=3)
            assert daemon.returncode == 0
        finally:
            if viewer and viewer.poll() is None:
                viewer.terminate()
                viewer.wait(timeout=3)
            if master is not None:
                os.close(master)
            if daemon.poll() is None:
                daemon.terminate()
                daemon.wait(timeout=3)
    print(json.dumps({'suite': 'viewer-exit', 'binary': binary, 'status': 'pass'}))


if __name__ == '__main__':
    integration(sys.argv[1])
