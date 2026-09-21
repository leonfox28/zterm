# Verification and publication evidence

## Local recovery implementation
- Six unittest groups passed, covering identity/evidence rejection, classification, asset integrity, fresh transient retry directories, retry/permanent-error bounds, and the exact publication PATCH boundary.
- actionlint and git diff --check passed.
- just ci-policy passed: existing release/operator/candidate fixtures, dependency and version policy, formatting, shell checks, Android packaging fixtures, new recovery fixtures, and in-memory Python syntax checks.
- Read-only live inspect successfully bound v0.1.35 / e44205beedf64cee1894a685cce4795b16d90e95 / original run 35559736119 / signed artifact 10621537362 / original draft 392689610.
- Before implementation, the 11 existing draft assets were downloaded successfully after the transient HTTP 500. Frozen-version Rust release verification passed; all local asset sizes and SHA-256 values matched the original draft metadata. Original snapshot: /tmp/zterm-v0.1.35-original-draft.json (local operational evidence only).

## Completed hosted acceptance
- Recovery PR #53: https://github.com/leonfox28/zterm/pull/53. All required CI passed in run 35567666064; merged as 2300173b937ee262a80e09f57bf799166a9d4fd3.
- Main-dispatched recovery run 35568506797 succeeded: https://github.com/leonfox28/zterm/actions/runs/35568506797. It reused original run 35559736119, artifact 10621537362 and draft 392689610; all frozen verification, provenance and publication steps passed.
- Published https://github.com/leonfox28/zterm/releases/tag/v0.1.35 at 2026-09-21T06:27:16Z. GitHub reports immutable=true, draft=false, prerelease=false and latest stable v0.1.35. All 11 asset IDs, names, sizes and digests equal the original draft snapshot.

## Provenance verification

`gh attestation verify` passed for the original Android APK, restricted to repository leonfox28/zterm, signer workflow .github/workflows/release-recover.yml, source ref refs/heads/main and workflow source digest 2300173b937ee262a80e09f57bf799166a9d4fd3. Its authenticated SLSA provenance statement contains all 11 original asset SHA-256 subjects. The recovery workflow source identifies the recovery operation; the signed product manifest continues to identify original build source e44205beedf64cee1894a685cce4795b16d90e95. No assets were rebuilt, re-signed, replaced or re-uploaded.

The original tag workflow remains failed as the HTTP 500 incident record. The successful recovery run is the publication completion record. The earlier macOS PR test flake did not recur in successful PR retry, exact-source main CI or recovery PR CI; no application/test behavior was changed for that transient failure.
