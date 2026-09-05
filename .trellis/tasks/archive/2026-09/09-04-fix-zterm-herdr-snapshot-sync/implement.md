# Implementation Plan

## 0. Planning and Safety Gates

- [ ] Re-read `prd.md`, `design.md`, the routed Trellis specs, and both research notes immediately
  before implementation.
- [ ] Preserve the selected D boundary: viewer daemon owns identity/Endpoint/connection/stream;
  frontend process owns Session semantics. Do not reintroduce a daemon-owned desired attachment,
  target/local attachment-ID mapping, revision cache, viewport cache, control retry, or snapshot
  acknowledgement.
- [ ] Preserve one IPC connection per frontend view and one QUIC service stream per remote tunnel,
  multiplexed over one active device-pair connection. Never share one IPC byte stream between CLI
  processes and never create an Iroh Endpoint in a CLI.
- [ ] Keep target Session wire major, persistent schema, strict acknowledgement contract, CLI
  syntax, and normal Iroh ALPN unchanged. A versioned same-UID-only tunnel envelope is allowed.
- [ ] Treat the architecture migration and Herdr correction as separate reviewable changes. D is
  not evidence that the acknowledgement bug is fixed.
- [ ] Retain unrelated worktree edits. Stop and reconverge the plan if the generic regression cannot
  expose the reported failure, the broker cannot expose an opaque admitted service stream without
  weakening authorization, or the tunnel would require unbounded buffering.

## 1. Establish the Application-Neutral Red Regression

- [ ] Extend `crates/cli/tests/daemon_autospawn.rs` with a task-private child mode that drives the
  real `run_terminal` path through an outer pseudo-TTY.
- [ ] Have the child shell emit generic DECSET 1049 and DECRST 1049 transitions, waiting on observed
  terminal state rather than sleeps, then accept another input probe and detach.
- [ ] Against the pre-fix code, record that Main→Alternate or Alternate→Main reliably reaches the
  `not_synchronized` failure class. Do not invoke or identify Herdr in this primary regression.
- [ ] Add focused evidence for the current asymmetry: the Local direct path forwards the UI's stale
  acknowledgement to strict `SessionWireServer`, while the old Remote semantic bridge drops it
  unless its own epoch is synchronizing at the exact expected revision. Treat the existing remote
  Herdr success as bridge-masking evidence, not as the future D implementation's green result.
- [ ] If the generic path does not demonstrate the causal chain, stop and revisit the root-cause
  classification; do not add application-name detection or timing-only assertions.

## 2. Fix the Shared Acknowledgement State Machine

- [ ] In `crates/cli/src/terminal_ui.rs`, capture at delta event entry whether the view was already
  `Synchronizing`, before applying/presenting the delta or deriving a mode-driven resize.
- [ ] Keep the current apply/present/resize order. An entry-`Active` delta may send one resize and
  enter `Synchronizing`, but must not acknowledge that old delta as the replacement snapshot.
- [ ] A contiguous delta received while already `Synchronizing` must still acknowledge its exact
  `to_revision`; pending mode geometry starts a later resize epoch only after activation.
- [ ] Add focused tests for both transitions, gap/sync-required handling, pending resize coalescing,
  and exact revision. Do not pass route metadata into the handler.
- [ ] Rerun the generic pseudo-TTY regression and require continued input plus clean detach before
  beginning the architecture migration. The final paired Local/Remote gate must remain green after
  the old Remote filter is removed.

## 3. Define the Local-Only Tunnel Envelope

- [ ] Add versioned protobuf messages and stable local wire kinds for Open, Opened, Data, Path,
  HalfClose, and Closed. Document that these kinds are accepted only on same-UID IPC and never on
  normal Iroh Session streams.
- [ ] Choose and enforce a fixed Data chunk ceiling below the global frame limit, bounded writer
  queue capacity, establishment deadline, idle/close behavior, and one terminal close reason.
- [ ] Extend proto compatibility tests for numeric stability, unknown-field behavior where
  required, payload limits, invalid state transitions, over-limit chunks, and redacted Debug.
- [ ] Add a frontend tunnel adapter that unwraps/reassembles arbitrary Data chunks into an ordinary
  async Session byte stream and exposes Path observations separately.
- [ ] Prove that inner frames split across many chunks, several inner frames in one chunk, zero-byte
  invalid data, EOF, half-close, and cancellation behave deterministically without Session decode
  in the daemon adapter.

