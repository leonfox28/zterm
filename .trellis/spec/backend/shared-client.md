# Shared client and Android bridge

## 1. Scope and trigger

Read before changing `crates/client`, `crates/android`, the desktop client
facades or the core viewport cache. `zterm-client` owns outbound pairing,
handshake, Session unary operations/replay, attachment synchronization/reconnect,
semantic surface updates and mode-aware key encoding. Desktop daemon modules
retain same-UID Unix sockets, local lifecycle and admin/store capabilities.
Android must not link the daemon, PTY, host Alacritty model or ANSI presenter.
`tests/terminal-dependency-policy.sh` enforces the dependency boundary.

The desktop composition owns host services (PTY, Session service and Alacritty
model) and may also act as a controller through its client facade. Android
composes only `NativeRuntime` → `IrohController` → shared protocol state, with no
inbound ALPN service or authorization registry. Protocol capability bits describe
supported RPC semantics; they do not replace directional host authorization.

`UnixAttachmentConnector` alone retains the desktop `RemoteDaemonRestarter`.
On a remote target's stopped IPC connection it uses the ordinary lifecycle-locked
launcher, then opens one replacement tunnel. A local target and network-only
failure never invoke that capability. Shared `SessionClient` owns no launcher,
socket path or process recovery hook; it retains protocol reconnect state only.

Both desktop broker and mobile controller call `handshake::controller_handshake`
for the outbound Hello/Welcome sequence. Inspect peer closure before adapter
cleanup: only the established application close code `CLOSE_UNAUTHORIZED` means
authorization rejection. A timeout stays `DeadlineExceeded`, a reset/read/write
failure stays `TransportUnavailable`, and malformed/version errors retain their
protocol kinds. Missing Welcome or free-text close reasons are insufficient
evidence for Unauthorized. Route candidate construction/order already lives in
`client::route`; verified cache storage, inbound admission and simultaneous
connection arbitration remain adapter/host responsibilities.

`NativeRuntime::shutdown` is an explicit async teardown: cancel normal operations
and observations, then await `IrohController::shutdown`/Endpoint close on the still
running owned executor. Foreign disposal/Drop may stop the executor abruptly;
tests or embedders requiring immediate same-identity restart must await shutdown
first. Shutdown is idempotent and never part of Activity/background transitions.
Otherwise an old host-side primary may survive until idle timeout and reject a
new connection through duplicate arbitration, even with valid authorization.

## 2. Signatures and owners

- `NativeRuntime.initialize(seed, dns_servers, hosts)` initializes one endpoint
  only after Android durably obtains its identity. Reinitializing with a different
  identity fails. `pair_ticket` returns a provisional `NativePairing`; Kotlin
  stores its public host then invokes idempotent `commit_pairing`.
- `connect_terminal(host, session, viewport, dark, takeover)` uses an exact Session
  ID and never creates or retargets implicitly. Named creation is a separate unary
  op. `connect_default_terminal(host, viewport, dark)` explicitly invokes the
  existing selector-free `create_main=true`, `takeover=false` attachment contract;
  the daemon alone owns reserved main creation/reuse. Both entry points share
  native preparation, ACK, resize and actor startup. `NativeTerminal.session_id()`
  returns the authoritative prepared ID; Kotlin must persist this returned ID.
  Attach viewport is a host creation hint, not a resize of retained Sessions.
  Before returning the native handle, submit its requested size to the actor;
  the actor applies it after Active/takeover and waits for synchronization.
  Never initialize desired geometry solely from the previous host snapshot.
- `NativeTerminal.commit_text(input_epoch, text, modifiers, paste)` and
  `send_key(input_epoch, key, modifiers, kind, text)` carry the epoch captured by
  the originating UI input to native actor admission. A later fresh frame cannot
  relabel stale queued input as belonging to its new epoch.
