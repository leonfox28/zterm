# Architecture-preserving client cleanup (2026-09-07)

## Approved scope and gaps

The user explicitly asks to finish the three earlier proposed simplifications.
Development 1015 fixed local Android selection/projection only; it did not finish
these items. Preserve desktop CLI → Unix IPC → local daemon → Iroh → host and
Android → Iroh → host. Work inline; no subagents or physical-phone testing.

1. Shared Session protocol state still owns `RemoteDaemonRestarter`, despite the
   design assigning local process lifecycle to the desktop adapter. Move that
   capability and stopped-socket recovery into `UnixAttachmentConnector`, injected
   by `LocalRuntime`. Retain remote-only recovery, lifecycle-locked launch, exact
   Session/ResumeView IDs, viewport/revision, and launch-error propagation. The
   shared owner continues protocol retries without knowing a launcher capability.
2. Host/controller composition is already separate: Android depends on client,
   core/proto and the native bridge; no daemon/platform/PTY/host terminal crate.
   Keep this existing enforced dependency boundary and outbound-only endpoint.
   Document composition explicitly; do not invent role negotiation or move the
   host authorization registry/bidirectional candidate arbitration into mobile.
3. Both adapters duplicate the normal Hello stream open/write/finish/Welcome
   sequence. Centralize that exact outbound handshake in `client::handshake`,
   leaving connection admission, identity, deadlines and candidate arbitration
   with their current owners. Route ordering already shares `client::route`;
   retain it rather than adding a second route owner.

## Error diagnosis and classification

`read_controller_welcome` converts every read error except malformed/oversize
frames into Unauthorized. This provably mislabels DeadlineExceeded and
TransportUnavailable; missing Welcome alone is not authorization evidence.
Classify this as a **local error-contract violation**. Preserve the original
typed read error. At the common Iroh handshake boundary, only the authenticated
peer's existing application close code `0x100` establishes Unauthorized. Inspect
the close reason before candidate/connection cleanup replaces it with a local
close. Never classify by free-text reason or expose it to user-facing errors.

The exact causes of the two earlier cold-start failures cannot be reconstructed
from their overwritten error labels; do not claim they were actual revocations,
timeouts or a firewall refusal. Tests will establish distinguishable failure
classes and emulator acceptance will exercise real host authorization/handshake.

## Expected files and validation

- `client/session.rs`, desktop `client/{session,transport,mod}.rs` and
  `operations.rs`: relocate lifecycle ownership; migrate the existing restart
  regression rather than replacing its real Unix tunnel fixture.
- `client/handshake.rs`, `iroh_controller.rs`, daemon `connection_broker.rs`:
  one outbound sequence, precise errors and one authorization close-code constant.
- Existing Android integration tests: actual known-host handshake and unknown
  controller denial, using emulator-owned runtimes and Sessions only.
- Shared-client, IPC/transport specs and task notes: record final ownership and
  validation limits. No wire format, protocol deadline, routing preference,
  deployment, installed host-daemon replacement or real-Iroh macOS test helper.

Establish failing Welcome error tests first. Then shared Session/handshake tests,
desktop adapter/lifecycle suites, dependency policy, formatter/Clippy, Android
build/lint/JVM and emulator integration. Final artifact follows 1015 (next 1016).

## Active cold-start investigation

Initial emulator test: known host Welcome and a disposable unknown identity's
explicit Unauthorized both pass. The immediately following NativeTerminal test
fails before Session creation with `transport_unavailable`, not Unauthorized.
The installed host records the previous primary at 11:59:30 UTC and its transport
closure at 12:00:01 UTC. NativeRuntime Drop currently shuts down its Tokio runtime
before dropping the controller/network. Investigate a retained prior connection
and duplicate arbitration; do not change retry/deadline policy based on timing.

Temporary diagnostic build 1016 maps only the verified duplicate application
close code 0x102 to `diagnostic_duplicate_handshake` through the native error
boundary. This is investigation-only, contains no peer text, and MUST be removed
before final APK/signing. All finalized APKs through 1015 remain untouched.

