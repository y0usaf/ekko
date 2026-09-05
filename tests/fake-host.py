#!/usr/bin/env python3
"""Small deterministic fake Kitty host for synthetic demo streams.

This is a protocol/scene fixture, not a real terminal or visual acceptance test.
Canvas is 64x32 pixels; pane A is x=0..23, pane B x=32..55, each 3x1 cells at
8x16 cell metrics. Chrome/sentinel pixels are checked outside those rectangles.
"""
import base64
import json
import re
import sys

W, H = 64, 32
PANES = {"A": (0, 0, 24, 16), "B": (32, 0, 24, 16)}
MAX_INPUT = 1 << 20
CSI = re.compile(rb"\x1b\[([0-9;]*)([Hf])")


class Reject(Exception):
    pass


def parse(stream):
    pos = [0, 0]
    placements = []
    outer_ids = set()
    i = 0
    while i < len(stream):
        if stream[i:i + 2] == b"\x1b[":
            match = CSI.match(stream, i)
            if not match:
                raise Reject("malformed CSI")
            fields = match.group(1).split(b";") if match.group(1) else [b"1"]
            if len(fields) > 2 or any(not f.isdigit() or int(f) < 1 for f in fields):
                raise Reject("invalid cursor position")
            row = int(fields[0]) - 1
            col = int(fields[1]) - 1 if len(fields) == 2 else 0
            if col * 8 >= W or row * 16 >= H:
                raise Reject("cursor outside canvas")
            pos[:] = [col * 8, row * 16]
            i = match.end()
            continue
        if stream[i:i + 3] != b"\x1b_G":
            raise Reject("unexpected continuation")
        end = stream.find(b"\x1b\\", i + 3)
        if end < 0:
            raise Reject("incomplete APC")
        body = stream[i + 3:end]
        try:
            header, encoded = body.split(b";", 1)
            pairs = [item.split(b"=", 1) for item in header.split(b",") if item]
            if any(len(pair) != 2 for pair in pairs) or len({pair[0] for pair in pairs}) != len(pairs):
                raise Reject("duplicate or malformed graphics attribute")
            attrs = dict(pairs)
            raw = base64.b64decode(encoded, validate=True)
            action = attrs[b"a"]
            outer_id = int(attrs[b"i"])
            fmt = int(attrs[b"f"])
            width, height = int(attrs[b"s"]), int(attrs[b"v"])
            cells_w = int(attrs[b"c"]) if b"c" in attrs else None
            cells_h = int(attrs[b"r"]) if b"r" in attrs else None
            offset_x = int(attrs.get(b"X", b"0"))
            offset_y = int(attrs.get(b"Y", b"0"))
        except (ValueError, KeyError, base64.binascii.Error):
            raise Reject("malformed graphics command")
        required = {b"a", b"f", b"s", b"v", b"m", b"q", b"i", b"p"}
        if not required.issubset(attrs) or not set(attrs).issubset(required | {b"c", b"r", b"X", b"Y"}) \
                or action != b"T" or attrs[b"m"] != b"0" or attrs[b"q"] != b"2" \
                or attrs[b"p"] != b"1" or (cells_w is None) != (cells_h is None) \
                or (cells_w is not None and (cells_w < 1 or cells_h < 1)) \
                or (cells_w is not None and (offset_x != 0 or offset_y != 0)) \
                or not (0 <= offset_x < 8 and 0 <= offset_y < 16):
            raise Reject("unknown or invalid graphics attributes")
        if outer_id < 1 or outer_id > 0xffffffff or outer_id in outer_ids \
                or fmt not in (24, 32) or width < 1 or height < 1 \
                or width > 4096 or height > 4096 \
                or width * height * (3 if fmt == 24 else 4) > MAX_INPUT:
            raise Reject("unsupported graphics command")
        if len(raw) != width * height * (3 if fmt == 24 else 4):
            raise Reject("wrong payload dimensions")
        if fmt == 32 and any(raw[index] != 255 for index in range(3, len(raw), 4)):
            raise Reject("non-opaque RGBA unsupported")
        pixel_w, pixel_h = ((width, height) if cells_w is None
                            else (cells_w * 8, cells_h * 16))
        placements.append((pos[0] + offset_x, pos[1] + offset_y, raw, fmt,
                           width, height, pixel_w, pixel_h))
        outer_ids.add(outer_id)
        i = end + 2
    return placements


