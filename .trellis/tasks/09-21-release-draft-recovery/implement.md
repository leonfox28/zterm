# Implementation and release plan

- [x] Capture approved scope, incident evidence, owning boundary and source identities.
- [x] Implement recovery helper and main-only workflow with pinned actions.
- [x] Add identity, inventory, retry and publication-boundary regression fixtures and portable CI integration.
- [x] Update runbook/spec, run focused tests, actionlint and just ci-policy; review all affected files.
- [x] Commit the approved change, open/merge a protected PR after required CI.
- [x] Dispatch recovery on main for v0.1.35, original run 35559736119, artifact 10621537362 and draft 392689610.
- [x] Verify immutable release, unchanged original asset IDs/digests, provenance and latest stable selection.
- [x] Archive task and journal after acceptance is complete; sync main.

The previously approved recovery proposal is the phase-transition approval. Inline implementation/check only; no agents. Use no production mutation for fixtures. Stop at any evidence mismatch or ambiguous public state; never remove/replace the draft or tag.
