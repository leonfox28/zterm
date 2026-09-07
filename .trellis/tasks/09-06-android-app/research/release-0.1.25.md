# Native host patch release 0.1.25

User authorized the new-version release flow after reviewing the host wide-cell
clipping fix. Scope: the three terminal projection/test/spec files only, then
the canonical two-file Cargo version commit. The Android application, shared
client extraction, Penpot configuration and other in-progress work remain in
the original holy-zebra workspace.

Release checkout: ../zterm-release-0.1.25, branch fix/wide-cell-resize, based on
freshly fetched origin/main de25a38. Release 0.1.24 is the preceding latest.
Use docs/releasing.md: just doctor, just check, product commit,
just release-prepare 0.1.25, reviewed exact-HEAD PR, just release 0.1.25 PR.
User publication authorization includes the required commits, PR push/merge,
tag and normal protected-environment approval. Do not bypass repository rules
or mutate protection/signing settings. No physical phone operations or restart
of the rejected wide_clip_fixture network executable. Native publication is
separate from disruptive activation of the user's live local daemon.

Local evidence is under target/android-toolchain/release-0.1.25-*.log in the
original workspace. Corrected-host Android end-to-end validation remains pending
and must not be claimed in release acceptance.


Preparation complete: `just doctor` and full `just check` passed on macOS arm64.
Product commit 01e0272; exact two-file version commit
9118b6a6a1a27cf0ef6fa2bb5ca919cd87a91948. PR:
https://github.com/leonfox28/zterm/pull/32. The canonical operator is running
`just release 0.1.25 32`, watching required PR CI before exact-head merge.


PR CI 34097073195 passed on all required jobs; PR 32 merged at
2026-09-07T07:52:12Z as 682fe463697a147d57b82b3d5396ebede31abf76.
Exact-main CI/candidate run: https://github.com/leonfox28/zterm/actions/runs/34097613110.


Exact-main CI and all three native candidates passed. Unsigned candidate:
artifact 10009377375, server digest
sha256:ad8278864ad612ac0d2119d7bdcfe11ce1a63e59c90f4ddec80227f8566a21b6.
Annotated v0.1.25 targets 682fe463697a147d57b82b3d5396ebede31abf76.
Formal run: https://github.com/leonfox28/zterm/actions/runs/34098182092.
After release-source validation passed, the normal release Environment approval
was submitted under the user's explicit publication authorization. GitHub
reported current_user_can_approve=true and prevent_self_review=false; no
protection settings were changed or bypassed. Deployment 6304433497 names the
same tag and exact source SHA. Signing/final installers/publication pending.


## Published

Formal run 34098182092 completed successfully. Signing, the three final signed
HTTPS installer proofs, late draft asset verification, attestation and immutable
publication all passed. GitHub Release 383918772:
https://github.com/leonfox28/zterm/releases/tag/v0.1.25
Published 2026-09-07T08:02:24Z; latest stable, draft=false, prerelease=false,
immutable=true. All eight expected native assets report uploaded. The normal
operator exited 0 and removed its detached publishing checkout. The separate
clean fix/wide-cell-resize checkout was also removed; its branch and commit
history remain. Original Android working files were not included in the release.

The optional slow local unsigned-candidate download was cancelled and its
partial file removed. It was not used as integrity evidence; exact-candidate
and final signed-asset verification were performed by the successful workflows.

Local Mac activation/restart and corrected-host Android end-to-end verification
remain separate pending steps. No physical phone operations or rejected network
fixture restarts were performed.

## Post-publication activation and emulator validation

The user subsequently updated the local host; installed `zterm --version` is
0.1.25. Against its normal daemon, final Android development 1012 on
emulator-5554 passes exact wide-cell clipping/native takeover (7.159 s), exact
clipping/App UI takeover (11.831 s), and real Herdr/native takeover (9.665 s).
The user's actual Herdr/Pi main also passes a manual App UI takeover, immediately
resizing from 140x39 to 56x53 and remaining connected without the original error
for over ten minutes. Touch scrolling responds. Host activation and acceptance
of the released wide-cell fix are now complete.

Two automated Herdr UI fixture runs fail before Session creation with
`unauthorized`; their cause remains separate and unclassified. Standard docked
keyboard resize is also unverified in this run due to emulator floating Gboard
mode. See `research/takeover-connection-exit.md` for complete evidence and limits.
No release assets, product source, APKs, physical devices or firewall settings
were changed during this post-update validation.
