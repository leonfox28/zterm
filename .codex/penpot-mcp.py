# /// script
# requires-python = ">=3.11"
# dependencies = ["mcp==1.26.0"]
# ///
"""Project-local stdio transport for Penpot; keep its token out of config/argv."""

import logging
import sys
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

import anyio
from mcp.client.streamable_http import streamablehttp_client
from mcp.server.stdio import stdio_server


async def main():
    url = Path(__file__).with_suffix(".url").read_text().strip()
    parsed = urlsplit(url)
    if (
        parsed.scheme != "https"
        or parsed.netloc != "design.penpot.app"
        or parsed.path != "/mcp/stream"
        or not parse_qs(parsed.query).get("userToken")
    ):
        raise ValueError("Expected the official Penpot MCP URL")

    # HTTP exception/log messages can contain the URL's secret query parameter.
    logging.disable(logging.CRITICAL)

    async with (
        streamablehttp_client(url) as (remote_read, remote_write, _),
        stdio_server() as (local_read, local_write),
        anyio.create_task_group() as group,
    ):
        async def forward(source, destination):
            async for message in source:
                if isinstance(message, Exception):
                    raise RuntimeError("Penpot transport failed")
                await destination.send(message)
            group.cancel_scope.cancel()

        group.start_soon(forward, local_read, remote_write)
        group.start_soon(forward, remote_read, local_write)


if __name__ == "__main__":
    try:
        anyio.run(main)
    except KeyboardInterrupt:
        pass
    except Exception:
        print("Penpot MCP connection failed; check .codex/penpot-mcp.url and network access.", file=sys.stderr)
        sys.exit(1)
