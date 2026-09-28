# Zterm logging audit

Date: 2026-09-26. Baseline: `main` at `acecb45` (v0.1.36).
Branch: `feat/logging-improvements`.

This is a source/configuration/test audit, not an inspection of deployed hosts or
private log files. Existing tests were inspected, not executed in this planning
pass. No product code has changed. Proposed priorities are not approved scope.

## 1. Producers, sinks and retention

| Producer | Actual behavior | Evidence |
| --- | --- | --- |
| Desktop daemon | Detached stdout/stderr append to the effective account's `~/.zterm/logs/daemon.log`. Only the internal daemon installs the text tracing subscriber: targets enabled, ANSI disabled, INFO/WARN/ERROR enabled. | `crates/platform/src/local_unix.rs:279`; `crates/platform/src/user_state.rs:89`; `crates/daemon/src/lifecycle.rs:184`, `:624` |
| Interactive CLI startup | Explicit best-effort writer to the same file, enabled only for existing committed setup; reopens each record. Contains PID/invocation ordinal/version/stage/category. | `crates/daemon/src/connection_progress.rs:14`, `:25` |
| One-shot updater | Separate writer to the same file, enabled during handoff preparation; reopens and syncs each record. | `crates/daemon/src/update.rs:352`, `:416` |
| Ordinary CLI | Output/errors go to stdout/stderr, with no general foreground file subscriber. Displayed errors are not automatically persisted. | `crates/cli/src/main.rs:8`, `:75` |
| Android | Five production Kotlin logging sites, all gated by `BuildConfig.DEBUG`, write `Log.d("ZtermState", ...)`. Acceptance/profile tests have additional logcat output. No application-managed persistent diagnostic sink/export flow was found. | `apps/android/app/src/main/java/io/github/leonfox28/zterm/AppRepository.kt:46`, `:140`, `:292`, `:446`; `apps/android/app/src/main/java/io/github/leonfox28/zterm/TerminalNotifications.kt:101` |
| Shared Rust client / Android bridge | Typed state/errors, but no logging subscriber or direct tracing dependency. Optional startup observations are not a general client event history. | `crates/client/Cargo.toml:8`; `crates/android/Cargo.toml:12`; `crates/client/src/progress.rs:113`; `crates/android/src/network.rs:101` |
| Self-hosted relay | Upstream container stdout/stderr use Docker's `local` log driver, separately from `zterm logs`. Runtime contents were not inspected. | `deploy/relay/compose.yaml:3`; `deploy/relay/README.md:28`; `docs/relay.md:42` |

The daemon filter is a global maximum level, not an application-target allowlist;
dependency INFO/WARN/ERROR may also reach the file. `RUST_LOG` is not consumed by
the initializer; workspace tracing-subscriber enables `fmt` and `registry`, not
`env-filter` (`Cargo.toml:54`). This audit does not claim a demonstrated
dependency-content leak.

- **Rotation:** only before spawning a daemon, at **4 MiB or more**, replace
  `daemon.log.1` with the previous current file. Neither file has a runtime hard
  cap (`crates/daemon/src/lifecycle.rs:33`, `:141`, `:585`).
- **Reading:** `zterm logs [-n|--lines N]` reads only the current file, once;
  default 100 lines, maximum 1,000. It seeks to the final 1 MiB based on initial
  metadata, then reads to EOF. Missing/empty logs have an English explanation;
  explicit zero lines stay empty. Reading does not start the daemon
  (`crates/cli/src/lib.rs:373`, `:1483`; `crates/daemon/src/operations.rs:1284`).
- **Read-bound defect:** concurrent appends after the metadata sample can make
  `read_to_end` consume more than 1 MiB. The initial seek can also cut a long
  record. These are source-derived edge cases, not reproduced runtime incidents
  (`crates/daemon/src/operations.rs:1303`).
- **Inventory:** reset/uninstall permits exactly `daemon.log` and `daemon.log.1`;
  extra filenames need coordinated ownership changes
  (`crates/platform/src/user_state.rs:652`).
- No time/level/Session filter, archive selection, follow mode, or dedicated
  incident export exists in the current CLI log arguments.

## 2. Events already recorded

