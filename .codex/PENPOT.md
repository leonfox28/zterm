# Penpot MCP for this project

The `penpot` entry in `.codex/config.toml` is project-scoped. Run Codex from
this repository root in a trusted checkout. No user-level MCP registration is
needed. Restart/reload the client's MCP tools after changing configuration.

The official hosted Penpot server requires a secret `userToken` in its URL.
`.codex/penpot-mcp.py` uses the official Python MCP SDK to forward stdio messages
to that server without putting the secret in tracked configuration, command
arguments, or HTTP logs. It preserves the server's initialization instructions
and tool schemas. `uv` installs the script's pinned SDK dependency on first use.

## Local credential setup

1. In Penpot, open Your account → Integrations → MCP Server.
2. Copy the existing server URL into `.codex/penpot-mcp.url` as one plain-text
   line. This file is ignored by Git. Do not paste the URL into task notes,
   screenshots, or `.codex/config.toml`.
3. On macOS/Linux, run `chmod 600 .codex/penpot-mcp.url`.
4. Open the intended Penpot design file and click MCP / Connect. Keep that
   browser tab open and connected while working.
5. Reload MCP tools or reopen the Codex session. `codex mcp get penpot` checks
   configuration discovery; it does not by itself prove a working connection.

The registration is project-scoped, but Penpot operates on the active connected
browser page. Always inspect the file/page IDs before editing; changing the
active Penpot file can change the target. Read `high_level_overview` once before
using its API tools, then use `penpot_api_info` for unfamiliar APIs.

## Paseo reload issue observed on 2026-09-06

With the installed Paseo 0.7.2 and Codex CLI 0.153.4, Reload agent failed with
`already has an active writer`. Inspection of the installed agent manager showed
that it calls `resumeSession` before `closeReloadedSession`, while the original
Codex process still holds the thread's writer lock. This is a session handoff
failure; the Penpot transport had already passed its read/write checks.

After the active reply finishes, release the original agent's Codex process
before resuming the same saved thread. Identify the current owning PID with
`lsof` on that thread's file under `~/.codex/thread-writer-locks/`, then terminate
only that verified process. Restarting the Paseo daemon is a broader alternative
that interrupts its other active agents. Merely interrupting a turn does not
close the agent session. Do not remove a live writer's lock file.

The installed application was only inspected; it was not patched or restarted.

## Reference

- [Penpot MCP setup](https://help.penpot.app/mcp/)
- [Codex MCP configuration](https://developers.openai.com/codex/mcp/)
- [Official Python MCP SDK](https://github.com/modelcontextprotocol/python-sdk)
