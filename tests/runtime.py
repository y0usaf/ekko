import base64
import fcntl
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import shutil
import struct
import subprocess
import sys
import tempfile
import termios
import time
import tty
import zlib

ESC = b"\x1b"


def child(directory, color):
    root = Path(directory)
    info = {"tty": all(os.isatty(fd) for fd in range(3)),
            "session_leader": os.getsid(0) == os.getpid(),
            "foreground": os.tcgetpgrp(0) == os.getpgrp()}
    (root / f"{color}.json").write_text(json.dumps(info))
    tty.setraw(0)
    os.write(1, ESC + b"[?1003h" + ESC + b"[?1006h" + ESC + b"[?1016h")
    os.write(1, ESC + b"[16t" + ESC + b"[?u")
    os.write(1, ESC + b"_Gi=300,a=q,t=d,f=32,s=1,v=1;AAAA/w==" + ESC + b"\\")
    os.write(1, ESC + b"]52;c;U0hPVUxEX05PVF9FU0NBUEU=" + b"\x07")
    rgba = bytes([255, 0, 0, 255] if color == "red" else [0, 0, 255, 255]) * 1000 * 40
    # An uncompressed zlib stream forces a multi-chunk upload through real
    # backpressure; Host validates every decoded pixel after batching/reconnect.
    payload = base64.b64encode(zlib.compress(rgba, 0 if color == "red" else 6))
    frame = bytearray(ESC + b"[H")
    for offset in range(0, len(payload), 4096):
        header = b"a=T,f=32,o=z,s=1000,v=40,i=7,p=1,C=1,q=2," if offset == 0 else b""
        more = int(offset + 4096 < len(payload))
        frame.extend(ESC + b"_G" + header + f"m={more};".encode()
                     + payload[offset:offset + 4096] + ESC + b"\\")
    shared_name = f"/ekko-test-{os.getpid()}"
    if os.environ.get("EKKO_TEST_SHARED"):
        frame = ESC + b"[H" + ESC + b"_Ga=T,t=s,f=32,s=1000,v=40,i=7,p=1,C=1,q=2;" + base64.b64encode(shared_name.encode()) + ESC + b"\\"
    log = open(root / f"{color}.input", "ab", buffering=0)
    active = True
    last = 0
    while True:
        if active and time.monotonic() - last > 0.15:
            if os.environ.get("EKKO_TEST_SHARED"):
                Path("/dev/shm" + shared_name).write_bytes(rgba)
            fragment = 4093 if color == "red" else 17
            for offset in range(0, len(frame), fragment):
                os.write(1, frame[offset:offset + fragment])
            last = time.monotonic()
        ready, _, _ = select.select([0], [], [], 0.05)
        if ready:
            data = os.read(0, 65536)
            log.write(data)
            if b"DELETE" in data:
                os.write(1, ESC + b"_Ga=d,d=A,q=2" + ESC + b"\\")
                active = False
            if b"PAUSE" in data:
                active = False
            if b"INCOMPLETE" in data:
                active = False
                os.write(1, ESC + b"_Ga=T,f=32,s=1,v=1,i=99,q=2,m=1;AAAA" + ESC + b"\\")


