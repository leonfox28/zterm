# M9 implementation evidence

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
- The initial candidate version was fixed at `0.1.2`; four targets, glibc 2.28/macOS 13 floors,
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
- Comprehensive `main` CI adds native release-mode builds for both macOS
  architectures and both digest-pinned Debian 10/glibc-2.28 Linux
  architectures while retaining Windows shared-boundary validation. The
  separate exact-tag workflow requires a successful `ci.yml` `push` run on
  `main` for the same SHA, rebuilds all four assets, and uses the protected
  `release` Environment only for signing. It then tests, round-trips, attests,
  publishes, and verifies the immutable formal Release without a second
  Environment approval.
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
release workflow or CI. The Ubuntu release-policy job checks maintained shell
sources, while the Ubuntu assembly job checks the generated formal installer
exactly once before protected signing. Every macOS/Linux installer job
independently retains POSIX syntax and the real platform fixture. A full
`cargo test --workspace` was intentionally not run on this macOS host because it
includes real Iroh/Endpoint targets whose execution is hosted-Linux-owned.

## External checkpoint / remaining evidence

Completed on 2026-08-25 before any workflow run, tag, signing, draft, or formal
installation:

1. Immutable Releases now returns
   `enabled=true,enforced_by_owner=false` from the authenticated repository API.
2. The protected `release` Environment requires reviewer `leonfox28`; it has
   no branch-policy override and permits the sole reviewer to approve a run
   they triggered.
3. `ZTERM_RELEASE_SIGNING_KEY` exists only as an Environment secret. The
   checkpoint operator recorded that the seed was generated without xtrace,
   argv, environment, repository file, terminal output, or persistent local
   file; the shell owner was unset immediately after `gh secret set` consumed
   stdin.
4. The checkpoint operator recorded `release/public-key.hex` as the matching
   Ring-derived public key for key ID `zterm-release-ed25519-v1`. The initial
   bootstrap uses the bounded, zeroizing `zterm-release-tool derive-public-key`
   command and an RFC 8032 fixed-vector test rather than a second OpenSSL/DER
   derivation path.
5. The independent implementation checker and hosted `ci.yml` run
   `32752412818` passed before the public-key checkpoint.

GitHub exposes only the secret's metadata, never its value. Independent
repository review can therefore confirm the secret's existence, the exact
public-key source shape, and the fail-closed signing comparison, but cannot
rederive the stored secret without violating the boundary. The protected signed
Release run remains the executable proof that the stored seed matches the
reviewed public key.

The comprehensive `ci.yml` run `32801933744` passed at `afd7f2f`, including all
four release-mode jobs. The immutable historical `v0.1.2` tag then triggered
release run `32802895706`: all four formal builds, unsigned inventory checks,
and protected signing passed, but every installer matrix job stopped at hosted
ShellCheck `SC2015` before executing the fixture. No draft Release, attestation,
or published client artifact was created. The failed tag is not moved, deleted,
or reused; the explicit-POSIX-`if` generator fix advances the next candidate to
`v0.1.3`.

The next comprehensive `ci.yml` run `32805701943` passed at exact main commit
`3f67d2477078a54b4adf8678b976839ab4979ec1`. Its immutable historical
`v0.1.3` tag triggered release run `32806708978`: exact-source validation, all
four formal builds, unsigned assembly, and protected signing passed. Both Linux
installer jobs then passed, proving the generated shell fix, while both macOS
installer jobs stopped before the fixture at `command -v shellcheck` because
the hosted macOS images do not provide that Ubuntu-preinstalled tool. The
publish job was skipped, so no draft, attestation, or `v0.1.3` Release exists.
The tag is not moved, deleted, or reused. The workflow now owns one fail-closed
ShellCheck gate on its Ubuntu assembly runner before protected signing and
keeps POSIX syntax plus the real fixture on every platform; the next candidate
advances to `v0.1.4` and still requires a new green exact-main CI before a
human creates that tag.

The focused `v0.1.4` repair gate passed the release-tool's three unit tests,
the seven shared release-contract tests, release-tool Clippy with `-D warnings`,
formatting, workflow/static policy, POSIX syntax, workspace-version, Relay
publication, source-checkout, secret-scan, task-context, YAML-parse, and diff
checks. ShellCheck is deliberately not claimed locally because the developer
host does not provide it; the exact-main Ubuntu policy job and tag-time Ubuntu
assembly job remain its executable owners.

The first `v0.1.4` exact-main candidate, commit
`389fcf5573820427d72a3f834dfbb9e172c5d07d`, ran CI as `32818194923`.
Eleven of twelve jobs passed, including every release-mode target, Windows,
Relay, dependency, and release-policy owner. Ubuntu x86_64 alone exposed a
test-only cleanup race in the socket-free `session_wire` checkpoint fixture:
the observation-only final reconnect repeated an unacknowledged explicit
detach after the synchronized stream had already proved that contract. Its
clean EOF replacement preserves the authoritative full-snapshot assertion and
removes the duplicate race; the exact test then passed 100/100 repetitions and
independent review. No `v0.1.4` tag was created from this failed CI commit; a
new green exact-main push remains required.

