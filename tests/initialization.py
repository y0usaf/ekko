"""Transactional initialization before child launch and configuration commit."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

from daily import Attachment, eventually


BASE = '''
(ekko/extensions:register-component :id :routing)
(ekko/extensions:register-keymap :component :routing :name :normal)
(ekko/extensions:register-keymap :component :routing :name :overlay :unbound :ignore)
(ekko/extensions:set-option :component :routing :name :initial-keymap :value :normal)
(ekko/extensions:register-command :component :routing :name "dismiss"
 :handler (lambda (s e) (declare (ignore s e))
   (list (ekko/extensions:action :set-keymap :name :normal))))
(ekko/extensions:bind-key :component :routing :map :overlay :key "Escape" :command "dismiss")
'''
INIT = '''
(ekko/extensions:register-component :id :welcome
 :reads '(:component-state :viewport :panes)
 :initialize
 (lambda (s e)
   (let* ((previous (cdr (assoc "welcome" (ekko/extensions:value s :component-state) :test #'equal)))
          (loads (1+ (getf previous :loads 0)))
          (viewport (ekko/extensions:value s :viewport)))
    (list (ekko/extensions:action :set-state :value
            (list :loads loads :cols (getf viewport :cols) :reason (getf e :reason)
                  :pid (getf (first (ekko/extensions:value s :panes)) :pid)))
          (ekko/extensions:action :set-keymap :name :overlay)
          (ekko/extensions:action :decorate :spans
            (list (list :x 1 :y 1 :text "WELCOME" :sgr '(0 37 40) :overlay t)))))))
'''


def integration(binary):
    with tempfile.TemporaryDirectory(prefix='ekko-initialize-') as directory:
        root = Path(directory)
        source = root / 'init.lisp'
        source.write_text(BASE + INIT)
        log = root / 'input'
        child = ('import os,tty,termios; from pathlib import Path; '
                 'tty.setraw(0,termios.TCSANOW); '
                 f'p=Path({str(log)!r}); p.write_bytes(b""); '
                 'f=p.open("ab",buffering=0); '
                 '\nwhile True: f.write(os.read(0,4096))')
        env = dict(os.environ, XDG_RUNTIME_DIR=directory, EKKO_CONFIG=str(source))
        daemon = subprocess.Popen([binary, '--serve', 'initialize', '--viewport', '120', '40', '8', '16',
                                   sys.executable, '-c', child], env=env,
                                  stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        attachment = None

        def cli(*args, ok=True):
            result = subprocess.run([binary, *args], env=env, capture_output=True, timeout=8)
            assert (result.returncode == 0) == ok, (args, result.stderr)
            return result

        def inspect():
            return json.loads(cli('inspect', 'initialize').stdout)

        def state():
            value = next(x['value'] for x in inspect()['component-state'] if x['owner'] == 'welcome')
            return dict(zip(value[::2], value[1::2]))

        def reload(text, ok=True):
            source.write_text(text)
            return cli('config', 'reload', 'initialize', ok=ok)

        try:
            eventually(lambda: log.exists() or daemon.poll() is not None)
            assert daemon.poll() is None, daemon.stderr.read().decode()
            first = inspect()
            pid = first['panes'][0]['pid']
            assert first['mode'] == 'overlay', first
            assert state()['loads'] == 1 and state()['reason'] == 'startup' and state()['pid'] is None, state()
            attachment = Attachment(root / 'ekko-v2/initialize.sock', version=11)
            attachment.send(2, b'x')
            attachment.pump(.1)
            assert log.read_bytes() == b''
            attachment.send(2, b'\x1b')
            eventually(lambda: inspect()['mode'] == 'normal')
            attachment.send(2, b'a')
            eventually(lambda: log.read_bytes() == b'a')
            generation = inspect()['generation']
            before_state = state()
            failures = [
                '(ekko/extensions:action :split :axis :columns)',
                '(ekko/extensions:action :set-keymap :name :missing)',
                '(ekko/extensions:action :set-state :value (make-string 17000 :initial-element #\\x))',
                '(ekko/extensions:action :decorate :spans \'((:x 0 :y 0 :text "bad" :overlay :invalid)))',
            ]
            for action in failures:
                result = reload(BASE + INIT + '''
(ekko/extensions:register-component :id :later :initialize
 (lambda (s e) (declare (ignore s e)) (list ''' + action + ')))', ok=False)
                assert inspect()['generation'] == generation
                assert inspect()['mode'] == 'normal'
                assert state() == before_state
                assert inspect()['panes'][0]['pid'] == pid
                assert result.stderr
            for body in ('(error "candidate failed")', '(sleep 1)'):
                reload(BASE + INIT + '''
(ekko/extensions:register-component :id :later :initialize
 (lambda (s e) (declare (ignore s e)) ''' + body + '))', ok=False)
                assert inspect()['generation'] == generation and state() == before_state
                attachment.send(2, b'b')
                attachment.pump(.1)
            assert log.read_bytes() == b'abb'
            # Queue real input while an initializer is outstanding. The failed
            # candidate must release it to the last committed routing map.
            marker = root / 'initializing'
            source.write_text(BASE + INIT + f'''
(ekko/extensions:register-component :id :later :initialize
 (lambda (s e) (declare (ignore s e))
  (with-open-file (out "{marker}" :direction :output :if-exists :supersede)
   (write-line "ready" out))
  (sleep 1)))''')
            loading = subprocess.Popen([binary, 'config', 'reload', 'initialize'], env=env,
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            eventually(marker.exists)
            attachment.send(2, b'c')
            loading.communicate(timeout=8)
            assert loading.returncode != 0
            eventually(lambda: log.read_bytes() == b'abbc')
            assert inspect()['generation'] == generation and state() == before_state
            reload(BASE + INIT)
            assert state()['loads'] == 2 and state()['reason'] == 'reload' and state()['pid'] == pid, state()
            assert inspect()['mode'] == 'overlay'
            # Owner removal reverses state and overlay. A surviving mode belongs
            # to routing, so explicitly choose the normal map before removing it.
            cli('command', '--session', 'initialize', 'dismiss')
            reload(BASE)
            assert not inspect()['component-state']
            assert not any(x['owner'] == 'welcome' for x in (inspect()['decorations'] or []))
            reload(BASE + INIT)
            assert state()['loads'] == 1
            assert inspect()['panes'][0]['pid'] == pid
            cli('stop', 'initialize')
            daemon.wait(timeout=4)
            attachment.close()
            attachment = None
            # A startup initializer failure happens before any child is spawned.
            log.unlink()
            source.write_text(BASE + INIT + '(ekko/extensions:register-component :id :later :initialize (lambda (s e) (declare (ignore s e)) (error "startup failed")))')
            failed = subprocess.run([binary, '--serve', 'initialize', sys.executable, '-c', child],
                                    env=env, capture_output=True, timeout=8)
            assert failed.returncode != 0 and not log.exists()
            print(json.dumps({'status': 'pass', 'binary': binary, 'input_hex': 'abbc'.encode().hex(),
                              'atomic_rejections': 7, 'startup_before_spawn': True,
                              'reload_preserved_child_pid': True, 'owner_removal': True}))
        finally:
            if attachment:
                attachment.close()
            if daemon.poll() is None:
                subprocess.run([binary, 'stop', 'initialize'], env=env, capture_output=True, timeout=4)
                daemon.wait(timeout=4)


if __name__ == '__main__':
    integration(sys.argv[1])
