# Local diagnostics contract

## 1. Scope / Trigger

Read before adding diagnostics or changing desktop/Android log storage, controls,
readers or exports. `zterm-diagnostics` owns the closed schema, admission and bounded
worker; desktop `zterm-platform::diagnostics::Store` and Android `DiagnosticFiles`
own storage. Existing operation owners emit observations; logs add no Session,
authorization, retry or remote protocol authority. See `docs/logging.md` for usage.

## 2. Signatures and owners

```text
zterm logs [-n|--lines N] [--level debug|info|warn|error]
           [--component NAME] [--session ID] [--since 30m] [--include-debug] [--json]
zterm logs debug on|off|status
zterm logs export --output PATH [--include-debug]
Recorder::new(Arc<dyn Sink>) -> io::Result<Recorder>
Recorder::record(Event) -> bool
Recorder::detail_enabled() -> bool
Recorder::flush(Duration) -> bool
Sink::{append(detail, bytes), control(), flush()}
Control::{enabled(now_ms), disabled(), remaining_ms(now_ms)}
LocalRuntime::{logs(&Filter), log_control(Option<bool>), export_logs(&Path, bool)}
```

Install once per process composition. Desktop `diagnostics::install` installs a
closed allowlist adapter for existing daemon tracing events and returns a bounded
flush guard. Shared client owners emit typed events through the same recorder.
Detached daemon/updater stdio uses null, never a permanently open rotating log.
Daemon fatal/panic reporting is content-free; graceful exit flushes with a deadline.
No arbitrary dependency subscriber, exception tree, environment-controlled trace
filter or remote log collection is permitted.

Android `ZtermApplication` installs `AppDiagnostics` before lazy native runtime
and repository initialization. The UniFFI `DiagnosticSink` callback runs only on
the Rust worker. Kotlin control/export runs on a serial IO dispatcher; Main and
terminal actors enqueue only. Defer even `Context.noBackupFilesDir` resolution:
Android can create/validate this directory in its getter. Callbacks must not call
back into Rust logging.

## 3. Contracts

### Schema, events and privacy

JSONL schema 1 has `timestamp` (UTC RFC3339 milliseconds), `level`, `event`,
`component`, fixed English `message`, `version`, `platform`, `pid`,
`process_instance`, `sequence` and nested `fields`. Optional fields include
`operation_id`, `connection_id`, canonical `session_id`/`attachment_id`, `elapsed_ms`,
`stage`, `outcome`, `category`, `committed`, counts and bounded numeric metadata.
The codec rejects unknown fields/kinds/categories, altered messages, invalid IDs
and records over 4 KiB including newline. Operation IDs are process-local; Session
and attachment IDs correlate known peers without propagating another wire ID.

INFO records lifecycle/committed outcomes: daemon/Session/controller/pairing,
network and route transitions, connect/reconnect, frontend transport state,
upload/update and Android application lifecycle. WARN is recoverable/actionable;
ERROR prevents continuation. Ordinary cancellation/detach is INFO. Emit commit
at its actual owner exactly once, including replay and takeover losers. Revocation
records `committed=true` immediately after SQLite commit; later cleanup failure
is a linked `partial_completion`. Update preparation failure is recorded when the
final owned config marker exists even if identity/database validation fails.
Before setup, updates and inspection create no diagnostics or identity state.

Never log terminal/input/clipboard/notification/file bodies, cwd/paths, environment,
identity keys, tickets/proofs/nonces, peer addresses/URLs, raw request/response or
error text. Use closed domain/frontend/network categories. Existing tracing
messages/Debug values are discarded except canonical public Session/attachment
IDs. Export validates the same closed codec, rather than trusting stored text.

Startup's `ProgressObserver` retains the full bounded screen journal but persists
only Starting, TerminalReady, Failed, Cancelled and SessionEnded at default level.
Intermediate stages are DEBUG. Retire all observer clones after initial Active;
reconnect diagnostics belong to the ongoing client owner. ConnectionCompleted
at snapshot acquisition means Synchronizing, not frontend input-ready.

### Admission and control

Key and detail lanes each have at most 256 queued records / 512 KiB serialized
bytes. Producers use bounded `try_send`, perform no filesystem IO and never wait
for sink/worker locks. Ordinary key events must not be dropped because a worker
briefly holds a queue mutex. Overflow/sink failures are counted; recovery emits a
bounded `records_lost` summary. Detailed events are capped at 100/s. Repeated
admission/listener/request failures and detailed sync/viewport/input summaries
coalesce by kind + category/Session for one second with at most 128 buckets.
No healthy per-frame/per-key/per-RTT event stream.

Explicit enable/renew persists schema 1 `started_ms` and `deadline_ms`, exactly
900,000 ms apart. Controls are at most 1 KiB, default OFF, polled at most one second
apart by an available worker. Missing/invalid/expired/rollback markers disable
detail. Running producers enforce a monotonic deadline even if storage blocks;
no clock rollback extends a running interval. Stop changes policy, not retained
history. Read failures, malformed markers and OFF observations preserve the last
enabled marker's original monotonic cap; restoring that marker is not a renewal.
Restart resumes only the remaining valid interval. Control is local to
the effective desktop account or Android application variant, never propagated.

### Storage, compatibility and reads

