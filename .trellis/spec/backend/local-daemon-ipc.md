# Local Daemon and IPC Contract

## 1. Scope / Trigger

Apply this contract to the per-user daemon, Unix socket service, peer
credentials, detached launch, setup/status/doctor/log commands, and lifecycle
locks. It also covers the Unix raw-terminal UI, its host mouse capture,
attachment-local history viewport, and Zterm-owned status/scrollbar chrome.
Use it whenever a frontend establishes either a direct local Session stream or
an opaque remote Session tunnel through its local viewer daemon, including
viewer-daemon restart and remote attachment resume.
Apply the presentation rules whenever a transport transition decides whether
the last observed connection path/RTT is still valid and whether stdout should
receive another complete frame.

## 2. Signatures

```rust
LocalClient::new(socket: impl Into<PathBuf>) -> LocalClient
LocalClient::readiness(&self) -> Result<DaemonReadiness, DaemonError>
LocalClient::status(&self) -> Result<DaemonStatus, DaemonError>
LocalClient::list_sessions(&self) -> Result<Vec<SessionSummary>, DaemonError>
LocalClient::create_session(&self, name, cwd, viewport)
    -> Result<SessionSummary, DaemonError>
LocalClient::rename_session(&self, session_id, name)
    -> Result<SessionSummary, DaemonError>
LocalClient::stop(&self, force: bool) -> Result<SessionImpact, DaemonError>

LocalPairingClient::create(&self, ttl_seconds: u32)
    -> Result<PairTicketText, DaemonError>
LocalPairingClient::accept(&self, ticket: PairTicketText, alias: Option<&DeviceAlias>)
    -> Result<DeviceSummary, DaemonError>
LocalDeviceClient::list(&self) -> Result<Vec<DeviceSummary>, DaemonError>
LocalDeviceClient::rename(&self, device_id: DeviceId, alias: &DeviceAlias)
    -> Result<DeviceSummary, DaemonError>
LocalDeviceClient::revoke(&self, device_id: DeviceId)
    -> Result<DeviceSummary, DaemonError>

LocalRuntime::ensure_configured_daemon(&self) -> Result<DaemonReadiness, DaemonError>
LocalRuntime::pair_create(&self, ttl_seconds: u32) -> Result<PairTicketText, DaemonError>
LocalRuntime::pair_accept(&self, ticket: PairTicketText, alias: Option<&str>)
    -> Result<CommandDeviceSummary, DaemonError>
LocalRuntime::session_list(&self, target: &str)
    -> Result<Vec<CommandSessionSummary>, DaemonError>
LocalRuntime::attach(
    &self,
    target: &str,
    selector: Option<&str>,
    create_main: bool,
    takeover: bool,
    viewport: Option<TerminalSize>,
)
    -> Result<PreparedTerminalView, DaemonError>
LocalRuntime::reset_identity(&self, expected_device_id: Option<DeviceId>, force: bool)
    -> Result<IdentityResetResult, DaemonError>

SessionClient::connect_resolved(socket, target, selector, create_main, takeover, viewport)
    -> Result<SessionClient, DaemonError>
SessionClient::set_remote_daemon_restarter(
    &mut self,
    restarter: Arc<dyn RemoteDaemonRestarter>,
)
SessionClient::take_initial_snapshot(&mut self) -> Option<TerminalSurfaceSnapshot>
SessionClient::begin_takeover(&mut self) -> Result<LocalTakeoverRetryToken, DaemonError>
PreparedTerminalView::new(client: SessionClient, takeover: bool, target: TerminalViewTarget)
    -> Result<PreparedTerminalView, DaemonError>
RemoteDaemonRestarter::ensure_running(&self) -> Future<Result<(), DaemonError>>

RemoteSessionService::serve_tunnel(stream, first, limits, deadline)
    -> Result<(), DaemonError>
serve_remote_session_tunnel(broker, target, local_stream, first, limits, deadline)
    -> Result<(), DaemonError>

run_terminal(request: TerminalRequest, runtime: &LocalRuntime)
    -> Result<(), CliError>

TerminalViewCommandWriter::request_history_window(query: TerminalHistoryWindowQuery)
    -> Result<(), DaemonError>
AttachmentSurface::from_snapshot(snapshot: &TerminalSurfaceSnapshot)
    -> Result<AttachmentSurface, CliError>
AttachmentSurface::candidate_after_delta(delta: &TerminalSurfaceDelta)
    -> Result<Option<AttachmentSurface>, CliError>
ChromeLayout::new(physical: TerminalSize, screen: ActiveScreen)
    -> ChromeLayout
ScrollbarGeometry::new(track_rows: u16, metrics: TerminalScrollMetrics)
    -> Option<ScrollbarGeometry>
ComposedFrame::compose(...) -> Result<ComposedFrame, CliError>
DesktopPresenter::present(writer: &mut impl Write, desired: ComposedFrame)
    -> Result<bool, CliError>

const MIN_VIEWPORT_PRESENT_INTERVAL: Duration = Duration::from_millis(16);

ViewportPresentationPacer::mark_dirty(&mut self, now: Instant)
ViewportPresentationPacer::deadline(&self) -> Option<Instant>
ViewportPresentationPacer::due(&self, now: Instant) -> bool
ViewportPresentationPacer::mark_presented(&mut self, now: Instant)
ViewportPresentationPacer::cancel(&mut self)

ViewportController::observe_presentation(&mut self)

StatusRenderer::reset_for_reconnect(&mut self)
render_transport_transition_view_with_writer(
    writer: &mut impl Write,
    viewport: &ViewportController,
    status: &mut StatusRenderer,
    transport_state: TerminalViewTransportState,
    resumed_from_snapshot: bool,
) -> Result<bool, CliError>

ViewportState::ResumePending {
    retained_input: Vec<u8>,
    snapshot_applied: bool,
    presented_scroll_metrics: Option<TerminalScrollMetrics>,
}

const HOST_INPUT_CAPTURE: &[u8] = b"\x1b[?1003h\x1b[?1006h";

spawn_inside_runtime<T>(runtime: &tokio::runtime::Runtime, spawn: impl FnOnce() -> T)
    -> T
```

Unary IPC is `varint length + WireFrame + write-half EOF -> one response`.
`TerminalAttachRequest` selects the duplex stream; lease allocation is the
strict unary `SessionOperationLeaseRequest -> SessionOperationLeaseResponse`.
For a remote view, the frontend first sends
`LocalSessionTunnelOpenRequest -> LocalSessionTunnelOpened` on that same
per-frontend IPC socket, then carries unchanged Session bytes in bounded
`Data` envelopes. `Path`, `HalfClose`, and `Closed` are local-only
sideband envelopes and are never valid on the normal Iroh Session stream.

The implementation boundary is `zterm_daemon::client`: `transport` owns Direct/
Tunnel bytes and epoch disposal; `session::SessionClient` owns the one attachment
interpreter, correlation and resume; `view` owns typed commands/events and the
bounded driver; `ipc` owns unary clients. `local_ipc` owns the server listener and
ingress, and retains existing low-level client re-exports only. `operations`
owns command use cases and lifecycle; CLI view code imports `client::view`.
No new crate, background owner, or second Session interpreter is introduced.

## 3. Contracts

- One installed `zterm` executable contains a hidden internal daemon entry.
  There is one daemon per OS user and no supervisor, PID fallback, system
  service, login item, or boot registration.
- `lifecycle.lock` is short-lived launcher/setup serialization;
  `daemon.lock` is held for the daemon lifetime. The daemon never waits for the
  lifecycle lock.
- The daemon alone may remove a stale socket, and only after holding
  `daemon.lock`, observing connect failure, and validating an owned real socket.
  Each bound listener also carries the socket path's device/inode/change-time
  token (Linux may immediately reuse an unlinked inode);
  fatal-listener rebind and final removal compare that exact token and refuse to
  unlink a same-UID path which was replaced after publication.
- Linux uses `SO_PEERCRED`; macOS uses `getpeereid`. Wrong UID is rejected
  before decoding bytes. Directory/socket permissions complement but do not
  replace the credential check.
- IPC uses one shared bounded frame codec and classifies a connection from its
  first decoded frame. Unary calls use one connection per request.
  The client half-closes its write side after the frame; the server requires
  request EOF before dispatch so trailing bytes arriving in a later read are
  rejected rather than silently ignored. A `TerminalAttachRequest` instead
  enters one long-lived duplex stream and preserves decoder state plus any
  complete frames received in the same read.
  All session calls carry one absolute deadline. Potentially blocking
  synchronous service/attachment work runs under `spawn_blocking`; the
  current-thread Tokio runtime never waits inline for a full actor mailbox or
  a PTY effect. Timing out drops only the waiter—an already-started mutation
  continues and records its exact replay result.
- Local pair/device kinds 12-21 use the same credential gate, decoder, strict
  unary EOF, and typed service-error response. Pair dispatch remains async;
  blocking Store/Directory/device projection work runs off the socket runtime.
  Sensitive first frames are moved rather than cloned, and ticket/request/reply
  buffers are zeroized after the one byte-identical retry window.
