"""Startup PTY geometry and terminal-less serve contracts."""
import errno
import fcntl
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
import tty

from daily import eventually


# Use a non-default cell size so the startup handoff carries pixel geometry
# through the whole path instead of passing only the common 8x16 case.
CELL_WIDTH = 9
CELL_HEIGHT = 17


def dimensions(fd):
    rows, cols, xpixels, ypixels = struct.unpack(
        "HHHH", fcntl.ioctl(fd, termios.TIOCGWINSZ, bytes(8)))
    return {"rows": rows, "cols": cols,
            "xpixels": xpixels, "ypixels": ypixels}


def append_event(path, event, fd=0):
    record = {"event": event, "winsize": dimensions(fd)}
    with Path(path).open("a", encoding="utf-8") as output:
        output.write(json.dumps(record, sort_keys=True) + "\n")
        output.flush()


def child(events_path):
    tty.setraw(0, termios.TCSANOW)

    def on_winch(_signum, _frame):
        append_event(events_path, "WINCH")

    signal.signal(signal.SIGWINCH, on_winch)
    append_event(events_path, "FIRST")
    os.write(1, b"STARTUP-GEOMETRY\r\n")
    while True:
        try:
            data = os.read(0, 4096)
        except OSError as error:
            if error.errno == errno.EINTR:
                continue
            raise
        if not data:
            return
        os.write(1, b"READY\r\n")


def read_events(path):
    if not Path(path).exists():
        return []
    return [json.loads(line) for line in Path(path).read_text().splitlines()]


def env_for(root, config):
    return dict(os.environ, XDG_RUNTIME_DIR=str(root), EKKO_CONFIG=str(config),
                TERM="xterm-256color", COLORTERM="truecolor",
                LANG="C.UTF-8", LC_ALL="C.UTF-8")


def cli(binary, env, *args, ok=True):
    result = subprocess.run([binary, *args], env=env, capture_output=True,
                            timeout=8)
    assert (result.returncode == 0) == ok, (
        args, result.returncode, result.stderr.decode(errors="replace"))
    return result


def status(binary, env, name):
    return json.loads(cli(binary, env, "status", name).stdout)


def kill_process(process):
    if process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=3)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=3)


def drain(fd):
    while True:
        try:
            if not os.read(fd, 65536):
                return
        except BlockingIOError:
            return
        except OSError as error:
            if error.errno in (errno.EAGAIN, errno.EWOULDBLOCK, errno.EIO):
                return
            raise


def wait_for(predicate, master, timeout=8):
    deadline = time.monotonic() + timeout

    def check():
        drain(master)
        return predicate()

    return eventually(check, timeout=max(0.1, deadline - time.monotonic()))


def expected_pane_geometry(state):
    panes = state["panes"]
    assert len(panes) == 1, panes
    pane = panes[0]
    assert pane["cols"] >= 1 and pane["rows"] >= 1
    return pane["cols"], pane["rows"]


def expected_content_geometry(state, pane, expected_viewport=None):
    viewport = state["viewport"]
    insets = viewport["insets"]
    outer = pane["outer_rect"]
    pane_insets = state["geometry"]["pane-insets"]
    outer_width = outer[2]
    outer_height = outer[3]
    content_width = max(1, outer_width - pane_insets[1] - pane_insets[3])
    content_height = max(1, outer_height - pane_insets[0] - pane_insets[2])
    assert viewport["cols"] >= 5 and viewport["rows"] >= 4
    assert len(insets) == 4
    if expected_viewport is not None:
        assert (viewport["cols"], viewport["rows"]) == expected_viewport, (
            viewport, expected_viewport)
        expected_outer = [insets[3], insets[0],
                          expected_viewport[0] - insets[1] - insets[3],
                          expected_viewport[1] - insets[0] - insets[2]]
        assert outer == expected_outer, (outer, expected_outer, state)
    assert (pane["cols"], pane["rows"]) == (content_width, content_height), (
        state, pane)
    return content_width, content_height


