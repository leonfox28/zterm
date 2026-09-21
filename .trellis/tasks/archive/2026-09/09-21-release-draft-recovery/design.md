# Design

Add a main-only workflow_dispatch workflow sharing the normal release tag concurrency group. Its four explicit inputs identify the original release; the existing tag workflow remains the normal publication owner.

A standard-library Python helper owns GitHub evidence validation, bounded draft downloading, stable asset identity comparison, and publishing by release ID. It has inspect/download/publish subcommands and emits one local execution plan consumed only within the same job. GitHub remains the recovery record; this plan is not a persistent release database.

Inspect authenticates GitHub API identities and successful source/signing/installer job evidence, including the annotated tag and signed artifact digest. Checkout the frozen source separately from the reviewed recovery helper; use its existing find-green-main-ci.sh and Rust release verifier. Pinned download-artifact verifies the signed artifact digest. Download draft assets into a fresh temporary directory per attempt (three attempts, transient errors only), compare all names and bytes, then run the frozen verifier over the round-trip inventory. The standard pinned provenance action attests those bytes. Immediately recheck tag/draft/asset identities and bytes, then PATCH only draft=false for the existing release ID and require immutable publication.

The workflow contains no release secrets, signing, product builds, tag writes, release creation/deletion, or asset writes. Existing GitHub provenance and signature mechanisms are retained. A failure before publish leaves the draft untouched; a failure after PATCH reports the published state for inspection rather than republishing or replacing anything.

Files: .github/workflows/release-recover.yml, tools/release/recover-draft.py, tests/release/recover_draft_test.py, justfile, tests/release/static.sh, docs/releasing.md, .trellis/spec/backend/distribution-lifecycle.md, and task/journal artifacts. No refactor of existing successful publication paths is necessary.
