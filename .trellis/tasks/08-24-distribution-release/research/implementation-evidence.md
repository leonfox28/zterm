# M9 implementation evidence (local / external checkpoint pending)

## Step 0 snapshot

- Implementation start/current base HEAD:
  `1ef46e9c3f04e31d861675e3ce96d651e371cf88` on `main`.
- The only initial working-tree change was this task's `task.json` activation;
  prior M7–M8 implementation was handed off and no remote Session/transport
  behavior is intentionally changed by M9.
- Latest completed `ci.yml` at the snapshot: run `32725142928`, success, source
  `d3cfc5697c4b6a5dcd10f3bf70689e29b3c797f8`, created
  `2026-08-24T12:04:35Z`.
- Existing Releases are stable `v0.1.0` and `v0.1.1`; neither is immutable and
  neither is treated as a native zterm client release.
- Repository Actions are enabled with `allowed_actions=all` and
  `sha_pinning_required=false`. M9's release workflow pins its own third-party
  actions despite the broader repository setting.
- GitHub has no Environment. Immutable Releases report
  `enabled=false,enforced_by_owner=false`.
- Product version is fixed at `0.1.2`; four targets, glibc 2.28/macOS 13 floors,
  64 KiB manifest and 128 MiB compressed-artifact limits are executable core
  constants.
- `release/public-key.hex` is the explicit `UNCONFIGURED` placeholder. Signing,
  installer verification, and update therefore fail closed until the external
  checkpoint; no seed/public key was generated during implementation.

## Implemented local contract

- Shared exact-byte Ed25519 manifest/build identity and typed content-free
  release errors.
- Deterministic single-file archive plus unsigned prepare, protected signing,
  exact-inventory verification, checksums, generated installer, and SPDX
  creation in the non-product release tool.
- Manual/draft-only four-target workflow with pinned Action commits and Debian
  10/glibc-2.28 image digest; signing and draft creation use the protected
  `release` Environment and no self-hosted runner. All downstream jobs use the
  validate job's frozen commit, draft creation rechecks the tag, and the
  signing tool is built before the seed-bearing step.
- Mutable bootstrap, generated versioned installer, and hosted local-HTTPS
  acceptance fixture. Installer activation is owned by the authenticated
  candidate and is atomic no-clobber.
- Native Rust HTTPS updater with fixed HTTPS-only origin/bounds, official-build
  proof before network access, candidate preparation before daemon contact,
  Session force boundary, stop/activation/post-check/rollback, and no daemon
  restart.
- Uninstall first rejects development, ordinary-CI, and `UNCONFIGURED` builds,
  then reuses identity-reset inventory and removes the exact validated
  executable last; setup/update write diagnostic `install.json` with mode
  `0600` only when managed state exists.
- User documentation and the executable backend code-spec are updated.

## Local verification completed

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test -p zterm-core release::tests                         # 7 passed
cargo test -p zterm-platform executable --lib                  # 3 passed
cargo test -p zterm-daemon distribution::tests --lib           # 3 passed
cargo test -p zterm-daemon distribution_lifecycle_requires_force_for_active_sessions --lib  # 1 passed
cargo test -p zterm-daemon update_rejects_every_incompatible_running_daemon_identity_field --lib  # 1 passed
cargo test -p zterm-daemon development_binary_cannot_update_or_uninstall_owned_executable_or_state --lib  # 1 passed
cargo test -p zterm-cli --test command_side_effects             # 1 passed
cargo test -p zterm-release-tool                                # 2 passed
cargo doc --workspace --no-deps
cargo deny check                                                # warnings only; policy passed
sh tests/release/static.sh
sh tests/relay/publication-channels.sh
sh tests/source-policy.sh
sh tests/workspace-version.sh
sh tests/secret-scan.sh
python3 .trellis/scripts/task.py validate .trellis/tasks/08-24-distribution-release
git diff --check
```

The pinned Debian image was also inspected locally as a multi-architecture
manifest list containing both `linux/amd64` and `linux/arm64/v8`; no release
endpoint or product network test was exercised.

Positive pre-setup/configured uninstall and reinstall identity-rotation
evidence correctly remains with the signed hosted candidate. Local tests prove
the destructive primitives and, separately, that a repository/development
binary cannot reach them merely because its path, owner, and mode look safe.

The developer host has no `shellcheck`; this is not silently skipped in the
release workflow or CI. The hosted installer jobs own `shellcheck`. A full
`cargo test --workspace` was intentionally not run on this macOS host because
it includes real Iroh/Endpoint targets whose execution is hosted-Linux-owned.

## External checkpoint / evidence still required

Before any workflow run, tag, signing, draft, or formal installation:

1. a repository administrator enables immutable Releases and records the
   authenticated `gh api repos/leonfox28/zterm/immutable-releases` result;
2. create the protected `release` Environment and required reviewer;
3. generate the Ed25519 seed outside logs/artifacts, commit only its reviewed
   public key, and add the seed as `ZTERM_RELEASE_SIGNING_KEY` in that
   Environment;
4. independent checker passes the implementation;
5. create the exact reviewed `v0.1.2` tag, run the manual workflow with
   `enabled-and-reviewed`, and retain the run ID, four digests, signed manifest
   key ID, installer matrix, attestation, and verified draft round-trip.

The workflow deliberately leaves the Release as a draft. This implementation
created no Environment, secret, key, tag, Actions run, Release, attestation, or
published external state.

## Independent checker fixes and final local gate (2026-08-25)

The independent checker verified and fixed six concrete boundary defects:

1. a repository `target/debug/zterm` satisfied the old owner/mode checks and
   could delete or replace itself; destructive lifecycle now additionally
   requires a configured-key official build with explicit canonical
   `ZTERM_SOURCE_COMMIT`, before update network access or uninstall state
   observation;
2. downstream release jobs followed a still-mutable tag, and the secret-bearing
   step invoked Cargo; every later checkout now uses the validate job's frozen
   commit, draft creation rechecks the tag, and only the prebuilt reviewed tool
   receives the seed;
3. executable activation temporarily removed the target and could lose its
   rollback owner on first directory-sync failure; it now retains a no-clobber
   hard-link backup, performs one atomic replacement, and restores on that
   failure;
4. the versioned installer could chmod an extracted symlink before Rust path
   validation; it now rejects a non-direct regular candidate first;
5. the mutable bootstrap accepted noncanonical `v`-prefixed strings; it now
   enforces canonical SemVer, including leading-zero prerelease rules;
6. shell downloads were size-checked only after completion; both bootstrap
   stages now apply curl byte caps and an OS file-size limit, with exact
   post-download checks retained.

The final local checker gate passed formatting, workspace/all-target/all-feature
Clippy with `-D warnings`, focused release/platform/distribution/CLI tests,
workspace documentation, dependency policy (existing duplicate-version
warnings only), release/Relay/source/version/secret static checks, Trellis
context validation, and `git diff --check`. It intentionally did not execute
real Iroh/Endpoint/UDP/DNS tests or mutate GitHub settings, keys, tags,
environments, workflows, or Releases. Signed four-target hosted evidence
remains the explicit external checkpoint above.
