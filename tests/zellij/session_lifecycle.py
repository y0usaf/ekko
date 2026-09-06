"""Pinned quit/detach observations with real children and restored host termios."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import termios
import time

from pane_workflow_differential import (
    QueryTerminal, app_paths, child_args, env_for, fixture_state, read_bytes,
    read_events, verify_reference, wait_for, write_ekko_config, write_spawn_shell,
    write_zellij_layout,
)

MODES = {'normal': [], 'pane': [b'\x10'], 'move': [b'\x08'],
         'rename': [b'\x10', b'c'], 'session': [b'\x0f']}


def alive(pid):
    try:
        return Path(f'/proc/{pid}/stat').read_text().rsplit(')', 1)[1].split()[0] != 'Z'
    except FileNotFoundError:
        return False


def terminal_flags(flags):
    return [*flags[:6], [item.hex() if isinstance(item, bytes) else item for item in flags[6]]]


def run_side(kind, binary, args, root, scenario):
    work = root / 'session'
    if work.exists():
        shutil.rmtree(work)
    work.mkdir()
    shell = write_spawn_shell(root)
    paths = app_paths(work)
    env = env_for(work)
    if kind == 'zellij':
        config = work / 'config.kdl'
        config.write_text((args.reference / 'config.kdl').read_text())
        layout = write_zellij_layout(work, shell, paths)
        argv = [binary, '--config', str(config), '--new-session-with-layout',
                str(layout), '--session', 'workflow']
        attach = [binary, '--config', str(config), 'attach', 'workflow']
        stop = [binary, 'kill-session', 'workflow']
    else:
        config = write_ekko_config(work, args.profile, shell)
        env['EKKO_CONFIG'] = str(config)
        argv = [binary, 'run', '--session', 'workflow']
        for i, label in enumerate(paths):
            if i:
                argv.append(':::')
            argv += child_args(shell, paths, label)
        attach = [binary, 'attach', 'workflow']
        stop = [binary, 'stop', 'workflow']
    output = args.output / scenario / kind
    output.mkdir(parents=True, exist_ok=True)
    terminal = QueryTerminal(argv, env, 80, 24)
    checkpoints = []
    result = {'binary': binary, 'argv': argv, 'attach_argv': attach,
              'fixture_argv': {label: child_args(shell, paths, label) for label in paths},
              'environment': {key: value for key, value in env.items() if key != 'EKKO_CONFIG'},
              'checkpoints': checkpoints}
    pids = []

    def record(name, sent=b''):
        value = {'name': name, 'sent_hex': sent.hex(), **terminal.snapshot(),
                 'fixture': fixture_state(paths), 'client_exit': terminal.process.poll()}
        if kind == 'ekko' and terminal.process.poll() is None:
            inspection = subprocess.run([binary, 'inspect', 'workflow'], env=env,
                                        capture_output=True, timeout=5)
            if inspection.returncode == 0:
                value['inspect'] = json.loads(inspection.stdout)
        checkpoints.append(value)
        (output / (name + '.ansi')).write_bytes(terminal.raw)
        return value

    def send(name, data):
        os.write(terminal.fd, data)
        terminal.pump(.4)
        return record(name, data)

    def exited(name):
        wait_for(lambda: terminal.process.poll() is not None, terminal)
        terminal.pump(.1)
        before = terminal_flags(terminal.termios_before)
        after = terminal_flags(termios.tcgetattr(terminal.fd))
        result[name] = {'termios_before': before, 'termios_after': after,
                        'termios_restored': before == after,
                        'exit_code': terminal.process.returncode}
        record(name)
        assert result[name]['termios_restored'], result[name]
        assert terminal.process.returncode == 0, result[name]

    try:
        wait_for(lambda: all(read_events(pair[1]) for pair in paths.values()), terminal)
        pids = [read_events(pair[1])[0]['pid'] for pair in paths.values()]
        result['pids'] = pids
        terminal.pump(1)
        record('startup')
        send('dismiss-release-notes', b'\x1b')
        if scenario == 'detach':
            # Locked Ctrl-q and Ctrl-o must be literal child input.
            send('lock', b'\x07')
            before = read_bytes(paths['A'][0])
            send('locked-quit', b'\x11')
            send('locked-session', b'\x0f')
            wait_for(lambda: read_bytes(paths['A'][0]) == before + b'\x11\x0f', terminal)
            assert all(alive(pid) for pid in pids)
            send('unlock', b'\x07')
            send('session-enter', b'\x0f')
            before = read_bytes(paths['A'][0])
            send('detach-key', b'd')
            exited('detached')
            assert read_bytes(paths['A'][0]) == before
            result['children_survive_detach'] = all(alive(pid) for pid in pids)
            assert result['children_survive_detach']
            terminal.close()
            terminal = QueryTerminal(attach, env, 80, 24)
            terminal.pump(1)
            record('reattached')
            before = read_bytes(paths['A'][0])
            send('reattach-input', b'm')
            wait_for(lambda: read_bytes(paths['A'][0]) == before + b'm', terminal)
            assert [read_events(pair[1])[0]['pid'] for pair in paths.values()] == pids
        else:
            for index, key in enumerate(MODES[scenario.removeprefix('quit-')]):
                send(f'mode-{index}', key)
        before = {label: read_bytes(pair[0]) for label, pair in paths.items()}
        send('quit-key', b'\x11')
        exited('quit')
        wait_for(lambda: not any(alive(pid) for pid in pids), terminal)
        result['children_exit_on_quit'] = True
        result['quit_not_forwarded'] = all(read_bytes(paths[label][0]) == old for label, old in before.items())
        assert result['quit_not_forwarded']
        result['functional_slice_passed'] = True
    finally:
        result['raw_output'] = 'final.ansi'
        (output / 'final.ansi').write_bytes(terminal.raw)
        terminal.close()
        cleanup = subprocess.run(stop, env=env, capture_output=True, timeout=8)
        deadline = time.monotonic() + 3
        while any(alive(pid) for pid in pids) and time.monotonic() < deadline:
            time.sleep(.02)
        result['cleanup'] = {'exit': cleanup.returncode,
                             'stderr': cleanup.stderr.decode(errors='replace'),
                             'owned_live_child_pids': [pid for pid in pids if alive(pid)]}
        (output / 'report.json').write_text(json.dumps(result, indent=2) + '\n')
        assert not result['cleanup']['owned_live_child_pids'], result['cleanup']
    return result


def main():
    parser = argparse.ArgumentParser()
    for flag in ('zellij', 'ekko'):
        parser.add_argument('--' + flag, required=True)
    for flag in ('profile', 'reference', 'output'):
        parser.add_argument('--' + flag, type=Path, required=True)
    parser.add_argument('--only', action='append', choices=['detach', *('quit-' + m for m in MODES)])
    args = parser.parse_args()
    args.profile = args.profile.resolve()
    args.reference = args.reference.resolve()
    verify_reference(args.zellij, args.reference)
    report = {'full_parity': False, 'coverage_complete': False, 'scenarios': {}}
    with tempfile.TemporaryDirectory(prefix='ekko-session-lifecycle-') as temp:
        for scenario in args.only or ['detach', *('quit-' + m for m in MODES)]:
            pair = [run_side(kind, binary, args, Path(temp), scenario)
                    for kind, binary in [('zellij', args.zellij), ('ekko', args.ekko)]]
            assert pair[0]['fixture_argv'] == pair[1]['fixture_argv']
            assert pair[0]['environment'] == pair[1]['environment']
            diffs = []
            for z, e in zip(pair[0]['checkpoints'], pair[1]['checkpoints'], strict=True):
                assert z['name'] == e['name']
                diffs.append({'stage': z['name'], 'input_equal': z['fixture']['inputs'] == e['fixture']['inputs'],
                              'cursor_equal': z['cursor'] == e['cursor'],
                              'differing_cells': sum(a != b for zr, er in zip(z['cells'], e['cells'])
                                                     for a, b in zip(zr, er))})
            report['scenarios'][scenario] = {'functional_slice_passed': all(r['functional_slice_passed'] for r in pair),
                                             'differences': diffs}
    (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps({'functional_slice_passed': True, 'scenarios': list(report['scenarios']),
                      'full_parity': False, 'output': str(args.output)}))


if __name__ == '__main__':
    main()