- `LocalPairingClient` and `LocalDeviceClient` are doc-hidden daemon-internal /
  test adapters. They never spawn a daemon, open SQLite, read `identity.key`, or
  bind Iroh. Public clap reaches them only through `LocalRuntime`, which owns
  committed-setup validation, singleflight daemon launch, exact target
  freezing, destructive preflight, and the safe human/JSON projections. The
  CLI never receives a socket path, `UserPaths`, store, identity key, Endpoint,
  route, or operation lease.
- Public clap exposes setup/status/doctor/logs, pair create/accept, device
  list/rename/revoke, connect, Session list/new/attach/rename/close, daemon
  status/stop/restart, and `reset --identity`. Bare invocation observes first:
  not-setup prints fixed setup guidance without creating state; configured
  invocation is exactly local `main` create-or-attach. Help, version, parsing
  failures, status, doctor, logs, daemon status, and daemon stop never spawn.
  Setup and restart explicitly spawn; configured pair/device/connect/Session
  commands may singleflight-start one daemon but never perform setup.
- Pair accept has no ticket positional argument, flag, or environment input.
  Its default owner is a no-echo TTY line reader; non-TTY input is rejected
  before reading unless `--stdin` explicitly selects the 16 KiB-bounded EOF
  reader. Both paths transfer immediately into `PairTicketText`/zeroizing
  owners. Pair create writes the ticket once to stdout; Debug and error
  projections remain redacted.
- User target resolution accepts reserved `local`, one exact case-sensitive
  outbound alias, or one canonical 64-lowercase-hex DeviceId. Exact aliases
  precede rejection of hex-looking short/prefix text; a full DeviceId candidate
  must be lowercase and a full-ID/exact-alias collision is ambiguous.
  Session selectors are exact names or canonical 32-lowercase-hex SessionIds.
  A default `connect` uses atomic `create_main`; after setup, bare invocation
  resolves to the same path. Ordinary attach never steals a controller;
  explicit takeover is the only CLI request for replacement.
- `SessionOperationLeaseRequest/Response` is the mutation-only control exchange
  for a daemon-issued lease. A logical `LocalClient` requests it lazily before
  its first mutation and caches it; readiness, status, and session list do not
  allocate a lease or write replay state. Request IDs, lease ordinals, and
  operation sequences fail explicitly at exhaustion and never wrap.
- A mutation is encoded once. Only an ambiguous transport failure may trigger
  one retry, using byte-identical bytes, request ID, operation ID, payload, and
  the same absolute deadline. A complete response, including typed
  `operation_outcome_unknown`, is definitive. Outcome unknown poisons the
  cached lease; that logical mutation is not retried under a new lease, while a
  later independent operation may request one.
- `LocalSessionUnaryRequest` is an ordinary 1 MiB-bounded control payload which
  contains exactly one allowed preencoded Session unary frame. The daemon
  validates its frozen full target and correlation without using the payload as
  a second codec or retry source. A remote mutation outer envelope is never
  replayed after a full or partial Unix write: missing EOF, malformed framing,
  wrong ID/kind, or invalid typed payload immediately becomes
  `operation_outcome_unknown`. Only a read-only outer request may retry once;
  stateful lease allocation uses one outer attempt and one remote service-stream
  attempt, returning its typed post-write failure without allocating again. The
  daemon-owned `RemoteUnaryClient` alone owns the one possible Iroh mutation
  retry for this unary path; live terminal Session ownership remains in the
  frontend client described below.
- Every frontend view owns exactly one same-UID IPC connection and exactly one
  transport-independent Session client. A local view sends Session frames
  directly on that socket. A remote view uses the local-only tunnel envelope;
  its daemon owns identity, the single Iroh Endpoint, one shared broker
  connection per device pair, and one admitted service stream per tunnel.
  Multiple frontends therefore have independent IPC sockets and service
  streams while sharing the peer connection. No frontend creates an Endpoint
  and no IPC byte stream is shared between frontend processes.
- The tunnel daemon validates same-UID admission, exact non-self outbound
  DeviceId, protocol version, frame headers, and a 64 KiB nonempty Data ceiling.
  It then pumps bytes with direct bounded backpressure and one owner per writer.
  It never decodes inner Session frames and never owns a SessionId,
  AttachmentId, ResumeViewId, revision, viewport, acknowledgement decision,
  operation lease, or pending control. One malformed, stalled, reset, or
  cancelled tunnel releases only its demand/stream/socket ownership and cannot
  close the shared peer connection or another tunnel.
- The frontend Session client freezes the target and SessionId, generates one
  stable ResumeViewId, accepts target-issued attachment IDs unchanged, tracks
  the latest successfully presented revision and desired viewport, and owns
  request correlation plus mutation ambiguity. On retryable remote loss it
  resolves an outstanding history query once as a content-free Gap, emits
  `Reconnecting`, opens a fresh tunnel/service stream, and attaches with the
  same ResumeViewId and SessionId plus the latest applied revision/viewport.
  Every successful replacement epoch projects `Synchronizing` and then an
  explicit frontend-owned `Unknown` path before any collected Direct/Relay
  samples or target state. Consecutive identical path samples are suppressed.
  It never blindly replays input, resize, snapshot acknowledgement, history, or
  a possibly committed takeover. A reconnect may accept only a target snapshot
  or a delta contiguous with the advertised applied revision.
- A production remote view receives one narrow daemon-restart capability from
  `LocalRuntime`. If opening its replacement IPC socket reports
  `DaemonStopped`, that capability calls the ordinary `DaemonLauncher::ensure`
  with the same `UserPaths`; the lifecycle lock provides cross-frontend
  singleflight, after which each frontend independently reopens its tunnel.
  A local view never receives this capability because its target Session ended
  with the stopped daemon and cannot be resumed into a new daemon incarnation.
  Network-only failures never launch the viewer daemon.
- Remote history has exactly one renderer-neutral operation:
  `TerminalHistoryWindowRequest` kind 317 followed by one correlated
  `TerminalSemanticHistoryWindowFrame` kind 318 on the authenticated attachment
  stream. One request retains its complete anchor/target/margins beside the
  pending correlation. Both local and remote adapters validate attachment ID,
  anchor, viewport, disposition, translated target, signed start, semantic
  rows, and exact row count before exposing a complete window. There is no
  capability negotiation, pager, stateful server viewport, unsupported
  sentinel, or fallback representation in wire major 2.
- Wheel, Page, gutter drag, and future touch gestures update the CLI-owned
  `ViewportCache<TerminalSurfaceRow>`. Relative targets saturating-coalesce and
  the latest absolute target wins while at most one history-window request is
  in flight; returning live supersedes queued history work. Stream-epoch loss
  completes the pending query once as a correlated content-free nonzero `Gap`
  whose revision is not older than the saved query, then emits `Reconnecting`.
  The query is never replayed on another epoch and the daemon/Session stores no
  attachment scroll target.
- `LocalSessionTunnelPath` is same-UID/local-only. The tunnel emits an
  initial/reattachment unknown sample and changed selected-path/RTT samples no
  faster than once per second; the frontend projects it as connection status
  beside immutable `TerminalViewTarget { route, display_name }` metadata.
  Device IDs, addresses, relay URLs, tickets, and terminal bytes never enter
  this sideband, Debug, status text, or logs. A display name never selects a
  route and a local view rejects network-path sideband.
- `TerminalClipboardWrite` kind 322 is a transient semantic host effect, not a
  terminal revision or replayable control response. Terminal ingress validates
  child OSC 52 once, Session targets only the controller that exists at effect
  publication time, and the one frontend Session client validates the target
  attachment identity plus bounded text. Operations owns a separate
  latest-only clipboard slot with a payload-free wakeup, so a slow UI cannot
  fill or block the capacity-eight lifecycle queue. Target changes, stream loss,
  detach, and Session end clear pending content; observers and later
  controllers receive no replay. The wire carries no raw OSC.
- A local or remote initial attachment has one deadline covering every
  pre-snapshot transport-state frame through the complete correlated snapshot
  or typed service error. For `create_main`, encode/connect failures before the
  request write remain definitive; any full or partial write followed by an
  unvalidated timeout, transport close, malformed response, or correlation
  failure is `operation_outcome_unknown`. A complete correlated service error
  remains definitive. An existing-session attach retains its exact bounded
  transport/protocol failure because it has no create side effect.
- The high-level raw-terminal owner validates stdin/stdout before starting any
  attachment work. Once `session new` or `create_main` may have submitted its
  stateful request, local detach, stdin EOF, and SIGINT/SIGTERM/SIGHUP record
  cancellation but continue polling the same owned future to its exact bounded
  result. Exact success reports the stable SessionId and detaches only the
  view; create-then-attach failure preserves `CreatedSessionAttach`; an
  unprovable post-submit result remains `operation_outcome_unknown`.