## 4. Implement the Viewer-Daemon Tunnel Pump

- [ ] Add a local IPC first-frame branch for remote tunnel Open. Resolve/validate the exact remote
  target and reject self-target before acquiring network resources.
- [ ] Acquire one `ConnectionBroker` demand and one admitted
  `StreamPurpose::Service` bidirectional stream; emit Opened only after successful remote
  authorization and stream establishment.
- [ ] Pump opaque chunks in both directions with one owner per writer and bounded backpressure.
  Serialize Path/Closed sideband with remote Data so envelope frames never interleave.
- [ ] Reset path/RTT to unknown for every stream epoch and project only validated observations from
  the selected broker candidate.
- [ ] Propagate half-close exactly once, cancel/reap both pumps on terminal failure, and prove all
  demand/queue/stream/metric permits release on every exit path.
- [ ] Add tests showing a slow, reset, cancelled, or malformed tunnel affects only that tunnel and
  does not close the shared peer connection or starve an unrelated peer.

## 5. Extract One Frontend-Owned Session Client

- [ ] Refactor `LocalAttachmentClient` into a transport-independent Session client plus route
  establishment adapters. It may remain library code in the daemon crate, but all instances used by
  terminal views must execute in the frontend process.
- [ ] Make the client own target-generated `SessionId`/`AttachmentId`, one stable frontend-generated
  `ResumeViewId`, last applied revision, latest viewport, request correlation, operation lease and
  sequence, pending controls/queries, and mutation ambiguity.
- [ ] Local adapter opens one direct same-UID Session IPC connection and uses `target.local=true`.
  Remote adapter opens one IPC tunnel and sends unchanged inner Session frames naming the exact
  remote `DeviceId`.
- [ ] Move remote reconnect/resume behavior out of `remote_attachment.rs`: after retryable tunnel
  loss, reopen a tunnel and attach using the same ResumeViewId, frozen SessionId, known applied
  revision, and latest viewport; accept the target's new epoch attachment ID unchanged.
- [ ] Keep connection recovery strategy outside the route-neutral frame interpreter. Target errors,
  snapshots, deltas, leases, controls, history and clipboard must use one decode/correlation path.
- [ ] Preserve post-write mutation ambiguity and replay-lease rules in the frontend. The tunnel
  daemon must neither retry nor declare a potentially committed mutation successful/failed.

## 6. Switch Remote and Remove the Semantic Bridge

- [ ] Route new remote terminal views through the tunnel Session client while keeping local views on
  the direct adapter behind the same frontend API.
- [ ] During development only, use current bridge tests/traces as a parity oracle. Do not ship a
  hidden A fallback or runtime flag that leaves two Session clients authoritative.
- [ ] Add paired fixtures that feed the same target Session trace through local direct IPC and
  remote tunnel adapters and assert identical target IDs, revisions, events, commands,
  acknowledgement decisions, and viewport convergence.
- [ ] Add a two-frontend integration: two IPC tunnels map to two service streams on one broker peer
  connection; closing/resetting one keeps the other live.
- [ ] Add peer-loss integration: all live clients observe reconnect, broker creates one replacement
  connection, and each client independently resumes its own attachment.
- [ ] Delete daemon-owned desired attachment/ID translation/revision/viewport/pending-control
  machinery and its obsolete tests once all parity/fault gates pass. Retain only reusable transport
  admission or target-side Session code.

## 7. Carry Explicit View Metadata and Universal Chrome

- [ ] Replace optional-alias-as-route inference with an immutable redaction-safe view target:
  explicit Local/Remote route plus frozen display name. Keep exact routing authority private.
- [ ] Derive Local display name from committed daemon configuration/status and Remote display name
  from the exact target's safe alias. A non-empty name must never select Remote behavior.
- [ ] Keep tunnel Path/RTT as a presentation side channel. Do not inject it into Session bytes or
  pass route/path into snapshot/delta/resize handlers.
- [ ] Make `ChromeLayout` reserve one bottom row for both routes whenever physical rows are at least
  two, while preserving the one-row child fallback and Main gutter.
- [ ] Render exactly `<device> | local` for Local, with no third/latency field, and
  `<device> | <direct|relay|--> | <integer ms|-->` for Remote.
- [ ] Preserve reverse-video whole-row style, Unicode display-cell clipping, atomic presentation,
  cursor/style restoration, and child/gutter/mouse/selection coordinate isolation.
