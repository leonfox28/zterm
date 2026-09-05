# Bug Analysis: shared synchronization barriers and return-to-live presentation

## 1. Root Cause Category

- **R1: B / E, with D as a detection gap.** SessionClient discarded the correlated
  reconnect delta's barrier meaning; UI reconstructed it from Synchronizing.
  A queued ordinary delta could cross resize and gain an illegal ACK. This was a
  missing cross-layer distinction, shared by Direct and Tunnel.
- **R2: C / D.** The existing one-frame presenter contract was sound, but snapshot
  installation composed ResumePending from retained history and Active skipped
  presentation unconditionally. This was an implementation violation of that
  contract, not evidence that another renderer or terminal model was needed.

## 2. Why Earlier Fixes and Tests Failed

1. Capturing the delta handler's entry state protected the delta that initiated
   resize, but not another delta already queued behind it. Its test repeated the
   same predicate and omitted the command/server boundary.
2. Real Herdr mode changes yielded full snapshots. The recorded failure began
   with snapshot ACK -> Active -> deferred width resize -> queued ordinary delta.
   The real UI regression now covers this sequence and physical resize. The
   target-still-Active schedule explicitly holds the frontend sync fence before
   target resize processing; it does not claim to inject a physical signal there.
3. The driver black-box bypassed CLI ACK/presentation and ran Herdr --no-session.
   That startup sequence was a useful control, not the reported persistent client
   attaching to an existing server. The latter was reproduced on both real routes.
4. Scroll acceptance stopped after wheel-up and counted frames. It did not check
   child cells on return-to-live. Chrome could report offset zero over old cells.

## 3. Prevention Mechanisms

| Priority | Mechanism | Implemented action |
| --- | --- | --- |
| P0 | Typed owner contract | Only the correlated reconnect response emits ResumeDelta; the driver preserves it. Ordinary deltas never infer ACK authority from state. |
| P0 | Real consumer regression | Actual TerminalUiSession, command driver, SessionWireServer and SessionService verify continued input and absence of duplicate snapshot cascades. A valid typed resume delta is also applied/presented/ACKed and releases the input fence. |
| P0 | Transactional presentation | Snapshot composition explicitly selects live cells for ResumePending; surface/layout/readiness commit after write/flush. Active uses the presenter's real equality check. |
| P0 | Observable screen regression | Local/Remote display modes with/without new output compare every child row before Active/click; failed flush, pinned history, cursor restoration and identical-frame no-op are checked. |
| P1 | End-to-end screen acceptance | The existing outer-PTY wheel test now returns live and compares all 23 x 79 child cells. It reuses SessionService's sole host model to replay captured ANSI in an isolated fixture; no second parser or engine dependency is added. |
| P1 | Discriminating checks | Restoring old ACK inference makes the maintained UI regression fail with NotSynchronized; restoring retained-history composition makes the maintained cell comparison fail. Both mutations were reverted immediately. |

## 4. Systematic Expansion and Evidence Limits

- Direct/Tunnel SessionClient tests submit resize between target output and event
  consumption, verify ordinary Delta identity and exact wire command parity.
  View-driver queue tests preserve ResumeDelta across Active/Synchronizing.
  Existing correlated remote reconnect, takeover, lease and strict ACK tests remain.
- Production changes are confined to the shared client/view/UI boundary. No target
  ACK relaxation, protocol version change, application/route patch, retry or sleep
  is added. ResumeDelta uses the existing candidate application/presentation path
  and also completes ResumePending readiness after successful presentation.
- Real local default persistent Herdr passed 3/3 at 50 x 180; the monolithic
  control passed at 40 x 140. The actual dev route, with an already primed isolated
  Herdr server, passed at 40 x 140 using the built CLI and existing 0.1.16 daemons.
- Remote test Session zterm-causal-930d7d0fac's close reply was ambiguous. A subsequent
  read-only `session list dev --json` showed only the user's main Session, proving
  the owned test Session was already removed. The initial runner exit 1 is retained
  as a cleanup-response issue, not misreported as a fully green runner invocation.
- The historical successful remote run has no trace; no specific latency is
  retroactively asserted. Real paired evidence is distinct from Linux-owned
  simulated Iroh CI, which the macOS host cannot execute.

## 5. Knowledge Capture

- Updated backend/local-daemon-ipc.md with the explicit typed ACK contract,
  snapshot commit boundary, cursor/equality behavior and owning regression points.
- Updated backend/session-service.md to preserve the strict target ACK authority
  and explain the stale-ACK replacement/duplicate cascade.
- Updated guides/root-cause-and-architecture-thinking-guide.md with queued-event
  meaning, route-comparison evidence limits and real-consumer assertions.
- No `src/templates/markdown/spec/` exists in this application repository; no
  template mirror applies. Spec changes belong in the same proposed work commit.
- The project workflow requires one concrete Phase 3.4 commit-plan confirmation;
  the implementation authorization does not skip that repository checkpoint.