- Each non-`Active` terminal transition advances the input epoch and clears the
  prefix. Returning to `Active` first joins the old stdin reader, flushes queued
  kernel input, advances the epoch, installs the replacement reader, and only
  then accepts input. `SIGWINCH` has one CLI owner: after successfully
  submitting a changed size it immediately treats the view as
  `Synchronizing`, coalesces only the latest observation, and sends no further
  resize until an authoritative `Active` event. If that event finds a different
  pending size, the owner submits it and remains `Synchronizing`; only an
  `Active` event with no changed pending size reopens input. The owner tracks
  the last submitted size and suppresses identical repeated signals because a
  semantic no-op need not produce another replacement snapshot or completion
  barrier. Because server output can begin another snapshot after the client
  observed `Active`, Session admits replaceable resize from the exact current
  controller while its sync target remains `Active`; input, history, and
  semantic viewport may cross that same window only when the attachment was
  already Active in the current stream epoch. Fresh/takeover and stale/non-
  controller attachments retain strict synchronization/lease checks. The CLI
  never retries or replays resize. Viewport
  publication or a local socket-write acknowledgement alone is not an input
  fence. Pure UI and Session tests own the exact reader-replacement,
  resize-state, and in-flight-snapshot ordering. The multiprocess PTY fixture
  uses the production `run_terminal` entry and bounded idempotent shell probes;
  it must not add renderer markers or test branches to the product loop.
- Snapshot ACK authority belongs to the update's origin, never to the UI's
  current or entry transport state. `SessionClient` emits
  `LocalAttachmentEvent::ResumeDelta(TerminalSurfaceDelta)` only for the correlated
  reconnect response contiguous with its advertised applied revision. The view
  driver preserves it as `TerminalViewEvent::ResumeDelta`; ordinary streamed
  updates remain `Delta` even across queued resize/sync/Active transitions.
  Full snapshots and explicit resume deltas ACK their exact revision once after
  successful application/presentation. Ordinary deltas only advance the applied
  revision. `Synchronizing` still fences input and coalesces resize; it grants no
  ACK authority. No wire-format change, server tolerance, route/application
  branch, retry, or delay participates in this distinction.
- The raw-terminal UI distinguishes physical size from child size: every local
  and remote view reserves the physical bottom row when rows are at least two;
  a one-row view gives its only row to the child. Before both initial attach and
  every resize, the sole child-size
  projection clamps rows and columns to the shared `ResourceLimits` viewport
  maximum; the daemon and wire boundary still reject an independently supplied
  oversized viewport. Status placement uses the uncapped physical bottom row.
  Local is exactly `<device> | local`, with no latency field. Remote is exactly
  `<device> | <direct|relay|--> | <integer ms|-->`. Both use theme-default
  reverse video across every cell, clip on display-cell boundaries, and
  save/reset/restore child cursor and style. Wheel/Page routing depends only on
  authoritative main/alternate, mouse, and alternate-scroll modes; there are
  no tmux/Herdr/application-name branches.
- On the main screen, usable widths greater than four reserve exactly the final
  column for Zterm chrome; the child receives `N-1` columns. Widths 1–4 and the
  alternate screen give the child the full usable width. The status row is
  outside both the child and gutter. A pinned history presentation remains
  effectively Main even if background child output enters alternate, so it is
  not resized or overwritten until return-to-live. Main/alternate transitions
  submit at most one geometry change, and `ResizeCoalescer::last_submitted`
  suppresses the resize-produced same-screen replacement.
- Gutter ownership follows the effective layout, not the prior desired layout.
  `ViewportController` retains the last successfully presented gutter column
  and advances it only after the complete outer transaction succeeds. While
  both the presented and current layouts own different gutters, chrome clears
  the old column before drawing the current gutter last; this ordering repairs
  any right-margin clamp after a width shrink. When Alternate or a width of at
  most four removes the gutter, the reclaimed column is child-owned: the
  authoritative child snapshot or physical resize replaces/clips the old
  pixels, and chrome must not clear that column after child content. Multiple
  unpresented layout changes always compare the final desired gutter with the
  last committed gutter. Alternate-to-Main may draw the newly reserved gutter
  after child content because ownership has transferred back to Zterm.
- A main gutter with no valid history metrics is cleared and blank. With
  history it renders `▕` track and a proportional, minimum-one-row `▐` thumb;
  live maps to bottom and oldest maps to top using overflow-safe arithmetic.
  Track click and drag emit absolute offsets. A drag remains chrome-owned when
  motion/release leaves the gutter, clamps to the usable track, and always ends
  on release/capture loss. Child mouse coordinates are never clamped into the
  gutter or status row.
- Wheel ownership is evaluated in this order: an already-pinned history view,
  the Zterm gutter, the live child's declared mouse-reporting mode, live
  alternate screen with alternate-scroll, then Zterm main history. Zterm wheel
  navigation is one logical row per complete SGR report and PageUp/PageDown is
  `rows - 1`. A child mouse branch forwards exactly one encoded mouse event;
  alternate-scroll emits exactly one cursor-key sequence. Main plus
  alternate-scroll alone still
  belongs to Zterm, and alternate without either child mode receives no
  invented scroll input. This mode-driven rule is what makes nested Herdr,
  PiAgent, tmux, and other TUIs coexist without process-name detection.
- One exhaustive pointer router also owns text selection. Active gutter drag,
  existing selection capture, gutter hit, history navigation, child mouse,
  alternate-scroll, live history, and eligible unmodified left selection are
  mutually exclusive outcomes; a cancelled selection consumes its remaining
  release instead of synthesizing an orphan child event. Selection coordinates
  and exact presented-source identity live only in the CLI attachment. Core
  normalizes ranges and extracts bounded semantic text for both desktop and
  future mobile clients; the compositor applies inverse-style XOR only to its
  cloned frame before chrome and the final cursor. The presenter applies an
  overlay or selection-driven keyboard elevation only when those coordinates
  match the exact source of the next frame; input decoding follows the
  host-input projection from the last successfully flushed transaction, never
  an uncommitted desired mode.
- The terminal guard owns one stack-scoped Kitty keyboard entry. It pushes a
  disabled value on entry, mirrors nonzero child flags, temporarily requests
  flags 7 (disambiguation, event types, and alternate keys) only while a
  finalized local selection overlays a zero-flag child, and pops on every exit.
  The sole host-input codec preserves raw bytes when outer and child modes
  agree; during that temporary elevation it consumes one Ctrl/Super+C press and
  its matching repeat/release lease or performs the complete legacy downgrade
  for other valid keys. Any other outer/child mismatch remains byte-preserving;
  it is not treated as recoverable through legacy reinterpretation. No timer,
  terminal brand, process name, or screen-text heuristic participates.
- Physical input encoding is a presenter-owned host effect, not a
  visual-history property. Its projection contains exactly application cursor,
  application keypad, bracketed paste, focus reporting, and derived outer Kitty
  keyboard flags. Child mouse mode/encoding and alternate-scroll remain
  Zterm-routed semantics and are never mirrored to the outer terminal. When a
  live delta arrives, the CLI first validates a candidate surface and previews
  compact post-delta viewport/cache-anchor metadata plus a reconciled selection
  candidate. That post-delta selection identity, not the pre-delta viewport,
  determines any flags-7 elevation. The preview does not clone cached rows.
  For a pinned history view, the sole `DesktopPresenter` emits only controls
  for changed projected fields in one buffered `write_all + flush`; it emits no
  rows, chrome, cursor, mouse reset/capture, or visual transaction. An unchanged
  projection does no I/O. Only after successful output (or a proven no-I/O
  projection) may the caller commit the surface, live metrics, cache-anchor
  observation, selection, and presenter selection/projection together. A
  write/flush failure commits none of those candidates and invalidates the
  presenter baseline so the next presentation is a complete recovery frame.
  Live full-frame delta presentation uses the same staged selection/viewport
  ordering rather than mutating them before the outer write.
- Snapshot, applied delta, resync replacement, history, status, and scrollbar
  changes first converge as one semantic `ComposedFrame`. The sole
  `DesktopPresenter` then emits one buffered outer transaction: DEC 2026 begin,
  terminal/history cells and chrome, cursor/mode policy, `HOST_INPUT_CAPTURE`,
  DEC 2026 end, one `write_all`, and exactly one flush. No daemon/model/tunnel
  path constructs presentation ANSI. Child modes may change semantic input
  routing but cannot leave physical outer capture disabled. A partial write or
  flush failure clears the presenter's committed baseline, makes a best-effort
  DEC 2026 end while preserving the original error, and forces the next retry
  to perform a full clear plus complete repaint. Raw-mode cleanup begins by
  ending DEC 2026, then disables capture/restores the user's terminal on normal
  exit, signal, error, and panic.
- The same sole presenter is the desktop clipboard sink. Both a finalized local
  selection copy action and a validated remote kind-322 effect become exactly
  one canonical `OSC 52;c;<standard padded Base64>BEL` write and one flush.
  Clipboard output is capped at 512 KiB decoded UTF-8, diagnostics are
  content-free, and success or failure does not mutate the visual presenter
  baseline or enter Session replay state.
