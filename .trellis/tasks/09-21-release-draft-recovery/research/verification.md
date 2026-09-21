# Verification and publication evidence

## Local recovery implementation
- Six unittest groups passed, covering identity/evidence rejection, classification, asset integrity, fresh transient retry directories, retry/permanent-error bounds, and the exact publication PATCH boundary.
- actionlint and git diff --check passed.
- just ci-policy passed: existing release/operator/candidate fixtures, dependency and version policy, formatting, shell checks, Android packaging fixtures, new recovery fixtures, and in-memory Python syntax checks.
- Read-only live inspect successfully bound v0.1.35 / e44205beedf64cee1894a685cce4795b16d90e95 / original run 35559736119 / signed artifact 10621537362 / original draft 392689610.
- Before implementation, the 11 existing draft assets were downloaded successfully after the transient HTTP 500. Frozen-version Rust release verification passed; all local asset sizes and SHA-256 values matched the original draft metadata. Original snapshot: /tmp/zterm-v0.1.35-original-draft.json (local operational evidence only).

## Remaining hosted acceptance
- Required recovery PR CI and protected merge.
- Main-dispatched recovery using original identities, real artifact download, frozen verifier, provenance and immutable publication.
- Compare published asset IDs/digests to the original snapshot and confirm latest stable selection.
