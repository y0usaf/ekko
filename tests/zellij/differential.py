"""Real-PTY differential runner. Green routing is not a visual parity claim."""
import argparse
import codecs
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import select
import shutil
import struct
import subprocess
import sys
import tempfile
import termios
import time
import tty


SETTLED_SCENARIOS = [('normal', b'a', b'a'),
             ('enter-locked', b'\x07', b'a'),
             ('locked-input', b'\x10b', b'a\x10b'),
             ('leave-locked', b'\x07', b'a\x10b'),
             ('normal-again', b'c', b'a\x10bc')]
SETUP_STAGES = {'startup', 'dismiss-release-notes'}
EXPECTED_STAGES = ['startup', 'dismiss-release-notes'] + [name for name, _, _ in SETTLED_SCENARIOS]


def check(condition, message):
    if not condition:
        raise AssertionError(message)


def child(log):
    tty.setraw(0, termios.TCSANOW)
    Path(log).write_bytes(b'')
    os.write(1, b'\x1b[2J\x1b[Hfixture ready')
    while True:
        data = os.read(0, 4096)
        with open(log, 'ab', buffering=0) as out:
            out.write(data)


class Terminal:
    def __init__(self, argv, env, cols, rows):
        import pyte
        self.cols, self.rows = cols, rows
        self.screen = pyte.Screen(cols, rows)
        self.stream = pyte.Stream(self.screen)
        self.decoder = codecs.getincrementaldecoder('utf-8')('replace')
        self.raw = bytearray()
        self.fd, slave = pty.openpty()
        try:
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH', rows, cols, cols*8, rows*16))
            self.process = subprocess.Popen(argv, stdin=slave, stdout=slave, stderr=slave,
                                            env=env, start_new_session=True)
        except BaseException:
            for descriptor in (slave, self.fd):
                try:
                    os.close(descriptor)
                except OSError:
                    pass
            raise
        os.close(slave)

    def pump(self, duration=.25):
        deadline = time.monotonic() + duration
        while time.monotonic() < deadline:
            if select.select([self.fd], [], [], .01)[0]:
                try:
                    data = os.read(self.fd, 65536)
                except OSError:
                    break
                if not data:
                    break
                self.raw.extend(data)
                self.stream.feed(self.decoder.decode(data))

    def snapshot(self):
        return {'cursor': [self.screen.cursor.x, self.screen.cursor.y, self.screen.cursor.hidden],
                'cells': [[self.screen.buffer[y][x]._asdict() for x in range(self.cols)]
                          for y in range(self.rows)]}

    def close(self):
        try:
            if self.process.poll() is None:
                try:
                    self.process.terminate()
                except ProcessLookupError:
                    pass
                try:
                    self.process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    try:
                        self.process.kill()
                    except ProcessLookupError:
                        pass
                    self.process.wait()
        finally:
            if self.fd is not None:
                try:
                    os.close(self.fd)
                except OSError:
                    pass
                self.fd = None