- Transport synchronization and connection-path validity are independent.
  Same-stream `SyncRequired`/`Synchronizing` during return-to-live preserves the
  last observed direct/relay path and RTT. It emits no standalone transition
  frame while a valid complete history presentation is retained. The
  authoritative replacement snapshot atomically paints live content,
  offset-zero scrollbar, status row, capture, and cursor state; the following
  `Active` event still completes the input fence, buffered-input forwarding,
  and pending-resize state. It passes the desired frame to `DesktopPresenter`,
  which skips actual equality and presents any changed cursor/modes/chrome;
  resume completion alone must never suppress that comparison. A true `Reconnecting` transition is the connection-observation epoch
  boundary: clear path/RTT before any replacement-stream synchronization and
  show unknown until a new validated status observation arrives.
- The only history path is a client-owned
  `ViewportCache<TerminalSurfaceRow>` fed by bounded 317/318 semantic windows.
  A full cached slice applies wheel/Page/drag to the
  desired offset locally without a request. All host events decoded from one
  stdin delivery are reduced before presentation; one CLI-owned dirty bit and
  one non-sliding deadline present only the latest complete slice, with a
  16-millisecond minimum interval between eligible host-owned history
  presentations.
  This is event-driven and has no idle ticker; it never paces ordinary live PTY
  deltas, child-owned mouse, or alternate-scroll input. Misses, absolute jumps,
  and half-screen low-water edges retain one complete pending query and coalesce
  later movement to the latest desired target; request/prefetch effects remain
  immediate, drag network requests are paced at 33 ms, and release always
  delivers and presents the final complete target. Request start, loading,
  resume, resize, and content-free Changed/Gap never paint an intermediate
  blank or partial history frame. The last complete presentation stays visible
  until one full replacement is locally available.
- Received history/cache state and presented state are separate authorities.
  A coalesced window result that immediately triggers a newer request has not
  been painted and must not replace the retained frame or scrollbar metrics.
  The window-cache reducer may advance a locally presentable desired
  offset before its paced frame reaches stdout; that reducer value is likewise
  not presentation authority. `ViewportController` therefore retains separate
  last-successfully-presented metrics and advances them only after the complete
  outer DEC-2026 transaction succeeds. When a painted history view enters
  `ResumePending`, it snapshots that baseline rather than the cache target. A
  `SyncRequired` chrome repair continues to use those metrics; after the
  authoritative replacement snapshot is presented, that same atomic snapshot
  transaction explicitly uses `compose_live_candidate` with the new live cells
  and offset-zero metrics while the viewport remains `ResumePending` for input.
  `install_snapshot_with_writer` commits surface, layout and snapshot readiness
  only after successful write/flush; pinned History stays pinned. It must never
  render an unseen target or empty gutter between those two complete states. A
  resize clears retained metrics whose `viewport_rows` no longer match, and a
  true reconnect clears both live and retained metrics because the new stream
  epoch cannot authenticate their identity. Resume, snapshot/resync, resize,
  reconnect, transport replacement, detach, cleanup, or another immediate frame
  cancels/satisfies pending cadence work before a stale timer may repaint.
- A window response is installable only when it is the exact shape of the
  saved query and contains the latest desired full-height slice. Same-epoch
  append translates a pinned offset by history growth. Epoch/size change,
  extent decrease, unsafe live-prefetch revision, explicit resume, reconnect,
  and takeover invalidate cached coordinates; resize refetch begins only after
  the authoritative resized snapshot supplies the new anchor. The daemon and
  Session retain no client cache or window target.
- The UI history state is exhaustively `Live`, `History`, or `ResumePending`.
  Pinned views drain live revisions without replacing the visible semantic
  history surface; returning
  live requests one full sync and retains at most the fixed input bound, then
  forwards retained key/paste bytes exactly once only after snapshot
  acknowledgement and `Active`. The sole host-input codec aggregates a complete
  bracketed paste across stdin chunks into one bounded event, including its
  delimiters. Paste content bypasses detach-prefix and gesture parsing; an
  oversized unterminated or complete paste fails with a typed resource error
  without forwarding a partial prefix. If `Active` arrives while a resume paste
  is incomplete, the UI keeps consuming that paste under the old input epoch,
  then performs the authoritative reader join/flush/new-epoch fence before
  forwarding the complete retained unit exactly once.
- `TerminalSyncRequired`/replacement snapshot is a background visual sync, not
  a transport reconnect: an already-pinned complete semantic history frame,
  drag state, and server scroll baseline stay intact, and the CLI does not send
  a redundant sync request. A true `Reconnecting` transition invalidates the
  logical cache, drag/queued actions, and live metrics, but leaves the last
  complete terminal pixels visible until an authoritative replacement. A live
  view may enter new history only while transport state is `Active`; gutter input during a
  fresh synchronization is swallowed rather than leaked to the child.
- Closure correlation has two ordered owners under the same bounded window.
  First, command-side EPIPE/reset-equivalent local attachment closure drains a
  buffered typed lease/session/service outcome. Second, the spawned terminal
  driver sets one latest-only latch only after its final typed event/error has
  successfully entered the existing event queue; if a command send or oneshot
  response owner then closes, the writer suppresses its channel fallback and
  lets that queued event remain the sole user-visible outcome. Without either
  authoritative event, one content-free normalized closure is returned. Raw OS
  or `terminal attachment driver closed` text is never user-visible, and no
  command is replayed.
- Frontend control submission creates one absolute five-second deadline covering
  bounded-channel admission, dequeue and transport I/O. Expired or abandoned
  queued commands cannot write. The UI retains its existing two-second detach
  wait; normal idle event reads have no control timeout. History remains an
  asynchronous correlated response and never blocks the command owner.
- Initial attach and each reconnect attempt share one establishment deadline
  across Open, attach write and initial response. Initial metadata and unrelated
  frames collected while waiting for a lease are bounded to eight entries;
  deferred frame payload plus bounded path metadata may not exceed 8 MiB.
  Overflow returns `resource_exhausted` and discards the epoch, without silently
  skipping a delta. Takeover lease acquisition, write and its pending correlated
  response share one five-second deadline. A silent lease expires definitively;
  an unanswered sent takeover is `operation_outcome_unknown` and is not replayed.
- A write future temporarily owns its transport epoch. Timeout or cancellation
  drops that socket, preventing any later write from following a partial frame.
  A write-half closure may retain only the existing 100 ms typed-outcome read
  drain; the Session client rejects every subsequent write on that epoch.
  Remote recovery creates a fresh epoch and preserves existing non-replay rules.
- Prepared view construction takes the initial snapshot exactly once. Running
  clients retain identity, applied revision and size, with no obsolete initial
  screen. Semantic protobuf snapshot/delta/history conversions consume their
  messages directly after decoding; they do not deep-clone before conversion.
- `TerminalUiSession` owns the live UI fields and transport/event transitions.
  `run` funnels both loop outcomes and propagated handler errors through the
  single `finish_terminal_view` cleanup. Repeated prefix/viewport operations
  and transport transitions call owner methods. The delta handler captures its
  entry sync decision before candidate presentation or resize; semantic surface,
  pinned history and the presenter's last committed physical frame remain
  distinct state owners with their existing successful-flush commit contracts.
- The frontend Session client owns its bounded correlated lease, takeover, and
  history state. Epoch loss completes a sent takeover as
  `operation_outcome_unknown` and a pending history query as one Gap; neither
  is replayed. A correlated ordinary `ServiceError` preserves its typed code
  and request correlation but discards untrusted peer text. An uncorrelated
  response or fatal authorization, wire, protocol, Session, or lease outcome
  terminates only that view.
- Lifecycle stop first performs bounded concurrent session cleanup. Only full
  ownership release may produce a successful stop response; that response is
  flushed and its socket shut down before listener exit is signaled. A cleanup
  deadline, failed response flush, or dropped caller leaves the listener and
  owned socket running for status and retry. Already-stopped remains
  idempotent at the CLI boundary.
- Interrupted/would-block/connection-aborted/reset/timed-out accept errors stay
  inside the listener loop and do not drop live session ownership. A fatal
  accept error aborts only its connection handlers and returns to the
  `run_daemon` ownership loop. That loop first attempts bounded concurrent
  session cleanup. If any owner remains, it keeps the process, daemon lock,
  store, service, and child registry alive, compare-rebinds its exact socket
  token with bounded backoff, and resumes status/stop service. It exits and
  removes the exact socket only after all ownership is released.
- M4 session list/create/rename/close remain strict unary calls. The attachment
  reader accepts only snapshot acknowledgement, sync, input, resize, detach,
  and takeover for the bound attachment. A protocol error flushes one typed
  error before closing only that stream.
- Attachment output uses one fixed-capacity control queue plus latest-only
  revision/lifecycle watches. A slow socket writer cannot backpressure the PTY
  reader or accumulate one message per terminal revision.
  Natural finalization may close the driver revision watch before the actor
  publishes its final drained update and `SessionEnded`. The socket writer
  disables revision polling on that expected closure (or a racing
  `SessionNotFound`) and keeps the stream alive for the lifecycle channel,
  which is the authority for terminal termination.
- Local readiness, status, setup validation, stop, and update preflight do not
  require Iroh, DNS, Relay, or Internet access.
