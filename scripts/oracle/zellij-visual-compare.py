"""Actual Kitty screenshots on a private compositor; differences are evidence."""
import argparse
import errno
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import select
import shutil
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
import tty


def save(path, value):
    Path(path).write_text(json.dumps(value, indent=2) + "\n")


def winsize():
    return list(struct.unpack("HHHH", fcntl.ioctl(0, termios.TIOCGWINSZ, bytes(8))))


def write_all(fd, data):
    while data:
        data = data[os.write(fd, data):]


def owned_processes(work):
    token = os.fsencode(str(work))
    found = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            command = (entry / "cmdline").read_bytes()
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            continue
        if token in command:
            found.append({"pid": int(entry.name),
                          "cmdline": command.replace(b"\0", b" ").decode("utf-8", "replace").strip()})
    return found


def wait_owned_processes(work, seconds=5):
    deadline = time.monotonic() + seconds
    remaining = owned_processes(work)
    while remaining and time.monotonic() < deadline:
        time.sleep(.05)
        remaining = owned_processes(work)
    return remaining


def fixture(path):
    tty.setraw(0)
    save(path, {"winsize_rows_cols_xpixels_ypixels": winsize()})
    os.write(1, b"\x1b[2J\x1b[Hfixture ready")
    with open(str(path) + ".input", "ab", buffering=0) as log:
        while data := os.read(0, 4096):
            log.write(data)


def workflow_event(path, label, event, data=b""):
    size = winsize()
    with open(path, "a", encoding="utf-8") as output:
        output.write(json.dumps({"event": event, "input_hex": data.hex(),
                                 "winsize": size}, sort_keys=True) + "\n")


def workflow_app(label, input_path, events_path):
    """Deterministic app used by each pane in the optional workflow capture."""
    tty.setraw(0)
    resized = False

    def on_winch(signum, frame):
        del signum, frame
        nonlocal resized
        resized = True

    signal.signal(signal.SIGWINCH, on_winch)
    workflow_event(events_path, label, "FIRST")
    os.write(1, f"WORKFLOW-{label} READY\r\n".encode())
    with open(input_path, "ab", buffering=0) as inputs:
        while True:
            if resized:
                workflow_event(events_path, label, "WINCH")
                resized = False
            ready, _, _ = select.select([0], [], [], .1)
            if not ready:
                continue
            data = os.read(0, 4096)
            if not data:
                return
            inputs.write(data)
            workflow_event(events_path, label, "READ", data)


def workflow_spawn():
    directory = Path(os.environ["WORKFLOW_EVENT_DIR"])
    # Child processes start concurrently after a split. Allocate a stable
    # per-workdir ordinal rather than exposing process IDs in fixture output.
    counter = directory / "spawn-counter"
    with open(counter, "a+", encoding="ascii") as stream:
        fcntl.flock(stream.fileno(), fcntl.LOCK_EX)
        stream.seek(0)
        text = stream.read().strip()
        ordinal = int(text) + 1 if text else 1
        stream.seek(0)
        stream.truncate()
        stream.write(str(ordinal))
        stream.flush()
        fcntl.flock(stream.fileno(), fcntl.LOCK_UN)
    label = "spawn-" + str(ordinal)
    workflow_app(label, directory / (label + ".input"),
                 directory / (label + ".events"))


def proxy(config_path):
    """Relay a real controlling PTY to Kitty, preserving replies and raw bytes."""
    config = json.loads(Path(config_path).read_text())
    work = Path(config_path).parent
    tty.setraw(0)
    size = winsize()
    save(work / "outer.json", {"winsize_rows_cols_xpixels_ypixels": size})
    pid, fd = pty.fork()
    if pid == 0:
        fcntl.ioctl(0, termios.TIOCSWINSZ, struct.pack("HHHH", *size))
        os.execvpe(config["argv"][0], config["argv"], config["env"])
    control = os.open(work / "control", os.O_RDWR | os.O_NONBLOCK)
    child_reaped = False

    def child_finished():
        nonlocal child_reaped
        _, status = os.waitpid(pid, 0)
        child_reaped = True
        save(work / "child-exit.json", {"wait_status": status,
                                        "exit_code": os.waitstatus_to_exitcode(status)})
        if config.get("hold_on_exit"):
            # Keep the exact final terminal image without adding shell output.
            # The private compositor owns this observer and closes it at cleanup.
            while True:
                ready, _, _ = select.select([0, control], [], [], .2)
                for source in ready:
                    if not os.read(source, 65536):
                        return

    try:
        with open(work / "output.ansi", "wb", buffering=0) as output, \
                open(work / "terminal-input.bin", "wb", buffering=0) as inputs:
            while True:
                ready, _, _ = select.select([0, fd, control], [], [], .2)
                for source in ready:
                    try:
                        data = os.read(source, 65536)
                    except OSError as error:
                        if error.errno == errno.EIO:
                            child_finished()
                            return
                        raise
                    if not data:
                        child_finished()
                        return
                    if source == fd:
                        output.write(data)
                        write_all(1, data)
                    else:
                        inputs.write(data)
                        write_all(fd, data)
    finally:
        os.close(control)
        os.close(fd)
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        if not child_reaped:
            os.waitpid(pid, 0)