- `send_pointer(source, row, column, wheel_lines)` carries the drawn source and
  zero-based cell, with zero meaning an atomic tap and signed bounded steps for
  wheels. Require live/Active and source origin, epoch, screen, dimensions and
  input modes to still agree. Never retain pointer input through recovery.
  `zterm-client::mouse` owns child-mode filtering/encoding; desktop preserves its
  original raw-SGR passthrough and gesture routing around this shared encoder.
- `begin_selection(source, row, column)` / `extend_selection(source, row,
  column, anchor)` require the opaque `NativeFrameSource` of the actual drawn
  frame. The source pins an immutable accounted semantic page; attachment origin,
  input epoch, active screen, viewport and monotonic `geometry_generation`
  fence its use. Equal A-B-A dimensions never revive a retired source.
- `ViewportCache::with_budget(ViewportCacheBudget { rows, bytes })` opts Android
  into multiple pages. Default construction preserves desktop single-window
  behavior. `install_accounted_window` requires nested row allocation bytes.
- `NativeFrame.content_generation` identifies an attachment-local immutable
  presentation window (page/range/resolved colors); zero is the eager fallback.
  Metadata-only frames carry no row DTOs. `source.presentation_rows()` resolves
  at most three viewports plus one row; `first_row` is its logical window start
  and `window_offset` locates it in the current reading-offset basis.
- `source.viewport_source(first_row)` rebinds O(1) to a full viewport contained
  in that same exported window. It retains the accounted page and original
  origin/input/geometry/screen fences; an out-of-window request returns
  `selection_changed`. Inactive presentation sources carry no usable input
  epoch. Coordinate authority is never revived by matching pixels or dimensions.
- `NativeFrame.connection_path: NativeConnectionPath` (Unknown/Direct/Relay) and
  `rtt_ms: Option<u32>` project the shared `ConnectionStatus` event for this
  attachment. Never use history-query RTT or a second diagnostic polling owner
  for connection chrome. True reconnect/end/lease loss/closure clears both;
  same-attachment synchronization preserves them. Metadata-only changes retain
  the existing content-generation/page ownership.
- `NativeFrame.stats` contains query counts, local hits/misses, retained rows,
  allocation/peak bytes and smoothed response RTT. It contains no content or peer
  addresses. `notice` is a recoverable local reading error, distinct from `state`
  and terminal transport `error`.

## 3. Contracts

`wait_for_frame(after_generation)` delivers a newer final frame even when actor
cancellation becomes ready simultaneously. Re-read the frame on cancellation
before returning Closed; otherwise sleeping observers can miss lease_lost/ended.
The actor publishes the final complete state before canceling its token.

`IrohSessionIo::read` reobserves the selected QUIC path/rounded RTT after at most
one second without Session bytes. Its timeout cancels only the cancel-safe
RecvStream.read future; partial decoded frames remain in the adapter. Emit only
changed observations, send no application ping/input, and let normal stream
errors retain their types. This observation is separate from input readiness.

The Iroh `SessionUnaryTransport` adapter converts validated ServiceError frames
from RemoteUnaryClient into typed redacted errors before the shared unary client
can decode success payloads. RemoteUnaryClient itself retains frames for desktop
forwarding. In particular OperationOutcomeUnknown retires the cached operation
lease; only a subsequent explicit mutation requests a new lease. Otherwise a
daemon restart can turn valid domain errors into malformed_frame indefinitely.
`restarted_daemon_error_retires_unary_lease_without_replaying_mutation` verifies
lease retirement, no hidden mutation retry and preserved error/redaction types.