The new explicit-shutdown→same-identity-reconnect fixture reproduced
`diagnostic_duplicate_handshake`: verified peer code 0x102, not authorization
denial (`architecture-shutdown-baseline.log`). `NativeRuntime::shutdown` only
cancels local observations; subsequent Drop stops Tokio before network/Peer
destruction can deliver connection closure. This is a second **local lifecycle
contract violation**, with the existing controller/runtime as the correct owner.
Extend this increment to `android/lib.rs` and controller shutdown: await the
existing Iroh Endpoint close on the live executor for explicit shutdown. Preserve
Drop as abrupt disposal and never call shutdown for Activity/background events.
Use explicit shutdown in temporary native test runtimes. Verify immediate
same-identity reconnect, final-observer wakeup and idempotence; do not add retries,
sleep delays, another executor or change host duplicate arbitration. This proves
the cause of the newly captured fixture, not every historical cold-start error.

## Final implementation and evidence

- Removed the launcher trait, field and setter from production shared Session
  state. The desktop adapter owns/invokes it, composed only for remote views by
  LocalRuntime. Existing stopped-daemon launch failure, local exclusion and real
  Unix tunnel resume tests pass with the same exact IDs, viewport and revision.
- Both broker and IrohController use one outbound handshake. Route ordering was
  already shared and remains unchanged. Updated the task diagram to show the
  actual desktop IPC/tunnel path separately from mobile direct Iroh; documented
  host/controller composition and dependency boundaries.
- Original Welcome deadline and connection-reset tests each failed as
  Unauthorized (`architecture-handshake-baseline.log`). They now retain their
  own categories. Missing/partial frames and explicit 0x100 vs other close codes
  have independent regressions. A first malformed-byte fixture accidentally
  encoded an incompatible wire major; replaced it with a truncated valid frame
  instead of changing the decoder's correct rejection.
- Working-tree Cargo build cache initially referred to a deleted release
  checkout's protobuf build script. Rebuilt only the zterm-proto package cache;
  no protobuf/source contract changes were needed.
- Workspace tests: 610 passed, 7 platform-scoped tests ignored, 53 passing suite
  reports (`architecture-workspace-tests.log`). No macOS real-Iroh listener test
  was enabled. Final touched client/native tests pass (72 + 12), whole-workspace
  all-targets/all-features Clippy with `-D warnings` passes, along with dependency
  policy and formatting. Debug/Release lint and Debug JVM checks pass.
- Final emulator run: **8 tests / 36.474 seconds**, all pass
  (`architecture-emulator-final.log`). Includes explicit shutdown then four
  immediate same-identity reconnects without sleep/retry changes, known-host
  Welcome and unpaired identity denial, repeated shutdown/observer closure,
  actual Session input/copy/resize/rename/detach and isolated Herdr interactions.
- Temporary diagnostic mapping was removed from Rust/FFI and verified absent in
  both finalized APK native libraries. Only explicit shutdown became async;
  Activity/background behavior and ordinary connection retention are unchanged.

## Delivery

Final artifacts under `target/android-apk/1016/`:

- Release acceptance SHA-256:
  `1bd2d4fd54c73be7099b63fe5b96ad317f16d166edc286207d684dae0b8de311`.
- Development SHA-256 and emulator retained-data results: `SHA256SUMS` and
  `dev-install.json` in that directory. Both signatures and 16 KiB native/ZIP
  checks pass. Development keeps the standard debug certificate.

Emulator-5554 updated to development 1016 at 20:14:11; first install remains
15:10:59, with identity and saved-state hashes unchanged. Dedicated headless
emulator-5556 stopped after testing. No physical phone operation, host-daemon
replacement, native release publication, subagent, code commit or task archive.
Desktop changes are tested working-tree code, not the installed v0.1.25 binary.
The broader Android task remains in progress. Abrupt OS process termination is
not equivalent to awaited explicit shutdown and was not claimed fixed by this
teardown change; old historical generic-error reports cannot be reconstructed.

API source: [Iroh connection closure](https://docs.rs/iroh/1.0.3/iroh/endpoint/struct.Connection.html#method.close_reason)
and [application close codes](https://docs.rs/iroh/1.0.3/iroh/endpoint/struct.ApplicationClose.html).