def run(kind, binary, root, profile, reference, cols, rows, output):
    work = root / 'session'
    if work.exists():
        shutil.rmtree(work)
    work.mkdir()
    env = {k: v for k, v in os.environ.items()
           if not k.startswith(('ZELLIJ', 'EKKO', 'XDG_'))}
    env.update(HOME=str(work), XDG_RUNTIME_DIR=str(work), XDG_CONFIG_HOME=str(work),
               XDG_CACHE_HOME=str(work), XDG_DATA_HOME=str(work), TERM='xterm-256color',
               COLORTERM='truecolor', LANG='C.UTF-8', LC_ALL='C.UTF-8', SHELL='/bin/sh')
    log = work / 'input.bin'
    app = [sys.executable, str(Path(__file__).resolve()), '--child', str(log)]
    if kind == 'zellij':
        # Only replace the central application; preserve the reference chrome.
        layout = (reference / 'default.kdl').read_text().replace('\n    pane\n',
            '\n    pane command=' + json.dumps(app[0]) + ' {\n        args ' +
            ' '.join(json.dumps(a) for a in app[1:]) + '\n    }\n')
        (work / 'layout.kdl').write_text(layout)
        argv = [binary, '--config', str(reference / 'config.kdl'), '--new-session-with-layout',
                str(work / 'layout.kdl'), '--session', 'oracle']
        cleanup = [binary, 'kill-session', 'oracle']
    else:
        env['EKKO_CONFIG'] = str(profile)
        argv = [binary, 'run', '--session', 'oracle'] + app
        cleanup = [binary, 'stop', 'oracle']
    terminal = Terminal(argv, env, cols, rows)
    stages = []
    try:
        deadline = time.monotonic() + 15
        while not log.exists() and time.monotonic() < deadline:
            terminal.pump(.1)
        if not log.exists():
            raise AssertionError(f'{kind} failed to start: {bytes(terminal.raw)!r}')
        terminal.pump(1)
        startup_input = log.read_bytes()
        stages.append(dict(terminal.snapshot(), name='startup', input_hex=startup_input.hex(),
                           input_delta_hex=startup_input.hex(), sent_hex=''))

        before = log.read_bytes()
        os.write(terminal.fd, b'\x1b')
        terminal.pump(.25)
        actual = log.read_bytes()
        stages.append(dict(terminal.snapshot(), name='dismiss-release-notes', input_hex=actual.hex(),
                           input_delta_hex=actual[len(before):].hex(), sent_hex='1b'))
        baseline = actual

        for name, keys, suffix in SETTLED_SCENARIOS:
            expected = baseline + suffix
            before = log.read_bytes()
            os.write(terminal.fd, keys)
            deadline = time.monotonic() + 4
            while time.monotonic() < deadline:
                terminal.pump(.1)
                if log.read_bytes() == expected:
                    break
            terminal.pump(.25)
            actual = log.read_bytes()
            check(actual == expected, (kind, name, actual, expected, terminal.screen.display))
            state = terminal.snapshot()
            state.update(name=name, input_hex=actual.hex(), input_delta_hex=actual[len(before):].hex(), sent_hex=keys.hex())
            stages.append(state)
        return stages
    finally:
        (output / f'{kind}.ansi').write_bytes(terminal.raw)
        (output / f'{kind}.json').write_text(json.dumps(stages, indent=2)+'\n')
        cleanup_error = None
        try:
            result = subprocess.run(cleanup, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=5)
            if result.returncode != 0:
                cleanup_error = AssertionError(
                    f'{kind} cleanup failed ({result.returncode}): {result.stderr.decode(errors="replace")}')
        except BaseException as error:
            cleanup_error = error
        finally:
            terminal.close()
        if cleanup_error is not None:
            # Keep a scenario assertion or startup failure as the primary
            # exception, while still reporting cleanup failures explicitly.
            if sys.exc_info()[0] is None:
                raise cleanup_error
            print(f'{kind} cleanup failed: {cleanup_error}', file=sys.stderr)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--zellij', required=True)
    parser.add_argument('--ekko', required=True)
    parser.add_argument('--profile', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--cols', type=int, default=80)
    parser.add_argument('--rows', type=int, default=24)
    parser.add_argument('--require-parity', action='store_true')
    args = parser.parse_args()
    check(args.cols > 0 and args.rows > 0, 'terminal dimensions must be positive')
    reference = Path(__file__).resolve().parent / 'reference'
    pin = json.loads((reference / 'pin.json').read_text())
    args.output.mkdir(parents=True, exist_ok=True)
    expected_version = 'zellij ' + pin['release']
    actual_version = subprocess.check_output([args.zellij, '--version']).decode().strip()
    check(actual_version == expected_version, f'expected {expected_version!r}, got {actual_version!r}')
    for name, digest in pin['files'].items():
        actual_digest = hashlib.sha256((reference / name).read_bytes()).hexdigest()
        check(actual_digest == digest, f'{name} hash changed: expected {digest}, got {actual_digest}')
    with tempfile.TemporaryDirectory(prefix='ekko-zellij-') as temp:
        root = Path(temp)
        results = {kind: run(kind, binary, root, args.profile.resolve(), reference,
                             args.cols, args.rows, args.output)
                   for kind, binary in [('zellij', args.zellij), ('ekko', args.ekko)]}
        for kind in results:
            (args.output / f'{kind}.json').write_text(json.dumps(results[kind], indent=2)+'\n')
    for kind, stages in results.items():
        check([stage['name'] for stage in stages] == EXPECTED_STAGES,
              f'{kind} stages differ from expected sequence')
    check(len(results['zellij']) == len(results['ekko']), 'reference and candidate stage counts differ')
    differences = []
    for z, e in zip(results['zellij'], results['ekko']):
        cells = [{'x': x, 'y': y, 'zellij': zc, 'ekko': ec}
                 for y, (zr, er) in enumerate(zip(z['cells'], e['cells']))
                 for x, (zc, ec) in enumerate(zip(zr, er)) if zc != ec]
        differences.append({'stage': z['name'], 'input_equal': z['input_hex'] == e['input_hex'],
                            'input_delta_equal': z['input_delta_hex'] == e['input_delta_hex'],
                            'cursor_equal': z['cursor'] == e['cursor'], 'cells': cells})
    report = {'reference': pin['release'], 'dimensions': [args.cols, args.rows],
              'input_slice_passed': all(d['input_delta_equal'] for d in differences
                                        if d['stage'] not in SETUP_STAGES),
              'application_input_parity': all(d['input_equal'] for d in differences),
              'modeled_cell_parity': all(not d['cells'] and d['cursor_equal'] for d in differences),
              'full_parity': False,
              'coverage_complete': False,
              'limitations': ['pyte cell model is incomplete; raw streams retained',
                              'no physical-terminal screenshots yet',
                              'only Normal/Locked routing exercised'],
              'normalizations': [], 'differences': differences}
    (args.output / 'report.json').write_text(json.dumps(report, indent=2)+'\n')
    if args.require_parity and not (report['full_parity'] and report['coverage_complete']
                                    and report['modeled_cell_parity']
                                    and report['application_input_parity']):
        raise SystemExit('Parity gate incomplete or modeled/input differences remain; see report.json')


if __name__ == '__main__':
    if len(sys.argv) > 1 and sys.argv[1] == '--child':
        child(sys.argv[2])
    else:
        main()
