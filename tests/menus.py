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
        row_log = root / 'row'
        row_command = ' '.join(json.dumps(arg) for arg in (sys.executable, __file__, '--child', str(row_log)))
        config.write_text(('(load (merge-pathnames "desktop.lisp" *default-pathname-defaults*))' if bare else '') +
                          f"\n(ekko/extensions:set-option :component :{'desktop-windows' if bare else 'defaults'} "
                          f":name :shell :value '({row_command}))")
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
            # Resize the shared tiled border without changing child PTYs until release.
            left, right = sorted(state()['panes'], key=lambda p: p['outer_rect'][0])
            x, y, w, h = left['outer_rect']
            before = [p['outer_rect'] for p in state()['panes']]
            mouse(35, x+w-1, y+2)
            mouse(0, x+w-1, y+2)
            mouse(32, x+w+7, y+2)
            assert [p['outer_rect'] for p in state()['panes']] == before
            mouse(0, x+w+7, y+2, True)
            resized = state()['panes']
            assert all(not p['floating'] for p in resized)
            assert resized[0]['cols'] > left['cols'] and resized[1]['cols'] < right['cols']
            assert sum(p['outer_rect'][2] for p in resized) == 120
            assert [p['pid'] for p in resized] == pids
            # Either side of the border works; Escape cancels and does not reach the child.
            right = resized[1]
            x, y, w, h = right['outer_rect']
            before = [p['outer_rect'] for p in resized]
            mouse(0, x, y+2); mouse(32, x-4, y+2)
            key(b'\x1b'); mouse(0, x-4, y+2, True)
            assert [p['outer_rect'] for p in state()['panes']] == before
            mouse(0, x, y+2); mouse(32, x-4, y+2); mouse(0, x-4, y+2, True)
            assert state()['panes'][1]['cols'] > right['cols']
            assert all(p.read_bytes() == b'' for p in logs), 'border drag leaked into application'
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
            # The menu advertises the same keyboard toggle, returning to normal mode.
            key(b'\x10'); key(b't')
            assert not state()['panes'][1]['floating'] and state()['mode'] == 'normal'
            key(b'\x10'); key(b't')
            assert state()['panes'][1]['floating'] and state()['mode'] == 'normal'
            p = state()['panes'][1]
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
            # Top corners and the small top-edge handle resize without moving the opposite edge.
            for offset, dx, dy in ((0, 2, 2), (-1, 2, 1), (1, 0, -1)):
                before = state()['panes'][1]['outer_rect']
                x,y,w,h = before
                handle_x = x+w-1 if offset == -1 else x+offset
                mouse(0,handle_x,y); mouse(32,handle_x+dx,y+dy)
                assert state()['panes'][1]['outer_rect'] == before
                mouse(0,handle_x+dx,y+dy,True)
                p = state()['panes'][1]
                after = p['outer_rect']
                assert after[1] == y+dy and after[1]+after[3] == y+h, (before, after)
                if offset == 0:
                    assert after[0] == x+dx and after[0]+after[2] == x+w, (before, after)
                elif offset == -1:
                    assert after[0] == x and after[2] == w+dx, (before, after)
                else:
                    assert after[0] == x and after[2] == w, (before, after)
            # Placement survives viewer replacement, with the same PTYs.
            floating=p['floating']
            x,y,w,h=p['outer_rect']
            mouse(0,x+w-1,y+2); mouse(32,x+w+1,y+2)
            assert state()['panes'][1]['floating'] == floating
            attached.close(); time.sleep(.05)
            attached=Attachment(root / 'ekko-v2/menus.sock',version=13)
            mouse(0,x+w+1,y+2,True)
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
            # A real row split can be resized from its upper window's bottom border.
            menu = open_menu(2)
            click(0, menu['x']+2, menu['y']+7)
            eventually(lambda: Path(str(row_log) + '.ready').exists())
            attached.pump(.2)
            lower = state()['panes'][-1]
            upper = next(p for p in state()['panes'][:-1]
                         if p['outer_rect'][1]+p['outer_rect'][3] == lower['outer_rect'][1]
                         and p['outer_rect'][0] == lower['outer_rect'][0])
            x,y,w,h = upper['outer_rect']
            mouse(0,x+2,y+h-1); mouse(32,x+2,y+h+2)
            assert next(p for p in state()['panes'] if p['id'] == upper['id'])['rows'] == upper['rows']
            mouse(0,x+2,y+h+2,True)
            assert next(p for p in state()['panes'] if p['id'] == upper['id'])['rows'] > upper['rows']
            assert state()['panes'][-1]['rows'] < lower['rows']
            assert all(not p['floating'] for p in state()['panes'])
            assert row_log.read_bytes() == b''
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
