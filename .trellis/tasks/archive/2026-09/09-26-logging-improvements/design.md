# Design: bounded local diagnostics on desktop and Android

Status: approved by the user ("开始吧"), implementation active, 2026-09-28.
Confirmed policy: key events are always retained; detailed diagnostics are opt-in.
Requirements and evidence: `prd.md`; `research/logging-audit.md` (G1–G9).

## 1. Approved product behavior

| Concern | Concrete behavior |
| --- | --- |
| Default | Persist INFO lifecycle/outcome events and actionable WARN/ERROR on desktop and release Android. |
| Detailed diagnostics | Explicitly enable for 15 minutes; stop early or enable again to renew. Restarting a process preserves only the unexpired interval. No automatic renewal. |
| Desktop control | `zterm logs debug on`, `off`, `status`; changes apply locally within one second, without starting/restarting the daemon or ending Sessions. No setup means an explanation and no new state. |
| Android control | Settings contains a Diagnostics group with a detail switch, remaining-time indication, and Export logs action. Available in both build variants. |
| Desktop storage | Key: 4 MiB current + 4 MiB archive. Detail: separate 4 MiB current + 4 MiB archive. At most 16 MiB of retained event data. |
| Android storage | Key: 2 MiB current + 2 MiB archive. Detail: separate 2 MiB current + 2 MiB archive. At most 8 MiB of retained event data. |
| Retention | Rotate before an append would exceed a file limit; evict the oldest archive. Detail cannot evict key events. These are byte limits, not promises of a particular number of days. |
| Inspection | Existing `logs -n/--lines` stays readable and one-shot; include the key archive when needed. Add level/component/Session/relative-time filters and explicit inclusion of detail. |
| Export | Explicit bounded JSONL export with safe build/platform context. No network collection, automatic upload, or full config/status/identity dumps. |

The sizes and 15-minute duration were approved together in the final review.
Disabling detail stops future collection; already recorded detail remains under
its own bounded retention. A diagnostic interval affects only this OS account
or Android application, not a remote host. Hosts can enable their own interval.

## 2. Ownership and dependencies

Introduce a small `zterm-diagnostics` workspace library for the event contract,
encoding, gates, bounded dispatch, and reader/filter logic. It may depend on core
domain types and the existing serialization/time primitives, but never on the
daemon, PTY, terminal engine, Iroh, or Android UI. Keep core/proto independent of
the new library. The existing mobile dependency policy must continue to pass.

- Existing Session, broker, pairing, update and frontend owners decide when an
  event actually happened. No logger-owned Session/connection registry.
- The diagnostics library owns event IDs, safe fields, formatting, correlation,
  admission and the distinction between key and detail records.
- Desktop filesystem operations remain in `zterm-platform`; daemon composition
  supplies account-authoritative paths and setup eligibility.
- Android Kotlin owns private filesystem/control state and system export APIs.
  A narrow UniFFI sink connects the common Rust recorder to this filesystem
  owner. The Rust logger worker performs sink callbacks off Main; callbacks
  neither invoke terminal operations nor reenter the logger.
- `ZtermApplication` installs diagnostics before identity/network initialization.
  Kotlin emits through a closed set of typed application-event methods on the
  bridge, not arbitrary JSON/messages. Rust and Kotlin use the same serializer.
  Preserve existing constructors with disabled/injected recorders for fixtures.
- Logger handles/sinks are injectable. Production installs one composition per
  process. Tests must not race to replace a global subscriber; use local sinks
  or the existing exact child-test pattern for process-global integration.

This library is shared infrastructure, not a standalone logging service. No new
daemon, socket, remote RPC, collector, or telemetry backend is introduced.

## 3. Event and correlation contract

New records are one JSON object per line, schema version 1. `zterm logs` formats
them as readable English text; `--json` emits the validated structured records.
Use UTC RFC3339 timestamps with millisecond precision and monotonic operation
durations. Use the pinned time implementation already represented in Cargo.lock
instead of implementing calendar parsing/formatting.

Common envelope:

```text
schema, timestamp, level, component, event, message,
process_instance, pid, version, platform, sequence,
fields: { operation_id?, connection_id?, session_id?, attachment_id?,
          stage?, outcome?, category?, elapsed_ms?, bounded typed event fields }
```

- `event`/component/category/message come from fixed definitions; dynamic fields
  are validated IDs, booleans, enums and bounded numbers, plus already-approved
  validated Session names. No generic request/error Debug field or free-form
  payload map. A record, including newline, is at most 4 KiB.
- Process-instance and operation/connection IDs are logging-only opaque values,
  unrelated to identity keys, protocol nonces, proofs or operation capabilities.
  Scope ordinal request IDs by their owning connection/process.
- Reuse a context across clones/retries belonging to one logical local operation.
  Attach Session/attachment IDs when known. Frontend and host can correlate the
  authoritative Session ID without adding secret-bearing fields.
- This iteration does not add a distributed tracing protocol. An early failure
  before Session identity exists is correlated within its emitting process;
  do not falsely claim a globally unique request or causal ordering across hosts.
