# v0.1.35 draft recovery proposal

## Current verified state
- Release PR #52 merged as e44205beedf64cee1894a685cce4795b16d90e95.
- Exact main CI 35559047085 succeeded; unsigned candidate artifact 10620614788.
- Annotated tag v0.1.35 identifies this source.
- Release run 35559736119 passed source validation, protected signing, and all three native installer checks.
- Signed inventory artifact 10621537362 (signed-release-v0.1.35-1).
- Draft release 392689610 holds all 11 uploaded assets and remains unpublished.
- Publish failed on HTTP 500 while downloading asset 578193893 for the draft round-trip.
- A later local download of all 11 draft assets succeeded.

## Proposed bounded change
Add a manually dispatched draft recovery workflow and corresponding validation helper/tests, runbook, and distribution-spec contract. Route the change through a protected PR with normal CI before dispatch.

The recovery job must bind the exact annotated tag, successful main source CI, original signing/install run, signed artifact ID and existing draft release ID. It must reject a published/immutable release, source mismatch, missing or extra assets, digest/signature mismatch, or missing original signing/installer evidence.

Download the retained signed artifact with digest validation; download the existing draft assets with bounded retries; verify both with the frozen source release verifier and compare every byte. Generate GitHub build provenance for these verified existing assets, then publish that same draft and require immutable=true.

No APK/native rebuild, new signing, new tag, draft deletion, replacement upload, asset overwrite, or protection changes are part of recovery. The original failed run remains as audit evidence; a new recovery run records completion.

## Validation
- Regression fixtures for wrong tag/source/run/artifact/draft identity, published release, corrupt/incomplete inventory, and transient download failure.
- actionlint, ShellCheck/Python syntax as applicable, and repository portable checks.
- Required PR CI before merge.
- Dispatch recovery for exactly v0.1.35 / source e44205beedf64cee1894a685cce4795b16d90e95 / release run 35559736119 / signed artifact 10621537362 / draft 392689610.
- Confirm published immutable Release has the original asset IDs and digests, latest stable tag v0.1.35, and a successful provenance/recovery run.
