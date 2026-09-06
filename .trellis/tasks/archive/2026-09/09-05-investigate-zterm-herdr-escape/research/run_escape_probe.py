"""Research-only outer PTY probe linked to unchanged production libraries.

Reuses the archived local-only runtime fixture, with private state and Herdr
socket paths. Input CSI-u bytes simulate a compliant outer terminal; this does
not claim automated physical-key or remote-network coverage.
"""

import fcntl
import importlib.util
import json
import os
from pathlib import Path
import pty
import re
import select
import shlex
import struct
import subprocess
import sys
import tempfile
import termios
import time

sys.dont_write_bytecode = True
ROOT = next(p for p in Path(__file__).resolve().parents if (p / "Cargo.toml").is_file())
ARCHIVED = ROOT / ".trellis/tasks/archive/2026-09/09-05-fix-terminal-sync-scroll/research"
OUTPUT = ROOT / "target/herdr-escape-probe"
HERDR = Path("/opt/homebrew/bin/herdr")


def build():
    OUTPUT.mkdir(parents=True, exist_ok=True)
    spec = importlib.util.spec_from_file_location("previous_probe", ARCHIVED / "run_cli_herdr_probe.py")
    previous = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(previous)
    return previous.build(ROOT, ARCHIVED, OUTPUT)


def run_case(binary, name, flags=None, herdr=False):
    with tempfile.TemporaryDirectory(prefix="zt-esc-", dir="/tmp") as temporary:
        case = Path(temporary)
        config = case / "herdr.toml"
        config.write_text('onboarding = false\n[terminal]\ndefault_shell = "/bin/sh"\n'
                          'shell_mode = "non_login"\nnew_cwd = "current"\n')
        environment = dict(os.environ, TERM="xterm-256color", COLORTERM="truecolor",
                           XDG_CONFIG_HOME=str(case / "config"), XDG_STATE_HOME=str(case / "state"),
                           HERDR_CONFIG_PATH=str(config), HERDR_SOCKET_PATH=str(case / "herdr.sock"),
                           HERDR_DISABLE_SOUND="1")
        prefix = [str(binary), str(case)]
        child = None
        master = slave = None
        captured = bytearray()
        outcome = {"case": name}

        def drain(seconds, predicate=None):
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                if predicate and predicate():
                    return
                if select.select([master], [], [], min(0.05, max(0, deadline - time.monotonic())))[0]:
                    try:
                        data = os.read(master, 65536)
                    except OSError:
                        return
                    if not data:
                        return
                    captured.extend(data)
                elif child.poll() is not None:
                    return

        def presented_flags():
            return [int(value) for value in re.findall(rb"\x1b\[=(\d+)u", captured)]

        try:
            subprocess.run(prefix + ["setup", "--name", "escape-probe", "--profile", "official-n0"],
                           env=environment, capture_output=True, timeout=20, check=True)
            master, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 140, 0, 0))
            child = subprocess.Popen(prefix + ["connect", "local"], env=environment,
                                     stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
            os.close(slave)
            slave = None
            # Retry a harmless marker to cross the existing activation fence.
            for _ in range(12):
                os.write(master, b"printf '\\132\\124\\137\\122\\105\\101\\104\\131\\n'\r")
                drain(0.25, lambda: b"ZT_READY" in captured)
                if b"ZT_READY" in captured:
                    break
            assert b"ZT_READY" in captured and child.poll() is None, "shell did not become interactive"

            if herdr:
                command = shlex.join(["env", f"HERDR_CONFIG_PATH={config}",
                                      f"HERDR_SOCKET_PATH={case / 'herdr.sock'}", "HERDR_DISABLE_SOUND=1",
                                      "SHELL=/bin/sh", str(HERDR)])
                os.write(master, command.encode() + b"\r")
                drain(8, lambda: any(presented_flags()))
                drain(0.5)
                assert child.poll() is None and any(presented_flags()), "Herdr keyboard mode missing"
                flags = presented_flags()[-1]
                outcome["herdr_visible"] = b"spaces" in captured
            elif flags is not None:
                fixture = case / "keyboard_fixture.py"
                fixture.write_text(
                    "import os, pathlib, tty\n"
                    "tty.setraw(0)\n"
                    f"os.write(1, b'\\x1b[>{flags}uMODE_READY')\n"
                    f"with pathlib.Path({str(case / 'received.bin')!r}).open('wb', buffering=0) as capture:\n"
                    "    while True:\n"
                    "        data = os.read(0, 4096)\n"
                    "        if not data: break\n"
                    "        capture.write(data)\n")
                os.write(master, shlex.join([sys.executable, str(fixture)]).encode() + b"\r")
                drain(5, lambda: b"MODE_READY" in captured and flags in presented_flags())
                assert b"MODE_READY" in captured and flags in presented_flags(), "generic mode missing"

            outcome["presented_flags"] = presented_flags()
            if flags is None:
                os.write(master, b"\x1d.")
                drain(3)
                outcome["legacy_exit"] = child.poll()
                assert outcome["legacy_exit"] == 0
            else:
                payload = b"\x1b[93;5u"
                if flags & 2:
                    payload += b"\x1b[93;5:3u"
                payload += b"\x1b[46u" if flags & 8 else b"."
                os.write(master, payload)
                drain(0.5)
                outcome["enhanced_payload_hex"] = payload.hex()
                outcome["enhanced_exit"] = child.poll()
                assert child.poll() is None, "baseline unexpectedly detaches enhanced input"
                if not herdr:
                    received = (case / "received.bin").read_bytes()
                    outcome["child_received_hex"] = received.hex()
                    assert received == payload, "enhanced chord was not forwarded unchanged"
                os.write(master, b"\x1d.")
                drain(3)
                outcome["legacy_fallback_exit"] = child.poll()
                assert child.poll() == 0, "legacy control did not detach in the same mode"
            assert b"not_synchronized" not in captured
            return outcome
        finally:
            if child and child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait(timeout=5)
            for descriptor in (master, slave):
                if descriptor is not None:
                    os.close(descriptor)
            if herdr and (case / "herdr.sock").exists():
                stopped = subprocess.run([str(HERDR), "server", "stop"], env=environment,
                                         cwd=case, capture_output=True, timeout=10)
                outcome["herdr_cleanup_exit"] = stopped.returncode
                assert stopped.returncode == 0, stopped.stderr.decode(errors="replace")
            stopped = subprocess.run(prefix + ["daemon", "stop", "--yes"], env=environment,
                                     capture_output=True, timeout=20)
            outcome["daemon_cleanup_exit"] = stopped.returncode
            (OUTPUT / f"{name}.ansi").write_bytes(captured)
            (OUTPUT / f"{name}.json").write_text(json.dumps(outcome, indent=2) + "\n")
            assert stopped.returncode == 0, stopped.stderr.decode(errors="replace")


if __name__ == "__main__":
    binary = build()
    results = [run_case(binary, "legacy-shell"),
               run_case(binary, "generic-flags-3", flags=3),
               run_case(binary, "generic-flags-9", flags=9),
               run_case(binary, "herdr-persistent", herdr=True)]
    (OUTPUT / "outcomes.json").write_text(json.dumps(results, indent=2) + "\n")
    print(json.dumps(results, indent=2))
