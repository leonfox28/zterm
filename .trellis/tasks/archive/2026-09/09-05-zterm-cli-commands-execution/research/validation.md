# Validation evidence

## Scope and isolation

All runtime tests use task-private UserPaths and existing process/PTY fixtures.
No actual account daemon, installed executable or identity was changed. No
real macOS Iroh acceptance, signed release, deployment or publication ran.

## Focused implementation gates

- `cargo +1.98.0 test -p zterm-daemon --lib`: 209 passed; one ignored child
  helper is executed by its parent capture test. The Session log test uses an
  isolated child process to avoid parallel tracing callsite-interest interference.
- `cargo +1.98.0 test -p zterm-cli`: 62 library tests passed, three existing
  helpers ignored; main ticket writer and all integration targets passed.
- `cargo +1.98.0 test -p zterm-daemon --test local_session_ipc --test local_device_ipc --test terminal_recovery`:
  passed.
- CLI and daemon Clippy with `--all-targets -- -D warnings`: passed.
- Root separately ran `cargo +1.98.0 test -p zterm-cli --test daemon_autospawn`:
  passed after adding the real PTY confirmation case. A detached main remains
  live while the English prompt is shown; y completes stop in that invocation,
  then configured restart succeeds with an empty registry.
- Actual compiled help for top-level/logs/daemon stop/pair accept/session list
  matches the new public syntax. Help was the only real binary invocation.
- Task manifest validation: 9 real entries per action, no missing/truncated
  context warnings after condensing distribution context.

## Behavioral coverage

CLI parser/input fixtures cover removed JSON/force/name flags, y/yes and direct
confirmation, defaults, safely quoted aliases and human diagnostics. Session
registry/IPC tests cover admitted work retained by an unapproved stop and
approved bounded cleanup. Existing log-tail fixtures retain no-autospawn and
line/byte bounds; logs follow has no implementation or continuous reader.

Actual Session create/replay/end/controller and network changes are captured
without terminal/cwd/Relay sentinel content. The existing pair-create/replay
fixture captures a single offer event and checks that the returned ticket is
absent. Explicit-close/daemon-stop reasons now survive a racing driver health
observation instead of being mislabeled as driver failure.

Update startup is verified through the injected post-commit policy (configured
running/stopped, no setup, startup failure, installed version/wire/schema
mismatch) plus existing launcher/activation/trust fixtures and code composition
review. This is not a signed end-to-end executable replacement run.

## Final integration

Independent check completed with no remaining verified blocker. It fixed two
log semantics: prepared takeover attachment cancellation/EOF no longer claims
controller detachment, and ordinary broker closure says transport_closed rather
than endpoint_reset. The extended isolated fixture verifies the original
controller remains effective and only a real detach emits that event. Focused
capture, both-package all-target Clippy, fmt and diff checks passed after fixes.

Root-owned `just check` completed with exit 0. It passed portable policy,
workspace Clippy/tests, secret scan, documentation, both dependency/advisory
checks, isolated Relay probe compile/lint, upstream artifact checks and Relay
static checks. Log: `/tmp/zterm-cli-check.oBnHhF`.

Hosted Linux/other supported hosts, glibc floor, real network execution and
protected signed distribution retain their existing hosted evidence boundary.
Final `git diff --check` passed. No product code changed after this gate.
