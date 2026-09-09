"""Context menus and outline transitions with live PTYs, default and bare."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
from daily import Attachment, eventually
from selection import child


def integration(binary, profile, bare=False):
    with tempfile.TemporaryDirectory(prefix='ekko-menus-') as directory:
        root = Path(directory)
        for helper in Path(profile).parent.glob('*.lisp'):
            (root / helper.name).write_bytes(helper.read_bytes())
        config = root / 'init.lisp'
        config.write_text('(load (merge-pathnames "desktop.lisp" *default-pathname-defaults*))' if bare else '')
        env = dict(os.environ, XDG_RUNTIME_DIR=directory, EKKO_CONFIG=str(config))
        logs = [root / 'a', root / 'b', root / 'c']
        commands = [sys.executable, __file__, '--child', str(logs[0]), ':::',
                    sys.executable, __file__, '--child', str(logs[1]), ':::',
                    sys.executable, __file__, '--child', str(logs[2])]
        output = open(root / 'daemon.log', 'wb')
        daemon = subprocess.Popen([binary, '--serve', 'menus', '--viewport', '120', '40', '8', '16', *commands],
                                  env=env, stdout=output, stderr=output)
        attached = None
        def cli(*args):
            result = subprocess.run([binary, *args], env=env, capture_output=True, timeout=8)
            assert result.returncode == 0, (args, result.stderr, (root / 'daemon.log').read_text())
            return result.stdout
        def state():
            s = json.loads(cli('inspect', 'menus'))
            assert not s['error'] and not s['disabled-hooks'], s['error']
            return s
        def mouse(button, x, y, up=False):
            attached.send(4, f'\x1b[<{button};{x*8+1};{y*16+1}{"m" if up else "M"}'.encode())
            attached.pump(.04)
        def click(button, x, y):
            mouse(button, x, y)
            mouse(button, x, y, True)
        def key(data):
            attached.send(2, data)
            attached.pump(.12)
        def open_menu(pane=1, dock=False):
            s = state()
            if dock:
                span = next(span for d in s['decorations'] for span in (d['spans'] or [])
                            if span['action'] and span['action'][2] == pane and span['y'] == 39)
                x,y = span['x'],span['y']
            else:
                p = next(p for p in s['panes'] if p['id'] == pane)
                x,y = p['outer_rect'][:2]
            click(2,x,y)
            return eventually(lambda: state()['popup'])
        try:
            eventually(lambda: (root / 'ekko-v2/menus.sock').exists())
            attached = Attachment(root / 'ekko-v2/menus.sock', version=13)
            eventually(lambda: all(Path(str(p) + '.ready').exists() for p in logs))
            attached.pump(.2)
            assert all(p['visible'] for p in state()['panes']), 'startup tree omitted a window'
            third=state()['panes'][2]['outer_rect']
            click(0,third[0]+third[2]-3,third[1])
            attached.pump(.1)
            logs.pop()
            pids = [p['pid'] for p in state()['panes']]
            menu = open_menu(2)
            assert menu['x'] + 26 <= 120
            assert any('Rename' in scene and 'Close window' in scene for scene in attached.scenes)
            # Hover and click Rename on the unfocused window.
            mouse(35, menu['x']+2, menu['y']+5)
            assert state()['popup']['selected'] == 5
            click(0,menu['x']+2,menu['y']+5)
            assert not state()['popup'] and state()['mode'] == 'rename'
            key(b'Window two\r')
            assert state()['panes'][1]['name'] == 'Window two'
            menu = open_menu(2,dock=True)
            assert menu['y'] + 10 <= 40
            # Keyboard chooses Minimize: first Down selects Focus, next selects Minimize.
            start = len(attached.scenes)
            key(b'\x1b[B')
            assert state()['popup']['selected'] == 1, state()['popup']
            key(b'\x1b[B')
            assert state()['popup']['selected'] == 2, state()['popup']
            key(b'\r')
            assert state()['panes'][1]['minimized']
            attached.pump(.25)
            assert len(attached.scenes) > start + 2, 'transition did not generate frames'
            menu = open_menu(2,dock=True)
            key(b'\x1b[B'); key(b'\r')
            assert not state()['panes'][1]['minimized']
            attached.pump(.2)
            assert [p['pid'] for p in state()['panes']] == pids
            menu = open_menu(1)
            attached.send(5,b'NOT-APP-INPUT')
            attached.pump(.05)
            click(0,119,20)
            assert not state()['popup']
            assert all(p.read_bytes() == b'' for p in logs)
            open_menu(1); key(b'\x1b')
            assert not state()['popup']
            # Applications still receive their own right-clicks in content.
            p=state()['panes'][0]
            click(2,p['x']+2,p['y']+2)
            assert b'\x1b[<2;3;3M' in logs[0].read_bytes()
            assert not state()['popup']
            # Float from the public menu; dragging preserves the running process.
            menu = open_menu(2)
            click(0, menu['x']+2, menu['y']+4)
            attached.pump(.2)
            p = state()['panes'][1]
            assert p['floating'], state()
            underneath=logs[0].read_bytes()
            click(2,p['x']+2,p['y']+2)
            assert logs[0].read_bytes()==underneath, 'click leaked through floating window'
            assert b'\x1b[<2;3;3M' in logs[1].read_bytes()
            before = p['outer_rect']
            mouse(0,before[0]+5,before[1])
            mouse(32,before[0]+3,before[1]+12)
            # A preview must not resize the child while held.
            assert state()['panes'][1]['outer_rect'] == before
            mouse(0,before[0]+3,before[1]+12,True)
            p = state()['panes'][1]
            assert p['floating'] and p['outer_rect'] != before, p
            before = p['outer_rect']
            # Bottom-right corner resizes on release.
            x,y,w,h = before
            mouse(0,x+w-1,y+h-1); mouse(32,x+w-5,y+h-3)
            assert state()['panes'][1]['outer_rect'] == before
            mouse(0,x+w-5,y+h-3,True)
            p=state()['panes'][1]
            assert p['cols'] == w-6 and p['rows'] == h-4, p
            # Placement survives viewer replacement, with the same PTYs.
            floating=p['floating']
            attached.close(); time.sleep(.05)
            attached=Attachment(root / 'ekko-v2/menus.sock',version=13)
            assert state()['panes'][1]['floating'] == floating
            assert [p['pid'] for p in state()['panes']] == pids
            # Escape cancels without committing or leaking an Escape to the app.
            before=p['outer_rect']
            mouse(0,before[0]+4,before[1]); mouse(32,before[0]+8,before[1]+2)
            key(b'\x1b'); mouse(0,before[0]+8,before[1]+2,True)
            assert state()['panes'][1]['outer_rect'] == before
            # Snap at the left of the remaining tile, then swap by its center.
            mouse(0,before[0]+4,before[1]); mouse(32,1,15); mouse(0,1,15,True)
            assert not state()['panes'][1]['floating']
            assert state()['panes'][1]['outer_rect'][0] == 0
            p=state()['panes'][1]; q=state()['panes'][0]
            mouse(0,p['outer_rect'][0]+5,p['outer_rect'][1])
            tx=q['outer_rect'][0]+q['outer_rect'][2]//2
            ty=q['outer_rect'][1]+q['outer_rect'][3]//2
            mouse(32,tx,ty); mouse(0,tx,ty,True)
            assert state()['panes'][1]['outer_rect'][0] > 0
            assert [p['pid'] for p in state()['panes']] == pids
            # Empty taskbar space opens the session menu.
            click(2,119,39)
            assert state()['popup']
            key(b'\x1b')
            open_menu(1)
            config.write_text('(ekko/extensions:unregister-component :defaults)')
            cli('config','reload','menus')
            attached.pump(.1)
            assert not state()['popup']
            print(json.dumps({'binary':Path(binary).name,'menus':'pass','input_isolation':'pass',
                              'application_right_click':'pass','transition':'pass','unmount':'pass'}))
        finally:
            if attached:
                attached.close()
            subprocess.run([binary,'stop','menus'],env=env,capture_output=True,timeout=8)
            try:
                daemon.wait(timeout=8)
            except subprocess.TimeoutExpired:
                daemon.kill(); daemon.wait()
            output.close()

if __name__ == '__main__':
    if sys.argv[1] == '--child':
        os.write(1,b'\x1b[?1000h\x1b[?1006h')
        child(sys.argv[2])
    else:
        integration(*sys.argv[1:])