- `setup` and `daemon restart` may spawn. Status, doctor, logs, daemon status,
  and daemon stop never spawn a daemon. A successful stop responds after
  session shutdown and removes only its own socket during the normal daemon
  lifecycle handoff. Restart then waits within the existing bounded deadline
  for readiness to disappear, the socket to be absent, and `daemon.lock` to be
  missing or unlocked before launching the replacement; socket disappearance
  alone is not daemon ownership release.
- Daemon stop/restart with active Sessions requires explicit `--force`; the
  public surface has no implicit interactive bypass. Close, revoke, and
  identity reset instead use exact preflight plus interactive `yes` or
  noninteractive `--yes`; identity reset additionally requires `--force` when
  Sessions are active. Reset performs a bounded stop, acquires
  `lifecycle.lock`, rechecks stopped ownership and the confirmed identity, and
  removes only the validated managed state root. It sends no `RevokeSelf`,
  does not remove the binary, and does not run setup.
- Detached spawn redirects stdio, uses a stable home cwd, and the child calls
  safe `setsid()` before runtime threads. It does not use `pre_exec` or unsafe
  code.
- The detached daemon composes synchronous owners before building its
  current-thread Tokio runtime. Every daemon-owned `tokio::spawn` performed
  from that synchronous path must run inside this exact runtime's `enter()`
  guard. The guard covers only task creation and is released before the
  listener loop; subsequent `runtime.block_on` calls drive the bound tasks.
  Production startup must never rely on an ambient runtime inherited from the
  launcher or from a `#[tokio::test]`.
- Local session and terminal calls use the single transport-independent
  `SessionService`; they never pair, resolve an alias, bind Iroh, or self-dial.
  `SessionClient` is library-internal, but each instance executes in
  the frontend process below the public raw-terminal UI. It owns that
  frontend's socket, one Session decoder, route adapter, target IDs, operation
  lease, resume checkpoint, and request correlation; none are projected into
  clap or renderer APIs.
- Human and JSON status are projections of one typed daemon observation.
  Running state comes from IPC; configured/stopped state may open SQLite only
  after the socket proves no `StoreActor` is live.
- Doctor validates account, committed state, and socket/lock agreement without
  spawning. Linux lifecycle output names the `systemd-logind` logout limit but
  never changes linger or installs a service.

## 4. Validation & Error Matrix

| Condition | Required result |
| --- | --- |
| peer effective UID differs from daemon owner | close with zero response bytes before frame decode |
| unary request has trailing bytes, missing EOF, malformed frame, or excessive size | typed/connection-local protocol failure; listener remains usable |
| tunnel Open is wrong-version, zero-request, self-targeted, unknown/revoked, or not outbound-authorized | correlated local service error; acquire no unauthorized service stream |
| post-Open tunnel envelope has nonzero request/deadline, an invalid kind, zero-byte Data, or Data above 64 KiB | one `ProtocolError` Closed best effort, then end only that tunnel |
| remote tunnel read/write resets or peer connection is lost | one `TransportLost` Closed best effort; frontend emits Reconnecting and independently resumes its attachment |
| replacement remote tunnel cannot open because the viewer daemon stopped | lifecycle-singleflight `ensure` the same configured viewer daemon, then retry with the frozen SessionId/ResumeViewId/revision/viewport; surface launch failure; local views never auto-restart |
| initial or replacement snapshot acknowledgement loses its tunnel write | remain Reconnecting/Synchronizing; never emit Active for the dead epoch |
| entry-Active delta changes Main/Alternate geometry and submits resize | present it and enter Synchronizing, but do not acknowledge the old delta as the new resize epoch |
| ordinary delta consumed while Synchronizing (including snapshot -> Active -> deferred resize) | apply and record `to_revision`; never send snapshot ACK |
| correlated ResumeDelta | present the valid contiguous candidate, mark resume presentation ready, ACK exact `to_revision` once; Active completes the input fence |
| request deadline expires before dispatch | `deadline_exceeded`, no effect begins |
| request times out after actor start | drop only waiter; accepted effect completes into replay state |
| ambiguous same-UID loss on a local-target mutation | retry once with identical bytes/ID/deadline |
| remote mutation outer envelope was partially/fully written but has no fully validated correlated response | `operation_outcome_unknown`; do not reconnect or replay the envelope |
| read-only remote outer envelope has a post-write failure | retry once with identical envelope bytes and the same deadline |
| remote Session stream is lost with a pending lease / takeover | original typed transport failure / `operation_outcome_unknown`; remove the pending cell and never replay it |
| remote Session stream is lost with a pending history-window response | resolve its original correlation once as a content-free nonzero Gap not older than the saved query, then reconnect; remove the pending query and never replay it |
| history-window frame is uncorrelated, predates/contradicts its request, exceeds 240 rows, contains invalid semantic rows, or does not contain the exact requested range | malformed frame scoped to the view; retain the prior complete cache/presentation and never partially install rows |
| live wheel occurs during fresh synchronization | swallow Zterm gutter/history navigation and do not forward it to the child; an already-pinned view may continue only across background replacement sync |
| child declares mouse reporting / alternate+alternate-scroll | forward exactly one mouse report / one cursor-key sequence; do not move Zterm history |
| one pointer event could match selection, gutter, history, or child routing | evaluate the single exhaustive router and execute exactly one owner; cancelled capture consumes its own trailing release |
| selection source identity changes through resize/reflow/screen/reconnect/gap/navigation | clear the attachment-local selection before reuse; never apply coordinates to unproven rows |
| child OSC 52 is a read, malformed/noncanonical, wrong selector, empty, NUL, invalid UTF-8, or over 512 KiB | consume atomically through its terminator, emit no reply/render/effect, and keep diagnostics content-free |
| a valid clipboard effect has no current controller or its target changes before take | drop/clear it; never broadcast, queue per write, replay to a later controller, or block PTY drain |
| remote kind 322 is nonzero-request, wrong-ID, or outside Active/previously-active same-epoch synchronization | reject the view-local frame; do not retain it in reconnect state |
| finalized-selection copy arrives as press/repeat/release | emit one clipboard write for the press and consume the matching repeat/release lease; never infer phases with a timer |
| local selection elevates a zero-mode child to Kitty flags 7 | downgrade every other valid key to its legacy child representation and preserve unknown raw input; pop the owned stack entry on cleanup |
| presenter clipboard write or flush fails | return a content-free error and preserve the committed visual baseline; do not end the Session or expose clipboard text |
| composed render path omits DEC 2026 closure/host capture or flushes before chrome/capture | renderer contract failure; snapshot/delta/history/chrome tests must compare exact byte order, one write, and one flush |
| Main transfers its gutter to Alternate, or a resize reduces usable width to at most four | let the authoritative child snapshot repaint, or the physical resize clip, the former gutter; emit no post-child clear into the reclaimed column |
| Main retains a gutter but its column changes, including multiple layouts before one presentation | compare with the last successfully presented column, clear that old column first, and draw the final current gutter last in the same transaction |
| a transaction that would change gutter presentation fails during write or flush | retain the previous presented-gutter baseline so a retry performs the same required ownership-safe repair |
| cache miss/loading/resume/resize/content-free outcome has no complete new frame | issue/retain only the bounded request and keep the prior complete presentation; never blank or partially repaint content |
| one stdin delivery contains multiple host-owned wheel reports | apply every one-line report to the desired offset, then emit at most one latest complete history/chrome transaction for that delivery; never flush once per report |
| repeated host-owned updates arrive before the 16 ms deadline | keep one dirty bit and the original deadline, send request/prefetch effects immediately, and present only the latest complete target when due; do not slide the deadline or build a frame queue |
| a paced desired target has not reached stdout when resume, cache miss, resize, reconnect, or authoritative replacement begins | retain only the last-successfully-presented metrics/pixels and cancel the stale deadline; never preserve or repaint the unseen desired target |
| child mouse/alternate-scroll is active while host cadence work exists | forward each child-owned event immediately and cancel/ignore any now-unpresentable host frame; never delay PTY input behind the viewport deadline |
| compatible same-epoch live history grows while a complete pinned frame is pending | translate the cached target and keep the valid pending presentation; do not discard it merely because the background revision advanced |
| `ResumePending` receives `SyncRequired` before its replacement snapshot | keep the last painted, valid scrollbar geometry; do not turn missing replacement metrics into a blank gutter frame |
| replacement snapshot is observed while resuming | use its validated live metrics in the same snapshot/chrome transaction; do not wait for a later `Active` repaint to restore the thumb |
| same-epoch return-to-live enters `SyncRequired`/`Synchronizing` with a known path/RTT | emit no intermediate presentation and retain the last complete status row until the replacement snapshot atomically paints the same observation |
| `Active` follows the authoritative resume snapshot with no newer visual state | complete the input fence, buffered-input forwarding, and pending-resize transition without a redundant stdout transaction |
| transport enters true `Reconnecting` | clear the prior path/RTT observation before rendering or synchronizing the replacement stream; show unknown until that stream supplies a validated status |
| a coalesced semantic window immediately schedules a newer target | advance cache/request state but retain the last painted frame/metrics; an unpainted result is not presentation authority |
| resume geometry changes or the attachment truly reconnects | clear mismatched retained metrics / clear both live and retained metrics; never project stale thumb geometry onto a new size or stream epoch |
| command write closure races a buffered typed lifecycle event | publish the typed event; suppress raw `Broken pipe`/OS text and do not retry the command |
| terminal driver command sender/response owner closes after its final typed event/error entered the event queue | suppress the command-channel fallback and let the queued event win; if no event is confirmed within the same bounded window, return normalized `daemon_stopped` |
| `create_main` request was written but no complete correlated initial result is validated | `operation_outcome_unknown`; do not claim the default Session was absent or retry under a new identity |
| control command expires during queue admission / before dequeue | `deadline_exceeded`; no write from the expired command |
| peer never reads input / lease peer remains silent | bounded control failure; release the current epoch |
| waiting lease receives more than eight deferred frames/paths or over 8 MiB payload | `resource_exhausted`; close the epoch, never discard deltas and continue |
| sent takeover has no correlated outcome before its deadline | `operation_outcome_unknown`; no new-lease retry |
| write future is cancelled after a partial frame | socket ownership drops with the future; next command cannot reuse it |
| existing-session initial attach receives states but no snapshot before its total deadline | `deadline_exceeded`; close only that view |
| post-active frozen-session attach receives `session_occupied` while the old host reader is half-open | close the rejected epoch, remain reconnecting, drain/drop input and coalesce viewport for 250 ms, then retry the same SessionId without `create_main` |
| first-ever attach receives `session_occupied` | flush the typed terminal error and close the local view; do not retry |
| active remote stream returns a correlated ordinary service error | forward it, remove the pending cell, and keep the attachment alive |
| inner remote Session frame is malformed or fatally unauthorized/incompatible | target/ frontend Session validation terminates only that view; the tunnel daemon never decodes or rewrites it |
| definitive outcome unknown | do not retry that mutation under a new lease |
| a daemon-requiring public command observes no committed setup | `not_setup` with `zterm setup` guidance; do not create identity or state |
| pair accept uses a non-TTY without `--stdin` | usage error before reading ticket bytes or starting ticket parsing |
| PTY test infers input readiness from viewport publication alone | invalid evidence; use the pure input-fence test or a bounded idempotent fixture probe |
| target is short/prefix/uppercase ID, ambiguous full ID/alias, self ID without `local`, or inbound-only for Session access | exact selector/direction error; do not acquire remote demand |
| ordinary attach finds a controller | `session_occupied`; do not input, resize, detach, or replace it |
| daemon stop/restart or identity reset would end Sessions without `--force` | refuse before stop; report only the safe Session count/impact |
| close/revoke/reset is noninteractive without `--yes` | refuse before mutation |
| identity-reset deadline fixture starts while a dropped listener can still transiently connect | invalid fixture evidence; settle the stale socket under a separate setup bound before starting the production stop deadline |
| stop cleanup or response flush fails | keep listener/socket and ownership available for status/retry |
| fatal accept while a child remains owned | exact-token rebind under held daemon lock; resume service |
| socket path was replaced after bind | never unlink or overwrite the replacement |
| synchronous daemon composition calls `tokio::spawn` outside its owned runtime | invalid startup boundary; the pure runtime regression fails deterministically before release |

