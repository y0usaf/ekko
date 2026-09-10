"""Regression: stalled viewer must not be dropped by the daemon (UI dies).

Starts a real daemon with the desktop defaults, attaches a real viewer
process on a PTY, then SIGSTOPs the viewer past the daemon's 10-second
scene-acknowledgement deadline and SIGCONTs it. On the buggy baseline the
daemon drops the peer, the viewer sees EOF and exits (the reported "UI
randomly dies"). After the fix the viewer survives and keeps rendering.
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

            def viewer_alive():
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
