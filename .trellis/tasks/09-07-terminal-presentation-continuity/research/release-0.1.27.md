# v0.1.27 release — 2026-09-08

User instruction after explicit disclosure of the visual evidence gap:
“先走发布流程吧”. This authorizes the necessary work commits, version preparation,
PR push/review/merge, exact-main tag, normal protected signing approval and formal
publication. Preserve repository protections. Do not update/restart the existing
Mac daemon or touch its `main` Session.

Scope: coherent DEC 2026 host publication and bounded history progress; healthy
resize input/IME lifetime; independent coordinate generation; Android integral
layout and direct animation events; desktop final-cell diff/full blank coverage.
Unchanged wire format and current native/Android publication target set.

Known issue: Android keyboard-close can still expose a temporary blank top region
before history refill. Endpoint pixel equality is verified, but full-transition
flicker improvement is not accepted. Real Herdr and desktop GUI visual acceptance
were not completed. Publish the current improvements without claiming zero flicker,
and keep the task active for this follow-up.

Baseline/upstream main: be66a16. Prior latest: v0.1.26. Next: v0.1.27, Android
versionCode 102799. `just doctor`, full `just check`, final Android build/lint,
focused native/desktop checks and emulator acceptance passed as detailed in
`runtime-acceptance.md`. No product source changed after those final owner checks.

## Published evidence

- Product commits: `ed7c170` (host publication) and `a0065d9` (client continuity).
- Version-only commit: `807fd41b691afea3e8d5d476c5c4ce6c0ffad7eb`; exactly
  Cargo.toml/Cargo.lock changed, with no external dependency upgrades.
- [PR #35](https://github.com/leonfox28/zterm/pull/35) merged through normal
  protections. Merge source: `cb19c754641d6c0a715ace9d02354b88c57abb2c`;
  its tree exactly equals the reviewed PR head.
- [PR CI](https://github.com/leonfox28/zterm/actions/runs/34173146157) and
  [exact-main CI](https://github.com/leonfox28/zterm/actions/runs/34173688282)
  passed every applicable job, including the required CI gate.
- Exact-main candidate artifact `10036732492`, digest
  `sha256:bf0aa81bb6acc636ce65821bc67e31318d4745e44ffba73b4e798f8b9336af3c`.
- Annotated tag `v0.1.27` identifies the merge source above. Normal protected
  `release` environment approval followed successful source validation and
  confirmed reviewer eligibility; no protection changes or bypass were used.
- [Formal release run](https://github.com/leonfox28/zterm/actions/runs/34174373737)
  passed source validation, retained-APK signing, manifest/checksum signatures,
  final signed HTTPS installer proofs on all three native targets, draft asset
  download verification, attestation and immutable publication.
- [v0.1.27](https://github.com/leonfox28/zterm/releases/tag/v0.1.27) published
  at `2026-09-08T00:48:29Z` (08:48:29 Asia/Shanghai), Release ID `384380094`.
  Verified `draft=false`, `prerelease=false`, `immutable=true`, and exactly 11
  uploaded assets: three native archives, APK, Android metadata, native manifest
  and signature, SHA256SUMS and signature, installer and SBOM.
- Downloaded metadata hashes match GitHub asset digests. Native/Android metadata
  identify the exact source; APK metadata confirms `102799`, arm64-v8a, API 26+,
  the formal package, and the pinned existing certificate. Formal workflow owns
  full artifact/signature verification; local inspection did not install them.
- Release notes preserve generated changelog entries and explicitly document
  the remaining keyboard-close blank/history refill and visual evidence gaps.

`just release 0.1.27 35` exited successfully and removed its own temporary
detached worktree, preserving the caller's feature branch. No existing remote
artifact was replaced or tag force-moved. No local daemon activation occurred.
The task remains in progress for visual continuity follow-up.