def wait_for(predicate, process, seconds=15):
    deadline = time.monotonic() + seconds
    while not predicate():
        if process.poll() is not None:
            raise RuntimeError("private compositor exited before readiness")
        if time.monotonic() > deadline:
            raise RuntimeError("private terminal readiness timed out")
        time.sleep(.05)


def run_side(kind, args, root):
    work = root / "session"
    work.mkdir(mode=0o700)
    dest = args.output / kind
    dest.mkdir()
    for directory in ("runtime", "home", "config", "cache", "data"):
        (work / directory).mkdir(mode=0o700)
    env = {key: os.environ[key] for key in
           ("PATH", "LIBGL_DRIVERS_PATH", "__EGL_VENDOR_LIBRARY_FILENAMES", "FONTCONFIG_FILE")
           if key in os.environ}
    env.update(HOME=str(work / "home"), XDG_RUNTIME_DIR=str(work / "runtime"),
               XDG_CONFIG_HOME=str(work / "config"), XDG_CACHE_HOME=str(work / "cache"),
               XDG_DATA_HOME=str(work / "data"), WAYLAND_DISPLAY="wayland-0",
               LIBGL_ALWAYS_SOFTWARE="1", MESA_LOADER_DRIVER_OVERRIDE="llvmpipe",
               WLR_BACKENDS="headless", WLR_RENDERER="pixman", WLR_HEADLESS_OUTPUTS="1",
               WLR_LIBINPUT_NO_DEVICES="1", WLR_NO_HARDWARE_CURSORS="1",
               TERM="xterm-256color", COLORTERM="truecolor", LANG="C.UTF-8", LC_ALL="C.UTF-8")
    env["DBUS_SESSION_BUS_ADDRESS"] = "unix:path=" + str(work / "no-session-bus")
    fixture_argv = [sys.executable, str(Path(__file__).resolve()), "--fixture", str(work / "fixture.json")]
    if kind == "zellij":
        source = (args.reference / "default.kdl").read_text()
        if source.count("    pane\n") != 1:
            raise RuntimeError("reference layout central pane changed")
        replacement = "    pane command=" + json.dumps(fixture_argv[0]) + " {\n        args "
        replacement += " ".join(map(json.dumps, fixture_argv[1:])) + "\n    }\n"
        (work / "layout.kdl").write_text(source.replace("    pane\n", replacement))
        argv = [args.zellij, "--config", str(args.reference / "config.kdl"),
                "--new-session-with-layout", str(work / "layout.kdl"), "--session", "oracle"]
        stop = [args.zellij, "kill-session", "oracle"]
    else:
        argv = [args.ekko, "run", "--session", "oracle", *fixture_argv]
        stop = [args.ekko, "stop", "oracle"]
    child_env = dict(env)
    if kind == "ekko":
        child_env["EKKO_CONFIG"] = str(args.profile)
    save(work / "proxy.json", {"argv": argv, "env": child_env})
    save(dest / "conditions.json", {"environment": child_env, "fixture_argv": fixture_argv})
    os.mkfifo(work / "control", 0o600)
    command = ["cage", "-d", "--", "kitty", "--config", "NONE",
               "--override", "linux_display_server=wayland",
               "--override", "font_family=DejaVu Sans Mono", "--override", "font_size=16",
               "--override", "cursor_blink_interval=0", "--override", "remember_window_size=no",
               "--class", "ekko-zellij-visual", sys.executable, str(Path(__file__).resolve()),
               "--proxy", str(work / "proxy.json")]
    process = None
    stopped = None
    stop_error = None
    with open(dest / "cage.out", "wb") as stdout, open(dest / "cage.err", "wb") as stderr:
        try:
            process = subprocess.Popen(command, env=env, stdout=stdout, stderr=stderr)
            wait_for(lambda: (work / "fixture.json").exists(), process)
            renderer = subprocess.run(["eglinfo", "-B", "-p", "wayland"], env=env,
                                      capture_output=True, timeout=10, check=True)
            (dest / "eglinfo.txt").write_bytes(renderer.stdout)
            if b"OpenGL core profile renderer: llvmpipe" not in renderer.stdout:
                raise RuntimeError("private EGL renderer is not llvmpipe")
            for name, keys in (("startup", b""), ("escape", b"\x1b")):
                if keys:
                    fd = os.open(work / "control", os.O_WRONLY | os.O_NONBLOCK)
                    try:
                        os.write(fd, keys)
                    finally:
                        os.close(fd)
                time.sleep(1)
                if process.poll() is not None:
                    raise RuntimeError("private compositor exited before screenshot")
                subprocess.run(["grim", str(dest / (name + ".png"))], env=env,
                               capture_output=True, timeout=10, check=True)
                shutil.copyfile(work / "output.ansi", dest / (name + ".ansi"))
                save(dest / (name + ".json"), {
                    "sent_hex": keys.hex(), "fixture": json.loads((work / "fixture.json").read_text()),
                    "outer": json.loads((work / "outer.json").read_text()),
                    "input_hex": (work / "fixture.json.input").read_bytes().hex()
                    if (work / "fixture.json.input").exists() else ""})
        finally:
            try:
                try:
                    stopped = subprocess.run(stop, env=child_env, capture_output=True, timeout=10)
                    (dest / "stop.out").write_bytes(stopped.stdout)
                    (dest / "stop.err").write_bytes(stopped.stderr)
                except Exception as error:
                    stop_error = repr(error)
                    (dest / "stop.err").write_text(stop_error + "\n")
            finally:
                if process is not None and process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=5)
                remaining = wait_owned_processes(work)
                save(dest / "cleanup.json", {
                    "stop_exit_code": stopped.returncode if stopped is not None else None,
                    "stop_error": stop_error,
                    "remaining_pids": remaining,
                })
                for name in ("output.ansi", "terminal-input.bin", "outer.json", "fixture.json"):
                    if (work / name).exists():
                        shutil.copyfile(work / name, dest / name)
            if stop_error:
                raise RuntimeError(f"{kind} session cleanup command failed; runtime retained at {work}: {stop_error}")
            if stopped is None or stopped.returncode or remaining:
                raise RuntimeError(f"{kind} session cleanup failed; runtime retained at {work}")
    shutil.rmtree(work)