Desktop current files and one archive each are capped at 4 MiB: `daemon.log`,
`daemon.log.1`, `daemon.debug.log`, `daemon.debug.log.1` (16 MiB total). Control is
`diagnostics.json`; empty `writer.lock` is the stable coordination inode. Validate
UID/type/mode, reject symlinks, use 0700 directories / 0600 files. Acquire the
writer lock for normalization/rotate/reopen/append with bounded one-second retries
on the worker. Never lock a rotating inode or hold the lock while exporting.
Android has equivalent `events.jsonl[.1]`, `detail.jsonl[.1]`, `control.json` below
`noBackupFilesDir/diagnostics`, 2 MiB/file (8 MiB total), one Application owner.
Oversized input is normalized to a bounded complete-line tail with private atomic
replacement; migration may use one extra file-sized scratch buffer/file.

A live legacy daemon still owns inherited descriptors. Until its normal
replacement, preserve these files and report retention pending; do not restart
Sessions or claim a runtime cap. New daemons register the exact owned socket
incarnation (dev/ino/ctime/nsec) in the control document while holding DaemonLock.
The installed recorder and lifecycle share the same Store. Retain that exact
socket proof before attempting its bounded marker write, and repair a lost or
malformed marker on worker polling or append under writer.lock. Polling matters
when the daemon is idle but foreground/updater writers are active. Control corruption or
transient registration contention must not permanently disable retention. Never
reuse cached proof for a replacement socket; unregistered legacy writers still
keep their inherited descriptors.
New reset/uninstall inventory recognizes all files and precise atomic siblings;
older binaries may reject new names. No identity/config/database migration.

Snapshot readers capture safe handles, lengths and offsets under short writer
coordination, then release the lock. Tail reads at most 1 MiB, returns default100 /
max1000 lines and supports current/archive. Detail inclusion divides the input
budget equally between lanes. Zero lines reads no records. Filtering searches
this bounded tail only. Merge timestamps as parsed time values: RFC3339 fractional
seconds can omit trailing zeroes, so text ordering is incorrect. Unfiltered human output preserves legacy lines; structured
filter/JSON/export skips them and reports omissions. JSON diagnostics go to stderr,
never contaminate stdout records. No follow mode or watcher.

Export streams validated records from bounded snapshots plus a small schema/build/
platform/omitted/truncated/loss header, without config/status/identity dumps.
Desktop creates a private no-overwrite destination outside managed state and
removes incomplete files on failure. Android uses explicit system CreateDocument;
save both the detail checkbox and launched-export choice across Activity
recreation while the picker is open. Cancellation has no terminal side effect.
No automatic upload.

## 4. Validation & Error Matrix

| Condition | Required behavior |
| --- | --- |
| Missing setup / inspection / zero lines | No state creation, no daemon start; human missing-log guidance |
| Expired/invalid control or wall clock rollback | Detail OFF; key events continue |
| Blocked sink / full lane | Bounded producer/flush; count loss, preserve other lane capacity |
| Symlink / wrong owner / unsafe mode | Reject, never alter external target |
| Live legacy daemon | Keep descriptors valid; expose pending retention cutover |
| Registered daemon loses control marker / registration write times out | Worker poll/append repairs exact-socket registration so all writers apply caps |
| Android Activity recreated during CreateDocument | Returned export retains the launched detail choice |
| Same network/route with changed RTT/counters | No repeated key event |
| Replayed Session create or abandoned prepared takeover | No duplicate commit / false controller detach |
| Durable revoke/update followed by cleanup/startup failure | Preserve commit; report partial completion |
| Updater log path invalid before handoff | Fail preflight before stopping daemon |
| Log failure after business commit | Never roll back successful operation |
| JSON/filter/export sees legacy or hostile fields | Skip with count; never copy unknown payloads |
| Existing export path / path inside state | Refuse overwrite / managed-state destination |

## 5. Good / Base / Bad Cases

- Good: one reconnect operation links old/new attachment IDs and elapsed duration,
  then a distinct frontend Active transition follows exact synchronization.
- Base: `zterm logs -n 50` renders the last available key records once.
- Bad: dumping peer errors, enabling dependencies, logging each frame, or using a
  dropped diagnostic record as grounds to retry a business mutation.

## 6. Tests Required

Use actual owner paths, not a copied list of formatted examples. Session/network
and pairing replay fixtures assert event counts and sentinel absence. Configured
progress tests prove retirement, rotation and no-setup behavior. Updater fixtures
prove early preparation failure and committed partial completion. Local device IPC
forces cleanup failure after durable revoke and checks linked outcomes.

Diagnostics tests cover closed decoding, clocks, blocked sink/producer bounds,
independent lanes and recovery loss. Platform fixtures cover cooperating processes,
stable lock inode, concurrent snapshots, oversized legacy normalization, unsafe
paths and legacy daemon cutover. When process-spawning fixtures run alongside
Unix listener tests, allow a bounded retirement wait after close: an inherited
descriptor can remain live until exec. Assert live-socket refusal separately;
never relax the production rule against unlinking a live socket.
CLI fixtures cover bounded tail, no-autospawn,
controls and no-overwrite export. Shared-client fixtures use a real tunneled resume
and 100 repeated route samples to prove correlation and quiet defaults. Android
instrumentation verifies both variants, Settings/recreation, finite controls,
rotation, export and sentinel absence. Exercise the real picker with a stopped
Activity recreation and verify the returned file's header and detail records;
checking only the visible checkbox misses the launched-export snapshot.
Release-runtime/network claims require
actual installed artifact/disposable fixture evidence; build success is insufficient.

## 7. Wrong vs Correct

Wrong: `tracing::warn!(?request, %error, "request failed")`, a global DEBUG
subscriber, or a producer queue `try_lock` that loses ordinary events on contention.

Correct: `record(Event::new(Kind::UploadFailed).error(error.kind()).operation(&op))`
with explicit severity/outcome at the existing owner, a bounded independent lane,
and safe sink/export validation. Logging stays observational and best effort.