def assert_first_geometry(events, pane, outer):
    assert events and events[0]["event"] == "FIRST", events
    first = events[0]["winsize"]
    assert (first["cols"], first["rows"]) == pane, (first, pane)
    assert (first["xpixels"], first["ypixels"]) == (
        first["cols"] * CELL_WIDTH, first["rows"] * CELL_HEIGHT), first
    assert (outer["xpixels"], outer["ypixels"]) == (
        outer["cols"] * CELL_WIDTH, outer["rows"] * CELL_HEIGHT), outer


def assert_child_first(events_path, pane, cell_width=CELL_WIDTH,
                       cell_height=CELL_HEIGHT):
    events = read_events(events_path)
    assert events and events[0]["event"] == "FIRST", (events_path, events)
    # A WINCH before the first observation means the child was born at a
    # provisional size and corrected after spawn.  Startup handoff must make
    # the first ioctl already equal to the layout's final pane rectangle.
    assert len(events) == 1, (events_path, events)
    first = events[0]["winsize"]
    assert (first["cols"], first["rows"]) == (pane["cols"], pane["rows"]), (
        events_path, first, pane)
    assert (first["xpixels"], first["ypixels"]) == (
        first["cols"] * cell_width, first["rows"] * cell_height), first


def assert_initial_l_geometry(inspected, cols, rows):
    """Require the public tree to be the requested 50/50 L split."""
    top, right, bottom, left = inspected["viewport"]["insets"]
    width = cols - left - right
    height = rows - top - bottom
    column_gap, row_gap = inspected["viewport"]["gaps"]
    left_width = (width - column_gap) // 2
    right_width = width - column_gap - left_width
    top_height = (height - row_gap) // 2
    bottom_height = height - row_gap - top_height
    expected_layout = [
        [left, top, left_width, height],
        [left + left_width + column_gap, top, right_width, top_height],
        [left + left_width + column_gap, top + top_height + row_gap,
         right_width, bottom_height],
    ]
    panes = sorted(inspected["panes"], key=lambda pane: pane["id"])
    assert [pane["layout_rect"] for pane in panes] == expected_layout, inspected
    for pane, expected in zip(panes, expected_layout):
        assert pane["outer_rect"] == expected, (pane, inspected)


