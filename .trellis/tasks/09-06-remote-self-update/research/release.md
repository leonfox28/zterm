# v0.1.24 release evidence

The user explicitly requested the release workflow on 2026-09-06 after reviewing
the concrete work commit plan. The repository operator completed successfully.

- Work commit: `ca7eb7e` (`fix(update): complete upgrades after the originating session ends`).
- Version commit: `4048885` (`chore: prepare v0.1.24 release`).
- Reviewed [PR #30](https://github.com/leonfox28/zterm/pull/30) merged at
  `1ded7f93a356f97346512a58812960594e6e1902` with the exact-head merge guard.
- [PR CI](https://github.com/leonfox28/zterm/actions/runs/34035113162): success.
  All 14 updater tests, including real originating PTY termination, passed on
  macOS arm64, Linux arm64 and Linux x64. Local `just check` also passed.
- [Exact main CI](https://github.com/leonfox28/zterm/actions/runs/34035436342):
  success, including all three native builds and candidate assembly.
- Candidate artifact `9990103403`,
  `release-candidate-1ded7f93a356f97346512a58812960594e6e1902-1`, was retained
  and had server digest
  `sha256:67a6e68ebc5f9bd25940c824a851af8cd885a4ed114140c5a829f5b2917cbf22`.
- Annotated tag `v0.1.24` identifies the exact main merge SHA.
- [Signed release workflow](https://github.com/leonfox28/zterm/actions/runs/34035804008):
  success. The authorized maintainer account approved the protected `release`
  Environment after source/candidate checks; no protection setting was changed.
  Signature verification, all three HTTPS installer proofs, asset round-trip
  verification, provenance attestation and publication passed.
- [Published Release](https://github.com/leonfox28/zterm/releases/tag/v0.1.24):
  `draft=false`, `immutable=true`, published `2026-09-06T13:23:10Z`. All eight
  expected assets are uploaded and expose SHA-256 digests.
- `just release 0.1.24 30` exited 0 and removed its private release worktree.

## Remaining real-server acceptance

No user server or installed zterm was changed by this release operation. First
install v0.1.24 via an independent terminal such as SSH, then use a newer signed
compatible release to validate update from a real remote zterm connection.
Confirm Session shutdown, subsequent daemon readiness, manual reconnect, target
version, preserved identity/pairing and final log outcome. Keep this task active
until that evidence exists; publication and isolated process tests do not claim
it has already occurred.
