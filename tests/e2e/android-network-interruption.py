#!/usr/bin/env python3
"""Interrupt only a disposable emulator app's transport, restoring rules on exit."""

import argparse
from pathlib import Path
import subprocess
import time
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--serial", required=True)
    parser.add_argument("--adb", default="adb")
    parser.add_argument("--host-name", default="presentation-fixture")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not args.serial.startswith("emulator-") or not args.host_name.startswith("presentation-"):
        parser.error("use an explicit emulator and a disposable presentation host")
    adb = [args.adb, "-s", args.serial]
    package = "io.github.leonfox28.zterm.dev"

    def call(*command, check=True):
        return subprocess.run(adb + list(command), capture_output=True, text=True,
                              timeout=10, check=check)

    if call("shell", "id", "-u").stdout.strip() != "0":
        parser.error("the disposable emulator must support adb root")
    uid = call("shell", "run-as", package, "id", "-u").stdout.strip()
    if not uid.isdigit() or int(uid) < 10000:
        parser.error("unexpected test app UID")
    call("shell", "run-as", package, "rm", "-f", "cache/network-ready", "cache/network-interrupted")
    rule = ["-m", "owner", "--uid-owner", uid, "-m", "comment", "--comment",
            "zterm-fixture-" + uuid.uuid4().hex, "-j", "DROP"]
    installed = []
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w") as log:
        test = subprocess.Popen(adb + [
            "shell", "am", "instrument", "-w", "-r",
            "-e", "networkInterruption", "1", "-e", "hostName", args.host_name,
            "-e", "class", "io.github.leonfox28.zterm.NetworkInterruptionTest",
            package + ".test/androidx.test.runner.AndroidJUnitRunner",
        ], stdout=log, stderr=subprocess.STDOUT)

        def wait_marker(name, seconds):
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                if test.poll() is not None:
                    raise RuntimeError(f"instrumentation ended before {name}; see {args.output}")
                if call("shell", "run-as", package, "test", "-f", "cache/" + name,
                        check=False).returncode == 0:
                    return
                time.sleep(0.2)
            raise TimeoutError(name)

        try:
            wait_marker("network-ready", 60)
            try:
                for binary in ("iptables", "ip6tables"):
                    call("shell", binary, "-I", "OUTPUT", "1", *rule)
                    installed.append(binary)
                print("Dropped only the test app's IPv4/IPv6 transport.", flush=True)
                wait_marker("network-interrupted", 105)
                print("Client entered reconnecting and rejected disconnected input.", flush=True)
            finally:
                failures = []
                for binary in reversed(installed):
                    result = call("shell", binary, "-D", "OUTPUT", *rule, check=False)
                    if result.returncode:
                        failures.append(binary)
                if failures:
                    raise RuntimeError(f"could not restore {failures}; rule: {rule}")
                print("Restored the test app's transport.", flush=True)
            test.wait(timeout=65)
        finally:
            if test.poll() is None:
                call("shell", "am", "force-stop", package, check=False)
                test.wait(timeout=10)
    result = args.output.read_text()
    print(result)
    if "OK (1 test)" not in result:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