| Area | Events and fields | Evidence |
| --- | --- | --- |
| Daemon | Ready with version/PID; stopping; listener recovery; cleanup/rebind/listener/handler failures. Some warnings have only error kind and message. | `crates/daemon/src/lifecycle.rs:362`, `:423`, `:453`, `:508`; `crates/daemon/src/local_ipc.rs:236`, `:280` |
| Session | Creation with ID/name; end with ID and natural exit, explicit close, daemon stop or driver failure. Natural exit includes exit code/signal when available; cleanup/driver failures WARN. | `crates/daemon/src/session.rs:1286`, `:3339` |
| Controller | Attach/detach/takeover with Session and attachment IDs. Ordinary detach INFO; prepared takeover cancellation does not falsely detach the current controller. | `crates/daemon/src/session.rs:3400`, `:3790`, `:4130`, `:4163` |
| Network | Previous/new endpoint state, reason, publish/lookup state. Degraded WARN, other transitions INFO; unchanged counters/RTT silent. | `crates/daemon/src/network.rs:286` |
| Primary connection | Established/closed and reason; no local connection/peer correlation in these records. | `crates/daemon/src/connection_broker.rs:2509` |
| Pairing | Offer created with TTL, offer failed, outbound acceptance committed/failed, inbound authorization committed with generation; request failures with request ID/category. Coverage does not include every early return. | `crates/daemon/src/pairing.rs:553`; `crates/daemon/src/pairing_service.rs:473`, `:857`; `crates/daemon/src/service.rs:583` |
| Revocation | Revoked with generation; request failure with request ID/category. Success logged after connection/Session cleanup. | `crates/daemon/src/service.rs:715`, `:800`, `:824` |
| Initial interactive connection | Terminal init, local service, target/channel resolution, connection reuse/dial/handshake/selection, Session create/attach, snapshot/display/sync, ready or failed/cancelled/ended. Timestamp/PID/invocation ordinal/version/stage/category. | `crates/core/src/connection_progress.rs:5`; `crates/daemon/src/connection_progress.rs:25` |
| Update after handoff | Awaiting handoff, acceptance, continuing/stopping/activating/starting, committed activation, success/failed/partial-completion; PID, authenticated target version, accepted flag/category. | `crates/daemon/src/update.rs:320`, `:352`, `:389` |
| Android debug | Terminal state/error/viewport, operation category/exception class, reminder-save and notification failures. | Kotlin emitters in section 1 |

The detailed startup taxonomy already exists and should be reused. Its observer
retires at the first Active input/presentation fence, so it does not cover later
reconnects (`crates/cli/src/terminal_ui/session.rs:960`;
`crates/client/src/progress.rs:179`).

## 3. Prioritized gaps and proposed improvements

P1: missing/misleading persistent incident evidence or incomplete bounds.
P2: diagnostic depth/usability beyond the baseline. Both remain proposals.

### G1 — P1: Android release diagnostic history

All five application emitters are debug-only. The Rust bridge/client has no
persistent sink. OS logcat/crash output may exist, but a release user has no
application-controlled durable history or export experience.

Propose bounded app-private logs in release builds for client/network/lifecycle
outcomes and explicit local export. Existing state owners already expose
reconnect/sync/lease loss and network changes; do not log each frame or IME
callback. Evidence: section 1; `crates/android/src/terminal.rs:958`;
`apps/android/app/src/main/java/io/github/leonfox28/zterm/PlatformNetwork.kt:12`;
`apps/android/app/src/main/java/io/github/leonfox28/zterm/AppRepository.kt:78`.

### G2 — P1: Actionable failures missing from default logs

- Local attachment/tunnel errors are DEBUG and suppressed by the production
  INFO ceiling (`crates/daemon/src/local_ipc.rs:371`, `:406`).
- The broker discards the remote service handler's result
  (`crates/daemon/src/connection_broker.rs:2324`). Session unary failures become
  replies without a general operation-failure record
  (`crates/daemon/src/session_wire.rs:912`).
- Upload success/cancel/failure exists on the wire without an owner event
  (`crates/daemon/src/session_wire/upload.rs:10`, `:122`, `:126`).
- Early pairing validation/admission precedes outcome emitters
  (`crates/daemon/src/pairing.rs:520`). Normal handshake rejection/overload also
  lacks a consistent bounded application event policy.

Propose classified outcomes at existing owners, with expected detach/cancellation
kept separate from faults. Summarize/rate-limit repeated rejection and overload.
Upload records may include safe size/duration/outcome, never names/paths/content.

### G3 — P1: Early update failures and failure severity

`log_enabled` starts false; preparing/verified happen before handoff enables it.
Download/verification failures and pre-handoff cancellation can leave no file
outcome even for configured users. Every updater record is INFO, including
failure and partial completion (`crates/daemon/src/update.rs:297`, `:309`, `:320`,
`:352`, `:374`, `:416`).

Propose safe configured-user logging before preparation and correct stage/outcome
severity. Preserve the existing no-setup/no-state-creation behavior.

### G4 — P1: Correlation and duration

Session/attachment IDs, startup PID/ordinal, pairing request IDs and bare primary
connection events cannot reliably join a concurrent multi-process incident.
Most events have no elapsed duration. No application tracing spans were found.
Evidence: `connection_progress.rs:42`; `connection_broker.rs:2525`;
`session.rs:1286`; `service.rs:583` (all in `crates/daemon/src/`).

Propose consistent process-instance and local operation/connection references,
with safe Session/attachment references and durations where useful. Do not
reuse proof/nonces or capability-bearing IDs as correlation. Cross-device
correlation needs an explicit contract; request IDs are not globally unique.

### G5 — P1: Runtime storage and strict read bounds

Startup rotation cannot bound a long-running daemon; additional coverage raises
event volume. The current seek-based tail can exceed its intended byte budget
under appends. See section 1 for evidence.

