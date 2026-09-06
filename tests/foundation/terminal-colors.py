#!/usr/bin/env python3
"""Bounded read-only color queries and semantic swatches; no palette setters."""

import os
import select
import sys
import termios
import time
import tty


def main():
    if not sys.stdin.isatty() or not sys.stdout.isatty():
        raise SystemExit("run in an interactive terminal (see terminal-colors.md)")
    queries = bytearray()
    for start in range(0, 256, 32):
        pairs = "".join(f";{index};?" for index in range(start, start + 32))
        queries.extend(f"\x1b]4{pairs}\x1b\\".encode())
    for role in (10, 11, 12, 17, 19):
        queries.extend(f"\x1b]{role};?\x1b\\".encode())
    queries.extend(
        b"\x1b]21;cursor=?;cursor_text=?;selection_foreground=?;"
        b"selection_background=?\x1b\\"
        b"\x1b[?996n\x1b[?2031$p\x1b[5n"
    )
    fd = sys.stdin.fileno()
    saved = termios.tcgetattr(fd)
    replies = bytearray()
    fenced = False
    try:
        tty.setraw(fd, when=termios.TCSANOW)
        sys.stdout.buffer.write(queries)
        sys.stdout.buffer.flush()
        deadline = time.monotonic() + 0.25
        while len(replies) < 65536:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([fd], [], [], remaining)[0]:
                break
            chunk = os.read(fd, min(4096, 65536 - len(replies)))
            if not chunk:
                break
            replies.extend(chunk)
            if b"\x1b[0n" in replies or b"\x9b0n" in replies:
                fenced = True
                break
    finally:
        termios.tcsetattr(fd, termios.TCSANOW, saved)
    print(f"Replies: {len(replies)} bytes; status boundary received: {fenced}")
    # Never replay terminal response bytes as controls.
    print(repr(bytes(replies)))
    try:
        print("Indexed palette (compare all 256 cells between outer and ZTerm):")
        for row in range(16):
            print("".join(
                f"\x1b[48;5;{row * 16 + column}m  \x1b[0m"
                for column in range(16)
            ))
        print("\x1b[39;49mDefault FG/BG  "
              "\x1b[7mInverse default\x1b[0m  "
              "\x1b[38;2;255;128;0mLiteral RGB orange\x1b[0m")
        for shape in range(1, 6):
            print(f"\x1b[31;4:{shape};58;5;2mUnderline {shape}, red text / green line"
                  "\x1b[0;59;48;2;1;2;3m  reset + RGB background\x1b[0m")
    finally:
        sys.stdout.write("\x1b[0m")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
