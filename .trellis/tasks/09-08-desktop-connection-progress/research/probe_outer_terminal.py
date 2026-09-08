#!/usr/bin/env python3
"""Read-only terminal query reproduction; never connects to a ZTerm daemon.

Run directly in a spare ordinary Ghostty shell. All probes only query colors and
cursor position. Raw/alternate-screen modes are restored before printing results.
"""

import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import re
import select
import sys
import termios
import time
import tty


def palette_queries(batch):
    return b"".join(
        b"\x1b]4"
        + b"".join(f";{index};?".encode() for index in range(start, start + batch))
        + b"\x1b\\"
        for start in range(0, 256, batch)
    )


def collect(fd, seconds):
    result = bytearray()
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        ready, _, _ = select.select([fd], [], [], max(0, deadline - time.monotonic()))
        if not ready:
            break
        result.extend(os.read(fd, 4096))
        if b"\x03" in result or len(result) > 65536:
            raise RuntimeError("probe cancelled or reply budget exceeded")
    return bytes(result)


def run(fd, payload, chunk_size):
    # Cursor movement is an application-neutral observation of visible leakage.
    os.write(fd, b"\x1b[2J\x1b[H\x1b[6n")
    before = collect(fd, 0.2)
    # Match the CLI's already-running stdin pump: consume replies while the
    # terminal is processing requests, rather than fill the PTY input queue.
    with ThreadPoolExecutor(max_workers=1) as reader:
        replies = reader.submit(collect, fd, 0.6)
        for start in range(0, len(payload), chunk_size):
            remaining = payload[start : start + chunk_size]
            while remaining:
                remaining = remaining[os.write(fd, remaining) :]
        os.write(fd, b"\x1b[6n")
        after = replies.result()
    pattern = rb"\x1b\[(\d+);(\d+)R"
    before_positions = re.findall(pattern, before)
    after_positions = re.findall(pattern, after)
    return {
        "query_bytes": len(payload),
        "write_chunk_size": chunk_size,
        "cursor_before": [list(map(int, p)) for p in before_positions],
        "cursor_after": [list(map(int, p)) for p in after_positions],
        "reply_bytes": len(after),
        "reply_capture": after.decode("ascii", errors="backslashreplace"),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", action="store_true", help="query this terminal")
    parser.add_argument("--candidate", action="store_true", help="only query the corrected encoding")
    args = parser.parse_args()
    queries = palette_queries(32)
    roles = b"".join(f"\x1b]{role};?\x1b\\".encode() for role in [10, 11, 12, 17, 19])
    extra = (
        b"\x1b]21;foreground=?;background=?;cursor=?;cursor_text=?;"
        b"selection_foreground=?;selection_background=?\x1b\\"
        b"\x1b[?996n\x1b[?2031$p\x1b[5n"
    )
    cases = [
        ("production-probe", queries + roles + extra, 65536),
        ("palette-batch-32", queries, 65536),
        ("palette-split-1024", queries, 1024),
        ("palette-single-index", palette_queries(1), 65536),
    ]
    if args.candidate:
        cases = [
            ("candidate-probe", palette_queries(1) + roles + extra, 65536),
            ("candidate-split", palette_queries(1) + roles + extra, 1024),
        ]
    if not args.run:
        print(json.dumps({name: len(payload) for name, payload, _ in cases}, indent=2))
        print("Pass --run in a spare ordinary Ghostty shell to execute the read-only probes.")
        return
    if not sys.stdin.isatty() or not sys.stdout.isatty():
        parser.error("requires a real terminal on stdin and stdout")
    fd = os.open("/dev/tty", os.O_RDWR)
    original = termios.tcgetattr(fd)
    results = {"terminal": os.environ.get("TERM_PROGRAM"), "version": os.environ.get("TERM_PROGRAM_VERSION")}
    try:
        tty.setraw(fd, termios.TCSANOW)
        os.write(fd, b"\x1b[?1049h\x1b[?25l")
        results["cases"] = {name: run(fd, payload, chunk) for name, payload, chunk in cases}
    finally:
        os.write(fd, b"\x1b[?25h\x1b[?1049l")
        termios.tcsetattr(fd, termios.TCSANOW, original)
        os.close(fd)
    destination = Path(__file__).with_name("outer-terminal-candidate-results.json" if args.candidate else "outer-terminal-results.json")
    destination.write_text(json.dumps(results, indent=2) + "\n")
    for name, result in results["cases"].items():
        print(f"{name}: cursor {result['cursor_before']} -> {result['cursor_after']}, replies={result['reply_bytes']} bytes")
    print(f"Saved: {destination}")


if __name__ == "__main__":
    main()
