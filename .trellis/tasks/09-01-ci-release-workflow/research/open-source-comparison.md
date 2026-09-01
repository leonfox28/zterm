# Open-source workflow comparison update

> Planning snapshot captured before the 2026-09-01 implementation. The zterm
> flow described below as proposed/target-state is now implemented; preserve
> the comparison as research and use `docs/development.md` and
> `docs/releasing.md` for current repository operation.

- Date: 2026-09-01
- Primary comparison: `herdrdev/herdr`
- Existing detailed baseline: parent task
  `08-24-distribution-release/research/release-workflow-comparison.md`

## Herdr current snapshot

Current inspected revision:
[`8a6d697308162874a236c84aac0b0f8c7ac01f83`](https://github.com/herdrdev/herdr/tree/8a6d697308162874a236c84aac0b0f8c7ac01f83).

### Practices worth borrowing

- The repository gives developers one vocabulary: `just lint`, `just test`,
  `just ci`, and `just check`. CI invokes `just ci`, so the real commands live
  outside YAML and are runnable locally
  ([justfile](https://github.com/herdrdev/herdr/blob/8a6d697308162874a236c84aac0b0f8c7ac01f83/justfile)).
- `just check` composes portable checks, maintenance tests, and a local Windows
  target lint. It catches a useful subset of hosted Windows failures before
  push without claiming to replace a Windows runner.
- CI runs on PRs and pushes only to named integration branches, uses the PR
  number/ref in concurrency, adds explicit job timeouts, restores Rust/build
  caches, and uses `cargo nextest` failure-focused output
  ([ci.yml](https://github.com/herdrdev/herdr/blob/8a6d697308162874a236c84aac0b0f8c7ac01f83/.github/workflows/ci.yml)).
- Maintainer guidance defaults substantive work to a task branch and pull
  request, watches checks to completion, and leaves final merge to the human
  ([AGENTS.md](https://github.com/herdrdev/herdr/blob/8a6d697308162874a236c84aac0b0f8c7ac01f83/AGENTS.md)).
- Stable release has one visible command, `just release 0.x.y`, while the
  implementation is deliberately split into `release-prepare` and
  `release-publish`. Prepare validates version/cleanliness, runs pre-release and
  normal checks, updates version/lock/changelog and creates a reviewable commit.
  Publish revalidates clean branch/version/upstream ancestry, pushes the release
  commit if needed, creates an annotated tag and pushes it. This is simple for
  the operator without hiding the review boundary.

### Practices zterm should not copy blindly

- Herdr's publish recipe pushes the release commit and immediately pushes the
  tag; it does not wait for or query a successful exact-SHA `master` CI run.
  zterm's signed update channel already requires that evidence.
- Herdr's stable workflow freshly builds five targets and then publishes with
  `softprops/action-gh-release`, but has no zterm-equivalent detached signed
  manifest, protected signing Environment, four-target authenticated installer
  matrix, draft round trip, or explicit immutable-response assertion
  ([release.yml](https://github.com/herdrdev/herdr/blob/8a6d697308162874a236c84aac0b0f8c7ac01f83/.github/workflows/release.yml)).
- Herdr's shorter YAML reflects a smaller release trust contract, not merely
  better syntax. zterm can copy the entry-point ergonomics and cache/command
  ownership while retaining its stronger publication state machine.

## Other relevant projects

The parent research already pins and compares:

- [ripgrep](https://github.com/BurntSushi/ripgrep/tree/3fce3b5bb0236da2df6d99672afb8a719642eca7):
  broad PR/main debug CI, fresh release-LTO tag builds, checksums/attestations,
  early draft left for later human publication.
- [uv](https://github.com/astral-sh/uv): conditional planning for expensive
  jobs, reusable release builders, an automated release-preparation PR and a
  separately approved/manual release workflow. It rebuilds rather than
  promoting ordinary CI binaries.
- [just](https://github.com/casey/just/tree/b20386abdbae867a49cdff6c3c0f2b547faa9b23):
  one `just ci` local command and a simple local publish recipe, but direct
  per-target publication can expose partial public releases and is unsuitable
  for zterm's exact signed inventory.

Shared conclusion: all inspected projects give maintainers a small local
command surface and rebuild formal release artifacts. None provides evidence
that zterm should promote ordinary CI binaries or discard its signed manifest.

## Proposed synthesis for planning

1. Borrow Herdr's `just check` / `just release` operator surface.
2. Keep zterm's exact-SHA green-main gate and fresh formal rebuild.
3. Split release preparation from irreversible tag publication internally,
   even if a top-level command presents them coherently.
4. Borrow explicit timeouts, Rust caching and failure-focused test output only
   after confirming they preserve current test/doc coverage.
5. Use the approved PR/main separation to make CI preventative; direct-main is
   no longer the normal development or release-commit path.

## Target-state zterm versus Herdr

The proposed zterm flow deliberately converges with Herdr at the maintainer
interface while retaining different release trust and distribution contracts.

### Operator flow

Herdr currently exposes one composed `just release <version>` command. It runs
local pre-release checks, creates a release commit, pushes that commit directly
to `master` when necessary, creates an annotated tag and pushes it. The tag
workflow runs Nix and release-document checks, builds five binaries, publishes
the public GitHub Release directly, then separately closes referenced issues
and commits versioned docs plus `distribution/latest.json` back to `master`.

The approved zterm flow does not copy that exact one-process sequence because
the protected-main/required-CI contract also applies to version bumps. Its
state machine is `release-prepare` (create a reviewable version PR), human merge
and exact-main CI, then `release-publish` (validate the remote SHA and push the
canonical tag). After the tag, zterm performs no ordinary test suite; it builds
formal assets, signs and exercises the final inventory, then publishes native
and relay outputs explicitly.

### Material differences after optimization

| Boundary | Herdr | Proposed zterm |
| --- | --- | --- |
| Shared command owner | `just check` / `just ci` | Same pattern; CI and local docs call one repository command |
| Integration | PRs by default, but small changes and release commit may push `master` directly | Substantive work and the release commit use required PR CI; `main` has no normal direct-push path |
| Exact green source | Release tag checks Cargo version but does not query a successful exact-SHA `master` CI run | Tag is rejected unless its exact commit has a successful `main` push CI run |
| CI platform scope | Core Ubuntu/macOS/Windows jobs plus an extra Windows ConPTY package contract | Linux/macOS runtime evidence, Windows shared-boundary evidence, relay bundle and four release-readiness targets |
| Tag-time general checks | Runs Nix flake checks and documentation/release-input tests after the tag | Ordinary lint/tests/docs/dependency checks stay before the tag; tag time has release-artifact checks only |
| Formal targets | Five, including Windows; Linux is musl and cross-built | Four Unix targets; Linux is native glibc 2.28 and Windows is not a shipped asset |
| Release publication | Direct `softprops/action-gh-release` publication | Late draft, exact upload/download round trip, then immutable publication |
| Update trust | Mutable `distribution/latest.json` committed after release | Detached Ed25519-signed exact manifest consumed by installer/updater |
| Secret approval | No protected signing approval in the stable workflow | Required reviewer gates the single release-key-bearing signing job |
| Final install proof | Packaging-specific checks, especially Windows ConPTY | Signed installer exercises every shipped native target over local HTTPS |
| Integrity metadata | No zterm-equivalent detached exact inventory | Checksums, SBOM, signature and GitHub provenance attestation |
| Companion service | No release-coupled service image | Same version explicitly publishes the multi-architecture relay image |
| Post-release automation | Closes issues, snapshots docs, updates latest manifest | Not planned for the CI/release MVP; focus stays on binary/relay integrity and clear recovery |
| Channels | Stable plus scheduled/manual preview channel | Stable/prerelease tag classification; no separate scheduled preview workflow in scope |

The practical conclusion is that zterm should feel Herdr-like before the tag
but will remain intentionally stricter after the tag. The additional zterm
stages are each owned by the signed updater, multi-asset atomicity, protected
key, or relay distribution contract; they are not copies of general CI.
