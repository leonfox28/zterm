# Logging gap analysis

- Query: determine how to improve current daemon log content with minimal machinery.
- Scope: application event emitters, subscriber, file ownership, tail/rotation,
  and existing state/Session/network owners.
- Date: 2026-09-05.
- Latest decision: the user excluded logs -f/--follow. Improve existing event
  coverage and retain one-shot tail reading; no continuous reader is in scope.

## Current evidence

- A source scan finds only 13 explicit tracing event sites in daemon source:
  8 in `crates/daemon/src/lifecycle.rs`, 5 in `local_ipc.rs`. CLI/platform have
  no explicit tracing sites. Dependencies can emit additional tracing events,
  so this is an application-source count, not a count of all possible log lines.
- Lifecycle records ready/stopping/listener recovery and cleanup failures;
  local IPC records listener/handler failures and two DEBUG-level attachment/
  tunnel errors. Session lifecycle, pairing outcomes and network transitions
  have no application-owned tracing instrumentation in their owning modules.
- `lifecycle.rs:603` installs the default text fmt subscriber, without ANSI or
  target display. There is no explicit Zterm event policy or configured filter.
  `Cargo.toml:47` enables fmt/registry, not env-filter.
- `platform/src/local_unix.rs:279` opens the managed daemon.log and redirects
  the detached child's stdout/stderr to it. The subscriber runs only inside
  the daemon entry; tracing calls added solely to CLI update operations would
  not automatically be persisted by that subscriber.
- `lifecycle.rs:564` checks size only before daemon launch. At >=4 MiB it
  replaces daemon.log.1 with the preceding daemon.log. It is not a runtime
  hard-cap on a long-running daemon's log size.
- `platform/src/user_state.rs:651` permits exactly daemon.log and daemon.log.1
  in the managed logs directory. Adding other filenames without updating this
  owner would make reset/uninstall refuse the managed inventory.
- `operations.rs:1045` reads only the current file's recent tail (default 100
  lines, max 1000 lines and 1 MiB). Missing file returns an empty result; there
  is no follow reader, filtering flag, or application-specific retention config.
- `.trellis/spec/backend/logging-guidelines.md` is still a placeholder. Actual
  constraints currently live in local-daemon, transport-auth, effective-state,
  and Session contracts; fill this guideline from the implemented result.

## Existing event owners to reuse

| Observable event | Owning code / evidence |
| --- | --- |
| Daemon ready, stop, cleanup/recovery failure | lifecycle.rs:356, :411 and :436 onward |
| Session publication and natural/explicit end | session.rs:1114, :2796, :3071, :3083 |
| Controller attachment, takeover, detach | session.rs:3273, :3418, :3507, :3772 |
| Network state and degradation category | NetworkReporter::update, network.rs:286; relay state at :1141 |
| Peer primary connection promoted/removed | connection_broker.rs:2188 and :2436 |
| Pair creation/accept result and revoke | pairing_service.rs operation commit owners; service.rs device_revoke_reply |
| Local request/attachment failure | local_ipc.rs:372, :407 and unary dispatch :420 onward |

Record committed lifecycle transitions at their actual owner, not again in each
CLI/IPC/remote adapter. Retries/replayed mutations must not manufacture duplicate
created/closed events. Request failures may record operation kind and stable
error category, but must not decode a second copy of payloads to obtain log data.

## Recommended scope

1. Human-readable English operational logs with timestamp, level, component,
   event/outcome, safe Session correlation and typed reason. Include daemon
   version/PID at startup so restarts/updates can be recognized.
2. Session create/end (with typed natural exit status or explicit close reason),
   controller attached/detached/taken over; normal detach is not a warning.
3. Network degraded/recovered and primary peer connection established/closed.
   Record state/path-class changes and typed errors rather than every poll,
   frame, stream counter, RTT sample or keepalive. No logging-owned observer
   or duplicate connection registry is needed.
4. Pair operation success/failure and authorization revoke without ticket bytes,
   keys, proof/nonces, payload dumps or untrusted peer-authored error text.
5. WARN/ERROR for failures needing attention with operation/stage and stable
   reason; INFO for normal committed lifecycle changes. Default logs must be
   useful without a debug flag. Avoid enabling raw dependency debug logs as a
   substitute for application instrumentation.
6. Keep updater phase information on the invoking CLI and daemon startup/stop
   version events in daemon.log. The daemon is intentionally stopped during
   activation; do not pretend its subscriber captures the old CLI's work or
   add another unsynchronized file writer just to persist these phases.
7. Reuse the current managed file inventory and startup rotation in this slice.
   State-change-only event volume limits routine noise; no new continuous
   rotation/retention engine or log-forwarding service is justified by the
   current evidence. Document the existing startup-only size check accurately.

## Log content boundaries

Use validated bounded Session names/IDs and local opaque correlation as needed.
Do not log remote Device IDs, addresses or Relay URLs from selected-path
sidebands (`local-daemon-ipc.md:279`), identity keys, tickets, proofs, clipboard,
terminal input/output, cwd or environment dumps. Preserve typed error categories
instead of formatting a full request/error/source object that may hold these.
Logging failures must not change successful Session/connection operations.

## Reading remains one-shot

Add -n as an alias of --lines and an English empty-log message. Reuse
LocalRuntime::log_tail with its existing limits, paths and no-autospawn behavior.
The rejected follow proposal would have needed a continuous reader, not another
recording pipeline; both the follow feature and its supporting state machine
are now excluded. Do not add watchers, offsets retained between CLI invocations,
rotation-follow fixtures, or extra managed log files.

## Meaningful evidence

- Isolated text subscriber capture verifies committed Session create/end,
  network degradation/recovery, and classified failure fields. Assert absence
  of a sentinel terminal/ticket/cwd value, not merely a hard-coded redaction
  string. Normal detach and unchanged state must not generate warning spam.
- Existing bounded-tail/no-autospawn fixtures remain the reading evidence;
  adapt them for -n and the empty-log text without adding follow scenarios.
- Keep the current managed inventory and reset/uninstall fixture passing.
  Do not mutate the user's real ~/.zterm or run real Iroh on developer macOS.
