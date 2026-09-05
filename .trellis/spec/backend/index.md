# Backend Development Guidelines

> Best practices for backend development in this project.

---

## Overview

This directory contains guidelines for backend development. Fill in each file with your project's specific conventions.

---

## Guidelines Index

| Guide | Description | Status |
|-------|-------------|--------|
| [Directory Structure](./directory-structure.md) | Module organization and file layout | To fill |
| [Database Guidelines](./database-guidelines.md) | ORM patterns, queries, migrations | To fill |
| [Error Handling](./error-handling.md) | Error types, handling strategies | To fill |
| [Quality Guidelines](./quality-guidelines.md) | Code standards, forbidden patterns | To fill |
| [Logging Guidelines](./logging-guidelines.md) | Structured logging, log levels | To fill |
| [Relay Infrastructure and Deployment Contract](./relay-deployment.md) | Official N0 default plus optional self-hosted Relay contracts | Active |
| [Host-Authoritative Terminal Model Contract](./terminal-model.md) | Host-only Alacritty boundary, ingress caps, semantic projection, safe replies, snapshots, deltas, and history windows | Active |
| [PTY Lifecycle Contract](./pty-lifecycle.md) | Account login shell, PTY ownership, validation, and termination authority | Active |
| [Retained Terminal Driver Contract](./terminal-driver.md) | Bounded PTY drain, latest-only attachments, and transport-independent lifetime | Active |
| [Core and Wire Domain Contract](./core-wire-domain.md) | Shared IDs, revisions, operation replay, semantic wire-v2 DTOs, framing, and protocol limits | Active |
| [Effective-User State Contract](./effective-user-state.md) | Per-user paths, configuration, identity, SQLite, and safe file operations | Active |
| [Local Daemon and IPC Contract](./local-daemon-ipc.md) | Same-UID Unix IPC, opaque remote Session tunneling, frontend attachment state, and sole desktop presentation | Active |
| [Persistent Session Service Contract](./session-service.md) | Daemon-lifetime sessions, attachments, controller leases, and count/dimension admission | Active |
| [Iroh Transport, Pairing, and Device Authorization Contract](./transport-auth.md) | Endpoint ownership, route fallback, pairing, directional authorization, revoke, security, and platform evidence boundaries | Active |
| [Signed Distribution and Executable Lifecycle Contract](./distribution-lifecycle.md) | Native Release trust, installer, explicit update/rollback, uninstall, and protected draft workflow | Active |

---

## How to Fill These Guidelines

For each guideline file:

1. Document your project's **actual conventions** (not ideals)
2. Include **code examples** from your codebase
3. List **forbidden patterns** and why
4. Add **common mistakes** your team has made

The goal is to help AI assistants and new team members understand how YOUR project works.

---

**Language**: All documentation should be written in **English**.
