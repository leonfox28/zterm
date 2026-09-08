# Desktop startup presentation design

## Boundaries

Keep the existing desktop CLI, LocalRuntime, semantic Session protocol and sole
DesktopPresenter. The task has two connected responsibilities: diagnose/correct
physical probe leakage, and present meaningful startup state before a Session
surface exists. Current source anchors and uncertainty are in research/findings.md.

The isolated reproduction and Ghostty log trace confirmed a per-command response
allocation failure for the 32-index OSC 4 batch. Use single-index complete OSC
commands in one write/flush and one bounded observation round. The existing
presenter/probe/decoder ownership is sufficient; no stream architecture change is
needed. Details and direct upstream anchors are in research/findings.md.

## Startup state and visible behavior

Use fixed transport-neutral stage values, an optional in-process observer and a
bounded chronological journal. Existing connection owners emit observations at
real boundaries; retries and reused connections remain truthful. The initial
screen displays the target and recent journal lines without elapsed time or
shortcut hints. The first validated Session frame takes over promptly.

Data flow: LocalRuntime / Session client / broker -> typed observer -> bounded
same-UID progress prefix (for remote broker stages) -> desktop observer -> sole
presenter + existing daemon.log. Progress never becomes remote Session protocol
or terminal content. The broker's existing per-peer state/Notify owns dialing
observations; register notification before inspecting state. There is no second
connection worker, connection authority or log-tail reader. The local adapter
validates stage enum, opt-in, request ID, deadline zero and event count before
notifying the CLI. Watch snapshots preserve a burst of at most 64 retained events.

Actual stages: invocation start; physical terminal/color initialization; local
service readiness; immutable target resolution; local socket / remote channel
request; checking the primary; either reuse or address lookup, secure transport,
protocol/access handshake and connection selection; opening the service stream;
explicit named Session create if requested; attach request, initial snapshot
receipt/validation, first presentation, synchronization and effective Active input
fence. Events are observations, not inferred `[done]` states. Subsequent real
attempts can produce retry events. Local attachment does not invent remote stages.

The core stage enum and client observer contain no OS/network/terminal payload.
The same-UID protocol adds optional field-3 `report_progress` flags on tunnel Open
and remote unary requests and kind 31 LocalConnectionProgress; each fixed payload
is capped at 64 bytes, each response prefix at 64 records. Existing default callers
send no progress request. The unary reader still requires one final correlated
reply and EOF; tunnel Open preserves coalesced trailing envelopes. All requests
retain their existing deadlines, demand/replay owners and cancellation outcomes.

The configured-CLI recorder appends fixed events to existing managed daemon.log,
with Unix milliseconds, severity, CLI PID, per-invocation ordinal, version, stage
code and typed failure category. It reopens for each append after rotation and
validates existing committed state/directories. It never creates setup state or
changes operation outcomes if diagnostic writing fails. No raw target, cwd,
terminal bytes, secrets, payload or error tree is logged. Existing Session and
network lifecycle records remain owned by their actual committers. Active emits
ready and retires every observer clone; early failure/cancellation or Session
ending before Active records the actual final outcome without exposing source strings.

Paint initial progress immediately after terminal/presenter setup, before the
first color query. Keep the same screen while preparation is unresolved; redraw
only for observed events and physical geometry. Clip target and journal through
the existing cell composer. In one row, keep the latest stage. A cancellation
waiting on a submitted create keeps its explicit waiting indication at the top.

Once the first validated snapshot exists, show it promptly and retire the full
startup view; while its ACK/activation remains pending, show a compact truthful
synchronizing indication in client chrome. Active retires that indication. Do not
keep a full-screen loader over usable authoritative terminal content or write
progress lines into the child's semantic grid/history.

## One physical writer and committed state

All startup drawing and color commands go through DesktopPresenter. A startup
frame is client presentation, not an authoritative empty Session snapshot. Do not
install its text as semantic_baseline or history fallback. Use a dedicated
presentation method/variant with known physical coverage and no Session source;
the first semantic frame must fully replace that coverage and commit only after
a successful write/flush. Reuse existing buffered transactions, clipping and
physical-baseline invalidation rules instead of adding another stdout logger.

Painting progress must not be the workaround for broken query framing. Preserve
the initial 250 ms / 64 KiB observation contract, one round plus its DSR fence,
and the same decoder throughout startup, ACK, activation and refresh.
The reproduced-compatible single-index complete OSC frames stay in one bounded
round; do not serialize 256 timeout waits, remove palette observation, or guess RGB.

## Errors and cancellation

Keep TerminalGuard responsible for raw mode, alternate-screen and owned-2031
cleanup on normal exit, error, signal and panic. Error diagnostics are emitted
after restoration through the existing path. Keep `preserve_submitted_result`
and exact created-session diagnostics; if cancellation still awaits a submitted
operation, its display must reflect that wait rather than promise immediate exit.
Retain sole-reader/input epochs and never replay startup typing into a child.

Established-session reconnect continues retaining the last valid frame and its
existing status policy. Reusing startup text must not clear an active session on
reconnect, resize or history return.

## Compatibility and rollback

No config, database, remote Session dialect or Android presentation changes.
The additional wire kind is an opt-in same-UID prefix; existing non-observing
callers retain their exact reply shape. Unknown/unsupported terminal colors still
degrade according to the existing color contract. Revert a cohesive code change
if needed; testing/reversion must not restart or replace the user's live daemon.
Exact Ghostty visual acceptance remains separate from byte/unit-test evidence.
