"""Desktop controls through real PTYs and the public decoration API."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
from daily import Attachment, eventually
from selection import child


def integration(binary, profile, default=False):
    with tempfile.TemporaryDirectory(prefix='ekko-desktop-') as directory:
        root = Path(directory)
        for helper in Path(profile).parent.glob('*.lisp'):
            (root / helper.name).write_bytes(helper.read_bytes())
        env = dict(os.environ, XDG_RUNTIME_DIR=directory, EKKO_CONFIG=str(root / 'desktop.lisp'))
        if default:
            env['EKKO_CONFIG'] = str(root / 'init.lisp')
            Path(env['EKKO_CONFIG']).write_text('')
        logs = [root / 'a', root / 'b']
        commands = [sys.executable, __file__, '--child', str(logs[0]), ':::',
                    sys.executable, __file__, '--child', str(logs[1])]
        output = open(root / 'daemon.log', 'wb')
        daemon = subprocess.Popen([binary, '--serve', 'desktop', '--viewport', '120', '40', '8', '16', *commands],
                                  env=env, stdout=output, stderr=output)
        attached = None
        def cli(*args):
            result = subprocess.run([binary, *args], env=env, capture_output=True, timeout=8)
            assert result.returncode == 0, (args, result.stderr, (root / 'daemon.log').read_text())
            return result.stdout
        def state():
            data = json.loads(cli('inspect', 'desktop'))
            assert not data['error'], data['error']
            assert not data['disabled-hooks'], data['disabled-hooks']
            return data
        def click(op, pane, dock=False):
            def span():
                return next((s for d in state()['decorations'] for s in (d['spans'] or [])
                             if s['action'] == [op, 'pane', pane] and (not dock or s['y'] == 39)), None)
            target = eventually(span)
            for up in (False, True):
                attached.send(4, f'\x1b[<0;{target["x"] * 8 + 1};{target["y"] * 16 + 1}{"m" if up else "M"}'.encode())
                attached.pump(.1)
        try:
            eventually(lambda: (root / 'ekko-v2/desktop.sock').exists())
            attached = Attachment(root / 'ekko-v2/desktop.sock', version=12)
            eventually(lambda: all(Path(str(p) + '.ready').exists() for p in logs))
            attached.pump(.2)
            assert state()['mode'] == 'normal'
            attached.send(2, b'\x10')
            attached.pump(.1)
            assert state()['mode'] == 'pane'
            attached.send(2, b'c')
            attached.pump(.1)
            assert state()['mode'] == 'rename'
            attached.send(2, b'Desktop')
            attached.pump(.1)
            assert state()['panes'][0]['name'] == 'Desktop'
            attached.send(2, b'\x1b')
            attached.pump(.1)
            assert not state()['panes'][0]['name']
            assert state()['mode'] == 'pane'
            attached.send(2, b'\r')
            attached.pump(.1)
            assert state()['mode'] == 'normal'
            before = state()
            pids = [p['pid'] for p in before['panes']]
            assert before['viewport']['insets'] == [0, 0, 1, 0]
            assert all(p['y'] + p['rows'] <= 39 for p in before['panes'])
            click('zoom', 1)
            assert [p['id'] for p in state()['panes'] if p['visible']] == [1]
            click('zoom', 1)
            assert [p['outer_rect'] for p in state()['panes']] == [p['outer_rect'] for p in before['panes']]
            click('minimize', 1)
            assert state()['panes'][0]['minimized']
            click('minimize', 2)
            assert all(p['minimized'] and not p['visible'] for p in state()['panes'])
            attached.send(2, b'IGNORED')
            attached.send(5, b'IGNORED-PASTE')
            attached.pump(.1)
            assert all(p.read_bytes() == b'' for p in logs)
            attached.close()
            eventually(lambda: not json.loads(cli('status', 'desktop'))['attached'])
            attached = Attachment(root / 'ekko-v2/desktop.sock', version=12)
            attached.pump(.2)
            assert all(p['minimized'] for p in state()['panes'])
            click('restore', 1)
            click('restore', 2)
            assert [p['pid'] for p in state()['panes']] == pids
            assert [p['outer_rect'] for p in state()['panes']] == [p['outer_rect'] for p in before['panes']]
            assert all(p.read_bytes() == b'' for p in logs)
            click('minimize', 2, dock=True)
            assert state()['panes'][1]['minimized']
            click('restore', 2, dock=True)
            assert not state()['panes'][1]['minimized']
            click('focus', 1, dock=True)
            assert not any(p['minimized'] for p in state()['panes'])
            click('minimize', 1, dock=True)
            assert state()['panes'][0]['minimized']
            click('restore', 1, dock=True)
            assert [p['pid'] for p in state()['panes']] == pids
            assert all(p.read_bytes() == b'' for p in logs)
            click('close', 1)
            assert [p['id'] for p in state()['panes']] == [2]
            assert state()['panes'][0]['pid'] == pids[1]
            # Reload a profile without desktop; component-owned controls disappear.
            Path(env['EKKO_CONFIG']).write_text('(load (merge-pathnames "zellij.lisp" *default-pathname-defaults*))')
            cli('config', 'reload', 'desktop')
            attached.pump(.2)
            assert not any(s['action'] for d in state()['decorations'] for s in (d['spans'] or []))
            print(json.dumps({'binary': Path(binary).name, 'default': bool(default), 'desktop': 'pass', 'pty_preservation': 'pass',
                              'reserved_dock': 'pass', 'reattach': 'pass', 'unmount': 'pass'}))
        finally:
            if attached:
                attached.close()
            subprocess.run([binary, 'stop', 'desktop'], env=env, capture_output=True, timeout=8)
            try:
                daemon.wait(timeout=8)
            except subprocess.TimeoutExpired:
                daemon.kill()
                daemon.wait()
            output.close()

if __name__ == '__main__':
    if sys.argv[1] == '--child':
        child(sys.argv[2])
    else:
        integration(*sys.argv[1:])