## 5. Good / Base / Bad Cases

- **Good:** authenticate peer credentials, decode one shared frame, dispatch to
  `SessionService` through `spawn_blocking`, and flush a terminal error before
  closing only the offending attachment.
- **Base:** status/doctor/logs observe a stopped or running daemon without
  creating paths, allocating mutation leases, or starting a process.
- **Good:** parse one public target, let `LocalRuntime` freeze it to `local` or
  a full DeviceId, and pass only the typed request/view to the CLI renderer.
- **Good:** give every frontend one IPC socket and Session client; map a remote
  socket one-to-one onto an admitted service stream while reusing the broker's
  one active peer connection.
- **Base:** local and remote routes feed the same Session interpreter and
  renderer contract. Only their establishment adapters differ: direct
  same-UID Session bytes versus local tunnel envelopes.
- **Good:** when several remote frontends lose the same viewer daemon, each
  requests the narrow restart capability, the lifecycle lock starts at most one
  daemon, and each frontend then resumes its own independent tunnel. A local
  view ends instead of attaching its dead SessionId to the new incarnation.
- **Bad:** decode Session frames, cache revisions/viewports, rewrite attachment
  IDs, or decide acknowledgement inside the tunnel daemon.
- **Good:** prove shell-ready and eventual interactive echo through the
  production terminal entry; exercise resize plus signal restoration separately
  from prefix detach, while pure tests own the exact Active input fence.
- **Good:** let terminal modes select wheel ownership, retain one attachment's
  semantic history view across a background replacement snapshot, and compose
  terminal rows, chrome, cursor/modes, and host capture before the sole
  presenter performs one write and one flush.
- **Good:** when Main transfers the rightmost column to an Alternate child,
  leave the child's authoritative snapshot as the final writer of that column;
  advance the presented-gutter baseline only after the transaction succeeds.
- **Base:** when one Main layout replaces another, clear the last committed
  gutter before drawing the new gutter last, even if the old coordinate clamps
  at a narrower right margin.
- **Good:** satisfy wheel/drag from a complete local cached slice, prefetch only
  at a bounded low-water edge, and atomically replace the prior frame when a
  validated request-shaped window becomes ready.
- **Good:** reduce every host-owned report in one stdin delivery, send any
  request immediately, and use one 16 ms event-driven deadline to present only
  the latest complete cached target. Advance presented metrics only after that
  atomic write succeeds.
- **Bad:** flush each decoded wheel report independently, slide a pending
  deadline on every new report, or copy the cache's desired/presentable offset
  into resume chrome before the corresponding frame was actually written.
- **Good:** preserve the painted history thumb through `ResumePending` and
  `SyncRequired`, then replace it directly with the validated live-bottom thumb
  inside the authoritative snapshot transaction.
- **Good:** preserve a validated direct/relay + RTT observation through an
  in-epoch visual sync, paint it with the replacement snapshot, and let the
  following `Active` event perform state/input work without an unchanged frame.
- **Base:** a fresh attachment or true reconnect shows unknown connection
  details until that connection epoch reports a validated path and RTT. On a
  replacement epoch, the frontend must enqueue that `Unknown` projection
  itself before replaying collected tunnel path samples; it must not depend on
  the tunnel pump delivering an initial sideband. Consecutive duplicate samples
  are suppressed without reordering later Direct/Relay/Unknown transitions.
- **Bad:** derive chrome from the most recently received coalesced frame, clear
  the gutter while a replacement snapshot is pending, or retain metrics across
  a resize/reconnect merely because the old terminal pixels are still visible.
- **Bad:** append stale-gutter spaces after an Alternate child's full-width
  snapshot, infer the old gutter from an unpresented intermediate layout, or
  commit gutter presentation state while merely building a frame.
- **Bad:** equate every non-`Active` state with a lost connection observation,
  or emit separate Synchronizing, Snapshot, and unchanged Active transactions;
  DEC 2026 cannot make multiple transactions visually atomic as a group.
- **Bad:** trust socket permissions without peer credentials, decode before the
  UID gate, let clap accept a ticket/path/socket override, block the
  current-thread runtime on PTY work, or remove a socket by pathname without
  comparing the listener's device/inode/change-time token. It is also invalid
  to infer Active from independent resize/viewport notifications.
- **Good:** enter the daemon-owned runtime only while spawning an async owner,
  release the guard, then let the existing listener `block_on` drive it.
- **Bad:** call a production spawn seam from synchronous startup merely because
  all existing tests happen to run inside `#[tokio::test]`.

## 6. Tests Required

- Real same-UID unary and duplex tests run on macOS/Linux; Linux CI includes a reachable
  cross-UID rejection harness. A helper executed as the foreign UID must live
  below one test-private directory whose parents are searchable by that UID;
  the copied executable is execute-only, `sudo` is noninteractive, and the test
  requires both zero response bytes for the rejected peer and a successful
  owner request afterward. Running a binary directly from a CI workspace whose
  parent directories are not searchable is fixture failure, not peer-gate
  evidence.
- Multi-process tests prove concurrent launch singleflight, live/stale socket
  behavior, detach, bounded stop, restart identity preservation, and no
  spontaneous post-crash restart.
- The real outer-pseudo-TTY `daemon_autospawn` path drives production
  `run_terminal`, has the fixture shell emit generic DECSET/DECRST 1049,
  waits for Alternate 23x80 then Main 23x79 convergence at physical 24x80,
  proves another input advances model and presentation, detaches cleanly, and
  asserts the exact `<device> | local` row with no `not_synchronized`.