The shared driver remains the only wire synchronization/reconnect owner. Native
navigation is local presentation state, with one core pending history query.
Snapshot installation is acknowledged before Active permits input. Healthy
visual scroll-to-zero uses the continuously applied live surface without a new
sync, state change or epoch change. Preserve outstanding cache request correlation
so a late reply can be installed without replacing the live view. Selection at
zero remains pinned while selected. Copy/cancel at the bottom restores Live
locally, using the same transition as visual scroll-to-zero. Preserve older
reading positions (including rows made historical by new output), pending older
scroll intent and real recovery/barrier states; a duplicate late ActionMode exit
must not change them. Project only the actual displayed rows, not an unused live
grid followed by a second frozen-grid conversion.
An input-triggered history-to-live return retains first and ordered subsequent input within
`RESUME_INPUT_BOUND`, releasing it once after the snapshot barrier and Active.
Real reconnect, lease loss and Session end invalidate queued input/selection;
never replay actual-disconnect input. A healthy same-live-attachment resize
preserves `NativeFrame.input_epoch`, ordinary ordered input and preedit, while
advancing `geometry_generation` and retiring coordinate/selection sources.
`healthy_resize` is entered only by a local resize from Active Live; initial
attach, gap recovery and frozen-history return keep their existing barriers.
Snapshots/resume deltas still ACK immediately after native installation.
`NativeFrame.input_ready` and cursor readiness remain true for healthy resize;
coordinates remain fenced until Active with an applicable drawn source.

Android uses 4,096 accounted rows and 16 MiB including pinned sources. Cache
allocation includes nested cells/strings, row vectors and page metadata; bounded
selection/scratch metadata has a reserved allowance. Evict only unpinned pages.
Overlapping allocated pages count separately. Frozen pages do not evade budgets
when invalidated or removed from current lookup. Native frame, Repository and
View source handles have explicit close/retain ownership, independent of JVM GC.

Warm four screens using bounded replies; prefetch 2–8 screens according to
movement and observed latency. A prefetch response never moves an already frozen
reading viewport. Multi-page queries near either physical edge reserve enough of their existing
two-screen margin to also contain that complete edge viewport. A query centered
one row away must not force the waiting View to jump one row when it arrives.
Default single-window desktop query margins remain unchanged.
Source revisions may advance independently of historical page
anchors. Reuse older pages only where historical row identity is proven; mutable
live rows require matching content. Never display new selected cells while Copy
still extracts older pins.

Use core `TerminalTextRange` expansion/extraction across borrowed contiguous rows.
Preserve wide cells, combining text, soft wraps and spaces once for the complete
range. Copy is atomic within 512 KiB; gaps/conflicts refuse extension and retain
the previous copy. Same-epoch append/trim may preserve captured text; screen and
geometry changes clear it. Remote kind-322 clipboard effects are discarded by
this Android increment and never replayed on foreground return.

The Android Iroh adapter disables UDP segmentation offload. Real emulator
evidence isolated small successful requests followed by retransmission of larger
attach/create traffic; disabling GSO in this adapter restored both Mac and Linux
paths. Keep that platform choice out of shared deadlines, retries, MTU and wire
contracts. Platform DNS comes from Android LinkProperties, including IPv6 scopes;
do not hardcode a public resolver or replace the controller identity on changes.

## 4. Validation and error matrix

| Input/event | Required result |
| --- | --- |
| Invalid/oversize ticket | Shared decoder rejects before pairing |
| Same endpoint, different seed | `identity_state_mismatch` |
| Out-of-window presentation binding | `selection_changed`, no new coordinate authority |
| Stale source or input epoch | `selection_changed` / `input_not_ready`, no remote input |
| Cache full with pinned rows | `resource_limit`, keep Session and old copy |
| Failed speculative read | Recoverable `history_unavailable`, pending query retired |
| Conflicting/trimmed range join | Pause extension; existing capture remains copyable |
| Local saved-host deletion | Forget local route/connection only, no remote close/revoke |
| Post-write mutation/default-attach ambiguity | Preserve shared `operation_outcome_unknown`; never create again blindly |
| Default attach with a concurrent existing main | Host reuses its reserved main; no implicit takeover |
| Reconnect/end/closed frame | Unknown path and absent RTT; old metrics never revive on Active |
| Healthy same-attachment synchronization | Preserve selected path/RTT and unchanged content identity |
| Welcome read deadline / reset | `deadline_exceeded` / `transport_unavailable`, never infer Unauthorized |
| Authenticated peer explicitly closes with 0x100 | `unauthorized`; no authorized connection cached |
| Explicit runtime shutdown then immediate same-identity restart | Await endpoint closure before executor disposal; new handshake succeeds |