def terminal_run(binary, env, name, events_path, cols, rows, profile_command,
                 check_split):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ,
                struct.pack("HHHH", rows, cols, cols * CELL_WIDTH,
                            rows * CELL_HEIGHT))
    outer = dimensions(slave)
    child_argv = [sys.executable, str(Path(__file__).resolve()),
                  "--child", str(events_path)]
    process = subprocess.Popen(
        [binary, "run", "--session", name, *profile_command, *child_argv],
        stdin=slave, stdout=slave, stderr=slave, env=env,
        start_new_session=True)
    os.close(slave)
    os.set_blocking(master, False)
    try:
        wait_for(lambda: read_events(events_path), master)
        # Check this before asking the daemon for status, so a delayed WINCH
        # cannot be mistaken for a clean first-size observation.
        startup_events = read_events(events_path)
        assert len(startup_events) == 1 and startup_events[0]["event"] == "FIRST", (
            startup_events)
        state = status(binary, env, name)
        inspected = json.loads(cli(binary, env, "inspect", name).stdout)
        assert (inspected["viewport"]["cols"], inspected["viewport"]["rows"]) == (cols, rows), inspected
        top, right, bottom, left = inspected["viewport"]["insets"]
        assert inspected["panes"][0]["outer_rect"] == [
            left, top, cols - left - right, rows - top - bottom], inspected
        pane = expected_pane_geometry(state)
        expected_content_geometry(inspected, inspected["panes"][0], (cols, rows))
        events = read_events(events_path)
        assert_first_geometry(events, pane, outer)
        assert len(events) == 1, events
        pids = [item["pid"] for item in state["panes"]]
        assert len(pids) == 1 and pids[0] > 0

        # Closing the viewer leaves the daemon and its child PTY alive.
        kill_process(process)
        state_after_viewer = status(binary, env, name)
        assert [item["pid"] for item in state_after_viewer["panes"]] == pids

        # Reattach through the public run path on a fresh PTY pair.  Reusing
        # the old master would make the client and this harness compete for
        # the same terminal input stream.
        os.close(master)
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ,
                    struct.pack("HHHH", rows, cols, cols * CELL_WIDTH,
                                rows * CELL_HEIGHT))
        reattach_outer = dimensions(slave)
        assert (reattach_outer["cols"], reattach_outer["rows"]) == (cols, rows)
        assert (reattach_outer["xpixels"], reattach_outer["ypixels"]) == (
            cols * CELL_WIDTH, rows * CELL_HEIGHT)
        process = subprocess.Popen(
            [binary, "run", "--session", name, *profile_command, *child_argv],
            stdin=slave, stdout=slave, stderr=slave, env=env,
            start_new_session=True)
        os.close(slave)
        os.set_blocking(master, False)
        resized_cols = cols - 10 if cols > 30 else cols + 5
        resized_rows = rows - 4 if rows > 10 else rows + 4
        fcntl.ioctl(master, termios.TIOCSWINSZ,
                    struct.pack("HHHH", resized_rows, resized_cols,
                                resized_cols * CELL_WIDTH,
                                resized_rows * CELL_HEIGHT))
        def resized_state():
            candidate = status(binary, env, name)
            candidate_pane = candidate["panes"][0]
            return candidate if (
                candidate_pane["cols"] != pane[0]
                or candidate_pane["rows"] != pane[1]) else None

        resized = wait_for(resized_state, master)
        new_pane = expected_pane_geometry(resized)
        assert [item["pid"] for item in resized["panes"]] == pids
        events = read_events(events_path)
        assert any(item["event"] == "WINCH" for item in events[1:]), events
        last = events[-1]["winsize"]
        assert (last["cols"], last["rows"]) == new_pane, (last, new_pane)
        assert (last["xpixels"], last["ypixels"]) == (
            last["cols"] * CELL_WIDTH, last["rows"] * CELL_HEIGHT), last

        if check_split:
            # The public split command must acquire the final child geometry
            # before spawning the second PTY.  The old fixed 80x24 spawn would
            # be visible in this child's very first ioctl on both large and
            # tiny requested viewports.
            split_events = Path(events_path).with_name("split.events")
            split_child = [sys.executable, str(Path(__file__).resolve()),
                           "--child", str(split_events)]
            cli(binary, env, "split", "--session", name, "columns", *split_child)

            def split_state_ready():
                candidate = status(binary, env, name)
                return candidate if len(candidate["panes"]) == 2 else None

            split_state = wait_for(split_state_ready, master)
            assert len(split_state["panes"]) == 2, split_state
            split_pids = [item["pid"] for item in split_state["panes"]]
            assert all(pid > 0 for pid in split_pids), split_state
            split_inspected = json.loads(cli(binary, env, "inspect", name).stdout)
            assert len(split_inspected["panes"]) == 2, split_inspected
            for split_pane in split_inspected["panes"]:
                expected_content_geometry(split_inspected, split_pane)
            assert_child_first(split_events, split_inspected["panes"][1])

            # A failed spawn must leave the logical tree, geometry and durable
            # child identities untouched.  The command error notice is allowed
            # to update separately, so compare only owned pane state and layout.
            before_failed = json.loads(cli(binary, env, "inspect", name).stdout)
            failed = cli(binary, env, "split", "--session", name, "columns",
                         "/definitely/missing/ekko-startup-child", ok=False)
            assert failed.returncode != 0
            after_failed = json.loads(cli(binary, env, "inspect", name).stdout)
            pane_keys = ("id", "pid", "cols", "rows", "x", "y", "outer_rect", "visible")
            before_panes = [{key: pane[key] for key in pane_keys}
                            for pane in before_failed["panes"]]
            after_panes = [{key: pane[key] for key in pane_keys}
                           for pane in after_failed["panes"]]
            assert after_panes == before_panes, (before_failed, after_failed)
            assert after_failed["layout"] == before_failed["layout"], (
                before_failed, after_failed)
            assert [pane["pid"] for pane in after_failed["panes"]] == split_pids
    finally:
        cli(binary, env, "stop", name)
        kill_process(process)
        try:
            os.close(master)
        except OSError:
            pass


