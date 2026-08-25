#!/usr/bin/env python3
"""Small TLS file server used only by hosted installer acceptance jobs."""

from __future__ import annotations

import http.server
import pathlib
import socketserver
import ssl
import sys


class FixtureHTTPServer(http.server.ThreadingHTTPServer):
    """HTTP server whose bind path never performs a hostname lookup."""

    def server_bind(self) -> None:
        # The standard HTTP bind hook performs a fully qualified hostname
        # lookup, which can stall behind macOS local-network privacy after the
        # socket is bound but before it listens. TCPServer binds without DNS.
        socketserver.TCPServer.server_bind(self)
        self.server_name, self.server_port = self.server_address[:2]


def main() -> None:
    if len(sys.argv) != 6:
        raise SystemExit("usage: https_fixture.py ROOT CERT KEY PORT_FILE LOG_FILE")
    root, certificate, key, port_file, log_file = map(pathlib.Path, sys.argv[1:])

    class Handler(http.server.SimpleHTTPRequestHandler):
        def __init__(self, *args: object, **kwargs: object) -> None:
            super().__init__(*args, directory=str(root), **kwargs)

        def log_message(self, _format: str, *args: object) -> None:
            del args
            with log_file.open("a", encoding="utf-8") as output:
                output.write(f"{self.path}\n")

    server = FixtureHTTPServer(("127.0.0.1", 0), Handler)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(certificate, key)
    server.socket = context.wrap_socket(server.socket, server_side=True)
    port_file.write_text(str(server.server_port), encoding="ascii")
    server.serve_forever()


if __name__ == "__main__":
    main()
