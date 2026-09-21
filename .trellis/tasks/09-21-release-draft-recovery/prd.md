# Recover verified release drafts

## Goal
Resume a draft left by a transient GitHub asset download failure and publish v0.1.35 with its original signed assets and GitHub provenance.

## Background and diagnosis
Release run 35559736119 successfully validated source e44205beedf64cee1894a685cce4795b16d90e95, signed artifact 10621537362, and passed all three installer jobs. Its publish job uploaded 11 assets into draft 392689610, then GitHub returned HTTP 500 during round-trip download. Later local download, signature verification, and all server asset digest checks passed. The normal workflow only creates a new draft and has no continuation owner after draft creation. This is a missing release-recovery boundary, not an artifact defect. Draft lookup by tag returned 404 while the draft was visible by ID/list; recovery must use the explicit draft ID.

## Requirements and acceptance
- R1: Manual recovery accepts an exact tag, original release run ID, signed artifact ID and existing draft ID; it requires normal protected-main review before execution.
- R2: Require the original annotated tag/source, successful exact main CI, original successful source/signing/three installer jobs, retained digest-bound signed artifact and unpublished matching draft.
- R3: Reuse the original source verifier. Download existing draft assets with bounded transient retries, require exact inventory/byte equality to verified signed artifacts, and preserve asset IDs.
- R4: Generate GitHub build provenance before publishing the same draft; require immutable=true and verify the original asset inventory after publication.
- R5: Reject mismatched identities, expired/missing evidence, published releases, changed assets, incomplete/corrupt inventories, and permanent download errors without publishing.
- R6: Complete v0.1.35, verify latest stable status and unchanged asset IDs/digests, and record evidence.

## Scope
Recovery workflow, helper and regression tests; portable CI integration; release runbook and distribution contract. No application changes, product rebuild, signing keys, tag moves, replacement uploads, draft deletion, or protection changes.

## Approval
The user approved the full recovery proposal in research/approved-recovery.md after reviewing it, explicitly authorizing a Trellis task, recovery implementation and continued v0.1.35 publication. No blocking product or scope decisions remain.