def terminal_less(binary, env, name, events_path, profile_command):
    child_argv = [sys.executable, str(Path(__file__).resolve()),
                  "--child", str(events_path)]
    process = subprocess.Popen(
        [binary, "--serve", name, *profile_command, *child_argv],
        stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        env=env, start_new_session=True)
    try:
        eventually(lambda: read_events(events_path) or process.poll() is not None)
        assert process.poll() is None, process.stderr.read().decode(errors="replace")
        state = json.loads(cli(binary, env, "inspect", name).stdout)
        assert len(state["panes"]) == 1, state
        assert (state["viewport"]["cols"], state["viewport"]["rows"]) == (120, 36), state
        expected_content_geometry(state, state["panes"][0], (120, 36))
        # A detached server has no terminal query to provide cell pixels and
        # intentionally uses its documented 8x16 fallback.
        assert_child_first(events_path, state["panes"][0], 8, 16)
    finally:
        cli(binary, env, "stop", name)
        kill_process(process)


def initial_layout_run(binary, env, name, root, cols, rows):
    """Check declarative startup geometry before any post-spawn resize."""
    event_paths = [root / f"initial-{label}.events" for label in ("A", "B", "C")]
    child = [sys.executable, str(Path(__file__).resolve()), "--child"]
    commands = [[*child, str(path)] for path in event_paths]
    args = [binary, "run", "--session", name]
    for index, command in enumerate(commands):
        args.extend(command)
        if index != len(commands) - 1:
            args.append(":::")
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ,
                struct.pack("HHHH", rows, cols, cols * CELL_WIDTH,
                            rows * CELL_HEIGHT))
    process = subprocess.Popen(args, stdin=slave, stdout=slave, stderr=slave,
                               env=env, start_new_session=True)
    os.close(slave)
    os.set_blocking(master, False)
    try:
        wait_for(lambda: all(path.exists() for path in event_paths), master)
        # Inspect before asking for another terminal frame.  Any startup WINCH
        # would prove that a child was born with provisional geometry.
        inspected = json.loads(cli(binary, env, "inspect", name).stdout)
        assert (inspected["viewport"]["cols"], inspected["viewport"]["rows"]) == (
            cols, rows), inspected
        assert inspected["layout"] == ["columns", 50, 1, ["rows", 50, 2, 3]], inspected
        panes = sorted(inspected["panes"], key=lambda pane: pane["id"])
        assert [pane["id"] for pane in panes] == [1, 2, 3], inspected
        assert all(pane["pid"] > 0 for pane in panes), inspected
        assert_initial_l_geometry(inspected, cols, rows)
        for path, pane in zip(event_paths, panes):
            assert_child_first(path, pane)
        pids = [pane["pid"] for pane in panes]
        before_viewer = inspected

        # Reloading a changed or removed initializer must preserve the live
        # tree and PTYs.  A simultaneous pane-inset change proves that reload
        # still applies independent geometry options to the live layout.
        config_path = Path(env["EKKO_CONFIG"])
        original_config = config_path.read_text()
        initial_option = """(ekko/extensions:set-option :component :initial-layout-test :name :initial-layout
 :value '(:columns 50 1 (:rows 50 2 3)))
"""
        changed_option = initial_option.replace(
            "'(:columns 50 1 (:rows 50 2 3))",
            "'(:rows 50 1 (:columns 50 2 3))")
        assert original_config.count(initial_option) == 1, original_config
        changed_config = original_config.replace(initial_option, changed_option)
        changed_config += """(ekko/extensions:set-option :component :initial-layout-test :name :pane-insets
 :value '(0 0 0 0))
"""
        config_path.write_text(changed_config)
        old_generation = before_viewer["generation"]
        cli(binary, env, "config", "reload", name)

        def changed_reload():
            candidate = json.loads(cli(binary, env, "inspect", name).stdout)
            return candidate if candidate["generation"] > old_generation else None

        changed = wait_for(changed_reload, master)
        assert changed["layout"] == before_viewer["layout"], changed
        assert [pane["pid"] for pane in sorted(changed["panes"], key=lambda p: p["id"])] == pids
        assert changed["geometry"]["pane-insets"] == [0, 0, 0, 0], changed
        assert [(pane["cols"], pane["rows"]) for pane in
                sorted(changed["panes"], key=lambda p: p["id"])] != [
                    (pane["cols"], pane["rows"]) for pane in panes]

        removed_config = changed_config.replace(changed_option, "")
        config_path.write_text(removed_config)
        old_generation = changed["generation"]
        cli(binary, env, "config", "reload", name)

        def removed_reload():
            candidate = json.loads(cli(binary, env, "inspect", name).stdout)
            return candidate if candidate["generation"] > old_generation else None

        removed = wait_for(removed_reload, master)
        assert removed["layout"] == before_viewer["layout"], removed
        assert [pane["pid"] for pane in sorted(removed["panes"], key=lambda p: p["id"])] == pids
        assert removed["geometry"]["pane-insets"] == [0, 0, 0, 0], removed

        # Reattaching on a fresh PTY must preserve the declarative tree and all
        # child identities.  The new PTY is required so two clients never race
        # to consume the same input stream.
        kill_process(process)
        assert [pane["pid"] for pane in json.loads(
            cli(binary, env, "inspect", name).stdout)["panes"]] == pids
        os.close(master)
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ,
                    struct.pack("HHHH", rows, cols, cols * CELL_WIDTH,
                                rows * CELL_HEIGHT))
        process = subprocess.Popen(args, stdin=slave, stdout=slave, stderr=slave,
                                   env=env, start_new_session=True)
        os.close(slave)
        os.set_blocking(master, False)
        resized_cols = cols - 10 if cols > 30 else cols + 5
        resized_rows = rows - 4 if rows > 10 else rows + 4
        fcntl.ioctl(master, termios.TIOCSWINSZ,
                    struct.pack("HHHH", resized_rows, resized_cols,
                                resized_cols * CELL_WIDTH,
                                resized_rows * CELL_HEIGHT))

        def resized_state():
            candidate = json.loads(cli(binary, env, "inspect", name).stdout)
            return candidate if (candidate["viewport"]["cols"],
                                 candidate["viewport"]["rows"]) == (
                                     resized_cols, resized_rows) else None

        resized = wait_for(resized_state, master)
        assert resized["layout"] == before_viewer["layout"], resized
        resized_panes = sorted(resized["panes"], key=lambda pane: pane["id"])
        assert [pane["pid"] for pane in resized_panes] == pids, resized
        for path, pane in zip(event_paths, resized_panes):
            events = read_events(path)
            assert any(item["event"] == "WINCH" for item in events[1:]), events
            last = events[-1]["winsize"]
            assert (last["cols"], last["rows"]) == (pane["cols"], pane["rows"]), (
                path, last, pane)
            assert (last["xpixels"], last["ypixels"]) == (
                last["cols"] * CELL_WIDTH, last["rows"] * CELL_HEIGHT), last
    finally:
        cli(binary, env, "stop", name)
        kill_process(process)
        try:
            os.close(master)
        except OSError:
            pass