- Keep typed cause, durable commit and later cleanup outcomes distinct.
  Operation adapters may report request completion, but must not repeat the
  owner's committed lifecycle event. Replay stays silent for the original commit.
- Unknown raw dependency events are not automatically admitted. Explicitly
  classified adapter events explain transport failures without raw error trees.

## 4. Event coverage and levels

| Owner | Default key events | Opt-in detail |
| --- | --- | --- |
| Process/daemon | Start/version, ready, stop, fatal typed error, listener failure/recovery | Setup/readiness stage timing and resource-admission summaries |
| Session/controller | Create, rename, end/exit reason, attach/detach/takeover, cleanup failure | Synchronization/lease/input-fence transitions, revisions and viewport metadata |
| Transport/client | Connect result, primary established/closed, network degraded/recovered, route-class change, reconnect start/result | Safe dial/handshake stages, attempt counts/timing, resync cause and completion |
| Pair/auth | Offer/accept outcomes, authorization commit/revoke, later cleanup outcome | Bounded admission/rejection summaries, no tickets or peer-authored strings |
| Upload | Started/completed/cancelled/failed/unknown publication outcome | Byte counts and stage timing, never filename/path/content or each chunk |
| Update | Preparation/verification/acceptance/stop/activation/commit/start and final outcome | Per-stage elapsed times; no URLs/signatures/manifests/error dumps |
| Desktop frontend | Startup outcome; later terminal/driver failure, recovery and lease loss | Content-free presentation/resize/input-admission transitions and summaries |
| Android | App/native initialization, lifecycle/network transitions, connection state, operation and storage/update/notification failures | Input-epoch, geometry, sync and render summaries; no text, keys or notification bodies |

Ordinary detach, explicit cancellation and expected lease handoff are INFO.
Actionable recoverable failure is WARN; terminal/process failure preventing
continuation is ERROR. Failed/partial update outcomes must not be INFO.
Never record every frame, keystroke, packet, RTT sample or upload chunk.

Deduplicate unchanged state at its existing owner. Aggregate recurring hostile
rejection/overload by bounded component/category buckets, not a per-peer map.
High-frequency detailed summaries emit at most once per second per owner;
detail admission has an additional process-wide limit of 100 records/second.
Report suppressed/dropped counts when the sink next recovers.

The first-screen observer still retires at first Active. An independent
operational event reports later recovery; it must not replay `terminal_ready`
as another startup success. Update logging begins before preparation for an
already configured user, preserving the independent updater's lifetime and
post-commit partial-completion behavior.

## 5. Dispatch, storage and desktop cutover

Use bounded in-process dispatch, with independent key/detail budgets (up to 256
records and 512 KiB serialized bytes each). Producers do not perform disk I/O,
wait for locks, or block terminal/UI actors. Detail cannot consume key capacity.
Backpressure drops detail first; key overload is also bounded and counted.
This is best-effort diagnostics, not a lossless transaction audit ledger.

Desktop files under the existing private logs directory:

```text
daemon.log              daemon.log.1
daemon.debug.log        daemon.debug.log.1
diagnostics.json        writer.lock
```

Control state is at most 1 KiB; the stable lock file is empty. Both are owned
regular files mode 0600 inside the existing 0700 account directory. The worker
validates paths and uses one dedicated cross-process lock for rotate/open/append.
Reopen the current file inside that lock; never lock the rotating inode. Avoid
nested acquisition or reuse of a cloned locked descriptor. Reuse existing
`FileLock`/secure-file primitives; logging never acquires the lifecycle lock.
Use bounded lock retries on the writer thread, reporting losses on recovery.

All current application writers must move to this sink together. Remove the
daemon's permanent stdout/stderr append handles as the managed-record path;
use null stdio for the detached child and explicitly record internal fatal
results and a content-free panic category through the safe recorder. Install
fatal handling before runtime construction once paths are safely resolved.
If paths cannot be established, the launcher still reports the typed startup
failure. Do not promise a final record for SIGKILL/abort/power loss.

Normal shutdown requests a bounded flush. The updater retains pre-handoff path
preflight and an explicit durable-write barrier for acceptance/commit/outcome
records; later diagnostic errors never undo committed activation or convert
success into failure. Do not hold Session/auth registry locks over sink writes.

For readers, snapshot validated open handles and lengths, then release writer
coordination before reading/export. Explicit `take` bounds must cap reads under
concurrent appends. Rotation uses rename; already-open reader handles remain
usable. Never hold the writer lock for an entire export.

## 6. Detail interval control

Store a versioned start/deadline in the small local control document, atomically
replacing it under the same storage owner. It is independent of identity/config
schema and Android saved preferences. The worker refreshes it at most once per
second, also when idle, and publishes the cheap in-memory detail gate.