The replacement exact-main CI run `32821921982` passed all twelve jobs at
commit `62f5a3152581679028558227fa01a5100894632b`. Its immutable historical
`v0.1.4` tag triggered formal run `32830702052`: exact-source validation, all
four builds, unsigned assembly, protected signing, and both Linux installer
jobs passed. Both macOS installer jobs timed out waiting ten seconds for the
fixture port file because Python's `HTTPServer.server_bind` performs
`socket.getfqdn` after binding and before listening; on the affected GitHub
macOS runner images, local-network privacy can stall that lookup for about 35
seconds ([actions/runner-images#14409](https://github.com/actions/runner-images/issues/14409)).
The late publish job was skipped, so no draft, attestation, or `v0.1.4` Release
was created. The tag is not moved, deleted, or reused. The fixture now overrides
only that test-server bind path, delegates directly to `TCPServer.server_bind`,
and assigns the already-bound address without DNS. A socket-free source-policy
regression rejects restoring `getfqdn` or the inherited HTTP-server bind path;
the next lockstep candidate advances to `v0.1.5` and requires a new exact-main
CI success before a human-created tag.

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
remains the exact-main-CI then human-tag checkpoint above.

## Published v0.1.5 evidence and formal-install finding (2026-08-26)

- Exact-main CI run `32845499734` passed all 12 jobs at commit
  `4f7ed091038b5965bf77a32e98451373eb242a1f`.
- The immutable `v0.1.5` tag triggered formal release run `32846644770`; all
  12 jobs passed, including four native builds, protected signing, all four
  hosted HTTPS installer jobs, signed round-trip verification, publication,
  and attestations. Release API record `376385174` reports `draft=false`,
  `prerelease=false`, and `immutable=true`.
- The published manifest uses key ID `zterm-release-ed25519-v1`. Its four
  archive SHA-256 values, confirmed against the public Release API assets, are:
  `e70fa7ab6be7ea934191239c91bcf7102ab80f9b549ee9c2a9da7f39ba0fd7eb`
  (`aarch64-apple-darwin`),
  `c99803f71c403665675bfa5edc440a5218d3a9fdcdc9aa4ddf6b1b55be07e75e`
  (`x86_64-apple-darwin`),
  `456e55b2f87451624d4c873ebb9f4accd7bef8f4a47a447908b78874a00c7d91`
  (`aarch64-unknown-linux-gnu`), and
  `6d27eca4c38197eba5ef3329da3f659fd858845bcd6701a948d9febd1d4e8c70`
  (`x86_64-unknown-linux-gnu`). No signing secret was read or recorded.
- The user's first formal `v0.1.5` installation succeeded. `zterm setup
  --name my-mac` committed configuration and identity, then the detached daemon
  exited with status 101 before readiness. Post-failure observation was
  `configured_stopped`, zero Sessions, and a network Endpoint that had never
  bound.
- The daemon log identified one exact panic at
  `crates/daemon/src/network.rs:533`: `tokio::spawn(self.run())` ran with
  `there is no reactor running`. `run_owned_daemon_listener` had built its
  current-thread runtime but called `startup.spawn(handle)` outside
  `runtime.block_on` or `runtime.enter`.
- The minimum repair enters that owned runtime only while spawning the network
  supervisor. A pure current-thread Tokio regression reproduces the boundary
  without constructing or binding an Endpoint; the existing injected pre-bind
  network lifecycle test remains the no-UDP/DNS companion evidence.
- `v0.1.5` remains immutable and is never moved or reused. The repaired
  lockstep candidate advances to `v0.1.6`. Formal configured update/uninstall,
  reinstall identity rotation, and final user acceptance remain pending.

## Published v0.1.6 Linux installer finding (2026-08-26)

- The user successfully installed and started the immutable `v0.1.6` macOS
  build after removing the prior local state; this is user acceptance evidence,
  not a substitute for the hosted release matrix.
- A separate ordinary-account Linux one-line install stopped before artifact
  download with `install directory must not be writable by group or other
  users`. The observed account home and `~/.local` were mode `0700`, while the
  pre-existing `~/.local/bin` was mode `0775`, consistent with a user-private
  group and `umask 0002`. No successful Linux install is claimed yet.
- The installer directory UID/mode preflight is stricter than the requested
  writable-directory contract and rejects this common layout. The `v0.1.7`
  candidate removes those directory permission checks and their `id`/`stat`
  tool dependencies, while retaining absolute/direct/writable directory
  checks, existing-target no-clobber activation, authenticated assets, and
  current-UID non-group-writable executable validation. One POSIX local-HTTPS
  fixture owns the existing-default-`0775` regression without adding a
  permission matrix.