WORKFLOW_STAGES = (
    # The pinned Zellij config enables its welcome screen. Capture it first,
    # then dismiss it before workflow keys, matching the default comparator's
    # startup/Escape sequence. The delay lets the plugin finish mounting.
    ("startup-screen", b"", 1.0),
    ("startup", b"\x1b", 1.0),
    ("pane-enter", b"\x10", .4),
    ("direction-right", b"l", .4),
    ("fullscreen-on", b"f", .5),
    ("fullscreen-left-enter", b"\x10", .4),
    ("fullscreen-left", b"h", .5),
    ("restore", b"f", .6),
    # Settle each mode entry and key separately. A right-pane row split leaves
    # the newly focused child below the ten-row minimum, so the following
    # explicit d exercises the red flash without relying on batched input.
    ("prepare-enter", b"\x10", .4),
    ("prepare-right", b"l", .4),
    ("prepare-split", b"d", .8),
    ("failed-enter", b"\x10", .4),
    ("failed-split-flash", b"d", .3),
    ("failed-split-restored", b"", 1.2),
)

MOVE_STAGES = WORKFLOW_STAGES[:2] + (
    ("move-enter", b"\x08", .4),
    ("move-next", b"n", .5),
    ("move-back", b"p", .5),
    ("move-right", b"l", .5),
    ("move-down", b"j", .5),
    ("move-left", b"h", .5),
    ("move-up", b"k", .5),
    ("move-exit", b"\x1b", .5),
)

RENAME_STAGES = WORKFLOW_STAGES[:2] + (
    ("pane-enter", b"\x10", .4), ("rename-enter", b"c", .4),
    ("rename-first", b"ABC", .5), ("rename-commit", b"\r", .4),
    ("rename-pane", b"\x10", .4), ("rename-reenter", b"c", .4),
    ("rename-append", b"X", .5), ("rename-cancel", b"\x1b", .5),
)

UNICODE_TITLE_STAGES = WORKFLOW_STAGES[:2] + (
    ("pane-enter", b"\x10", .4), ("rename-enter-wide", b"c", .4),
    ("rename-wide", "界面".encode("utf-8"), .5),
    ("rename-commit-wide", b"\r", .5),
    ("rename-reenter-combining", b"\x10", .4),
    ("rename-enter-combining", b"c", .4),
    ("rename-clear-combining", b"\x7f" * 2, .4),
    ("rename-combining", "Cafe\u0301".encode("utf-8"), .5),
    ("rename-commit-combining", b"\r", .5),
    ("rename-reenter-long", b"\x10", .4), ("rename-enter-long", b"c", .4),
    ("rename-clear-long", b"\x7f" * 5, .4),
    ("rename-long", ("界Cafe\u0301-" * 12).encode("utf-8"), .7),
    ("rename-commit-long", b"\r", .5),
)
UNICODE_TITLE_PER_KEY_STAGES = WORKFLOW_STAGES[:2] + (
    ('pane-enter', b'\x10', .5),
    ('rename-enter-wide', b'c', .5),
    ('rename-wide', b'\xe7\x95\x8c\xe9\x9d\xa2', .5),
    ('rename-commit-wide', b'\r', .5),
    ('rename-reenter-combining', b'\x10', .5),
    ('rename-enter-combining', b'c', .5),
    ('rename-delete-wide-1', b'\x7f', .5),
    ('rename-delete-wide-2', b'\x7f', .5),
    ('rename-combining', b'Cafe\xcc\x81', .5),
    ('rename-commit-combining', b'\r', .5),
    ('rename-reenter-long', b'\x10', .5),
    ('rename-enter-long', b'c', .5),
    ('rename-delete-combining-1', b'\x7f', .5),
    ('rename-delete-combining-2', b'\x7f', .5),
    ('rename-delete-combining-3', b'\x7f', .5),
    ('rename-delete-combining-4', b'\x7f', .5),
    ('rename-delete-combining-5', b'\x7f', .5),
    ('rename-long', b'\xe7\x95\x8cCafe\xcc\x81-\xe7\x95\x8cCafe\xcc\x81-\xe7\x95\x8cCafe\xcc\x81-\xe7\x95\x8cCafe\xcc\x81-\xe7\x95\x8cCafe\xcc\x81-\xe7\x95\x8cCafe\xcc\x81-\xe7\x95\x8cCafe\xcc\x81-\xe7\x95\x8cCafe\xcc\x81-\xe7\x95\x8cCafe\xcc\x81-\xe7\x95\x8cCafe\xcc\x81-\xe7\x95\x8cCafe\xcc\x81-\xe7\x95\x8cCafe\xcc\x81-', .5),
    ('rename-commit-long', b'\r', .5),
)