def compose(placements):
    canvas = [[(7, 7, 7) for _ in range(W)] for _ in range(H)]
    for x, y, raw, fmt, source_w, source_h, pixel_w, pixel_h in placements:
        pane = next(((px, py, pw, ph) for px, py, pw, ph in PANES.values()
                     if px <= x and x + pixel_w <= px + pw
                     and py <= y and y + pixel_h <= py + ph), None)
        if pane is None:
            raise Reject("placement outside pane")
        for yy in range(y, y + pixel_h):
            for xx in range(x, x + pixel_w):
                if xx < W and yy < H:
                    sx = min(source_w - 1, (xx - x) * source_w // pixel_w)
                    sy = min(source_h - 1, (yy - y) * source_h // pixel_h)
                    stride = 3 if fmt == 24 else 4
                    start = (sy * source_w + sx) * stride
                    canvas[yy][xx] = tuple(raw[start:start + 3])
    return canvas


def command(color, ident, fmt=24, cells_w=3, cells_h=1, *, native=False, offset=(0, 0)):
    payload = bytes(color) + (b"\xff" if fmt == 32 else b"")
    width, height = (1, 1) if not native else (1, 2)
    if native:
        payload = bytes((35, 90, 220, 255, 220, 40))
    header = b"a=T,f=" + str(fmt).encode() + b",s=" + str(width).encode() + b",v=" + str(height).encode()
    header += b",m=0,q=2,i=" + str(ident).encode() + b",p=1"
    if not native:
        header += b",c=" + str(cells_w).encode() + b",r=" + str(cells_h).encode()
    if offset != (0, 0):
        header += b",X=" + str(offset[0]).encode() + b",Y=" + str(offset[1]).encode()
    header += b";"
    return b"\x1b_G" + header + base64.b64encode(payload) + b"\x1b\\"


def cropped_checkerboard():
    """A pane-local checkerboard made from producer-cropped fragments."""
    stream = bytearray()
    ident = 300
    # Each fragment is already clipped to its pane before egress.  The grid
    # reaches every pane edge, while the host remains a strict global oracle.
    for row in range(1):
        for col in range(3):
            color = (240, 32, 32) if (row + col) % 2 == 0 else (32, 64, 240)
            stream += b"\x1b[" + str(row + 1).encode() + b";" + str(col + 1).encode() + b"H"
            stream += command(color, ident, cells_w=1, cells_h=1)
            ident += 1
    # Opaque UI overlay covers the middle tile.  It must cover image pixels
    # while leaving the surrounding checkerboard and pane sentinels intact.
    stream += b"\x1b[1;2H" + command((16, 220, 96), ident, cells_w=1, cells_h=1)
    return bytes(stream)


def checkerboard_expected():
    expected = [[(7, 7, 7) for _ in range(W)] for _ in range(H)]
    for pane_x, _, pane_w, pane_h in PANES.values():
        for y in range(pane_h):
            for x in range(pane_w):
                if 8 <= x < 16:
                    color = (7, 7, 7)
                else:
                    sx, sy = (x + 8) // 8, (y + 8) // 8
                    color = (255, 220, 40) if (sx + sy) % 2 == 0 else (35, 90, 220)
                expected[y][pane_x + x] = color
    return expected


def native_expected():
    expected = [[(7, 7, 7) for _ in range(W)] for _ in range(H)]
    for pane_x, _, pane_w, pane_h in PANES.values():
        for y in range(pane_h):
            for x in range(pane_w):
                if 9 <= x < 15 and 3 <= y < 12:
                    continue
                expected[y][pane_x + x] = ((255, 220, 40)
                    if (x + 1 + y + 1) % 2 == 0 else (35, 90, 220))
    return expected


def main():
    if sys.argv[1:] == ["--self-test"]:
        valid = b"\x1b[1;1H" + command((255, 0, 0), 101)
        valid += b"\x1b[1;5H" + command((0, 0, 255), 202, 32)
        bad = [
            (valid[:-1], "truncated"),
            (valid + b"X", "continuation"),
            (command((255, 0, 0), 1) + command((0, 0, 255), 1), "duplicate-id"),
            (valid.replace(b"/wAA", b"!!!!"), "base64"),
            (valid.replace(b",c=3", b",c=0"), "dimensions"),
            (valid.replace(b",q=2", b",z=2"), "unknown-attribute"),
            (valid.replace(b",q=2", b",q=2,q=2"), "duplicate-attribute"),
            (b"\x1b[3;4H" + command((255, 0, 0), 303), "outside-pane"),
            (valid.replace(b"i=101", b"i=4294967296"), "id-overflow"),
            (valid.replace(b",r=1", b",x=999,r=1"), "offset-overflow"),
            (b"\x1b[1;1H" + command((255, 0, 0), 307, offset=(1, 0)), "offset-with-cell-placement"),
            (b"\x1b[1;1H" + command((255, 0, 0), 304, native=True, offset=(8, 0)), "offset-overflow"),
            (b"\x1b[1;1H" + command((255, 0, 0), 305, cells_w=1, cells_h=1).replace(b",r=1", b""), "partial-cell-attrs"),
            (b"\x1b[1;1H" + command((255, 0, 0), 306, native=True).replace(b"s=1,v=2", b"s=25,v=1"), "placement-bleed"),
        ]
        for stream, _name in bad:
            try:
                compose(parse(stream))
            except Reject:
                continue
            print(json.dumps({"status": "fail", "case": _name}, sort_keys=True))
            return 1
        cropped = compose(parse(cropped_checkerboard()))
        expected = [[(7, 7, 7) for _ in range(W)] for _ in range(H)]
        for row in range(1):
            for col in range(3):
                color = (240, 32, 32) if (row + col) % 2 == 0 else (32, 64, 240)
                for y in range(row * 16, (row + 1) * 16):
                    for x in range(col * 8, (col + 1) * 8):
                        expected[y][x] = color
        for y in range(16):
            for x in range(8, 16):
                expected[y][x] = (16, 220, 96)
        if cropped != expected:
            print(json.dumps({"status": "fail", "case": "checkerboard-crop-overlay"}, sort_keys=True))
            return 1
        native = compose(parse(b"\x1b[1;1H" + command((0, 0, 0), 400,
                                                        native=True, offset=(4, 8))))
        if native[8][4] != (35, 90, 220) or native[9][4] != (255, 220, 40):
            print(json.dumps({"status": "fail", "case": "native-partial-cell"}, sort_keys=True))
            return 1
        print(json.dumps({"fixture": "fake-host-isolation", "status": "self-test-pass",
                          "cases": len(bad) + 1, "checks": ["cropped-checkerboard", "overlay"]}, sort_keys=True))
        return 0
    fixture = None
    if sys.argv[1:2] == ["--fixture"]:
        if sys.argv[1:] not in (["--fixture", "checkerboard"], ["--fixture", "native"]):
            print(json.dumps({"fixture": "fake-host-isolation", "status": "reject",
                              "reason": "unknown-fixture"}, sort_keys=True))
            return 1
        fixture = sys.argv[2]
    elif sys.argv[1:]:
        print(json.dumps({"fixture": "fake-host-isolation", "status": "reject",
                          "reason": "unknown-argument"}, sort_keys=True))
        return 1
    stream = sys.stdin.buffer.read(MAX_INPUT + 1)
    if len(stream) > MAX_INPUT:
        print(json.dumps({"fixture": "fake-host-isolation", "status": "reject", "reason": "input-too-large"}, sort_keys=True))
        return 1
    try:
        canvas = compose(parse(stream))
        checks = {
            "gap_untouched": all(canvas[y][x] == (7, 7, 7) for y in range(32) for x in range(24, 32)),
            "right_border_untouched": all(canvas[y][x] == (7, 7, 7) for y in range(32) for x in range(56, 64)),
            "status_untouched": all(canvas[y][x] == (7, 7, 7) for y in range(16, 32) for x in range(64)),
        }
        if fixture is None:
            checks["pane_a_red"] = all(canvas[y][x] == (255, 0, 0) for y in range(16) for x in range(24))
            checks["pane_b_blue"] = all(canvas[y][x] == (0, 0, 255) for y in range(16) for x in range(32, 56))
        if fixture == "checkerboard":
            checks["checkerboard_crop_overlay"] = canvas == checkerboard_expected()
        elif fixture == "native":
            checks["native_partial_overlay"] = canvas == native_expected()
        ok = all(checks.values())
        result = {"fixture": fixture or "fake-host-isolation", "status": "pass" if ok else "fail", "checks": checks}
        print(json.dumps(result, sort_keys=True))
        return 0 if ok else 1
    except Reject as error:
        print(json.dumps({"fixture": "fake-host-isolation", "status": "reject", "reason": str(error)}, sort_keys=True))
        return 1


if __name__ == "__main__":
    sys.exit(main())
