# Bug Analysis: Main/Alternate resize sent a stale snapshot acknowledgement

## 1. Root Cause Category

- **Category**: B / E / D — Cross-layer contract, implicit assumption, and test coverage gap.
- **Specific cause**: the terminal UI handled one contiguous delta, presented it, then let a
  Main/Alternate layout change submit a resize that changed the local transport state from
  `Active` to `Synchronizing`. The same handler subsequently read that mutated state and treated
  the old delta as the acknowledgement barrier for the new resize epoch. The target Session
  correctly rejected that acknowledgement because its attachment was not awaiting that snapshot.
- **Architecture finding**: this was not a Local-only Session contract. The old Remote semantic
  bridge silently filtered the invalid acknowledgement unless its own epoch was synchronizing,
  so Remote success masked the shared UI defect. The superseded bridge was also an architecture
  boundary problem because it duplicated Session-client semantics in the viewer daemon. The old
  Local-without-status-row behavior, however, implemented the previously documented product scope;
  it became incorrect only when the product requirement changed.
- **Evidence and confidence**: the initial plausible causes were a UI transition error (45%), a
  target Session validation error (25%), and route-specific attachment architecture (30%). Code
  tracing showed that the UI changed state before deciding whether to acknowledge, while the target
  validator accepted acknowledgements only in `Awaiting` and the old Remote bridge dropped this
  exact command. The generic DECSET/DECRST 1049 pseudo-TTY regression and the Herdr 0.8.2 black-box
  now pass without weakening target validation. Confidence in the UI transition as the immediate
  cause is above 95%; the route split is the reason the defect was observable only on Local.

## 2. Why Earlier Fix Directions Would Fail

1. **Assume route unification fixes Herdr**: moving both routes onto one client would merely expose
   the same stale acknowledgement on Remote unless the common event transition were corrected.
2. **Relax target acknowledgement validation**: ignoring unexpected or duplicate acknowledgements
   would hide a corrupt client state transition and weaken the sole Session truth.
3. **Only add a Local status row**: it would make Local and Remote geometry consistent, but
   Main/Alternate still legitimately changes the Main gutter width and can start a resize epoch.
4. **Keep the Remote semantic bridge as a compatibility filter**: it would preserve two owners for
   revision/resume/attachment semantics and allow route-dependent behavior to recur.

## 3. Prevention Mechanisms

| Priority | Mechanism | Specific action | Status |
| --- | --- | --- | --- |
| P0 | Architecture | Keep identity/Endpoint/shared connection in the viewer daemon, but keep Session IDs, resume, revision, viewport, correlation, mutation ambiguity, and acknowledgement in one frontend Session client. | DONE |
| P0 | Runtime contract | Retain the target Session's strict `Awaiting` plus exact-revision validation. | DONE |
| P0 | State transition | Capture whether a delta acknowledges an existing synchronization epoch at handler entry, before presentation or resize side effects. | DONE |
| P0 | Regression test | Drive production `run_terminal` through generic DECSET/DECRST 1049 transitions, assert exact geometry, further input, clean detach, Local status, and absence of `not_synchronized`. | DONE |
| P1 | Protocol boundary | Use a bounded opaque same-UID tunnel so the viewer daemon cannot filter or rewrite inner Session semantics. | DONE |
| P1 | Type boundary | Carry target display and Local/Remote route as independent typed metadata; never infer route from an optional alias. | DONE |
| P1 | Documentation | Record route-neutral Session ownership and event-entry acknowledgement decisions in backend and cross-layer specs. | DONE |

## 4. Systematic Expansion

- **Similar issues**: reconnect activation deltas, takeover snapshots, history resynchronization, and
  any handler that both changes transport state and later branches on that state need the same
  event-entry-versus-post-effect distinction.
- **Design improvement**: transport adapters may differ in establishment, recovery, and path
  observation, but they must feed one Session interpreter. Sideband route/path information cannot
  participate in rendering, resize, revision, or acknowledgement semantics.
- **Process improvement**: paired Local/Remote tests must compare target-visible Session commands,
  not only rendered success. A route-specific layer that drops a command must be treated as masking
  evidence and investigated before declaring parity.

## 5. Knowledge Capture

- [x] Updated `.trellis/spec/backend/local-daemon-ipc.md` with frontend/daemon ownership,
  event-entry acknowledgement semantics, universal chrome, reconnect, and failure isolation.
- [x] Updated `.trellis/spec/backend/transport-auth.md` and
  `.trellis/spec/backend/core-wire-domain.md` with opaque tunnel and connection ownership.
- [x] Updated `.trellis/spec/backend/terminal-model.md` for universal layout ownership.
- [x] Updated `.trellis/spec/guides/cross-layer-thinking-guide.md` for the route-adapter flow; the
  independent checker verifies the reusable event-entry-state rule is explicit.
- [x] Added generic pseudo-TTY and Herdr 0.8.2 black-box evidence.
- [ ] Template sync is not applicable in this repository: there is no
  `src/templates/markdown/spec/` tree or project spec-sync script.
- [ ] Commit follows the project's Phase 3.4 one-shot user confirmation after the independent
  quality gate; no spec-only commit is made ahead of the coherent task commit batch.