EXIT_STAGES = WORKFLOW_STAGES[:2] + (("quit", b"\x11", .7),)

SESSION_STAGES = WORKFLOW_STAGES[:2] + (
    ("session-enter", b"\x0f", .4), ("session-unbound", b"q", .4),
    ("session-exit-toggle", b"\x0f", .4), ("session-reenter", b"\x0f", .4),
    ("session-to-pane", b"\x10", .4), ("session-from-pane", b"\x0f", .4),
    ("session-to-move", b"\x08", .4), ("session-from-move", b"\x0f", .4),
    ("session-exit-escape", b"\x1b", .4),
)

FRAME_STAGES = WORKFLOW_STAGES[:3] + (
    ("frames-off", b"z", .5), ("frames-pane", b"\x10", .4),
    ("frames-right", b"l", .4), ("frames-on", b"z", .5),
    ("frames-pane-again", b"\x10", .4), ("frames-off-again", b"z", .5),
    ("frames-zoom-pane", b"\x10", .4), ("frames-zoom", b"f", .5),
    ("frames-zoom-reenter", b"\x10", .4), ("frames-zoom-on", b"z", .5),
    ("frames-restore-pane", b"\x10", .4), ("frames-restore", b"f", .5),
)


def workflow_stages(args):
    return {"exit-workflow": EXIT_STAGES, "session-workflow": SESSION_STAGES, "frame-workflow": FRAME_STAGES, "move-workflow": MOVE_STAGES, "rename-workflow": RENAME_STAGES,
            "unicode-title-batched-workflow": UNICODE_TITLE_STAGES,
            "unicode-title-workflow": UNICODE_TITLE_PER_KEY_STAGES}.get(
        args.scenario, WORKFLOW_STAGES)



def workflow_paths(work):
    return {label: (work / (label + ".input"), work / (label + ".events"))
            for label in ("A", "B", "C")}


def write_workflow_shell(work):
    path = work / "workflow-shell"
    path.write_text("#!/usr/bin/env python3\n"
                    "import os, sys\n"
                    "os.execv(sys.executable, [sys.executable, "
                    + repr(str(Path(__file__).resolve()))
                    + ", '--workflow-spawn'])\n")
    path.chmod(0o700)
    return path


def write_workflow_layout(work, script, paths):
    def pane(label, indent):
        args = [str(script), "--workflow-fixture", label,
                str(paths[label][0]), str(paths[label][1])]
        pad = " " * indent
        return [pad + "pane command=" + json.dumps(sys.executable) + " {",
                pad + "    args " + " ".join(json.dumps(arg) for arg in args),
                pad + "}"]

    lines = ["layout {", '    pane split_direction="vertical" {']
    lines += pane("A", 8)
    lines += ['        pane split_direction="horizontal" {']
    lines += pane("B", 12)
    lines += pane("C", 12)
    lines += ["        }", "    }", "}", ""]
    path = work / "workflow.kdl"
    path.write_text("\n".join(lines))
    return path


def write_workflow_config(work, profile, shell):
    path = work / "workflow.lisp"
    path.write_text(
        "(load " + json.dumps(str(profile)) + ")\n"
        "(ekko/extensions:register-component :id :workflow-visual)\n"
        "(ekko/extensions:set-option :component :workflow-visual :name :shell :value '"
        + "(" + json.dumps(str(shell)) + "))\n"
        "(ekko/extensions:set-option :component :workflow-visual :name :viewport-insets :value '(0 0 0 0))\n"
        "(ekko/extensions:set-option :component :workflow-visual :name :initial-layout :value '(:columns 50 1 (:rows 50 2 3)))\n")
    return path


def workflow_fixture_snapshot(paths):
    paths = dict(paths)
    directory = next(iter(paths.values()))[0].parent
    for input_path in sorted(directory.glob("spawn-*.input")):
        label = input_path.stem
        paths.setdefault(label, (input_path, directory / (label + ".events")))
    result = {}
    for label, (input_path, events_path) in paths.items():
        result[label] = {
            "input_hex": input_path.read_bytes().hex() if input_path.exists() else "",
            "events": ([json.loads(line) for line in events_path.read_text().splitlines()]
                       if events_path.exists() else []),
        }
    return result


def workflow_stage_snapshot(work, paths, before):
    current = workflow_fixture_snapshot(paths)
    for label, value in current.items():
        old = before.get(label, {}).get("input_hex", "")
        value["input_delta_hex"] = value["input_hex"][len(old):]
    return current


def zellij_red_frame(path):
    from PIL import Image
    image = Image.open(path).convert("RGB")
    red = sum(1 for r, g, b in image.getdata()
              if r >= 120 and g <= 90 and b <= 90 and r > g + 50)
    return red >= 16


def zellij_flash_ready(work, dest, env, ansi_offset):
    """Detect the pinned red failed-split frame in the private screenshot."""
    output = work / "output.ansi"
    if output.exists() and b"CAN'T SPLIT!" in output.read_bytes()[ansi_offset:]:
        return True
    probe = dest / "failed-split-probe.png"
    try:
        subprocess.run(["grim", str(probe)], env=env, capture_output=True,
                       timeout=10, check=True)
        from PIL import Image
        image = Image.open(probe).convert("RGB")
        return zellij_red_frame(probe)
    except (OSError, subprocess.SubprocessError):
        return False