- The `operations` identity-reset deadline fixture drops its stale Unix
  listener and, under a separate one-second setup bound, waits until an owner
  connection is refused before invoking the reset with its 40-millisecond
  production deadline. Darwin may briefly complete a connection to a
  just-closed listener and then return EOF; that transport-settling interval is
  fixture setup, not evidence about the one shared readiness/socket/lock
  deadline. Keep the strict `deadline_exceeded` assertion. Do not replace this
  barrier with a sleep or paused Tokio time: the production deadline uses
  `std::time::Instant`.
- A pure synchronous lifecycle unit builds the same current-thread runtime,
  spawns through `spawn_inside_runtime`, and joins the task with that runtime.
  Removing the `enter()` guard must reproduce Tokio's no-reactor failure. The
  companion network lifecycle test injects every failure before Endpoint bind;
  neither test may open UDP, perform DNS, or contact a Relay.
- CLI tests own the complete help/side-effect matrix: the public tree has no
  state/identity/socket/ticket override; bare before/after setup, help/version,
  parse errors, every inspection command, daemon stop/restart, and each
  daemon-requiring command assert their exact create/spawn behavior. Pair tests
  prove no-echo restoration and zeroized/redacted success/error/panic paths;
  reset tests prove exact confirmation, active-Session force, no-follow fixed
  inventory, retryable partial deletion, and no implicit setup.
- `single_instance` and `detached_lifecycle` are harness-free multi-process
  executables using only task-private `UserPaths`; production argv has no state
  override. `detached_lifecycle` constructs `LocalRuntime::for_test` with its
  explicit launcher and exercises the public `restart` owner so a retiring
  process cannot race the replacement through a still-held `daemon.lock`.
- `local_session_ipc` proves session mutations, detach/reconnect, and daemon-stop
  events; it also drops a create response and retries the same operation ID on
  a new socket, blocks a real session-A PTY writer while status/session B
  progress, proves final output and the typed natural-exit event survive the
  revision-watch-close race, and proves a failed bounded stop keeps the listener
  available until ownership is released. `terminal_recovery` proves
  resynchronization and that an invalid attachment kind does not poison other
  sessions or the listener.
- `client::ipc` additionally proves a dropped mutation response is retried once
  with byte-identical request bytes and one server execution, and that a typed
  outcome-unknown response is not retried and rotates the lease only on the
  next independent mutation. It also proves a remote mutation outer envelope
  is sent exactly once for malformed/truncated, wrong-ID/kind, and invalid
  typed replies while read-only outer requests retain one byte-identical retry.
  `local_session_ipc` proves a recoverable injected accept failure preserves the
  listener and its live session, and an injected fatal accept in the actual
  `run_daemon` listener loop rebinds while a HUP-resistant child remains owned,
  then accepts a truthful stop retry.
- `remote_tunnel` tests prove opaque queued Data forwarding, one-way
  half-close, terminal close reason, malformed-stream isolation, and sibling
  liveness. `client::session` adapter tests prove split inner frames, multiple inner
  frames in one chunk, immediate Path delivery, zero-byte rejection, incomplete
  inner EOF, and Data-after-half-close rejection.
  `direct_and_tunnel_adapters_share_one_session_trace_and_command_interpreter`
  compares identical target IDs/revisions/events plus byte-identical target-
  visible acknowledgement, resize, input, and sync commands.
  `stopped_viewer_restart_opens_a_tunnel_and_resumes_the_same_frontend_state`
  proves one stopped-socket restart hook accepts a replacement tunnel and
  receives the same ResumeViewId, frozen SessionId, applied revision, and latest
  viewport before issuing a new target AttachmentId. The
  `shared_peer_loss_keeps_each_frontend_resume_checkpoint_independent` test
  proves two affected frontends retain distinct resume cells, while
  `reconnecting_frontends_resume_independently_through_one_viewer_listener`
  makes both cells resume concurrently through one viewer listener with their
  own SessionId, ResumeViewId, applied revision, viewport, and new target-issued
  AttachmentId. `single_instance` supplies the multi-process lifecycle-lock
  evidence that concurrent `ensure` calls start at most one daemon.
  Target-side `session_wire` tests continue to prove strict authenticated
  attachment identity, synchronization, clipboard, and takeover behavior
  without any viewer-daemon ID rewrite.
- `terminal_ui` pure tests cover universal rows-minus-one/one-row geometry,
  oversized physical-to-bounded child projection for initial attach and resize,
  stable main gutter/alternate reclaim, all scrollbar positions and drag
  clamping, same-delivery wheel-burst reduction, non-sliding 16 ms deadlines,
  cross-delivery latest-frame coalescing, reverse/clamp final offsets, cache-miss
  immediate requests, final drag release, child-owned bypass, compatible
  background-delta translation, and stale-deadline cancellation. Timing tests
  supply explicit `Instant` values and assert transaction counts rather than
  relying on an OS scheduler. They also cover exact DEC-2026-composed
  output/capture/write/flush order, complete
  reverse-video Unicode-safe status output, mode-derived one-report wheel/Page
  routing, cached hit/no-request, edge prefetch, 33 ms drag/release-final,
  resize refill, request/Changed/Gap frame retention, and exact-once input
  across Live/History/ResumePending/background sync. They must trace the full
  `History -> ResumePending -> SyncRequired -> Snapshot -> Active` sequence and
  assert actual child cells before any subsequent input/output, as well as
  every emitted chrome frame: the pre-snapshot frame retains the last
  painted thumb, the snapshot frame contains validated offset-zero chrome in
  the same DEC-2026 transaction, and the Active transition does not repair a
  blank intermediate or emit an unchanged repaint. It must still restore a
  hidden live cursor. Snapshot write/flush failure leaves the surface, layout,
  metrics and retained input uncommitted. The real outer-PTY scroll test must
  return to live and compare all child cells; frame counts alone are insufficient. For remote views, the same
  regression asserts the exact transaction count and requires every emitted
  in-epoch frame to contain the stable path/RTT status row. A separate true-
  reconnect regression resets that observation and proves it cannot reappear
  during replacement-stream synchronization. Coalesced-but-unpainted semantic
  windows, invalid replacement metrics, resize, and true reconnect are separate
  regressions. Gutter-ownership regressions first
  present a real Main gutter, then compare exact transactions for
  Main-to-Alternate child rightmost-column preservation, Main grow/shrink,
  multiple unpresented layouts, width-at-most-four removal, and failed
  write/flush retries. They require the final owner to be the last writer and
  forbid advancing the presented-gutter baseline on a failed transaction.
  Operations
  tests prove both local-stream closure and top-level command send/response-owner
  closure defer to an already queued typed terminal outcome, while closure with
  no event is normalized without raw OS text. The top-level schedule uses one
  legal viewport and reads the original typed event after the resize command
  completes. The tmux 3.7c and Herdr 0.8.2 black-box modes must pass through the
  same generic path.
- Selection and keyboard unit tests cover forward/reverse/pure-click ranges,
  live and history presented-source identity, stable monotonic append versus
  invalidation, wide/combining/blank/wrapped extraction and the 512 KiB atomic
  cap, every pointer owner and cancelled capture, highlight ordering, Kitty
  CSI-u fragmentation/validation, macOS Super+C, Linux Ctrl+Shift+C, one
  press/repeat/release copy lease, raw-equality forwarding, complete legacy
  downgrade, outer stack restoration, and exact canonical OSC 52 output.
  A pinned-history regression changes each of application cursor/keypad,
  bracketed paste, focus reporting, and Kitty keyboard mode through hidden
  deltas and requires exactly one buffered changed-controls write/flush, no
  row/chrome/cursor/mouse-capture bytes, an updated committed input baseline,
  preserved flags-7 selection elevation when the child returns to legacy, and
  no semantic commit after an injected host-write failure. Routed-only mouse/
  alternate-scroll changes and an otherwise unchanged projection do no host
  I/O. Presenter failure tests require content-free errors and the documented
  baseline recovery behavior.
- The CLI multiprocess PTY gate uses a task-private deterministic shell. The
  connect child proves ready -> eventual interactive echo -> default detach
  through the unmodified `run_terminal`; its scroll mode uses a real outer PTY,
  revision/echo barrier, and SGR wheel report to prove a target exactly one row
  above a 24-row live viewport is repainted, then detached/restored. Its copy
  mode uses the same outer PTY to make a local selection, send one enhanced
  copy press/repeat/release actuation, and observe exactly one canonical OSC 52
  between the owned keyboard-stack push and pop, without a child revision or a
  new presentation transaction. The bare child separately proves
  SIGWINCH revision/viewport, SIGTERM cancellation, termios restoration,
  bounded reap, and panic cleanup. Stress it sequentially and concurrently,
  then assert no fixture daemon is orphaned. No diagnostic may contain
  terminal/cwd bytes.

Client boundary regressions additionally assert silent lease expiry, ordinary idle
read survival, deferred frame count/byte bounds, tunnel path bounds, partial-write
timeout and cancellation disposal, sent-takeover ambiguity, queue admission
expiry, and a write using only the remaining queue budget. Initial-state fixtures
transfer their snapshot once and retain its revision for later acknowledgement.
Existing route parity, close correlation, clipboard and history tests move with
their owning client modules; platform/runtime coverage must not be inferred from
module extraction alone.

