#!/usr/bin/env python3
"""Capture physical-key output in an outer terminal, outside zterm.

Select the input method before starting. For each phase, press Ctrl+] then .
twice, releasing Ctrl before pressing period. Do not paste the chord. Flags 0
model an ordinary shell, 7 model observed Herdr, and 15 test key reporting.
No child process or remote connection is created. Terminal state is restored.
"""

import argparse
import json
import os
from pathlib import Path
import select
import signal
import sys
import termios
import time
import tty


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--label", required=True,
                        choices=("english", "doubao", "injected-smoke"))
    parser.add_argument("--seconds", type=float, default=8)
    args = parser.parse_args()
    if not 0.1 <= args.seconds <= 30:
        parser.error("--seconds must be between 0.1 and 30")
    if not sys.stdin.isatty() or not sys.stdout.isatty():
        parser.error("run directly in a fresh outer-terminal shell, outside zterm")

    root = Path(__file__).resolve().parents[4]
    output_dir = root / "target/herdr-escape-probe/physical-keys"
    output_dir.mkdir(parents=True, exist_ok=True)
    output_path = output_dir / f"{args.label}-{time.time_ns()}.json"
    capture_kind = ("injected-pty-smoke" if args.label == "injected-smoke"
                    else "outer-terminal-manual-capture")
    report = {"label": args.label, "kind": capture_kind,
              "completed": False, "phases": []}
    fd = sys.stdin.fileno()
    previous_termios = termios.tcgetattr(fd)
    pushed = False
    received = 0

    def stop(signum, _frame):
        raise SystemExit(128 + signum)

    previous_sigterm = signal.signal(signal.SIGTERM, stop)
    print("Input probe only. Keep the selected input method unchanged.", flush=True)
    try:
        tty.setraw(fd)
        # Push once; set each phase in our own stack entry; pop on every exit.
        pushed = True
        os.write(sys.stdout.fileno(), b"\x1b[>0u")
        for flags in (0, 7, 15):
            os.write(sys.stdout.fileno(), f"\x1b[={flags}u".encode())
            phase = {"flags": flags, "chunks": []}
            report["phases"].append(phase)
            print(f"\r\nFLAGS={flags}: press Ctrl+] then . twice ({args.seconds:g}s).\r",
                  flush=True)
            started = time.monotonic()
            while (remaining := args.seconds - (time.monotonic() - started)) > 0:
                ready, _, _ = select.select([fd], [], [], remaining)
                if not ready:
                    continue
                data = os.read(fd, 4096)
                if not data:
                    raise EOFError("terminal input closed")
                received += len(data)
                if received > 32768:
                    raise RuntimeError("capture exceeded 32 KiB; stopping")
                elapsed = round(time.monotonic() - started, 6)
                phase["chunks"].append({"seconds": elapsed, "hex": data.hex()})
                # Escape controls instead of letting captured bytes drive the UI.
                print(f"  +{elapsed:.3f}s {data!r}\r", flush=True)
        report["completed"] = True
    finally:
        try:
            if pushed:
                os.write(sys.stdout.fileno(), b"\x1b[<1u")
        finally:
            termios.tcsetattr(fd, termios.TCSANOW, previous_termios)
            signal.signal(signal.SIGTERM, previous_sigterm)
            with output_path.open("x") as output:
                json.dump(report, output, ensure_ascii=False, indent=2)
                output.write("\n")
            print(f"\nCapture saved: {output_path}", flush=True)


if __name__ == "__main__":
    main()
