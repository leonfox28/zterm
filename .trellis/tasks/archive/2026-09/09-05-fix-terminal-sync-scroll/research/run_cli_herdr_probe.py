"""Planning probe: real CLI, local IPC, SessionService, PTY and Herdr startup.

Uses task-private daemon state and an explicit /bin/sh spawner. Product source
and user daemon state remain unchanged. Does not claim real remote coverage.
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
import tempfile
import termios
import time

sys.dont_write_bytecode = True


def build(root, research, output, observed=False):
    compiled = subprocess.run(
        ["cargo", "+1.98.0", "build", "-p", "zterm-cli", "--all-features", "--locked", "--offline",
         "--message-format=json"], cwd=root, capture_output=True, text=True, check=True)
    (output / "cli-build.jsonl").write_text(compiled.stdout)
    (output / "cli-build.log").write_text(compiled.stderr)
    libraries, native_paths = {}, set()
    for line in (output / "cli-build.jsonl").read_text().splitlines():
        item = json.loads(line)
        if item.get("reason") == "compiler-artifact":
            if item["target"]["name"] == "rustix" and "event" not in item["features"]:
                continue
            for filename in item["filenames"]:
                if filename.endswith(".rlib"):
                    libraries[item["target"]["name"]] = filename
        elif item.get("reason") == "build-script-executed":
            native_paths.update(item["linked_paths"])
    observed_dir = None
    if observed:
        from instrument_cli_probe import instrument
        libraries, observed_dir = instrument(root, output, libraries, native_paths)
    source = output / "cli_runtime_probe.rs"
    source.write_text((research / source.name).read_text().replace(
        "__DAEMON_HARNESS__", str(root / "crates/daemon/tests/support/daemon_harness.rs")))
    binary = output / "cli-runtime-probe"
    command = ["rustc", "+1.98.0", "--edition=2024", str(source), "-o", str(binary),
               "-L", f"dependency={Path(libraries['zterm_core']).parent}"]
    for name in ("clap", "tokio", "nix", "zterm_cli", "zterm_core", "zterm_daemon", "zterm_platform"):
        command.extend(["--extern", f"{name}={libraries[name]}"])
    for path in sorted(native_paths):
        command.extend(["-L", path])
    if observed_dir:
        command.extend(["-L", f"dependency={observed_dir}"])
    subprocess.run(command, check=True)
    return binary


def run_case(binary, output, herdr, index, rows, columns, persistent):
    # Short path keeps the Unix socket below the platform's sun_path limit.
    with tempfile.TemporaryDirectory(prefix="zt-sync-", dir="/tmp") as temporary:
        case = Path(temporary)
        config = case / "herdr.toml"
        config.write_text('onboarding = false\n[terminal]\ndefault_shell = "/bin/sh"\n'
                          'shell_mode = "non_login"\nnew_cwd = "current"\n')
        environment = dict(os.environ, TERM="xterm-256color", COLORTERM="truecolor",
                           XDG_CONFIG_HOME=str(case / "config"), XDG_STATE_HOME=str(case / "state"),
                           HERDR_CONFIG_PATH=str(config), HERDR_SOCKET_PATH=str(case / "herdr.sock"),
                           HERDR_DISABLE_SOUND="1")
        environment["ZTERM_CAUSAL_TRACE"] = str(output / f"herdr-cli-{index}-trace.log")
        Path(environment["ZTERM_CAUSAL_TRACE"]).unlink(missing_ok=True)
        prefix = [str(binary), str(case)]
        child = None
        master = slave = None
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

        try:
            subprocess.run(prefix + ["setup", "--name", "causal-probe", "--profile", "official-n0"],
                                   env=environment, capture_output=True, timeout=20, check=True)
            master, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))
            child = subprocess.Popen(prefix + ["connect", "local"], env=environment,
                                     stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
            os.close(slave)
            slave = None
            drain(1)
            command = shlex.join(["env", f"HERDR_CONFIG_PATH={config}",
                f"HERDR_SOCKET_PATH={case / 'herdr.sock'}", "HERDR_DISABLE_SOUND=1", "SHELL=/bin/sh",
                str(herdr)] + ([] if persistent else ["--no-session"]))
            os.write(master, command.encode() + b"\r")
            drain(3)
            startup_exit = child.poll()
            outcome = {"case": index, "rows": rows, "columns": columns, "persistent": persistent,
                       "startup_exit": startup_exit,
                       "not_synchronized": b"not_synchronized" in captured,
                       "herdr_visible": b"spaces" in captured,
                       "bytes": len(captured)}
            if startup_exit is None:
                # Herdr's documented quit chord, then zterm's configured detach.
                try:
                    os.write(master, b"\x02q")
                    drain(0.5)
                    if child.poll() is None:
                        os.write(master, b"\x1d.")
                        drain(0.5)
                except OSError:
                    drain(0.1)
            outcome["post_quit_not_synchronized"] = b"not_synchronized" in captured
            print(json.dumps(outcome), flush=True)
            (output / f"herdr-cli-{index}.ansi").write_bytes(captured)
            return outcome
        finally:
            if child and child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
            if master is not None:
                os.close(master)
            if slave is not None:
                os.close(slave)
            if persistent and (case / "herdr.sock").exists():
                herdr_stop = subprocess.run([str(herdr), "server", "stop"], env=environment,
                                           cwd=case, capture_output=True, timeout=10)
                (output / f"herdr-cli-{index}-herdr-cleanup.log").write_bytes(
                    herdr_stop.stdout + herdr_stop.stderr)
                if herdr_stop.returncode:
                    print(f"HERDR_STOP_ERROR={herdr_stop.returncode}", flush=True)
            stop = subprocess.run(prefix + ["daemon", "stop", "--force"], env=environment,
                                  capture_output=True, timeout=20)
            (output / f"herdr-cli-{index}-cleanup.log").write_bytes(stop.stdout + stop.stderr)
            if stop.returncode:
                raise RuntimeError(f"isolated daemon cleanup failed: {stop.returncode}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--rows", type=int, default=24)
    parser.add_argument("--columns", type=int, default=80)
    parser.add_argument("--persistent", action="store_true")
    parser.add_argument("--observed", action="store_true")
    parser.add_argument("--label", help="Separate acceptance output from retained baseline evidence")
    parser.add_argument("--herdr", type=Path, default=Path("/opt/homebrew/bin/herdr"))
    args = parser.parse_args()
    research = Path(__file__).resolve().parent
    root = next(parent for parent in research.parents if (parent / "Cargo.toml").is_file())
    build_output = root / "target/terminal-sync-scroll"
    binary = build(root, research, build_output, args.observed)
    mode = "persistent" if args.persistent else "monolithic"
    observation = args.label or ("observed" if args.observed else "baseline")
    output = build_output / f"herdr-{args.rows}x{args.columns}-{mode}-{observation}"
    output.mkdir(exist_ok=True)
    outcomes = [run_case(binary, output, args.herdr, index, args.rows, args.columns, args.persistent)
                for index in range(args.runs)]
    (output / "herdr-cli-outcomes.json").write_text(json.dumps(outcomes, indent=2) + "\n")


if __name__ == "__main__":
    main()
