"""Live frame geometry lifecycle: child WINCH, reload, detach and ownership."""
import fcntl
import json
import os
from pathlib import Path
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import tty

from daily import Attachment, eventually

EXTRA = '''
(ekko/extensions:register-component :id :frame-test)
(ekko/extensions:set-option :component :frame-test :name :viewport-insets :value '(0 0 0 0))
(ekko/extensions:set-option :component :frame-test :name :initial-layout
 :value '(:columns 50 1 (:rows 50 2 3)))
'''


def child(path):
    tty.setraw(0, termios.TCSANOW)
    def record(event):
        size = struct.unpack('HHHH', fcntl.ioctl(0, termios.TIOCGWINSZ, b'\0' * 8))
        with open(path, 'a') as output:
            output.write(json.dumps([event, *size]) + '\n')
    signal.signal(signal.SIGWINCH, lambda *_: record('WINCH'))
    record('FIRST')
    while True:
        data = os.read(0, 4096)
        with open(path + '.input', 'ab') as output:
            output.write(data)


def integration(binary, profile):
    profile = Path(profile).resolve()
    with tempfile.TemporaryDirectory(prefix='ekko-frame-lifecycle-') as directory:
        root = Path(directory)
        for helper in profile.parent.glob('zellij-*.lisp'):
            (root / helper.name).write_bytes(helper.read_bytes())
        config = root / 'init.lisp'
        source = profile.read_text() + EXTRA
        config.write_text(source)
        env = dict(os.environ, XDG_RUNTIME_DIR=directory, EKKO_CONFIG=str(config))
        logs = [root / str(i) for i in range(3)]
        argv = [binary, '--serve', 'frames', '--viewport', '120', '40', '8', '16']
        for i, log in enumerate(logs):
            if i:
                argv.append(':::')
            argv += [sys.executable, str(Path(__file__).resolve()), '--child', str(log)]
        daemon = subprocess.Popen(argv, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        attached = None

        def cli(*args, ok=True):
            result = subprocess.run([binary, *args], env=env, capture_output=True, timeout=8)
            assert (result.returncode == 0) == ok, (args, result.stderr)
            return result

        def inspect():
            return json.loads(cli('inspect', 'frames').stdout)

        def sizes():
            return [(p['rows'], p['cols']) for p in inspect()['panes']]

        def histories():
            return [[json.loads(line) for line in p.read_text().splitlines()] for p in logs]

        def geometry(expected):
            eventually(lambda: sizes() == expected)
            eventually(lambda: [tuple(h[-1][1:3]) for h in histories()] == expected)
            assert [p['pid'] for p in inspect()['panes']] == pids

        def toggle(expected):
            before = [len(h) for h in histories()]
            attached.send(2, b'\x10')
            eventually(lambda: inspect()['mode'] == 'pane')
            attached.send(2, b'z')
            eventually(lambda: inspect()['mode'] == 'normal')
            geometry(expected)
            assert [len(h) for h in histories()] == [n + 1 for n in before]

        try:
            socket = root / 'ekko-v2/frames.sock'
            eventually(lambda: all(p.exists() for p in logs) or daemon.poll() is not None)
            assert daemon.poll() is None, daemon.stderr.read().decode()
            attached = Attachment(socket)
            initial = inspect()
            pids = [p['pid'] for p in initial['panes']]
            outer = [p['outer_rect'] for p in initial['panes']]
            on = [(38, 58), (18, 58), (18, 58)]
            off = [(40, 59), (19, 60), (20, 60)]
            geometry(on)
            toggle(off)
            assert [p['outer_rect'] for p in inspect()['panes']] == outer
            toggle(on)
            toggle(off)
            history = histories()
            # Owner state and geometry survive transactional reload and viewer loss.
            cli('config', 'reload', 'frames')
            geometry(off)
            assert histories() == history
            config.write_text('(error "frame reload must roll back")')
            cli('config', 'reload', 'frames', ok=False)
            geometry(off)
            attached.close()
            attached = None
            attached = Attachment(socket)
            geometry(off)
            assert histories() == history
            config.write_text(source)
            cli('config', 'reload', 'frames')
            toggle(on)
            toggle(off)
            # Removing the owner reverses only its geometry and state.
            config.write_text(source + '\n(ekko/extensions:unregister-component :zellij-frames)\n')
            cli('config', 'reload', 'frames')
            geometry(on)
            assert not inspect()['geometry']['contributions']
            assert not any(key == 'zellij-frames' for key, value in
                           (inspect().get('component-state') or []))
            assert all(not p.with_name(p.name + '.input').exists() for p in logs)
            cli('stop', 'frames')
            daemon.wait(timeout=3)
            assert daemon.returncode == 0
        finally:
            if attached:
                attached.close()
            if daemon.poll() is None:
                daemon.terminate()
                daemon.wait(timeout=3)
    print(json.dumps({'suite': 'pane-frames', 'binary': binary, 'status': 'pass'}))


if __name__ == '__main__':
    if sys.argv[1] == '--child':
        child(sys.argv[2])
    else:
        integration(sys.argv[1], sys.argv[2])
