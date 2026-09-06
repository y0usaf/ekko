#!/run/current-system/sw/bin/python3
import errno
import fcntl
import json
import os
import signal
import struct
import sys
import termios
import tty
from pathlib import Path

def size():
    rows, cols, xp, yp = struct.unpack("HHHH", fcntl.ioctl(0, termios.TIOCGWINSZ, bytes(8)))
    return [rows, cols, xp, yp]

def run(label, input_path, events_path, message):
    tty.setraw(0, termios.TCSANOW)
    events = Path(events_path)
    inputs = Path(input_path)
    def event(kind, data=b""):
        with events.open("a", encoding="utf-8") as output:
            output.write(json.dumps({"event": kind, "input_hex": data.hex(),
                                     "pid": os.getpid(), "winsize": size()},
                                    sort_keys=True) + "\n")
            output.flush()
    def on_winch(_signum, _frame):
        event("WINCH")
    signal.signal(signal.SIGWINCH, on_winch)
    title_outputs = {
        ord("0"): b"\x1b]0;OSC ZERO\x07",
        ord("2"): b"\x1b]2;OSC TWO\x1b\\",
        ord("E"): b"\x1b]2;\x07",
        ord("L"): b"\x1b]2;" + b"long title " * 24 + b"\x07",
        ord("W"): "\x1b]2;\u2003  trimmed title  \u3000\x07".encode("utf-8"),
    }
    event("FIRST")
    os.write(1, message)
    while True:
        try:
            data = os.read(0, 4096)
        except OSError as error:
            if error.errno == errno.EINTR:
                continue
            raise
        if not data:
            return
        with inputs.open("ab", buffering=0) as output:
            output.write(data)
        event("READ", data)
        for byte in data:
            if byte in title_outputs:
                os.write(1, title_outputs[byte])
                event("TITLE", bytes([byte]))
        if (not any(byte in title_outputs for byte in data)
                and message == b"WORKFLOW-" + label.encode() + b" READY\r\n"):
            os.write(1, b"WORKFLOW-READY\r\n")

if len(sys.argv) == 5 and sys.argv[1] == "--fixture":
    run(sys.argv[2], sys.argv[3], sys.argv[4],
        b"WORKFLOW-" + sys.argv[2].encode() + b" READY\r\n")
else:
    root = Path(__file__).resolve().parent / "session"
    lock_path = root / "spawn-sequence.lock"
    with lock_path.open("a+") as lock:
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX)
        lock.seek(0)
        previous = lock.read().strip()
        ordinal = int(previous or "0") + 1
        lock.seek(0)
        lock.truncate()
        lock.write(str(ordinal))
        lock.flush()
        os.fsync(lock.fileno())
        fcntl.flock(lock.fileno(), fcntl.LOCK_UN)
    label = "spawn-%d" % ordinal
    run(label, root / (label + ".input"),
        root / (label + ".events"), b"WORKFLOW-SPAWNED READY\r\n")