def run_workflow_side(kind, args, root):
    work = root / "session"
    work.mkdir(mode=0o700)
    dest = args.output / kind
    dest.mkdir()
    for directory in ("runtime", "home", "config", "cache", "data"):
        (work / directory).mkdir(mode=0o700)
    paths = workflow_paths(work)
    env = {key: os.environ[key] for key in
           ("PATH", "LIBGL_DRIVERS_PATH", "__EGL_VENDOR_LIBRARY_FILENAMES", "FONTCONFIG_FILE")
           if key in os.environ}
    env.update(HOME=str(work / "home"), XDG_RUNTIME_DIR=str(work / "runtime"),
               XDG_CONFIG_HOME=str(work / "config"), XDG_CACHE_HOME=str(work / "cache"),
               XDG_DATA_HOME=str(work / "data"), WAYLAND_DISPLAY="wayland-0",
               LIBGL_ALWAYS_SOFTWARE="1", MESA_LOADER_DRIVER_OVERRIDE="llvmpipe",
               WLR_BACKENDS="headless", WLR_RENDERER="pixman", WLR_HEADLESS_OUTPUTS="1",
               WLR_LIBINPUT_NO_DEVICES="1", WLR_NO_HARDWARE_CURSORS="1",
               TERM="xterm-256color", COLORTERM="truecolor", LANG="C.UTF-8", LC_ALL="C.UTF-8",
               DBUS_SESSION_BUS_ADDRESS="unix:path=" + str(work / "no-session-bus"),
               WORKFLOW_EVENT_DIR=str(work))
    shell = write_workflow_shell(work)
    script = Path(__file__).resolve()
    if kind == "zellij":
        config = work / "config.kdl"
        config.write_text((args.reference / "config.kdl").read_text()
                          + "\ndefault_shell " + json.dumps(str(shell)) + "\n")
        layout = write_workflow_layout(work, script, paths)
        argv = [args.zellij, "--config", str(config), "--new-session-with-layout",
                str(layout), "--session", "workflow"]
        stop = [args.zellij, "kill-session", "workflow"]
    else:
        config = write_workflow_config(work, args.profile.resolve(), shell)
        env["EKKO_CONFIG"] = str(config)
        argv = [args.ekko, "run", "--session", "workflow"]
        for index, label in enumerate(("A", "B", "C")):
            argv.extend([sys.executable, str(script), "--workflow-fixture", label,
                         str(paths[label][0]), str(paths[label][1])])
            if index != 2:
                argv.append(":::")
        stop = [args.ekko, "stop", "workflow"]
    fixture_argv = {label: [sys.executable, str(script), "--workflow-fixture", label,
                            str(paths[label][0]), str(paths[label][1])]
                    for label in ("A", "B", "C")}
    save(dest / "conditions.json", {"environment": env,
                                     "fixture_argv": fixture_argv,
                                     "workflow": True,
                                     "shell": str(shell), "argv": argv})
    os.mkfifo(work / "control", 0o600)
    command = ["cage", "-d", "--", "kitty", "--config", "NONE",
               "--listen-on", "unix:" + str(work / "kitty-control"),
               "--override", "allow_remote_control=socket-only",
               "--override", "linux_display_server=wayland",
               "--override", "font_family=DejaVu Sans Mono", "--override", "font_size=16",
               "--override", "cursor_blink_interval=0", "--override", "remember_window_size=no",
               "--class", "ekko-zellij-visual", sys.executable, str(script),
               "--proxy", str(work / "proxy.json")]
    child_env = dict(env)
    save(work / "proxy.json", {"argv": argv, "env": child_env, "hold_on_exit": args.scenario == "exit-workflow"})
    process = None
    stopped = None
    stop_error = None
    stages = []
    before = {}
    with open(dest / "cage.out", "wb") as stdout, open(dest / "cage.err", "wb") as stderr:
        try:
            process = subprocess.Popen(command, env=env, stdout=stdout, stderr=stderr)
            wait_for(lambda: all(pair[1].exists() for pair in paths.values()), process)
            renderer = subprocess.run(["eglinfo", "-B", "-p", "wayland"], env=env,
                                      capture_output=True, timeout=10, check=True)
            (dest / "eglinfo.txt").write_bytes(renderer.stdout)
            if b"OpenGL core profile renderer: llvmpipe" not in renderer.stdout:
                raise RuntimeError("private EGL renderer is not llvmpipe")
            for name, keys, delay in workflow_stages(args):
                flash_ansi_offset = ((work / "output.ansi").stat().st_size
                                     if name == "failed-split-flash" and
                                     (work / "output.ansi").exists() else 0)
                sent_at = time.monotonic()
                if keys:
                    fd = os.open(work / "control", os.O_WRONLY | os.O_NONBLOCK)
                    try:
                        os.write(fd, keys)
                    finally:
                        os.close(fd)
                flash_wait = None
                flash_signal = False
                if name == "failed-split-flash":
                    def flash_ready():
                        if kind == "zellij":
                            return zellij_flash_ready(work, dest, child_env,
                                                      flash_ansi_offset)
                        result = subprocess.run([args.ekko, "inspect", "workflow"],
                                                env=child_env, capture_output=True, timeout=8)
                        if result.returncode:
                            return False
                        state = json.loads(result.stdout)
                        return any(note.get("text") == "CAN'T SPLIT!"
                                   for note in (state.get("pane-notes") or []))
                    flash_started = time.monotonic()
                    deadline = flash_started + 4
                    while True:
                        if flash_ready():
                            flash_signal = True
                            break
                        if process.poll() is not None:
                            raise RuntimeError("private compositor exited before workflow flash")
                        if time.monotonic() > deadline:
                            raise RuntimeError(
                                f"{kind} failed-split frame was not observed; "
                                f"artifacts retained at {dest}")
                        time.sleep(.05)
                    flash_wait = time.monotonic() - flash_started
                    # Keep the capture close to the requested 300ms flash
                    # point when the actual signal arrived earlier.
                    if delay > flash_wait:
                        time.sleep(delay - flash_wait)
                else:
                    time.sleep(delay)
                if process.poll() is not None:
                    raise RuntimeError("private compositor exited before workflow screenshot")
                if name == "quit":
                    wait_for(lambda: (work / "child-exit.json").exists(), process)
                    child_exit = json.loads((work / "child-exit.json").read_text())
                    if child_exit["exit_code"] != 0:
                        raise RuntimeError(f"{kind} quit failed: {child_exit}")
                captured_at = time.monotonic()
                subprocess.run(["grim", str(dest / (name + ".png"))], env=env,
                               capture_output=True, timeout=10, check=True)
                exported = subprocess.run(
                    ["kitty", "@", "--to", "unix:" + str(work / "kitty-control"),
                     "get-text", "--extent", "screen", "--ansi", "--add-cursor", "--add-wrap-markers"],
                    env=env, capture_output=True, timeout=10, check=True)
                (dest / (name + ".kitty-text.ansi")).write_bytes(exported.stdout)
                if kind == "zellij" and name == "failed-split-flash":
                    if not zellij_red_frame(dest / (name + ".png")):
                        raise RuntimeError(
                            f"{kind} failed-split capture lost its red frame; "
                            f"artifacts retained at {dest}")
                if kind == "zellij" and name == "failed-split-restored":
                    if zellij_red_frame(dest / (name + ".png")):
                        raise RuntimeError(
                            f"{kind} failed-split frame did not restore; "
                            f"artifacts retained at {dest}")
                shutil.copyfile(work / "output.ansi", dest / (name + ".ansi"))
                inspect = status = None
                if kind == "ekko" and name != "quit":
                    inspect_result = subprocess.run([args.ekko, "inspect", "workflow"], env=child_env,
                                                    capture_output=True, timeout=8, check=True)
                    status_result = subprocess.run([args.ekko, "status", "workflow"], env=child_env,
                                                   capture_output=True, timeout=8, check=True)
                    inspect = json.loads(inspect_result.stdout)
                    status = json.loads(status_result.stdout)
                fixture = workflow_stage_snapshot(work, paths, before)
                if name == "prepare-split":
                    spawned = [value for label, value in fixture.items()
                               if label.startswith("spawn-")]
                    small = [value for value in spawned
                             if any(event.get("event") == "FIRST" and
                                    event.get("winsize", [0])[0] < 10
                                    for event in value["events"])]
                    if len(spawned) != 1 or len(small) != 1:
                        raise RuntimeError(
                            f"{kind} preparation did not create one small spawned pane; "
                            f"artifacts retained at {dest}")
                save(dest / (name + ".json"), {"sent_hex": keys.hex(), "fixture": fixture,
                                                "inspect": inspect, "status": status,
                                                "timing": {"delay_seconds": delay,
                                                           "flash_wait_seconds": flash_wait,
                                                           "flash_signal": flash_signal,
                                                           "sent_monotonic": sent_at,
                                                           "captured_monotonic": captured_at},
                                                "outer": json.loads((work / "outer.json").read_text())})
                stages.append(name)
                before = fixture
        finally:
            try:
                try:
                    stopped = subprocess.run(stop, env=child_env, capture_output=True, timeout=10)
                    (dest / "stop.out").write_bytes(stopped.stdout)
                    (dest / "stop.err").write_bytes(stopped.stderr)
                except Exception as error:
                    stop_error = repr(error)
                    (dest / "stop.err").write_text(stop_error + "\n")
            finally:
                if process is not None and process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=5)
                remaining = wait_owned_processes(work)
                save(dest / "cleanup.json", {"stop_exit_code": stopped.returncode if stopped else None,
                                              "stop_error": stop_error, "remaining_pids": remaining})
                for name in ("output.ansi", "terminal-input.bin", "outer.json", "child-exit.json"):
                    if (work / name).exists():
                        shutil.copyfile(work / name, dest / name)
            exited_cleanly = (args.scenario == "exit-workflow"
                              and (work / "child-exit.json").exists()
                              and json.loads((work / "child-exit.json").read_text())["exit_code"] == 0)
            if stop_error or stopped is None or (stopped.returncode and not exited_cleanly) or remaining:
                raise RuntimeError(f"{kind} workflow cleanup failed; runtime retained at {work}")
    shutil.rmtree(work)
    return stages