Propose runtime size retention and a hard read bound. Rotation must coordinate
the daemon's inherited open stdout/stderr descriptors with startup/updater
writers that reopen each event: naive rename can strand daemon output in the
archive (`local_unix.rs:286`; `connection_progress.rs:26`; `update.rs:357`).
Preserve safe-path checks, reset/uninstall ownership and successful business
results when diagnostics fail. This extends the previous retention contract.

### G6 — P1: Durable commit versus cleanup failure

Revocation persists and publishes authorization before connection closure and
Session detach. A later failure emits `revoke_failed` and bypasses `revoked`,
omitting the durable change (`crates/daemon/src/service.rs:800`, `:805`, `:810`,
`:824`). Pairing errors may appear at both owner and request adapter without
shared correlation (`pairing_service.rs:478`; `service.rs:583`).

Propose distinct committed-change and cleanup/request outcomes with safe
correlation, deduplicated at the existing owner. No extra mutation registry.

### G7 — P2: Ongoing connection and terminal recovery

Direct/relay changes update status/metrics, without an explicit application
path event (`connection_broker.rs:2452`, `:2625`); the network transition
comparator excludes these counts (`network.rs:286`). Startup logging stops at
ready (`crates/cli/src/terminal_ui/session.rs:960`). Shared client/native
reconnect, sync and lease boundaries lack a durable event sink.

Propose transition-only reconnect start/outcome/retry count, direct/relay change,
resync cause/completion and abnormal terminal/driver termination. Durations and
safe numeric revision/viewport/generation fields can diagnose stuck input or
presentation without recording contents. Detailed render/resize/IME metadata
should follow the user's collection policy, not be emitted each frame by default.

### G8 — P2: Format, level and crash context consistency

Three formatters use tracing text time, Unix milliseconds, and Unix seconds.
Fields vary among `reason`, `category`, `error_kind` and message-only warnings;
PID/version are not universal. Filtering is fixed and includes dependencies.
Evidence: `lifecycle.rs:624`; `connection_progress.rs:31`; `update.rs:368`.

Propose one safe field/time/level contract and application diagnostic filtering.
Structured export may be useful; arbitrary dependency DEBUG is not required.
Daemon stderr is captured, while the terminal deliberately suppresses panic
text and returns a safe error; after startup retirement it lacks an ongoing
file outcome (`crates/cli/src/main.rs:24`;
`crates/cli/src/terminal_ui.rs:194`, `:235`, `:1041`). Standardize fatal outcomes
without dumping raw panic values. No actual content leak is claimed here.

### G9 — P2: Incident retrieval

CLI retrieval cannot select time/level/Session, include an archive or export
safe build/platform context; Android lacks an end-user equivalent. Rotation can
split the stages of one attempt across files (`crates/cli/src/lib.rs:373`;
`crates/daemon/src/operations.rs:1284`).

Propose bounded local retrieval/export with clear archive semantics. Preserve
no-autospawn inspection. Previous scope explicitly excluded follow; keep it
excluded unless requested (`.trellis/tasks/archive/2026-09/09-05-zterm-cli-commands-execution/prd.md:39`).
Remote upload and centralized telemetry remain separate scope.

## 4. Existing validation to extend

- Committed Session/network events, replay deduplication, normal detach levels,
  unchanged-state silence and terminal/input/cwd/relay sentinels:
  `crates/daemon/src/session.rs:5787`.
- Pairing-safe Debug/display/tracing and committed offer/replay behavior:
  `crates/daemon/tests/pairing_secrets.rs:83`;
  `crates/daemon/src/pairing_service.rs:2085`.
- Startup correlation, rotation reopen, retirement, missing setup and symlinks:
  `crates/daemon/src/connection_progress.rs:134`.
- No-autospawn inspection and 1,000-line tail:
  `crates/cli/tests/daemon_autospawn.rs:262`.
- Updater completion persistence across frontend loss/rotation and no setup:
  `crates/daemon/src/update/tests.rs:343`, `:576`.

New tests should exercise approved behavior at real owners: release Android
persistence/export; early update failure; revoke commit then cleanup failure;
multiwriter runtime rotation; concurrent tail growth; safe correlation; bounded
noise. Avoid tests that only mirror hard-coded formatting strings.

## 5. Recommended direction and resolved collection decision

1. Unify safe event contracts/correlation, fill P1 outcome gaps, and complete
   storage/read bounds using existing owners.
2. Include release Android and shared/native client diagnostics.
3. Default to sparse useful events; enable bounded detailed diagnosis explicitly;
   provide user-controlled local export.
4. Exclude transcript/secret collection, remote telemetry, new Session/connection
   state owners, `logs -f`, and general-purpose profiling systems.

**User decision, 2026-09-28:** persist key events by default and enable detailed
diagnostics on demand. This keeps normal I/O/noise lower; detailed reproduction
may be needed for subtle faults. Retention, controls and export are specified
in the planning design. This audit itself does not authorize implementation.
