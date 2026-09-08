# Android reconnect and connection status design

## Diagnosis and Boundary

Classification: localized Android policy gap and omitted presentation projection; the existing shared protocol architecture is adequate. The previous exact-ID policy explains the symptom and is changed by this request. Session existence belongs to the daemon, entry selection to AppRepository, and route/RTT to the attachment transport. Do not place retargeting in SessionClient's same-ID reconnect loop.

Minimal reproduction: save a Session ID, restart its test daemon, then enter the host or press Retry; session_not_found repeats without a usable attachment. Separately, ConnectionStatus reaches the native actor and is discarded. Neither requires a pairing or protocol redesign.

## Entry and Default Attachment

1. Consolidate Repository machine-entry/retry selection while explicit row attachment stays exact. List first and prefer the remembered/current target if present. Only a typed missing/ended result or successful authoritative list proving absence permits fallback. Fence suspended results by navigation epoch and host.
2. Clear matching stale last-Session/recent references after confirmed absence. Empty list opens default main; one unoccupied Session attaches; multiple/occupied Sessions use the current picker.
3. Add a distinct native default-terminal entry point using SessionClient::connect with selector=None, create_main=true, takeover=false. Keep connect_terminal exact. Share preparation/adoption so viewport, ACK, actor and source ownership follow one path.
4. Expose the actual Session ID from PreparedTerminalView on NativeTerminal. Save it after adoption and refresh display metadata. Do not create main via ordinary named unary creation: it is reserved; daemon default attachment already handles concurrent creation.
5. Preserve operation_outcome_unknown and never automatically repeat a mutation after ambiguity. A later explicit Retry first resolves the live list. Preserve occupied confirmation and Back/epoch fencing.

Live-attachment transport recovery still resumes the exact ID. SessionEnded ends that attachment; user Retry invokes entry selection. No timer silently recreates exited shells.

## Connection Metadata

Data flow: Iroh selected path/RTT -> AttachmentTransportItem::Path -> TerminalViewEvent::ConnectionStatus -> native actor -> NativeFrame -> lightweight TerminalStatus -> header.

- Add a typed native unknown/direct/relay projection and optional RTT. Use the selected-path round-trip estimate, not history-query timings or a separate UI diagnostic poll.
- Consume connection events and publish with the same attachment frame. Clear path/RTT at reconnect, ended, lease loss and closure. Preserve through healthy synchronization/resize.
- IrohSessionIo::read samples before awaiting terminal input (`crates/client/src/iroh_controller.rs:721`), so an idle read can block new observations. Provide one bounded periodic local wakeup at this owner (about one second) if needed, emitting only changed samples. Preserve decoder progress, cancellation and Session event delivery; add no application pings or UI polling.
- Extend Repository's distinct metadata projection without resolving rows again; preserve NativeFrameSource ownership and content-generation reuse.
- Render host with ellipsis and reserve second-line room for status/RTT. Define `Direct` and `Relay` as non-translatable English string resources; other connection-state labels remain localized. Keep title, dropdown hit area, header height and grid geometry. Initial list/attach uses Repository busy state; healthy visual sync keeps its established route.

## Files and Compatibility

Expected product files: crates/android/src/terminal.rs for default attach, ID and status; crates/client/src/iroh_controller.rs only for idle observation; Android AppRepository.kt, TerminalScreen.kt and both strings.xml locales. Update affected NativeFrame fixtures and focused Android instrumentation tests. Generate Kotlin using the normal build; never edit generated bindings.

Update docs/android.md, .trellis/spec/frontend/android-app.md and .trellis/spec/backend/shared-client.md with the superseding policy and bridge contract. Historical task documents remain history. Reuse wire-v2 and existing dependencies; no host upgrade or storage migration is required.

## Risks and Rollback

Risk centers on async adoption/persistence, ambiguous default-create responses and stale RTT after real reconnect. Existing operation/navigation epochs remain the owner. Use test-owned daemons for restart evidence; do not restart user daemons. Distinguish runtime route/device evidence from build-only checks.

Rollback reverts Android policy, bridge signature, status projection and corresponding fixtures together; the saved host/Session-ID schema stays compatible.

## Validation finding: typed unary errors after restart

The restart fixture reached a new Active main and accepted input, then a named
mutation using the previous daemon's operation lease returned malformed_frame.
Root cause is local to the existing Iroh SessionUnaryTransport adapter:
RemoteUnaryClient deliberately returns a validated ServiceError frame for
forwarding consumers, but the Android frontend passed that frame to a success
payload decoder. Its OperationOutcomeUnknown never reached SessionUnaryClient's
existing lease-retirement branch. Project ServiceError to a typed, redacted Err
at the Iroh frontend boundary, matching the desktop adapter. No mutation is
replayed: a later explicit call obtains a fresh lease. This stays within the
planned Iroh adapter/error-contract boundary; no wire or host behavior changes.
