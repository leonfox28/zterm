# v0.1.28 release — 2026-09-08

The user accepted the updated development APK on their physical phone, then explicitly
requested all commits and the release workflow. All task-owned changes are committed;
no unrecognized dirty files were present. The work commit is `022ffb7` and the
version-only commit is `4eef9a4` (Cargo.toml/Cargo.lock only, no dependency upgrades).

- [PR #36](https://github.com/leonfox28/zterm/pull/36), reviewed source
  `4eef9a45e07d5446659d543adbee11c3e40ba955`, merged normally as
  `9c5ecad4d22efc9a2092da25957900c5726d977d`. Merge and PR trees are identical.
- [PR CI](https://github.com/leonfox28/zterm/actions/runs/34192932538) and
  [exact-main CI](https://github.com/leonfox28/zterm/actions/runs/34193657598)
  passed every applicable job and the required CI gate.
- Exact-main candidate `10043350143`, digest
  `sha256:a001447f0bba8fad07c505ac86208338e8b9779ba6ac194b888289e6bce5936c`.
- Annotated `v0.1.28` identifies that exact merge. Immutable-release setting was
  confirmed enabled before normal protected-environment approval. Reviewer eligibility,
  source validation and the successful exact candidate were checked; no protection bypass.
- [Formal release run](https://github.com/leonfox28/zterm/actions/runs/34194484710)
  passed APK/manifest/checksum signing, final signed HTTPS installation on all three
  native targets, uploaded asset round-trip verification, attestation and publication.
- [Published v0.1.28](https://github.com/leonfox28/zterm/releases/tag/v0.1.28),
  Release ID `384488930`, published `2026-09-08T06:26:55Z` (14:26:55 Asia/Shanghai).
  Verified non-draft, stable, immutable, exactly 11 expected assets.
- Locally downloaded native/Android metadata and SHA256SUMS match GitHub digests.
  Metadata identifies the exact source/version; Android is 102899, formal package,
  arm64-v8a, API 26+, signed with the pinned existing certificate. APK digest:
  `64f82c8a6825d6b203a2213849c0058475c0da699fc5ddb212ae0cb3f25c56c9`.
- Release notes describe local scrolling, row-render reuse and physical-phone acceptance,
  preserve the generated changelog, and explicitly retain the keyboard-close/history-refill
  and desktop visual follow-ups. Emulator drawing gains are not a phone FPS claim.

`just doctor` passed; preexisting full local check, Android build/lint/JVM, pixel,
native integration and real IME/Copy/Herdr evidence remain in scrolling acceptance.
No product code changed after those final checks. The release operator exited successfully
and removed its owned temporary worktree. No real Mac daemon/main Session operation
or additional phone installation occurred.

Keep this parent task active for the previously recorded visual follow-ups, as required
by its accepted release scope. Record this completed release session without falsely
archiving the unfinished parent task. Raw operator logs and downloaded metadata are
under `/tmp/zterm-release-0.1.28-*`.