def invalid_startup_viewports(binary, env, root):
    for index, viewport in enumerate(((4, 24, CELL_WIDTH, CELL_HEIGHT),
                                      (80, 24, 129, CELL_HEIGHT),
                                      ("oops", 24, CELL_WIDTH, CELL_HEIGHT))):
        events = root / f"invalid-{index}.events"
        name = f"invalid-startup-{index}"
        process = subprocess.run(
            [binary, "--serve", name, "--viewport",
             *(str(value) for value in viewport), sys.executable,
             str(Path(__file__).resolve()), "--child", str(events)],
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, env=env, timeout=8)
        assert process.returncode != 0, process.stdout.decode(errors="replace")
        assert not events.exists(), (name, events)


def invalid_initial_layout(binary, env, root):
    """Invalid layout syntax/counts must fail before any child is spawned."""
    config_path = Path(env["EKKO_CONFIG"])
    original_config = config_path.read_text()
    marker = "'(:columns 50 1 (:rows 50 2 3))"
    assert original_config.count(marker) == 1, original_config
    try:
        cases = (("duplicate", "'(:columns 50 1 1)"),
                 # Structurally valid, but two leaves for three commands.
                 ("count-mismatch", "'(:columns 50 1 2)"))
        for suffix, replacement in cases:
            config_path.write_text(original_config.replace(marker, replacement))
            events = root / f"invalid-initial-layout-{suffix}.events"
            name = f"invalid-initial-layout-{suffix}"
            child_argv = [sys.executable, str(Path(__file__).resolve()),
                          "--child", str(events)]
            process = subprocess.run([binary, "--serve", name, *child_argv],
                                     stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                     stderr=subprocess.PIPE, env=env, timeout=8)
            assert process.returncode != 0, process.stdout.decode(errors="replace")
            assert not events.exists(), (name, events)
    finally:
        config_path.write_text(original_config)


