# Shared upload service implementation plan

## Prerequisites

- [x] Parent final review explicitly approved; activate this child first.
- [x] Load trellis-before-dev and core-wire-domain, transport-auth, session-service,
  shared-client, effective-user-state and local-daemon-ipc specs.

## Ordered work

1. Trace source reuse and finalize domain/API names against parent constants.
2. Add domain/proto frames, strict converters, typed errors and registry tests.
3. Retain capability metadata per actual broker/Iroh connection and expose through
   the local tunnel. Verify old-peer absence before sending any upload frame.
4. Add Session admission/lifecycle revalidation and host file store/stream handler,
   keeping disk IO out of actor locks and reusing private-path ownership helpers.
5. Implement shared bounded uploader with concurrent ACK reads, progress/cancel,
   exact byte accounting, setup/idle/final budgets and no automatic replay.
6. Add local desktop facade and mobile Iroh adapter wiring; dependency policy passes.
7. Exercise real host service fixtures and publish the stable API for both children.

## Focused validation

- cargo +1.98.0 test -p zterm-core -p zterm-proto -p zterm-client -p zterm-platform -p zterm-daemon
- cargo +1.98.0 clippy -p zterm-core -p zterm-proto -p zterm-client -p zterm-platform -p zterm-daemon --all-targets --all-features -- -D warnings
- cargo +1.98.0 fmt --all -- --check
- sh tests/source-policy.sh
- sh tests/terminal-dependency-policy.sh

Meaningful new fixtures: 0/cap/cap+1, wrong total/offset/request/transfer ID, illegal
message state, missing capability, old broker metadata, unauthorized/wrong attachment,
takeover/detach mid-transfer, cancellation/finalization race, lost final reply,
private modes/symlink refusal/same-name uniqueness, disk write failure, bounded
backpressure and host-ACK byte semantics. Reuse deterministic stream/actor fixtures;
no network sleeps or synthetic tests that only mirror an encoder.

## Risk and handoff

Risky files: connection_broker.rs, session.rs, session_wire.rs, local tunnel DTOs and
shared Session connector abstractions. Preserve existing terminal/reconnect/auth
regressions. Record actual commands/results and public API in child research notes.
Only mark this child ready when the shared contract is verified; do not begin UI
against an unvalidated speculative wire shape. If product behavior changes, return
to parent review. No commit/publication is implied by the planning artifacts.
