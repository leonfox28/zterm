"""Real paired-device probe in a uniquely named disposable zterm Session.

Uses the existing authorized dev route. Starts an isolated Herdr server with
task-private XDG/socket paths and a shell cleanup trap; closes only the exact
newly created test Session. Never attaches, stops or takes over a user's Session.
"""

import argparse
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import shlex
import struct
import subprocess
import sys
import termios
import time
import uuid

sys.dont_write_bytecode = True

from run_cli_herdr_probe import build


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", default="dev")
    parser.add_argument("--rows", type=int, default=50)
    parser.add_argument("--columns", type=int, default=180)
    parser.add_argument("--observed", action="store_true")
    parser.add_argument("--reattach", action="store_true")
    parser.add_argument("--server-warmup", type=float, default=0)
    parser.add_argument("--prime-server", action="store_true")
    parser.add_argument("--installed-cli", type=Path)
    parser.add_argument("--inspect-existing", action="store_true")
    args = parser.parse_args()
    research = Path(__file__).resolve().parent
    root = next(parent for parent in research.parents if (parent / "Cargo.toml").is_file())
    build_output = root / "target/terminal-sync-scroll"
    if args.installed_cli:
        assert not args.observed, "installed executable must remain uninstrumented"
        binary = args.installed_cli
    else:
        binary = build(root, research, build_output, args.observed)
    name = "zterm-causal-" + uuid.uuid4().hex[:10]
    output = build_output / name
    output.mkdir()
    environment = dict(os.environ, TERM="xterm-256color", COLORTERM="truecolor",
                       ZTERM_CAUSAL_TRACE=str(output / "client-trace.log"))
    prefix = [str(binary)] + ([] if args.installed_cli else ["--existing-state"])
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", args.rows, args.columns, 0, 0))
    child = subprocess.Popen(prefix + ["session", "new", args.target, name, "--cwd", "/tmp"],
                             env=environment, stdin=slave, stdout=slave, stderr=slave,
                             start_new_session=True)
    os.close(slave)
    captured = bytearray()

    def drain(seconds):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if select.select([master], [], [], min(0.05, max(0, deadline-time.monotonic())))[0]:
                try:
                    data = os.read(master, 65536)
                except OSError:
                    break
                if not data:
                    break
                captured.extend(data)
            elif child.poll() is not None:
                break

    # Single foreground shell retains exact ownership of its isolated server PID.
    # Parent Session close sends HUP; a 30-second watchdog bounds cleanup even if
    # the client aborts during the startup trace.
    script = r'''
set -eu
umask 077
zterm_probe_case=$(mktemp -d /tmp/zt-herdr-causal.XXXXXX)
export XDG_CONFIG_HOME="$zterm_probe_case/config" XDG_STATE_HOME="$zterm_probe_case/state"
export HERDR_CONFIG_PATH="$zterm_probe_case/config.toml" HERDR_SOCKET_PATH="$zterm_probe_case/herdr.sock"
export HERDR_DISABLE_SOUND=1 SHELL=/bin/sh
printf 'onboarding = false\n[terminal]\ndefault_shell = "/bin/sh"\nshell_mode = "non_login"\nnew_cwd = "current"\n' > "$HERDR_CONFIG_PATH"
cd "$zterm_probe_case"
herdr server > "$zterm_probe_case/server.log" 2>&1 < /dev/null &
zterm_probe_server=$!
(sleep 45; kill "$zterm_probe_server" 2>/dev/null || :) &
zterm_probe_watchdog=$!
trap 'herdr server stop >/dev/null 2>&1 || :; kill "$zterm_probe_server" "$zterm_probe_watchdog" 2>/dev/null || :; wait 2>/dev/null || :; rm -rf "$zterm_probe_case"' EXIT
trap 'exit 0' HUP INT TERM
zterm_probe_wait=0
while [ ! -S "$HERDR_SOCKET_PATH" ]; do
  zterm_probe_wait=$((zterm_probe_wait + 1)); [ "$zterm_probe_wait" -lt 100 ] || exit 1
  sleep 0.02
done
sleep __SERVER_WARMUP__
__PRIME_SERVER__
printf '\132TERM_CAUSAL_OS='; uname -srm
herdr
printf 'ZTERM_CAUSAL_HERDR_RETURNED\n'
'''.replace("__SERVER_WARMUP__", str(max(0, min(args.server_warmup, 10))))
    prime = shlex.join(["python3", "-c", (research / "herdr_client_prime.py").read_text(),
                        "herdr", str(args.rows - 1), str(args.columns)]) if args.prime_server else ":"
    script = script.replace("__PRIME_SERVER__", prime)
    if args.inspect_existing:
        script = r'''
printf '\132TERM_INSPECT_BEGIN\n'
type -a herdr
herdr status server --json
zterm daemon status --json | python3 -c 'import json,sys; d=json.load(sys.stdin); print("ZTERM_DAEMON_START", d["started_at_unix"], d["version"])'
printf 'TERM=%s COLORTERM=%s TERM_PROGRAM=%s HERDR_SESSION=%s\n' "${TERM-}" "${COLORTERM-}" "${TERM_PROGRAM-}" "${HERDR_SESSION-}"
printf '\132TERM_INSPECT_END\n'
'''
    try:
        # Observe actual shell input/output readiness; remote establishment can
        # take longer than a fixed delay and Synchronizing legitimately drops input.
        drain(1)
        for _ in range(15):
            if child.poll() is not None:
                break
            os.write(master, b"printf '\\132TERM_CAUSAL_READY\\n'\r")
            drain(1)
            if b"ZTERM_CAUSAL_READY" in captured:
                break
        if b"ZTERM_CAUSAL_READY" not in captured:
            raise RuntimeError("paired shell did not confirm input readiness")
        if child.poll() is not None:
            raise RuntimeError("paired Session creation failed before probe input")
        command = script if args.inspect_existing else shlex.join(["/bin/sh", "-c", script])
        # The CLI negotiates bracketed paste; one pasted script avoids newline
        # fragments becoming commands while the shell is still consuming input.
        os.write(master, b"\x1b[200~" + command.encode() + b"\x1b[201~\r")
        drain(6 + args.server_warmup + (5 if args.prime_server else 0))
        outcome = {"name": name, "target": args.target, "rows": args.rows,
                   "columns": args.columns, "observed": args.observed,
                   "startup_exit": child.poll(), "not_synchronized": b"not_synchronized" in captured,
                   "script_started": b"ZTERM_CAUSAL_OS=" in captured,
                   "herdr_visible": b"ZTERM_CAUSAL_OS=" in captured and b"spaces" in captured.split(b"ZTERM_CAUSAL_OS=", 1)[1],
                   "server_warmup": args.server_warmup, "primed_server": args.prime_server,
                   "installed_cli": str(args.installed_cli) if args.installed_cli else None,
                   "inspection_complete": b"ZTERM_INSPECT_END" in captured}
        if args.reattach:
            (output / "initial-terminal.ansi").write_bytes(captured)
            if child.poll() is None:
                try:
                    os.write(master, b"\x1d.")
                    drain(0.5)
                except OSError:
                    pass
            child.wait(timeout=5)
            os.close(master)
            captured.clear()
            master, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ,
                        struct.pack("HHHH", args.rows, args.columns, 0, 0))
            child = subprocess.Popen(prefix + ["session", "attach", args.target, name],
                                     env=environment, stdin=slave, stdout=slave, stderr=slave,
                                     start_new_session=True)
            os.close(slave)
            drain(4)
            outcome["reattach_exit"] = child.poll()
            outcome["reattach_not_synchronized"] = b"not_synchronized" in captured
            outcome["reattach_herdr_visible"] = b"spaces" in captured
            if child.poll() is None and b"spaces" in captured:
                os.write(master, b"printf '\\132TERM_CAUSAL_REATTACH_OK\\n'\r")
                drain(1)
                outcome["reattach_input_roundtrip"] = b"ZTERM_CAUSAL_REATTACH_OK" in captured
        if child.poll() is None:
            try:
                if not args.inspect_existing:
                    os.write(master, b"\x02q")
                    drain(2)
                os.write(master, b"\x1d.")
                drain(0.5)
            except OSError:
                drain(0.1)
        outcome["post_quit_not_synchronized"] = b"not_synchronized" in captured
        (output / "outcome.json").write_text(json.dumps(outcome, indent=2) + "\n")
        print(json.dumps(outcome), flush=True)
    finally:
        if child.poll() is None:
            child.terminate()
            child.wait(timeout=5)
        os.close(master)
        (output / "terminal.ansi").write_bytes(captured)
        closed = subprocess.run(prefix + ["session", "close", args.target, name, "--yes"],
                                env=environment, capture_output=True, timeout=20)
        (output / "cleanup.log").write_bytes(closed.stdout + closed.stderr)
        if closed.returncode:
            raise RuntimeError(f"test Session cleanup failed: {closed.stderr.decode(errors='replace')}")
        print(f"CLEANUP=PASS OUTPUT={output}", flush=True)


if __name__ == "__main__":
    main()