class Host:
    def __init__(self, local=True):
        self.local = local
        self.replies = bytearray()
        self.paths = set()
        self.buffer = b""
        self.cursor = (0, 0)
        self.images = {}
        self.canvas = self.images
        self.seen = set()
        self.pending = None
        self.output = bytearray()
        self.uploads = 0
        self.placements = 0
        self.dimensions = {}

    def feed(self, data):
        self.output.extend(data)
        self.buffer += data
        while self.buffer:
            start = self.buffer.find(ESC)
            if start < 0:
                self.buffer = b""
                return
            self.buffer = self.buffer[start:]
            if len(self.buffer) < 2:
                return
            if self.buffer.startswith(ESC + b"_G"):
                end = self.buffer.find(ESC + b"\\", 3)
                if end < 0:
                    return
                command = self.buffer[3:end]
                self.buffer = self.buffer[end + 2:]
                header, has_payload, payload = command.partition(b";")
                fields = dict(pair.split(b"=", 1) for pair in header.split(b","))
                if fields.get(b"a") == b"q":
                    assert fields[b"t"] == b"f"
                    path = Path(base64.b64decode(payload, validate=True).decode())
                    assert path.is_file()
                    if self.local is None:
                        continue  # A host that never answers the local transport probe.
                    response = b"OK" if self.local else b"ENOTSUP"
                    self.replies.extend(ESC + b"_Gi=" + fields[b"i"] + b";" + response + ESC + b"\\")
                    continue
                if fields.get(b"a") == b"d":
                    assert fields[b"d"] == b"I", "unscoped outer deletion"
                    self.canvas.pop(int(fields[b"i"]), None)
                    self.dimensions.pop(int(fields[b"i"]), None)
                    continue
                if fields.get(b"a") == b"p":
                    # Placement reuses the decoded image bytes and updates
                    # only its destination/source rectangle.
                    image_id = int(fields[b"i"])
                    assert image_id in self.images, "placement references unknown image"
                    assert fields.get(b"p") == b"1" and fields.get(b"q") == b"2"
                    assert fields.get(b"C") == b"1" and not payload
                    old = self.images[image_id]
                    x = self.cursor[0] * 8 + int(fields.get(b"X", b"0"))
                    y = self.cursor[1] * 16 + int(fields.get(b"Y", b"0"))
                    width, height = int(fields[b"w"]), int(fields[b"h"])
                    sx, sy = int(fields[b"x"]), int(fields[b"y"])
                    image_width, image_height = self.dimensions[image_id]
                    assert 0 <= sx < sx + width <= image_width
                    assert 0 <= sy < sy + height <= image_height
                    assert 0 <= x < x + width <= 960
                    assert 16 <= y < y + height <= 39 * 16
                    self.canvas[image_id] = (old[0], x, y,
                                             width, height)
                    self.placements += 1
                    continue
                assert has_payload, "upload missing payload"
                if self.pending is None:
                    self.pending = (fields, bytearray(), self.cursor)
                self.pending[1].extend(payload)
                if fields.get(b"m", b"0") == b"0":
                    keys, encoded, cursor = self.pending
                    self.pending = None
                    if keys.get(b"t") == b"f":
                        assert self.local and keys[b"q"] == b"0"
                        path = Path(base64.b64decode(encoded, validate=True).decode())
                        self.paths.add(path)
                        raw = path.read_bytes()
                        self.replies.extend(ESC + b"_Gi=" + keys[b"i"] + b",p=1;OK" + ESC + b"\\")
                    else:
                        raw = zlib.decompress(base64.b64decode(encoded, validate=True))
                    w, h = int(keys[b"s"]), int(keys[b"v"])
                    assert len(raw) == w * h * 4
                    color = tuple(raw[:4])
                    assert color in [(255, 0, 0, 255), (0, 0, 255, 255)]
                    assert raw == bytes(color) * w * h
                    image_id = int(keys[b"i"])
                    assert image_id != 7, "child identifier escaped unchanged"
                    x = cursor[0] * 8 + int(keys[b"X"])
                    y = cursor[1] * 16 + int(keys[b"Y"])
                    width, height = int(keys[b"w"]), int(keys[b"h"])
                    assert y >= 16 and y + height <= 39 * 16
                    assert 0 <= x < 960 and x + width <= 960
                    self.canvas[image_id] = (color, x, y, width, height)
                    self.dimensions[image_id] = (w, h)
                    self.uploads += 1
                    self.seen.add(color)
            elif self.buffer.startswith(ESC + b"["):
                match = re.match(rb"\x1b\[([\x20-\x3f]*)([\x40-\x7e])", self.buffer)
                if not match:
                    return
                self.buffer = self.buffer[match.end():]
                if match[2] == b"H":
                    parts = [int(x or 1) for x in match[1].split(b";")]
                    self.cursor = (parts[1] - 1, parts[0] - 1)
                elif match[2] == b"J" and match[1] == b"2":
                    self.canvas.clear()
                elif match[1] == b"?2026" and match[2] == b"h":
                    self.canvas = dict(self.images)
                elif match[1] == b"?2026" and match[2] == b"l":
                    self.images = self.canvas
            else:
                self.buffer = self.buffer[2:]