def pyte_snapshot(ansi_path, outer_path):
    import pyte

    outer = json.loads(Path(outer_path).read_text())
    rows, cols, _, _ = outer["winsize_rows_cols_xpixels_ypixels"]
    screen = pyte.Screen(cols, rows)
    pyte.Stream(screen).feed(Path(ansi_path).read_bytes().decode("utf-8", "replace"))

    cells = []
    for row in range(rows):
        cells.append([screen.buffer[row][column]._asdict() for column in range(cols)])
    return {"rows": rows, "cols": cols, "cells": cells,
            "cursor": {"x": screen.cursor.x, "y": screen.cursor.y,
                       "hidden": screen.cursor.hidden}}


def compare_cells(zellij, ekko):
    differences = []
    for row, (zellij_row, ekko_row) in enumerate(zip(zellij["cells"], ekko["cells"])):
        for column, (zellij_cell, ekko_cell) in enumerate(zip(zellij_row, ekko_row)):
            if zellij_cell != ekko_cell:
                differences.append({"row": row, "column": column,
                                    "zellij": zellij_cell, "ekko": ekko_cell})
    return {"dimensions_equal": (zellij["rows"], zellij["cols"]) == (ekko["rows"], ekko["cols"]),
            "cursor_equal": zellij["cursor"] == ekko["cursor"],
            "differing_cells": len(differences), "differences": differences}


