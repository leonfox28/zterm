# Proposed commit plan

Status: user approved on 2026-09-05; work commit completed: `b4941e062cbd0772c6be469b1a23e322c8e99638`.

## Work commit

`feat: simplify CLI workflows and daemon diagnostics`

This single change set contains the selected human CLI, conditional Session
confirmation, updated-daemon startup, one-shot logs and existing event coverage,
with matching tests/docs/specs. Version and release publication are unchanged.

Files:

- `.trellis/spec/backend/distribution-lifecycle.md`
- `.trellis/spec/backend/effective-user-state.md`
- `.trellis/spec/backend/index.md`
- `.trellis/spec/backend/local-daemon-ipc.md`
- `.trellis/spec/backend/logging-guidelines.md`
- `.trellis/spec/backend/session-service.md`
- `Cargo.lock`
- `README.md`
- `crates/cli/Cargo.toml`
- `crates/cli/src/lib.rs`
- `crates/cli/tests/command_side_effects.rs`
- `crates/cli/tests/daemon_autospawn.rs`
- `crates/cli/tests/setup_permissions.rs`
- `crates/daemon/src/client/ipc.rs`
- `crates/daemon/src/connection_broker.rs`
- `crates/daemon/src/lifecycle.rs`
- `crates/daemon/src/network.rs`
- `crates/daemon/src/operations.rs`
- `crates/daemon/src/pairing.rs`
- `crates/daemon/src/pairing_service.rs`
- `crates/daemon/src/service.rs`
- `crates/daemon/src/session.rs`
- `crates/daemon/tests/local_device_ipc.rs`
- `crates/daemon/tests/local_session_ipc.rs`
- `crates/daemon/tests/terminal_recovery.rs`
- `docs/core-local-daemon.md`
- `docs/install.md`
- `docs/remote-cli.md`

## Bookkeeping after the work commit

Archive this completed Trellis task and record the session using the existing
Trellis scripts. Their archive and journal commits follow the work commit.
The user subsequently authorized the release workflow for v0.1.19; preparation follows task bookkeeping.

## Other changes

No unrecognized dirty paths. The untracked task directory contains this task
and its planning/research/validation artifacts; its archive belongs to the
bookkeeping step.