## 5. Good, base and bad cases

Good: an older drawn live frame supplies its own source after new output arrives;
selecting it still captures the exact old glyph. A cached swipe renders locally,
while independent prefetch fills another bounded page.

Base: an empty history viewport captures only its live rows and sends normal
input; single-window desktop behavior and its fixtures remain unchanged.

Bad: apply old screen coordinates to current live rows, concatenate separately
copied pages with invented newlines, evict a pinned selection or increase protocol
deadlines to hide platform packet loss.

## 6. Tests and assertion points

Preserve shared pairing/replay/attachment and daemon desktop-adapter fixtures.
Core cache tests cover default behavior, page reuse, budgets/pins, one pending
query, epoch validation and warmup.
`multipage_edge_queries_also_cover_the_waiting_full_viewport` checks full overlap
at both ends within the unchanged margin budget; native warmup tests prove that
Live and its adjacent row share a valid exported window. Source-window tests
check stable content IDs across metadata/row steps, changed color identity,
rebased selection coordinates and retained origin/input/geometry fences. Native navigation tests cover old rendered
source identity, cross-page exact copy, missing joins, trim, healthy-return ACK
barriers and disconnection invalidation. Native emulator integration verifies
a sleeping subscriber receiving its final lease_lost/ended/closed frame before
cancellation, plus actual host Session creation, Unicode input, scrolling, copy, resize, rename,
detach/cleanup and explicit stale input rejection. Report actual direct/relay
path separately; a compiled target is not runtime evidence.

## 7. Wrong versus correct

Wrong: Kotlin checks `currentFrame.inputEpoch`, then native stamps a newer epoch
when a queued call finally arrives. Correct: Kotlin passes the originating epoch
as data and the native actor compares it at the actual write boundary.

Wrong: call ordinary create_session with the reserved name main or add fallback
creation to the shared exact-ID reconnect loop. Correct: machine-entry policy
chooses a separate default-attach operation and stores the returned identity.

Wrong: retain selection pixels but free/unaccount their semantic source. Correct:
explicit frame-source handles pin pages inside the existing core cache budget.


## Initial connection observations

`progress::ProgressObserver` is an optional typed observation sink; its bounded
`ProgressHistory` keeps at most 64 chronological entries and coalesces adjacent
identical stages. Watch snapshots preserve bursts without a work queue or an
operation owner. `stop()` retires every shared clone at the initial Active fence.
An inactive observer is a no-op for Android and other callers.

`SessionClient::connect_with_progress` reports request write, waiting for the
initial snapshot, and successful initial snapshot validation. It keeps the same
attach deadline, input/ACK/replay semantics and outcome-unknown classification.
`RemoteUnaryClient::execute_validated_with_progress` and the transport's default
`demand_with_progress` hook preserve the sole unary retry owner; adapters without
an observer behave as before. No stage enters remote Session data or a semantic
surface. The desktop `LocalProgressDecoder` validates kind-31 opt-in, correlation,
fixed enum and 64-event bound before forwarding observations. Unary readers still
require one final response and EOF; tunnel readers retain coalesced post-Opened
bytes in the same decoder. See [Local IPC](./local-daemon-ipc.md).

Good: a pending observer update refreshes the startup screen while the submitted
create future remains owned. Bad: dropping/recreating that future to draw a stage,
replaying old dial stages for an existing connection, or retaining the observer
through later reconnect. Tests must check actual fragmented/coalesced local
prefixes, unchanged final responses, burst retention and clone retirement; a
copied list of labels alone is not evidence.

## Upload ownership

`zterm-client::upload` owns one bounded, non-replayed upload over `UploadConnector`.
The terminal driver captures `UploadOrigin` and validates it again for final input;
a stale upload-input error is local and must not close a replacement driver.
See [Single-file Upload](./file-upload.md) for signatures and queue/IME fences.
