# Semantic Presentation Direct-Cutover Research

## User decision

The user explicitly removed mixed-version compatibility from the semantic-presentation migration
and will upgrade all nodes after release. Code whose only consumer is legacy terminal presentation
must be deleted rather than retained behind adapters, capabilities, or negotiation.

This changes deployment compatibility, not the Session data model: trusted-device records, Session
identity, one-Session/one-PTY ownership, controller/takeover state, and persistent schema remain.

## Repository evidence

- `zterm_core::WIRE_MAJOR` is the authoritative build/readiness/normal transport version and old
  majors already produce `WireMajorMismatch`.
- `ZTERM_ALPN`/`ZTERM_PAIR_ALPN` identify the current protocol generation before application stream
  dispatch. Advancing them with the wire major prevents an old remote from reaching attachment code.
- Local readiness exposes the same wire major, so an old running daemon is rejected before a new CLI
  attempts to decode terminal content.
- Kinds 301/302 are the existing primary terminal snapshot/delta slots and 317/318 are the stateless
  history-window pair. Under a new wire major they can carry the only semantic payload without
  ambiguity; temporary additive kinds 319..321 are unnecessary.
- Capabilities 17/19/20 and their protocols form a compatibility ladder: pager 312/313, stateful
  viewport 315/316, then history window 317/318. With every peer on the same semantic wire major,
  pager and stateful viewport have no remaining product consumer.
- The in-progress implementation introduced family enums, typed variants, attach preference, bit 21,
  bridge downgrade, and a CLI legacy renderer solely for mixed-version operation. These are now
  superseded work and must not become permanent architecture.

## Direct-cutover contract

1. Increment product wire major and normal/pair ALPN identifiers together. Old nodes fail at the
   readiness or authenticated handshake boundary with a stable incompatibility error.
2. Move the protobuf source/package/generated module directly from wire namespace v1 to v2. Do not
   build or expose both schema generations; independently versioned persisted data shapes may remain
   unchanged without preserving a v1 transport.
3. Use semantic snapshot, delta, and semantic history-window as the only terminal presentation
   payloads. Keep generic acknowledgement, sync, input, resize, detach, lease, and end messages.
4. Delete presentation preference/encoding, bit 21, family selection/switch state, and all downgrade
   paths. A reconnect resumes only against a semantic checkpoint.
5. Delete legacy ANSI DTOs/encoders/conversions/renderers and the 312/313 plus 315/316 fallback
   protocols. Use one semantic history-window cache for wheel, PageUp, scrollbar drag, and return-live.
6. Keep the remote bridge content-neutral: validate envelope identity/revision/correlation/bounds,
   replace the bridge-private attachment ID, and forward the semantic message without translating it
   to another representation.
7. Keep one client `AttachmentSurface`, one `ComposedFrame`, and one desktop presenter. The Android
   seam remains core/proto semantic values and cache coordinates; Android UI stays deferred.

## Why wire major two instead of a silent same-major break

Deleting payloads while retaining wire major one would let old peers complete Hello/Welcome and fail
later on an unknown attachment kind. That failure is truthful but occurs at the wrong boundary and
can leave reconnect logic repeatedly retrying a permanently incompatible peer. A major/ALPN cutover
makes incompatibility immediate, typed, and connection-scoped while allowing canonical kind numbers
to remain compact.

## Deletion inventory

- Core/model: legacy `TerminalSnapshot`/`TerminalDelta`, ANSI row/full/delta/history encoders, and
  presentation-only byte tests.
- Proto/wire: legacy terminal payload messages, presentation encoding, capability bit 21, temporary
  319..321, pager 312/313, viewport 315/316, related conversions/allowlists, and the wire-v1
  source/package/generated module.
- Driver/session/operations/local IPC: presentation-family enums/variants and branching.
- Remote bridge: capability ladder, downgrade, family renegotiation, cross-family reset, and legacy
  response/control handling.
- CLI: ANSI renderer, byte history cache, pager/stateful viewport states, independent chrome writers,
  fallback tests, and obsolete helpers.
- Repository residue: modules, public aliases, generated artifacts, Cargo features/dependencies,
  errors, and fixtures with no current semantic/lifecycle consumer after the cutover.

Deletion must be driven by actual reachability after the semantic path is connected. Generic
lifecycle, security, resource-bound, correlation, and error code with a current semantic consumer is
not “compatibility code” and remains.

## Verification consequences

- Replace the four-way old/new matrix with latest/latest local/direct/relay coverage and explicit old
  wire-major/ALPN rejection tests.
- Add source/registry guards proving no legacy presentation DTO, ANSI producer, preference, family
  variant, capability ladder, downgrade, or secondary active writer remains.
- Preserve semantic shape/redaction/bounds, model replay, resume/ack/resync/final drain, compositor
  ownership, rightmost/wide/styled-blank, failed output retry, nested-TUI, macOS/Linux, unsafe-forbid,
  and no-performance-benchmark gates.

Rollback before release is a source rollback of the complete wire-major cutover. There is no runtime
flag that restores the deleted legacy path.
