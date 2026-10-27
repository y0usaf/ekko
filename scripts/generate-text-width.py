#!/usr/bin/env python3
import argparse
from pathlib import Path

LIMIT = 0x110000
BLOCK = 128


def property_points(path, wanted):
    points = set()
    for number, line in enumerate(path.read_text().splitlines(), 1):
        body = line.split("#", 1)[0].strip()
        if not body:
            continue
        span, prop = (part.strip() for part in body.split(";", 1))
        if prop != wanted:
            continue
        ends = span.split("..")
        start, end = int(ends[0], 16), int(ends[-1], 16)
        if len(ends) > 2 or not 0 <= start <= end < LIMIT:
            raise ValueError(f"{path}:{number}: invalid Unicode range")
        points.update(range(start, end + 1))
    if not points:
        raise ValueError(f"{path}: missing property {wanted}")
    return points


def code_info(widths, properties):
    table = bytearray((widths / "widths.bin").read_bytes())
    version = (widths / "unicode-version.txt").read_text().strip()
    if version != "16.0.0" or len(table) != LIMIT or any(v > 2 for v in table):
        raise ValueError("expected cudaterm Unicode 16.0.0 scalar widths")
    specs = (
        ("GraphemeBreakProperty-17.0.0.txt", "Extend", 4),
        ("emoji-data-17.0.0.txt", "Extended_Pictographic", 8),
        ("emoji-data-17.0.0.txt", "Emoji_Modifier_Base", 16),
        ("DefaultIgnorable-17.0.0.txt", "Default_Ignorable_Code_Point", 32),
        ("GraphemeExtendWidthful-17.0.0.txt", "Grapheme_Extend_Widthful", 64),
    )
    for name, prop, flag in specs:
        for cp in property_points(properties / name, prop):
            if flag == 4 and 0x1F3FB <= cp <= 0x1F3FF:
                continue
            if flag == 32 and cp == 0xAD:
                continue
            table[cp] |= flag
    for line in (properties / "emoji-variation-sequences-17.0.0.txt").read_text().splitlines():
        fields = line.split("#", 1)[0].split()
        if len(fields) >= 2 and fields[1] == "FE0F":
            table[int(fields[0], 16)] |= 128
    return table


def array(name, values):
    runs = []
    for value in values:
        if runs and runs[-1][1] == value:
            runs[-1][0] += 1
        else:
            runs.append([1, value])
    lines = [f"(defparameter {name}",
             f"  (let ((table (make-array {len(values)} :element-type '(unsigned-byte 8)))",
             "        (start 0))",
             "    (loop for (count value) on '("]
    for start in range(0, len(runs), 12):
        lines.append("      " + " ".join(f"{count} {value}" for count, value in runs[start:start + 12]))
    lines.extend(["    ) by #'cddr",
                  "          do (fill table value :start start :end (+ start count))",
                  "             (incf start count))",
                  "    table))",
                  f"(declaim (type (simple-array (unsigned-byte 8) ({len(values)})) {name}))"])
    return "\n".join(lines)

FUNCTIONS = '''(declaim (inline code-info))
(defun code-info (codepoint)
  "Return cudaterm's scalar width and cluster properties for a Unicode codepoint."
  (declare (type (integer 0 #x10ffff) codepoint))
  (aref *code-blocks*
        (+ (ash (aref *code-index* (ash codepoint -7)) 7)
           (logand codepoint 127))))
(defun cluster-step (state codepoint)
  "Return next cluster state, added cells, and whether CODEPOINT attaches to STATE."
  (declare (type (unsigned-byte 7) state)
           (type (integer 0 #x10ffff) codepoint))
  (let* ((info (code-info codepoint))
         (old-width (logand state 3))
         (width (logand info 3))
         (modifier (<= #x1f3fb codepoint #x1f3ff))
         (extend (logtest info 4))
         (ignorable (logtest info 32))
         (attach-modifier (and (plusp old-width) modifier (logtest state 8)))
         (attach-extend (and (plusp old-width) extend))
         (attach-zwj (and (plusp old-width) (logtest info 8) (logtest state 32))))
    (if (or (zerop width) ignorable attach-modifier attach-extend attach-zwj)
        (let* ((promote (and (= old-width 1)
                             (or attach-modifier attach-zwj
                                 (and attach-extend (logtest info 64))
                                 (and (= codepoint #xfe0f)
                                      (not (logtest state 4)) (logtest state 64)))))
               (next (logior (if promote 2 old-width) 4 (logand state 64))))
          (when (if (= codepoint #xfe0f)
                    (logtest state 8)
                    (and (not ignorable) (not extend) (not modifier)
                         (logtest info 16)))
            (setf next (logior next 8)))
          (cond
            ((= codepoint #x200d)
             (when (logtest state 16) (setf next (logior next 32))))
            (ignorable (setf next (logior next (logand state 48))))
            ((or extend modifier)
             (setf next (logior next (logand state 16))))
            ((logtest info 8) (setf next (logior next 16))))
          (values next (if promote 1 0) (plusp old-width)))
        (values (logior width
                        (if (logtest info 16) 8 0)
                        (if (logtest info 8) 16 0)
                        (if (logtest info 128) 64 0))
                width nil))))
(defun cluster-state (text)
  "Return the cluster state for one existing grid cell's text."
  (let ((state 0))
    (loop for character across text
          do (setf state (cluster-step state (char-code character))))
    state))
(defun display-width (value)
  "Return cudaterm's scalar character width or incremental string cell width."
  (etypecase value
    (character (logand (code-info (char-code value)) 3))
    (string
     (let ((state 0) (width 0))
       (loop for character across value
             do (multiple-value-bind (next added) (cluster-step state (char-code character))
                  (setf state next)
                  (incf width added)))
       width))))
'''


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("widths", type=Path)
    parser.add_argument("properties", type=Path)
    parser.add_argument("--output", type=Path, default=Path("src/text-width.lisp"))
    parser.add_argument("--info-output", type=Path)
    args = parser.parse_args()
    table = code_info(args.widths, args.properties)
    blocks, index, data = {}, [], bytearray()
    for start in range(0, LIMIT, BLOCK):
        block = bytes(table[start:start + BLOCK])
        if block not in blocks:
            blocks[block] = len(blocks)
            data.extend(block)
        index.append(blocks[block])
    if len(blocks) > 256:
        raise ValueError("Unicode table exceeds the byte-sized block index")
    license_text = (args.properties / "UNICODE-LICENSE.txt").read_text()
    license_text = license_text.replace('\\', '\\\\').replace('"', '\\"')
    text = ('(defpackage #:ekko/text (:use #:cl)\n'
            '  (:export #:display-width #:code-info #:cluster-step #:cluster-state)\n'
            f'  (:documentation "{license_text}"))\n(in-package #:ekko/text)\n'
            + array("*code-index*", index) + "\n"
            + array("*code-blocks*", data) + "\n" + FUNCTIONS)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(text)
    if args.info_output:
        args.info_output.write_bytes(table)
    print(f"{len(index)} index bytes + {len(data)} property bytes = {len(index) + len(data)} bytes")


if __name__ == "__main__":
    main()