def workflow_report(args, root):
    from PIL import Image, ImageChops

    zellij_conditions = json.loads((args.output / "zellij" / "conditions.json").read_text())
    ekko_conditions = json.loads((args.output / "ekko" / "conditions.json").read_text())
    if zellij_conditions["fixture_argv"] != ekko_conditions["fixture_argv"]:
        raise RuntimeError("workflow fixture argv differs between Zellij and Ekko")
    zellij_environment = dict(zellij_conditions["environment"])
    ekko_environment = dict(ekko_conditions["environment"])
    zellij_environment.pop("EKKO_CONFIG", None)
    ekko_environment.pop("EKKO_CONFIG", None)
    if zellij_environment != ekko_environment:
        raise RuntimeError("workflow fixture environment differs between Zellij and Ekko")
    checkpoints = []
    for name, _, _ in workflow_stages(args):
        z = Image.open(args.output / "zellij" / (name + ".png")).convert("RGB")
        e = Image.open(args.output / "ekko" / (name + ".png")).convert("RGB")
        if z.size != (1280, 720) or e.size != z.size:
            raise RuntimeError("workflow capture pixel dimensions mismatch")
        zs = json.loads((args.output / "zellij" / (name + ".json")).read_text())
        es = json.loads((args.output / "ekko" / (name + ".json")).read_text())
        if zs["outer"] != es["outer"]:
            raise RuntimeError("workflow outer terminal dimensions mismatch")
        zgrid = pyte_snapshot(args.output / "zellij" / (name + ".ansi"),
                              args.output / "zellij" / "outer.json")
        egrid = pyte_snapshot(args.output / "ekko" / (name + ".ansi"),
                              args.output / "ekko" / "outer.json")
        save(args.output / "zellij" / (name + ".pyte.json"), zgrid)
        save(args.output / "ekko" / (name + ".pyte.json"), egrid)
        cells = compare_cells(zgrid, egrid)
        cell_file = name + "-cell-report.json"
        save(args.output / cell_file, cells)
        difference = ImageChops.difference(z, e)
        difference.save(args.output / (name + "-difference.png"))
        differing_pixels = sum(pixel != (0, 0, 0) for pixel in difference.getdata())
        kitty_text = {side: (args.output / side / (name + ".kitty-text.ansi")).read_bytes()
                      for side in ("zellij", "ekko")}
        checkpoints.append({"stage": name, "differing_pixels": differing_pixels,
                            "kitty_text_and_cursor_equal": kitty_text["zellij"] == kitty_text["ekko"],
                            "kitty_text_files": {side: side + "/" + name + ".kitty-text.ansi" for side in kitty_text},
                            "same_pixels": differing_pixels == 0,
                            "cell_report": {"file": cell_file,
                                            "dimensions_equal": cells["dimensions_equal"],
                                            "cursor_equal": cells["cursor_equal"],
                                            "differing_cells": cells["differing_cells"]},
                            "zellij": zs, "ekko": es})
    report = {"scenario": args.scenario, "capture_complete": True,
              "full_parity": False, "coverage_complete": False,
              "workflow_coverage_complete": True,
              "normalizations": [], "checkpoints": checkpoints,
              "limitations": ["workflow checkpoints settle at fixed delays",
                              "hardware display latency is not measured"],
              "cleanup": {kind: json.loads((args.output / kind / "cleanup.json").read_text())
                          for kind in ("zellij", "ekko")}}
    save(args.output / "report.json", report)
    print(json.dumps(report))
    if args.require_parity:
        raise SystemExit("Pane workflow parity remains incomplete")