Every producer checks the gate before preparing detail. Missing/expired/invalid
state means OFF; control failures leave default key logging usable. Record
enable/disable/expiry as key events. Enabling again starts a fresh 15-minute
interval. Use monotonic remaining-time limits in running processes; clock
rollback or an implausible interval must not extend collection indefinitely.
Across a restart, resume only a valid unexpired interval, allowing startup
diagnosis. Inspector/status calls never create control files or directories.

## 7. Reading and export

Proposed CLI:

```text
zterm logs [-n N] [--level info|warn|error|debug] [--component NAME]
           [--session ID] [--since 30m] [--include-debug] [--json]
zterm logs debug on|off|status
zterm logs export --output PATH [--include-debug]
```

`--since` accepts a positive duration with `s`, `m`, `h` or `d`; no arbitrary
date-expression parser. No filters means the existing readable key-event tail.
Default 100/max 1,000 records and a hard total 1 MiB input budget remain. Read
newest current/archive data first, discard incomplete boundary records, and
explain that filtering searches a bounded tail rather than all historical data.
With detail included, reserve half the read budget per lane and merge records
for display; process sequence resolves ties locally, not causality across hosts.
`--level debug` implies detail inclusion. Zero lines avoids file-data reads.

Export includes a bounded metadata header (schema/version/platform and dropped,
omitted or truncation counts), followed by validated event records. Export all
selected retained files using length snapshots, not just the CLI tail. Maximum
output is the selected 16 MiB desktop / 8 MiB Android event budget plus a 4 KiB
header. Stream export without buffering the whole history. On desktop, create
the explicit output path privately and refuse overwrite; do not store exports
inside managed state. On Android, the user picks a destination with the system
create-document flow. Cancellation is ordinary cancellation; no remote host
logs, identity state or address book is bundled.

Android implements file I/O in its app-private no-backup diagnostics directory;
only the Application owns the sink. Export is available even if identity or
network initialization failed. Activity recreation never closes the logger or
native runtime. Regenerate all bridge bindings with the existing build tooling.

## 8. Compatibility, migration and risks

- Keep existing key filenames and CLI aliases. Existing text remains visible
  in unfiltered human inspection. JSON/filter/export accepts only recognized
  schema records and explains omitted legacy/unrecognized lines; it does not
  guess that arbitrary dependency text is safe structured metadata.
- Normalize oversized legacy files to a bounded complete-line tail at cutover,
  using one bounded private temporary file and validated atomic replacement.
  Steady-state limits apply after migration; peak migration scratch is at most
  one file limit. Add the exact files/temporary pattern to reset/uninstall.
- Capacity guarantees require participating new writers. Do not rotate under
  a still-running old daemon with inherited append descriptors: detect that
  compatibility window, retain legacy behavior and expose that retention is
  pending its normal replacement. Never restart/terminate live Sessions just
  for a logging migration. Normal self-update already owns the old/new handoff.
- No config/database or remote wire migration. Cross-host trace propagation,
  arbitrary dependency debugging and crash dumps are deferred.
- Older binaries may refuse reset/uninstall with new managed filenames, rather
  than delete unknown files. Rollback must leave logs/control readable by the
  new cleanup owner; document this explicit downlevel limitation.
- Diagnostic state and logs stay local. A user-selected Android document
  provider owns the chosen export destination; Zterm has no upload client.
- Shared encoding/gates avoid Rust/Kotlin drift; storage implementations need
  the same scenario fixtures. Disk failure and overload may lose records, which
  is visible through bounded counters/status when possible.

## 9. Evidence supporting implementation choices

Repository anchors for storage, configuration and Android ownership:
`crates/platform/src/user_state.rs:330`, `:358`, `:376`, `:652`;
`crates/daemon/src/config.rs:16`; `tests/terminal-dependency-policy.sh:27`;
`apps/android/app/src/main/java/io/github/leonfox28/zterm/ZtermApplication.kt:6`;
`apps/android/app/src/main/java/io/github/leonfox28/zterm/AppStore.kt:41`;
`apps/android/app/src/main/java/io/github/leonfox28/zterm/AppUi.kt:207`.

Primary API references checked on 2026-09-28:

- [Rust File locks](https://doc.rust-lang.org/std/fs/struct.File.html#method.try_lock):
  lock interaction with unlocked writers is platform-dependent, and reacquiring
  through a held/cloned handle can deadlock. All new writers must cooperate.
- [Android document creation](https://developer.android.com/training/data-storage/shared/documents-files#create-file):
  the system picker grants access to the selected destination, avoiding a broad
  storage permission or expansion of the update-only FileProvider.

## 10. Approved Store timeout correction found during review

The user approved the existing Store race correction after review. The gate
already distinguishes QUEUED, STARTED and EXPIRED; only its timeout result
classification was wrong. A waiter losing the QUEUED-to-EXPIRED CAS to another
expiry must return DeadlineExceeded, while an already-started operation remains
OperationOutcomeUnknown without a reply. Keep the actor, deadlines and wire/DB
contracts intact. One deterministic private-owner regression covers all three
gate states; the existing persistence suite verifies actual no-side-effect and
response-loss behavior. Details and before/after evidence are in `review.md`.
