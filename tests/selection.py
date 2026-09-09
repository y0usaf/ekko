"""Mouse selection through real PTYs, both runtimes, and the Zellij profile."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import termios
import tty

from daily import Attachment, eventually


def child(log):
    tty.setraw(0, termios.TCSANOW)
    os.write(1, b''.join(f'history-{i}\r\n'.encode() for i in range(80)))
    os.write(1, b'\x1b[H\x1b[31malpha beta\x1b[0m\x1b[2;1Hsecond line')
    Path(log + '.ready').touch()
    with open(log, 'ab', buffering=0) as output:
        while True:
            data = os.read(0, 4096)
            output.write(data)


def integration(binary, profile):
    with tempfile.TemporaryDirectory(prefix='ekko-selection-') as directory:
        root = Path(directory)
        profile = Path(profile).resolve()
        for helper in profile.parent.glob('zellij*.lisp'):
            (root / helper.name).write_bytes(helper.read_bytes())
        env = dict(os.environ, XDG_RUNTIME_DIR=directory, EKKO_CONFIG=str(root / profile.name))
        log = root / 'input'
        with open(root / 'daemon.log', 'wb') as output:
            daemon = subprocess.Popen([binary, '--serve', 'selection', '--viewport', '120', '40', '8', '16',
                                       sys.executable, __file__, '--child', str(log)],
                                      env=env, stdout=output, stderr=output)
        attached = None

        def cli(*args):
            result = subprocess.run([binary, *args], env=env, capture_output=True, timeout=8)
            assert result.returncode == 0, (args, result.stderr, (root / 'daemon.log').read_text())
            return result.stdout

        def pane():
            return json.loads(cli('status', 'selection'))['panes'][0]

        def mouse(button, col, row, up=False):
            p = pane()
            attached.send(4, f'\x1b[<{button};{(p["x"] + col) * 8 + 1};{(p["y"] + row) * 16 + 1}{"m" if up else "M"}'.encode())
            attached.pump(.08)

        try:
            eventually(lambda: (root / 'ekko-v2/selection.sock').exists())
            attached = Attachment(root / 'ekko-v2/selection.sock', version=12)
            eventually(lambda: Path(str(log) + '.ready').exists())
            attached.pump(.1)
            pid = pane()['pid']
            mouse(0, 1, 0)
            mouse(32, 4, 0)
            assert any('"lpha" (0 31 48 5 238)' in s for s in attached.scenes), attached.scenes[-1]
            mouse(0, 4, 0, True)
            assert cli('buffer', 'selection') == b'lpha'
            assert attached.clipboards[-1] == b'lpha'
            assert log.read_bytes() == b''
            mouse(0, 2, 1)
            mouse(32, 6, 0)
            mouse(0, 6, 0, True)
            assert cli('buffer', 'selection') == b'beta\nsec'
            attached.send(2, b'X')
            eventually(lambda: log.read_bytes() == b'X')
            assert not pane()['copy_mode']
            mouse(64, 1, 1)
            assert pane()['copy_mode']
            mouse(65, 1, 1)
            assert not pane()['copy_mode']
            assert pane()['pid'] == pid
            assert log.read_bytes() == b'X'
            # A legacy viewer gets the same selection and buffer, no new packet.
            attached.close()
            attached = None
            eventually(lambda: not json.loads(cli('status', 'selection'))['attached'])
            attached = Attachment(root / 'ekko-v2/selection.sock', version=11)
            mouse(0, 1, 0)
            mouse(0, 4, 0, True)
            assert cli('buffer', 'selection') == b'lpha'
            assert not attached.clipboards
            attached.send(5, b'\x1b[200~')
            attached.send(5, b'PASTE')
            attached.send(5, b'\x1b[201~')
            eventually(lambda: log.read_bytes() == b'XPASTE')
            assert not pane()['copy_mode']
            print(json.dumps({'binary': Path(binary).name, 'selection': 'pass', 'clipboard': 'pass',
                              'wheel': 'pass', 'legacy': 'pass', 'child_preserved': True}))
        finally:
            if attached:
                attached.close()
            subprocess.run([binary, 'stop', 'selection'], env=env, capture_output=True, timeout=8)
            try:
                daemon.wait(timeout=8)
            except subprocess.TimeoutExpired:
                daemon.kill()
                daemon.wait()


if __name__ == '__main__':
    if sys.argv[1] == '--child':
        child(sys.argv[2])
    else:
        integration(*sys.argv[1:])