def integration(binary, shared=False, local=True):
    with tempfile.TemporaryDirectory(prefix="ekko-runtime-") as directory:
        root = Path(directory)
        env = dict(os.environ, XDG_RUNTIME_DIR=directory)
        if shared:
            env["EKKO_TEST_SHARED"] = "1"
        name = "integration"
        host = Host(local=local)
        clients = []
        master = None

        def start(args):
            nonlocal master
            master, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 960, 640))
            process = subprocess.Popen([binary, *args], stdin=slave, stdout=slave, stderr=slave,
                                       env=env, start_new_session=True)
            os.close(slave)
            clients.append(process)
            return process

        def pump(seconds, replies=True):
            until = time.monotonic() + seconds
            while time.monotonic() < until:
                ready, _, _ = select.select([master], [], [], 0.03)
                if ready:
                    try:
                        data = os.read(master, 65536)
                    except OSError:
                        break
                    host.feed(data)
                if replies and host.replies:
                    os.write(master, host.replies)
                    host.replies.clear()

        def status():
            return json.loads(subprocess.check_output([binary, "status", name], env=env, timeout=5))

        try:
            process = start(["run", "--session", name, sys.executable, __file__, "--child", directory, "red",
                             ":::", sys.executable, __file__, "--child", directory, "blue"])
            pump(2)
            assert process.poll() is None, bytes(host.output[-2000:])
            first = status()
            assert all(json.loads((root / f"{c}.json").read_text()) ==
                       {"tty": True, "session_leader": True, "foreground": True} for c in ["red", "blue"])
            assert len(host.images) == 2, (first, bytes(host.output[-2000:]))
            red, blue = sorted(host.images.values(), key=lambda i: i[1])
            assert red[0] == (255, 0, 0, 255) and blue[0] == (0, 0, 255, 255)
            assert red[1] + red[3] <= 59 * 8 and blue[1] >= 60 * 8
            assert b"]52;" not in host.output
            for color in ["red", "blue"]:
                received = (root / f"{color}.input").read_bytes()
                assert b"[6;16;8t" in received and b"[?0u" in received and b"Gi=300;OK" in received, received
            if shared and local:
                # Hold terminal acknowledgements while producers keep replacing
                # frames. Current scene leases stay readable and storage stays bounded.
                pump(.6, replies=False)
                assert host.replies, "no host acknowledgement to delay"
                leased = [p for p in host.paths if p.exists()]
                assert leased and len(list((root / "ekko-v2").glob("*.frames/frame-*"))) <= 4
                pump(.4)
                assert all(not p.exists() for p in leased), "ack did not release old generations"
            os.write(master, b"LEFT" + b"\x02" + b"2" + b"RIGHT")
            pump(.4)
            assert b"LEFT" in (root / "red.input").read_bytes()
            assert b"RIGHT" not in (root / "red.input").read_bytes()
            assert b"RIGHT" in (root / "blue.input").read_bytes()
            os.write(master, ESC + b"[<0;490;40M" + ESC + b"[<0;490;40m")
            pump(.3)
            assert b"[<0;10;24M" in (root / "blue.input").read_bytes()
            os.write(master, ESC + b"[200~\x02q" + ESC + b"[201~")
            pump(.3)
            assert status()["attached"]
            os.write(master, b"\x02" + b"1DELETE")
            pump(.5)
            assert len(host.images) == 1 and next(iter(host.images.values()))[0] == (0, 0, 255, 255)
            before = status()
            process.kill()
            process.wait(timeout=3)
            os.close(master)
            master = None
            time.sleep(.4)
            after = status()
            assert not after["attached"]
            assert [p["pid"] for p in before["panes"]] == [p["pid"] for p in after["panes"]]
            assert after["panes"][1]["frames"] > before["panes"][1]["frames"]
            if shared:
                assert len(list((root / "ekko-v2").glob("*.frames/frame-*"))) == 1, "detached snapshots grew"
            host = Host(local=local)
            start(["attach", name])
            pump(1.6 if local is None else .6)
            assert len(host.images) == 1, status()
            os.write(master, b"\x02" + b"2PAUSE")
            pump(.4)
            uploads = host.uploads
            placements = host.placements
            input_before = (root / "blue.input").read_bytes()
            os.write(master, ESC)
            pump(.2)
            assert (root / "blue.input").read_bytes() == input_before + ESC, "standalone ESC lost"
            os.write(master, b"\x02" + b"2\x02z")
            pump(.5)
            assert status()["panes"][1]["cols"] == 120
            os.write(master, b"\x02z\x02s")
            pump(.4)
            assert host.images and next(iter(host.images.values()))[1] == 0, (status(), bytes(host.output[-2500:]))
            assert host.uploads == uploads, "layout change reuploaded static image"
            assert host.placements > placements, "layout did not update placements"
            errors = status()["panes"][1]["graphics_errors"]
            os.write(master, b"INCOMPLETE")
            pump(10.5)
            assert status()["panes"][1]["graphics_errors"] == errors + 1, "idle upload did not expire"
            subprocess.run([binary, "stop", name], env=env, check=True, timeout=5)
            for client in clients:
                client.wait(timeout=3)
            assert not list((root / "ekko-v2").glob("*.frames/frame-*")), "snapshot leaked after shutdown"
            print(json.dumps({"status": "pass", "checks": ["controlling-pty", "fragmented-rgba", "id-isolation",
                            "native-clipping", "scoped-delete", "reply-routing", "keyboard-focus", "pixel-mouse",
                            "paste", "client-death", "detached-updates", "reattach", "zoom", "swap",
                            "placement-reuse", "standalone-escape", "idle-upload-expiry", "shutdown"]}))
        finally:
            subprocess.run([binary, "stop", name], env=env, stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL, timeout=5)
            for process in clients:
                if process.poll() is None:
                    process.kill()
                process.wait()
            if master is not None:
                os.close(master)
            if shared and "first" in locals():
                for pane in first["panes"]:
                    Path(f"/dev/shm/ekko-test-{pane['pid']}").unlink(missing_ok=True)
            for log in (root / "ekko-v2").glob("*.log"):
                if sys.exc_info()[0]:
                    print(log.read_text(), file=sys.stderr)