def main():
    from PIL import Image, ImageChops
    parser = argparse.ArgumentParser()
    parser.add_argument("--zellij", required=True)
    parser.add_argument("--ekko", required=True)
    parser.add_argument("--profile", type=Path, required=True)
    parser.add_argument("--reference", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--scenario", choices=("startup", "exit-workflow", "session-workflow", "frame-workflow", "pane-workflow", "move-workflow", "rename-workflow", "unicode-title-workflow", "unicode-title-batched-workflow"), default="startup")
    parser.add_argument("--require-parity", action="store_true")
    args = parser.parse_args()
    args.output = args.output.resolve()
    args.profile = args.profile.resolve()
    args.reference = args.reference.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    pin = json.loads((args.reference / "pin.json").read_text())
    version = subprocess.check_output([args.zellij, "--version"]).decode().strip()
    if version != "zellij " + pin["release"]:
        raise RuntimeError("reference version mismatch")
    for name, digest in pin["files"].items():
        if hashlib.sha256((args.reference / name).read_bytes()).hexdigest() != digest:
            raise RuntimeError("reference fixture hash mismatch")
    font = subprocess.check_output(["fc-match", "-f", "%{family}|%{style}|%{file}", "DejaVu Sans Mono"])
    (args.output / "font-match.txt").write_bytes(font)
    if not font.startswith(b"DejaVu Sans Mono|Book|"):
        raise RuntimeError("font mismatch")
    root = Path(tempfile.mkdtemp(prefix="ekko-zellij-visual-"))
    # Retain runtime state on failure so cleanup can be diagnosed/retried.
    if args.scenario in ("exit-workflow", "session-workflow", "frame-workflow", "pane-workflow", "move-workflow", "rename-workflow", "unicode-title-workflow", "unicode-title-batched-workflow"):
        for kind in ("zellij", "ekko"):
            run_workflow_side(kind, args, root)
        workflow_report(args, root)
        root.rmdir()
        return
    for kind in ("zellij", "ekko"):
        run_side(kind, args, root)
    zellij_conditions = json.loads((args.output / "zellij" / "conditions.json").read_text())
    ekko_conditions = json.loads((args.output / "ekko" / "conditions.json").read_text())
    if zellij_conditions["fixture_argv"] != ekko_conditions["fixture_argv"]:
        raise RuntimeError("fixture argv differs between Zellij and Ekko")
    zellij_environment = dict(zellij_conditions["environment"])
    ekko_environment = dict(ekko_conditions["environment"])
    zellij_environment.pop("EKKO_CONFIG", None)
    ekko_environment.pop("EKKO_CONFIG", None)
    if zellij_environment != ekko_environment:
        raise RuntimeError("fixture environment differs between Zellij and Ekko")
    root.rmdir()
    checkpoints = []
    cell_reports = []
    for name in ("startup", "escape"):
        z = Image.open(args.output / "zellij" / (name + ".png")).convert("RGB")
        e = Image.open(args.output / "ekko" / (name + ".png")).convert("RGB")
        if z.size != (1280, 720) or e.size != z.size:
            raise RuntimeError("capture pixel dimensions mismatch")
        zs = json.loads((args.output / "zellij" / (name + ".json")).read_text())
        es = json.loads((args.output / "ekko" / (name + ".json")).read_text())
        if zs["outer"] != es["outer"]:
            raise RuntimeError("outer terminal dimensions mismatch")
        zgrid = pyte_snapshot(args.output / "zellij" / (name + ".ansi"),
                              args.output / "zellij" / "outer.json")
        egrid = pyte_snapshot(args.output / "ekko" / (name + ".ansi"),
                              args.output / "ekko" / "outer.json")
        save(args.output / "zellij" / (name + ".pyte.json"), zgrid)
        save(args.output / "ekko" / (name + ".pyte.json"), egrid)
        cells = compare_cells(zgrid, egrid)
        cell_report_path = name + "-cell-report.json"
        save(args.output / cell_report_path, cells)
        cell_reports.append({"stage": name, "file": cell_report_path,
                             "dimensions_equal": cells["dimensions_equal"],
                             "cursor_equal": cells["cursor_equal"],
                             "differing_cells": cells["differing_cells"]})
        difference = ImageChops.difference(z, e)
        difference.save(args.output / (name + "-difference.png"))
        count = sum(pixel != (0, 0, 0) for pixel in difference.getdata())
        checkpoints.append({"stage": name, "differing_pixels": count,
                            "same_pixels": count == 0,
                            "cell_report": {"file": cell_report_path,
                                            "dimensions_equal": cells["dimensions_equal"],
                                            "cursor_equal": cells["cursor_equal"],
                                            "differing_cells": cells["differing_cells"]},
                            "zellij": zs, "ekko": es})
    report = {"capture_complete": True, "full_parity": False, "coverage_complete": False,
              "normalizations": [], "checkpoints": checkpoints,
              "cell_reports": cell_reports,
              "limitations": ["startup and Escape only", "settling interval is one second",
                              "hardware display latency is not measured"]}
    save(args.output / "report.json", report)
    print(json.dumps(report))
    if args.require_parity:
        raise SystemExit("Full parity coverage remains incomplete")


if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--fixture":
        fixture(Path(sys.argv[2]))
    elif len(sys.argv) == 3 and sys.argv[1] == "--proxy":
        proxy(sys.argv[2])
    elif len(sys.argv) == 5 and sys.argv[1] == "--workflow-fixture":
        workflow_app(sys.argv[2], sys.argv[3], sys.argv[4])
    elif len(sys.argv) == 2 and sys.argv[1] == "--workflow-spawn":
        workflow_spawn()
    else:
        main()
