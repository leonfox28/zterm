# Implementation Verification

## Added acceptance evidence

- `local_ipc::tests::direct_and_tunnel_adapters_share_one_session_trace_and_command_interpreter`
  gives a Local direct client and a Remote tunnel client the same SessionId,
  target AttachmentId, semantic delta, clipboard event, and sync requirement.
  It asserts identical normalized events and byte-identical target-visible
  `TerminalSnapshotApplied`, resize, input, and sync frames, plus equal applied
  revision and viewport state. The only differing value is route establishment.
- `local_ipc::tests::stopped_viewer_restart_opens_a_tunnel_and_resumes_the_same_frontend_state`
  starts with no viewer-daemon socket. The production-shaped restart capability
  binds a task-private replacement listener, accepts Open/Data, and verifies the
  same ResumeViewId, SessionId, target, applied revision, and latest viewport.
  The frontend accepts the target's new AttachmentId and contiguous delta, and
  proves the replacement projection is `Synchronizing -> Unknown -> Delta`
  even when the replacement tunnel sends no path sideband.
- `local_ipc::tests::shared_peer_loss_keeps_each_frontend_resume_checkpoint_independent`
  delivers `TransportLost` to two tunnels for one target device and verifies
  both independently enter Reconnecting while retaining distinct ResumeViewIds,
  SessionIds, applied revisions, viewports, and attachment identities.
- `local_ipc::tests::reconnecting_frontends_resume_independently_through_one_viewer_listener`
  concurrently resumes those two frontend-owned checkpoints through one
  task-private viewer IPC listener. Each request retains its own SessionId,
  ResumeViewId, applied revision, and viewport, and each response installs a
  distinct target-issued AttachmentId and contiguous delta.
- `local_ipc::tests::remote_reconnect_reuses_view_and_session_with_latest_frontend_checkpoint`
  now injects `Unknown`, `Unknown`, `Direct`, `Direct` before the replacement
  Session delta. The frontend still projects exactly its own epoch-reset
  `Unknown`, one changed `Direct` sample, and then the delta. Fresh and resumed
  tunnel setup share this path-sample normalizer, so the viewer does not depend
  on a tunnel-supplied reset and does not expose consecutive duplicates.
- The Linux-only `two_daemon_transport` fixture now opens two real Iroh Service
  streams from two demands on the same promoted primary. It checks one peer
  candidate/connection and `active_stream_count == 2`, completes and drops the
  first stream, completes the still-live sibling at count 1, then observes zero
  streams without replacing the primary.

## Compositional failure and ownership evidence

- Broker connection singleflight:
  `demand_bookkeeping_is_checked_singleflight_and_drops_transient_routes`.
- Stream/handler permit reclamation:
  `service_handler_admission_quiesce_and_reclamation_use_raii_permits`,
  `service_handler_panic_releases_only_its_stream_permits`, and
  `per_peer_stream_observation_tracks_raii_lifetimes`.
- Tunnel-local failure and sibling liveness:
  `malformed_tunnel_closes_only_its_own_stream` and
  `frontend_data_after_half_close_is_a_tunnel_local_protocol_error`.
- Target-side authorization, synchronization, resume, and checkpoint authority
  remain covered by `session_wire` and `session` suites; the tunnel never
  decodes those frames.
- Remote screen-switch behavior is composed from the paired route-adapter trace,
  the route-free `delta_acknowledgement_uses_event_entry_state_before_mode_resize`
  UI test, and the real local outer-PTY Main -> Alternate -> Main regression.
  The UI delta/resize interpreter has no route argument, so adding a remote-only
  semantic branch to that transition is not possible through its API.

## Platform-limited evidence

- Real Iroh endpoint binding is deliberately Linux-only. On macOS the extended
  `two_daemon_transport` target is compiled with `--no-run`; its runtime test is
  ignored before any Endpoint bind. Linux CI must execute it.
- A single test combining real-Iroh peer replacement, two full Session handlers,
  and two frontend Unix tunnels would require extending the current two-daemon
  fixture beyond its intentionally narrow inbound fallback. No production seam
  was added for that purpose. Independent frontend retention, broker redial
  singleflight, real two-stream reuse, and target resume are instead executable
  at their existing ownership boundaries above.
- A Remote outer-PTY screen-switch test likewise needs a Linux two-owner fixture
  with a real target Session service plus a viewer-daemon IPC listener. The
  current Linux fixture intentionally has neither `run_daemon` nor a complete
  Session handler, so this task records compositional evidence rather than a
  mock-only production extension point.

## Focused results

- `cargo test -p zterm-daemon --lib --all-features`: 194 passed.
- `cargo test -p zterm-daemon --lib --all-features local_ipc::tests -- --nocapture`:
  32 passed after the final path-sample normalization review fix.
- `cargo clippy -p zterm-daemon --all-targets --all-features -- -D warnings`:
  passed.
- `cargo test -p zterm-daemon --test two_daemon_transport --all-features -- --nocapture`:
  1 passed and the real-Iroh runtime case was ignored on macOS; Linux runtime
  remains required.
- `tests/foundation/terminal-blackbox.sh --mode herdr`: Herdr 0.8.2 passed its
  Alternate-screen resize, detach-progress, resync, bounded-capacity, and
  cleanup gates.

## Final checker gate

- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.
- `cargo clippy --workspace --lib --bins --all-features -- -D warnings`: passed.
- `cargo test -p zterm-core -p zterm-proto -p zterm-platform -p zterm-terminal -p zterm-daemon -p zterm-cli --all-features`:
  passed on macOS. Expected explicit/platform skips remained: isolated CLI
  helper tests, real-Iroh loopback tests, cross-UID Linux gate, and the
  explicit-only terminal black-box target.