## 7. Wrong vs Correct

### Wrong

```rust
// The viewer daemon becomes a second Session client and rewrites identity.
let remote = daemon.decode_session_frames(tunnel)?;
daemon.cache_revision_and_viewport(remote);
daemon.rewrite_attachment_id_for_frontend(remote);

let request = decode(stream.read().await?)?; // peer not authenticated
service.dispatch(request);                   // may block Tokio inline
remove_file(socket_path)?;                   // pathname may be replaced

if process_name == "herdr" { forward_wheel() } // application heuristic
if state == Synchronizing { snapshot_applied(delta.to_revision).await?; } // lost origin
if resumed_from_snapshot { return Ok(false); } // suppresses necessary presentation

clear_screen_and_show_loading();
send_history_window_without_saving_query(query).await?;
```

### Correct

```rust
// Correlation is retained through the typed boundary.
match event {
    TerminalViewEvent::Delta(delta) => apply_and_record(delta)?,
    TerminalViewEvent::ResumeDelta(delta) => {
        apply_and_present(delta)?;
        writer.snapshot_applied(delta.to_revision).await?;
    }
    // Full snapshots also ACK only after their candidate presentation commits.
    _ => { /* existing event handling */ }
}

// The frontend owns one Session client regardless of route.
let transport = match target.route {
    Local => SessionTransport::Direct(open_ipc()?),
    Remote => SessionTransport::Tunnel(open_ipc_tunnel(target.device_id)?),
};
let session = FrontendSessionClient::new(transport, frozen_target, resume_view_id);

// The daemon owns only network admission and bounded opaque forwarding.
let demand = broker.demand(target.device_id, deadline).await?;
let service_stream = demand.open_bi(StreamPurpose::Service, deadline).await?;
pump_bounded_opaque_bytes(ipc_tunnel, service_stream).await?;

verify_same_uid(&stream)?;
let request = read_one_strict_frame_and_eof(&mut stream).await?;
let reply = spawn_blocking(move || service.dispatch(request)).await??;
remove_socket_only_if_token_matches(socket_path, listener_token)?;

match (pinned, gutter_hit, modes.mouse, screen, modes.alternate_scroll) {
    (true, ..) | (_, true, ..) => scroll_zterm_viewport(),
    (_, _, true, ..) => forward_one_mouse_report(),
    (_, _, false, Alternate, true) => forward_one_cursor_key(),
    _ => scroll_zterm_main_history(),
}

let update = viewport_cache.set_target(target);
if let Some(query) = update.request {
    save_complete_query_then_send(query).await?;
}
if update.render_local {
    viewport_pacer.mark_dirty(now); // state is current; no frame is queued
}

// After every event in this stdin delivery has updated the target:
if viewport_pacer.due(now) {
    present_latest_complete_cached_frame_atomically()?;
    viewport.observe_presentation(); // only after the outer write succeeds
    viewport_pacer.mark_presented(now);
}
```

```rust
// Wrong: the latest response was coalesced and never painted, while resume
// temporarily converts "no replacement metrics yet" into a blank gutter.
history.frame = Some(intermediate_frame);
state = ResumePending { snapshot_applied: false };
render_scrollbar(None);

// Correct: presentation authority changes only when a complete frame is
// committed. Preserve it until the replacement snapshot supplies valid live
// metrics, then compose those metrics into that same atomic snapshot frame.
if queued_target.is_some() {
    request_latest_without_replacing_presented_frame();
}
state = ResumePending {
    snapshot_applied: false,
    presented_scroll_metrics: last_painted_metrics,
    ..
};
```

```rust
// Wrong: this target is locally renderable, but the 16 ms frame is still pending.
last_presented_metrics = viewport_cache.desired_metrics();

// Correct: keep desired and actually presented state independent.
present_latest_complete_cached_frame_atomically()?;
viewport.observe_presentation();
```

```rust
// Wrong: the child has reclaimed the rightmost column, but stale Zterm chrome
// is composed afterward and erases a nested TUI's scrollbar.
write_child_alternate_snapshot()?;
clear_previous_gutter_column()?;

// Correct: only clear a stale column while both layouts assign it to Zterm.
// The child snapshot is authoritative when ownership transfers away, and the
// presented baseline advances only after the outer write and flush succeed.
write_child_alternate_snapshot()?;
write_current_owned_chrome_without_reclaimed_column_cleanup()?;
present_atomic_frame()?;
viewport.observe_presentation();
```

```rust
// Wrong: synchronization is treated as disconnection and every state change
// leaks another complete frame to the user's terminal.
if transport_state != TerminalViewTransportState::Active {
    status.show_unknown();
}
render_view_for_sync()?;
render_replacement_snapshot()?;
render_unchanged_active_view()?;

// Correct: only a real connection-epoch boundary invalidates path/RTT.
if transport_state == TerminalViewTransportState::Reconnecting {
    status.reset_for_reconnect();
}
retain_complete_frame_during_same_epoch_sync();
render_replacement_snapshot_with_content_and_all_chrome_atomically()?;
complete_active_input_and_resize_state_without_unchanged_repaint()?;
```

```rust
// Wrong: constructing a runtime does not make it ambient on this thread.
let runtime = tokio::runtime::Builder::new_current_thread().build()?;
let supervisor = startup.spawn(handle); // tokio::spawn panics: no reactor

// Correct: bind task creation to the owned runtime, then release the guard.
let supervisor = spawn_inside_runtime(&runtime, || startup.spawn(handle));
runtime.block_on(serve_local(...))?;
```

The duplex branch retains the same decoder leftovers and uses bounded control
state plus latest-only watches instead of a per-revision queue.

A viewport observation is not an Active input fence. Do not send a resize and
then immediately inject a one-shot input or detach. Keep exact reader-fence
ordering in pure tests; a process fixture may use bounded idempotent readiness
probes and must keep resize/signal restoration in a separate deterministic
phase when their synchronization can race input.

The same rule applies before measuring stale-socket shutdown:

```rust
// Wrong: on Darwin this may still connect once and fail as an unrelated EOF.
drop(stale_listener);
let error = reset_identity_with_stop_timeout(Duration::from_millis(40))
    .await
    .expect_err("stale socket blocks reset");

// Correct: settle fixture teardown independently, then test the one deadline.
drop(stale_listener);
wait_until_connect_is_refused(Duration::from_secs(1)).await?;
let error = reset_identity_with_stop_timeout(Duration::from_millis(40))
    .await
    .expect_err("stale socket blocks reset");
assert_eq!(error.kind(), DomainErrorKind::DeadlineExceeded);
```

## Forbidden patterns

- A Session frame decoder, attachment registry, replay engine, revision/
  viewport cache, acknowledgement decision, or ID translation in the daemon
  tunnel. The frontend tunnel adapter owns exactly one outer-envelope decoder
  and the transport-independent client owns exactly one inner Session decoder.
- Calling a blocking `SessionService`/PTY operation inline on the current-thread
  Tokio runtime.
- Calling `tokio::spawn` from synchronous daemon startup outside the exact
  daemon-owned runtime's `enter()` guard.
- Routing wheel input from process names, TERM, screen text, or special cases
  for tmux/Herdr/PiAgent instead of authoritative terminal modes.
- Passing raw child OSC 52 across Session/wire, broadcasting or replaying a
  clipboard effect, retaining clipboard payload in `watch`/lifecycle queues,
  allowing clipboard reads, or logging/formatting its content.
- A second mouse/pointer router, selection state in Session/model/wire, or
  desktop gesture/ANSI/clipboard dependencies in the renderer-neutral core
  range and extraction helpers.
- Leaving global Kitty keyboard enhancement enabled, modifying a caller's
  stack entry instead of owning one scoped entry, recognizing copy by terminal
  brand/raw-byte special case, or guessing press/repeat/release with a timer.
- Storing semantic scroll position in the shared model/resume checkpoint,
  recreating a stateful server viewport, or treating an in-epoch replacement
  snapshot as a new transport reconnect.
- Rendering loading/returning/Gap as replacement content, accepting a window
  without its saved query shape, or coupling the renderer-neutral cache reducer
  to ANSI, Tokio, mouse pixels, or clocks.
- Constructing or flushing ANSI terminal content before semantic
  status/gutter/cursor/capture composition, allowing any writer other than
  `DesktopPresenter` during an active attachment, permitting a child mode
  transition to leave outer `1003/1006` capture disabled, or omitting the outer
  DEC 2026 end from normal or cleanup paths.
- Clearing a former gutter after a full-width child snapshot has reclaimed the
  column, deriving stale cleanup from an unpresented layout, or advancing the
  presented-gutter baseline before the atomic write and flush succeed.
- Flushing one outer history frame per decoded wheel report, using an
  always-running/global PTY render ticker, sliding a pending cadence deadline on
  every input, or treating a locally presentable cache target as actually
  presented before its atomic outer write succeeds.
- Removing or rebinding a socket without the held daemon lock and exact
  device/inode/change-time ownership token.
- Reporting successful stop before every registry-owned child/thread/reservation
  is released.