- [ ] Update focused layout/composition tests for Main 23x79, Alternate 23x80 at physical 24x80,
  resize sequences, narrow/Unicode labels, reconnect reset, and one-row fallback.

## 8. End-to-End and Fault Validation

- [ ] Complete the generic pseudo-TTY test with both Local direct IPC and Remote tunnel fixtures.
  Require Main→Alternate→Main, exact viewport convergence, further shell input, clean detach, and
  no `not_synchronized` error.
- [ ] Run the real local-daemon path and assert an atomic frame contains `<device> | local` without
  a trailing third field.
- [ ] Run Herdr 0.8.2 only as an external blackbox smoke and verify return to the original Session
  shell after Herdr exits.
- [ ] Exercise target authorization/revoke during tunnel establishment and while streams are live;
  preserve target-side authenticated principal and remote resume rules.
- [ ] Exercise tunnel open failure, malformed outer envelope, slow consumer/backpressure,
  half-close, service-stream reset, peer-connection loss, viewer-daemon restart, and target Session
  end against the design's isolation matrix.
- [ ] Verify `N` remote views create `N` IPC sockets and service streams but one active device-pair
  connection; verify `N` local views create `N` IPC sockets and no Iroh demand.

## 9. Focused Commands

- [ ] Format touched Rust/protobuf-generated surfaces:

  ```bash
  cargo fmt --all
  ```

- [ ] Run protocol and daemon unit suites:

  ```bash
  cargo test -p zterm-proto --all-features
  cargo test -p zterm-daemon --lib --all-features local_ipc::tests
  cargo test -p zterm-daemon --lib --all-features connection_broker::tests
  cargo test -p zterm-daemon --lib --all-features session_wire::tests
  ```

- [ ] Run terminal client/UI and real local pseudo-TTY suites:

  ```bash
  cargo test -p zterm-cli --lib --all-features terminal_ui::unix::tests
  cargo test -p zterm-cli --test daemon_autospawn --all-features
  ```

- [ ] Run real-Iroh loopback and external smoke:

  ```bash
  cargo test -p zterm-daemon --test two_daemon_transport --all-features -- --nocapture
  tests/foundation/terminal-blackbox.sh --mode herdr
  ```

## 10. Durable Contract and Full Quality Gate

- [ ] Update `.trellis/spec/backend/local-daemon-ipc.md` with per-frontend IPC ownership, opaque
  remote tunnel framing/backpressure/failure isolation, daemon network ownership, frontend Session
  ownership, target-side route neutrality, universal chrome, exact status shapes, and event-entry
  acknowledgement semantics.
- [ ] Update any network/connection spec that currently assigns remote attachment semantics to the
  daemon. Preserve one-connection-per-device-pair and authenticated service-stream admission.
- [ ] Remove stale active-spec language for daemon-owned desired attachments, ID rewriting,
  remote-only status rows, local-without-status layout, and alias-presence route inference. Archived
  tasks remain historical records.
- [ ] Run:

  ```bash
  cargo fmt --all -- --check
  cargo clippy --workspace --lib --bins --all-features -- -D warnings
  cargo test -p zterm-core -p zterm-proto -p zterm-platform -p zterm-terminal -p zterm-daemon -p zterm-cli --all-features
  ```

- [ ] Search code/specs for obsolete boundaries and justify every remaining occurrence:

  ```bash
  rg -n "DesiredAttachment|remote_alias|terminal_request_is_remote|bridge_attachment|remote-only row" crates .trellis/spec
  ```

- [ ] Run the Trellis checker and review the full selector → route adapter → Session client → target
  wire → target Session → normalized event → renderer data flow. Confirm one owner for network,
  Session state, target display, layout, and each acknowledgement decision.

## 11. Completion Evidence and Rollback

- [ ] Record pre-fix red evidence, focused/full command results, multi-view connection cardinality,
  failure-isolation results, and the actual `zterm` → Herdr manual outcome in this task.
- [ ] Keep the acknowledgement fix independently reviewable and retain it if tunnel or chrome work
  must be rolled back.
- [ ] If the tunnel migration violates authorization, boundedness, or sibling-stream isolation,
  roll back that migration to the last passing stage; do not relax target acknowledgement, give the
  CLI the device key, or retain two production Session clients as a workaround.
- [ ] Do not declare completion until the old semantic bridge is gone, Local and Remote pass the
  same Session trace tests, exact status bars are correct, and Herdr plus the generic regression
  both pass.
