"""Regression: daemon-side hiccups must not take the UI down.

Starts a real daemon with the desktop defaults, attaches a real viewer
process on a PTY, then exercises two failures that previously ended the UI:

1. SIGSTOP the viewer past the daemon's 10-second scene-acknowledgement
   deadline: the daemon used to drop the peer, the viewer saw EOF and exited
   (the reported "UI randomly dies").
2. SIGSTOP the extension worker past a change-hook deadline: the desktop's
   repaint hook used to be disabled after one late answer, so its window
   frames and dock vanished for the rest of the session (the reported "UI
   disappears").

After both fixes the viewer survives, the chrome returns and the session
keeps its registered hooks.
"""
import json
import os
from pathlib import Path
import pty
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
import fcntl
import errno

from daily import eventually


def integration(binary, out_path, stall=12.0):
    with tempfile.TemporaryDirectory(prefix='ekko-ui-reliability-') as directory:
        root = Path(directory)
        config = root / 'init.lisp'
        config.write_text('')
        env = dict(os.environ, XDG_RUNTIME_DIR=directory, EKKO_CONFIG=str(config),
                   TERM='xterm-256color')
        name = 'reliability'
        argv = [binary, '--serve', name, '--viewport', '120', '40', '8', '16',
                sys.executable, '-c',
                "import os,time\n"
                "os.read(0,1)\n"
                "i=0\n"
                "while True:\n"
                "    os.write(1, b'line %d\\r\\n' % i); i+=1\n"
                "    time.sleep(0.1)\n"]
        output = open(root / 'daemon.log', 'wb')
        daemon = subprocess.Popen(argv, env=env, stdout=output, stderr=output)
        viewer = None
        fd = -1
        result = {}
        try:
            eventually(lambda: (root / f'ekko-v2/{name}.sock').exists()
                       or daemon.poll() is not None, timeout=8)
            assert daemon.poll() is None, (root / 'daemon.log').read_text()[:2000]

            fd, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', 40, 120, 960, 640))
            viewer = subprocess.Popen([binary, 'attach', name], stdin=slave,
                                      stdout=slave, stderr=slave, env=env,
                                      start_new_session=True)
            os.close(slave)
            os.set_blocking(fd, False)
            raw = bytearray()

            def drain():
                # Keep the PTY drained so post-recovery frames are never stuck
                # behind earlier output, and retain them for chrome assertions.
                while True:
                    try:
                        chunk = os.read(fd, 65536)
                    except OSError as error:
                        if error.errno in (errno.EAGAIN, errno.EIO):
                            return
                        raise
                    if not chunk:
                        return
                    raw.extend(chunk)

            def viewer_alive():
                drain()
                return viewer.poll() is None

            eventually(viewer_alive, timeout=4)
            time.sleep(1.0)
            assert viewer_alive(), 'viewer exited before the stall'

            # Stall the viewer past the daemon's 10s scene-ack deadline.
            os.kill(viewer.pid, signal.SIGSTOP)
            time.sleep(stall)
            os.kill(viewer.pid, signal.SIGCONT)

            # Give the pair ample time to fail (baseline) or recover (fixed).
            time.sleep(6.0)
            result['viewer_alive_after_stall'] = viewer_alive()
            result['daemon_alive'] = daemon.poll() is None

            # The daemon must still answer control commands either way.
            probe = subprocess.run([binary, 'status', name], env=env,
                                   capture_output=True, timeout=8)
            result['status_ok'] = probe.returncode == 0

            # Required outcome: the stalled viewer recovers and keeps running.
            assert result['viewer_alive_after_stall'], (
                'UI died across a viewer stall (daemon dropped the peer)')
            assert result['daemon_alive']
            assert result['status_ok']

            # A late change-hook answer must not disable the desktop repaint
            # hook. The dock clock changes every second, so stopping the
            # extension worker forces a dispatch past the hook deadline.
            def inspect():
                return json.loads(subprocess.run([binary, 'inspect', name], env=env,
                                                 capture_output=True, timeout=8).stdout)

            assert any(d['owner'] == 'defaults' for d in inspect()['decorations']), (
                'desktop decorations missing before the hook stall')
            mark = len(raw)
            worker = json.loads(subprocess.run([binary, 'status', name], env=env,
                                               capture_output=True, timeout=8).stdout)['extension_pid']
            os.kill(worker, signal.SIGSTOP)
            eventually(lambda: b'timed out' in (root / 'daemon.log').read_bytes(), timeout=10)
            live = json.loads(subprocess.run([binary, 'status', name], env=env,
                                             capture_output=True, timeout=8).stdout)
            if live['extension_pid'] == worker:
                os.kill(worker, signal.SIGCONT)

            # Required outcome: the repaint hook survives and the dock returns.
            def chrome_back():
                drain()
                return b'EKKO' in bytes(raw[mark:])

            eventually(chrome_back, timeout=10)
            state = inspect()
            assert not state['disabled-hooks'], state['disabled-hooks']
            assert any(d['owner'] == 'defaults' and d['spans'] for d in state['decorations'])
            result['hook_timeout_recovered'] = True
            result['viewer_alive_after_hook_timeout'] = viewer_alive()
            assert result['viewer_alive_after_hook_timeout'], 'viewer exited after hook timeout'

            subprocess.run([binary, 'stop', name], env=env, capture_output=True, timeout=8)
            daemon.wait(timeout=4)
            result['daemon_exit'] = daemon.returncode
        finally:
            if viewer and viewer.poll() is None:
                viewer.terminate()
                try:
                    viewer.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    viewer.kill()
                try:
                    os.killpg(os.getpgid(viewer.pid), signal.SIGKILL)
                except (ProcessLookupError, PermissionError):
                    pass
            if fd >= 0:
                try:
                    os.close(fd)
                except OSError:
                    pass
            if daemon.poll() is None:
                daemon.terminate()
                try:
                    daemon.wait(timeout=4)
                except subprocess.TimeoutExpired:
                    daemon.kill()
                    daemon.wait()
            output.close()
    Path(out_path).write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'status': 'pass', 'binary': binary, 'result': result}))


if __name__ == '__main__':
    integration(sys.argv[1], sys.argv[2], float(sys.argv[3]) if len(sys.argv) > 3 else 12.0)
