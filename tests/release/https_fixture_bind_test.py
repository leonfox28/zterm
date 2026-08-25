#!/usr/bin/env python3
"""Socket-free regression for the installer fixture's bind override."""

from __future__ import annotations

import http.server
import importlib.util
import pathlib
import socket
import socketserver
from unittest import mock


fixture_path = pathlib.Path(__file__).with_name("https_fixture.py")
spec = importlib.util.spec_from_file_location("zterm_https_fixture", fixture_path)
if spec is None or spec.loader is None:
    raise SystemExit("unable to load HTTPS fixture module")
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)

server = object.__new__(fixture.FixtureHTTPServer)
server.server_address = ("before-bind.invalid", 0)
bound_address = ("127.0.0.1", 43123)


def fake_tcp_bind(instance: object) -> None:
    if instance is not server:
        raise AssertionError("TCP bind received the wrong server instance")
    instance.server_address = bound_address  # type: ignore[attr-defined]


with (
    mock.patch.object(
        socketserver.TCPServer, "server_bind", side_effect=fake_tcp_bind
    ) as tcp_bind,
    mock.patch.object(
        http.server.HTTPServer,
        "server_bind",
        side_effect=AssertionError("inherited HTTPServer bind was called"),
    ),
    mock.patch.object(
        socket,
        "getfqdn",
        side_effect=AssertionError("hostname lookup was called"),
    ),
):
    fixture.FixtureHTTPServer.server_bind(server)

if tcp_bind.call_count != 1:
    raise AssertionError(f"TCP bind was called {tcp_bind.call_count} times")
if (server.server_name, server.server_port) != bound_address:
    raise AssertionError("fixture did not publish the post-bind address")
if not issubclass(fixture.FixtureHTTPServer, http.server.ThreadingHTTPServer):
    raise AssertionError("fixture no longer preserves threaded HTTP behavior")