def integration(binary, profile, bare, cols, rows):
    with tempfile.TemporaryDirectory(prefix="ekko-startup-geometry-") as directory:
        root = Path(directory)
        config = root / "init.lisp"
        profile_command = []
        split_command = '''
(in-package :cl-user)
(ekko/extensions:register-component :id :startup-geometry-test)
(ekko/extensions:register-command :component :startup-geometry-test :name "split-columns"
 :handler (lambda (snapshot event) (declare (ignore snapshot))
            (list (ekko/extensions:action
                    :split :axis :columns :argv (getf event :arguments)))))
'''
        if bare:
            config.write_text(split_command)
        else:
            profile = Path(profile).resolve()
            for helper in profile.parent.glob("zellij-*.lisp"):
                (root / helper.name).write_bytes(helper.read_bytes())
            config.write_text(profile.read_text() + split_command)
        env = env_for(root, config)
        initial_config = '''
(in-package :cl-user)
(ekko/extensions:register-component :id :initial-layout-test)
(ekko/extensions:set-option :component :initial-layout-test :name :initial-layout
 :value '(:columns 50 1 (:rows 50 2 3)))
'''
        initial_path = root / "initial-layout.lisp"
        initial_path.write_text(config.read_text() + initial_config)
        events = root / "run.events"
        terminal_run(binary, env, "startup-geometry", events, cols, rows,
                     profile_command, check_split=True)
        fallback_events = root / "fallback.events"
        terminal_less(binary, env, "terminal-less", fallback_events,
                      profile_command)
        initial_env = env_for(root, initial_path)
        invalid_initial_layout(binary, initial_env, root)
        initial_layout_run(binary, initial_env, "initial-layout", root, cols, rows)
        invalid_startup_viewports(binary, env, root)
    print(json.dumps({"status": "pass", "suite": "startup-geometry",
                      "bare": bare, "dimensions": [cols, rows]}))


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--child":
        child(sys.argv[2])
    elif len(sys.argv) == 6:
        integration(sys.argv[1], sys.argv[2], sys.argv[3] == "bare",
                    int(sys.argv[4]), int(sys.argv[5]))
    else:
        raise SystemExit(
            "usage: startup_geometry.py BINARY PROFILE regular|bare COLS ROWS")