def mixed_shell(binary):
    with tempfile.TemporaryDirectory(prefix="ekko-shell-") as directory:
        env = dict(os.environ, XDG_RUNTIME_DIR=directory)
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 960, 640))
        saved = termios.tcgetattr(slave)
        process = subprocess.Popen([binary, "run", "--session", "mixed", shutil.which("sh"), "-i", ":::",
                                    sys.executable, __file__, "--child", directory, "blue"],
                                   stdin=slave, stdout=slave, stderr=slave, env=env, start_new_session=True)
        host = Host()

        def wait_for(predicate, seconds=5):
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                ready, _, _ = select.select([master], [], [], .03)
                if ready:
                    host.feed(os.read(master, 65536))
                if predicate():
                    return
            raise AssertionError(bytes(host.output[-1500:]))

        try:
            wait_for(lambda: bool(host.images))
            os.write(master, b"printf EKKO_; printf SHELL_OK; printf '\\n'\r")
            wait_for(lambda: b"EKKO_SHELL_OK" in host.output)
            os.write(master, b"sleep 30\r")
            time.sleep(.2)
            os.write(master, b"\x1a")
            wait_for(lambda: b"Stopped" in host.output)
            os.write(master, b"fg\r")
            time.sleep(.2)
            os.write(master, b"\x03")
            time.sleep(.2)
            os.write(master, b"printf AFTER_; printf INTERRUPT_OK; printf '\\n'\r")
            wait_for(lambda: b"AFTER_INTERRUPT_OK" in host.output)
            assert host.images, "graphics pane disappeared during shell job control"
            process.terminate()
            wait_for(lambda: process.poll() is not None)
            process.wait(timeout=3)
            assert termios.tcgetattr(slave) == saved, "SIGTERM did not restore terminal attributes"
            result = json.loads(subprocess.check_output([binary, "status", "mixed"], env=env, timeout=5))
            assert all(p["exit_code"] is None for p in result["panes"]), result
            print(json.dumps({"status": "pass", "checks": ["shell-beside-graphics", "shell-output", "ctrl-z",
                            "foreground-job", "ctrl-c", "sigterm-restores-terminal"]}))
        finally:
            subprocess.run([binary, "stop", "mixed"], env=env, stdout=subprocess.DEVNULL,
                           stderr=subprocess.DEVNULL, timeout=5)
            if process.poll() is None:
                process.kill()
            process.wait()
            os.close(master)
            os.close(slave)


if __name__ == "__main__":
    if sys.argv[1] == "--child":
        child(sys.argv[2], sys.argv[3])
    else:
        integration(sys.argv[1])
        mixed_shell(sys.argv[1])
        integration(sys.argv[1], shared=True)
        integration(sys.argv[1], shared=True, local=False)
        integration(sys.argv[1], shared=True, local=None)
