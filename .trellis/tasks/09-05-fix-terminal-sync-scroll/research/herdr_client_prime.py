"""Prime only an isolated Herdr server, verify pane I/O, then detach its client."""

import fcntl
import json
import os
import pty
import select
import struct
import subprocess
import sys
import termios
import time


def main():
    program, rows, columns = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, 0, 0))
    child = subprocess.Popen([program], stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
    os.close(slave)
    received = bytearray()
    sent = False
    try:
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and child.poll() is None:
            if select.select([master], [], [], 0.1)[0]:
                data = os.read(master, 65536)
                if not data:
                    break
                received.extend(data)
            if b"spaces" in received and not sent:
                os.write(master, b"printf '\\132TERM_PRIME_READY\\n'\r")
                sent = True
            if b"ZTERM_PRIME_READY" in received:
                break
        assert b"ZTERM_PRIME_READY" in received, "isolated Herdr pane did not confirm readiness"
        os.write(master, b"\x02q")
        child.wait(timeout=5)
        assert child.returncode == 0, child.returncode
        status = subprocess.run([program, "status", "server", "--json"], capture_output=True,
                                text=True, check=True, timeout=5)
        running = json.loads(status.stdout)["running"]
        assert running, "primed server must survive client detach"
        print(json.dumps({"prime_pane_roundtrip": True, "prime_client_detached": True,
                          "prime_server_running": running, "rows": rows, "columns": columns}))
    finally:
        if child.poll() is None:
            child.terminate()
            child.wait(timeout=5)
        os.close(master)


if __name__ == "__main__":
    main()
