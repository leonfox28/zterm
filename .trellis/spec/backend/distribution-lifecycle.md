# Signed Distribution and Executable Lifecycle Contract

## Scenario: native release, install, explicit update, and uninstall

### 1. Scope / Trigger

- Trigger this spec when changing native Release assets, manifest/build identity,
  installer hidden entries, executable activation, `zterm update`, `zterm
  uninstall`, or managed `install.json`.
- Default publication targets are `aarch64-apple-darwin` and
  `aarch64/x86_64-unknown-linux-gnu`; floors are macOS 13.0 and glibc 2.28.
  macOS Intel, Windows CI/distribution, and relay image publication are paused
  by explicit user direction. Restore them only in a future task that expressly
  requests it. Historical assets and unrelated platform/runtime boundaries are
  not removed by this publication policy.
- This contract must not introduce background checks, package-manager channels,
  mirrors, services/login items, sudo installation, or a second signature
  format/verifier.

### 2. Signatures

```rust
// crates/core/src/release.rs: the single metadata/verification owner
pub fn verify_release_manifest(
    raw_manifest: &[u8],
    signature: &[u8],
    public_key: &[u8; 32],
) -> Result<ReleaseManifest, ReleaseError>;
pub fn validate_unsigned_manifest(manifest: &ReleaseManifest) -> Result<(), ReleaseError>;
pub fn sha256_reader(reader: impl Read, maximum: u64) -> Result<(u64, String), ReleaseError>;
pub fn require_official_distribution_build(
    build: &BuildIdentity,
) -> Result<(), ReleaseError>;

// crates/daemon/src/distribution.rs and operations.rs
pub fn ReleaseSelection::parse(tag: Option<&str>) -> Result<ReleaseSelection, DistributionError>;
pub async fn prepare_update(selection: ReleaseSelection) -> Result<PreparedRelease, DaemonError>;
pub async fn LocalRuntime::update(
    &self,
    exact_tag: Option<&str>,
    force: bool,
) -> Result<UpdateResult, DaemonError>;
pub async fn LocalRuntime::uninstall_preflight(&self) -> Result<UninstallPreflight, DaemonError>;
pub async fn LocalRuntime::uninstall(
    &self,
    expected_device_id: Option<DeviceId>,
    force: bool,
) -> Result<UninstallResult, DaemonError>;
```

Public CLI signatures are:

```text
zterm update [--version <vSEMVER>] [--force]
zterm uninstall [--yes] [--force]
```

The installer-only entries are hidden and handled before `LocalRuntime::current`:

```text
--internal-release-self-check
--internal-release-verify <MANIFEST> <SIGNATURE>
--internal-release-install <ABSOLUTE_DESTINATION>
```

The non-product maintainer bootstrap command is:

```text
zterm-release-tool derive-public-key
stdin:  64 lowercase hexadecimal seed bytes, with at most one trailing LF
stdout: 64 lowercase hexadecimal Ed25519 public-key bytes plus LF
```

Release preparation validates the proposed Cargo version through the same
non-product Rust/SemVer owner:

```text
zterm-release-tool validate-next-version <SEMVER>
```

The text must be canonical SemVer and strictly newer than the compiled
workspace version.

### 3. Contracts

- `zterm-release.json` authenticates schema/product/version/tag/classification,
  40-lowercase-hex source commit, second-resolution UTC timestamp, wire/state/
  bootstrap schema, public-key ID, and target entries within one 64 KiB document.
  Runtime verification authenticates the exact bytes and validates common
  release metadata, then `artifact_for_target` requires exactly one current
  target and validates only that entry. No runtime platform allowlist or
  platform-count limit exists. Other platforms may be omitted or unknown;
  their URLs, archives, floors, and optional platform fields are not validated
  by the current platform's updater. Schema-v1 common field types still apply.
  A missing current target returns `UnsupportedTarget`, mapped to the existing
  `UnsupportedPlatform` domain error; duplicate current entries are rejected.
  The candidate self-check must equal that selected artifact's build identity;
  it cannot choose a different platform entry by reporting its own target.
  `PUBLISHED_RELEASE_TARGETS` lives only in the private release tool. That tool
  requires its exact planned target set and validates every artifact before
  signing/publishing. Historical four-target signatures remain verifiable.
  Already installed versions through 0.1.17 require four targets and cannot
  consume three-target updates; first migration uses the authenticated installer
  and existing atomic activation boundary documented in `docs/install.md`.
  Each artifact has fixed filename/immutable GitHub URL, compressed length/
  SHA-256, one platform floor, and a complete embedded build identity.
- Authenticate the bounded exact JSON bytes with one raw 64-byte Ed25519
  signature before JSON parsing. `release/public-key.hex` is reviewed source;
  `UNCONFIGURED`, malformed, or all-zero values fail closed.
- Build identity embeds Cargo version, exact `TARGET`, source commit, wire/state
  versions, key ID, and stable/prerelease classification. Release builds set
  `ZTERM_SOURCE_COMMIT`; deterministic archive creation requires
  `SOURCE_DATE_EPOCH`. Ordinary development/CI builds remain `development`;
  ambient `GITHUB_SHA` is not managed-distribution authority. The protected-main
  candidate job explicitly sets `ZTERM_SOURCE_COMMIT`; PR test jobs do not.
- The non-product `zterm-release-tool` alone handles release seed material.
  `derive-public-key` accepts the bounded seed only on stdin, zeroizes both its
  encoded and decoded owners, and writes only the Ring-derived public key.
  Signing reads `ZTERM_RELEASE_SIGNING_KEY` and proceeds only when the same
  Ring-derived public key equals reviewed source. Build the reviewed tool
  before initial key generation; never put the seed in argv, a repository
  file, logs/debug output, an Action artifact, or a build/test job.
- Archive inventory is exactly one regular `zterm`, mode `0700`, uid/gid 0,
  with fixed tar/gzip timestamps. Formal assets are the three archives,
  manifest/signature, `SHA256SUMS`, generated installer, and SPDX JSON.
- The mutable bootstrap selects latest stable or one canonical exact tag. The
  generated immutable installer performs target/floor/destination validation
  before artifact download, verifies embedded manifest/archive hashes before
  executing the candidate, then uses only the three hidden entries above.
- An install directory must be absolute, a direct directory rather than a
  symlink, and writable for the current process. The installer does not reject
  it based on directory UID or group/other-write mode; this permits the common
  existing `0775` `~/.local/bin` created by a user-private group and `umask
  0002`. The installed executable itself remains a direct current-UID file
  without group/other write bits.
- Installer activation is same-directory, fsynced, and atomic no-clobber. It
  creates no zterm state and refuses every existing destination. Setup writes
  mode-`0600` `install.json`; metadata is diagnostic, never identity authority.
- Installer activation, update, and uninstall first require a configured
  reviewed public key plus canonical official build identity. This proof does
  not constrain a user-selected absolute writable install directory, but it
  rejects repository, development, ordinary-CI, and `UNCONFIGURED` binaries
  before a network request or destructive state observation.
- Update proves the current executable locally, then fully
  prepares/authenticates the candidate before daemon contact.
  Incompatible CLI/daemon, active Sessions without `--force`, or failed stop
  prevent activation. Activation retains the old executable, post-checks the
  new path, rolls back on failure, updates metadata only when state exists, and
  never restarts the daemon.
- Uninstall first proves the exact running managed executable, then reuses identity
  reset/Session force/managed-inventory deletion and removes the executable
  last. It never sends `RevokeSelf` or performs setup.
- A successful `ci.yml` push run on `main` owns the three native candidate
  builds plus deterministic unsigned assembly and generated-installer checks.
  The aggregate `CI gate` requires both ordinary evidence and this candidate.
  Code and version preparation may share one PR. Publication requires one
  annotated `v` + Cargo-version tag after exact green main evidence and a
  retained candidate are proven; SemVer build metadata remains unsupported.
- The host-only `zterm-terminal` crate and pinned official
  `alacritty_terminal` dependency compile in every native release build.
  Windows hosted validation is paused until explicitly requested. They are
  Rust-linked into the product binary and must not add a separately distributed
  terminal dynamic library.
  `zterm-core` and `zterm-proto` remain engine-free for remote/mobile clients;
  this isolation is not a claim of mobile local-PTY support.
- Wire-major-two releases are coordinated cutovers. Every node that may connect
  to another node must run the same release before terminal traffic resumes;
  ALPN `zterm/2` and `zterm-pair/2`, protobuf package `zterm.v2`, and semantic
  terminal kinds are one atomic compatibility boundary. The product ships no
  mixed-version adapter, downgrade, presentation negotiation, or old terminal
  kind fallback. If rollback is required before reopening user traffic, roll
  back the whole participating release set and accept that Sessions ended by a
  forced update are not resurrected.
- Containerized native jobs must add only the exact quoted `$GITHUB_WORKSPACE`
  as Git `safe.directory` after checkout and before Git-backed source-policy
  checks. A wildcard or broader trusted path is forbidden. Tool paths must be
  derived from the runtime `$HOME` (for example `$HOME/.cargo/bin`), never from
  the container user's assumed passwd home such as `/root`.
- `.github/workflows/release.yml` is tag-triggered, GitHub-hosted, and
  action/image digest-pinned. It selects an unexpired assembled candidate from
  a completed successful `ci.yml` main-push run for the exact source commit.
  `find-candidate.sh` binds the workflow/event/branch/SHA and a server-digested
  artifact ID; the downloader fails on a digest mismatch. It cannot select PR
  artifacts or another commit. Failed-job retries may reuse an earlier
  successful assembly within that exact run.
  Fetch paginated artifact pages successfully before querying them with external
  `jq`: GitHub CLI rejects `--slurp` combined with its `--jq` option. Lookup
  fixtures must preserve this CLI contract and exercise multiple response pages.
- Main native jobs build/self-check only the shipped binary and upload its raw
  bytes plus identity. Their ephemeral per-target uploads may be replaced during
  a CI retry. Ubuntu assembles deterministic archives and the exact unsigned
  inventory once; each assembly attempt has a new immutable artifact ID named
  `release-candidate-SHA-ATTEMPT`, retained for seven days. The tag workflow never
  recompiles `zterm` or re-archives its bytes. Missing/expired candidates require
  an explicit main-CI rerun before a new tag.
- Small signing and pre-publication verification tools are rebuilt from the
  frozen source. Signing re-verifies the unsigned inventory and exact manifest
  source SHA before key exposure. Signed/fixture uploads include the run attempt
  in their names and expose artifact IDs as signing-job outputs; retries never
  overwrite those bytes, and downstream jobs retain the successful signing
  attempt's IDs. Main assembly owns the single ShellCheck of
  the generated installer. The three-target final installer matrix retains
  POSIX syntax and authenticated local-HTTPS execution.
  That fixture binds its numeric loopback address through the test-only
  `TCPServer` path, publishes the already-bound address, and performs no
  hostname/FQDN lookup; this is fixture portability, not product behavior.
  Protected `release` Environment approval gates only the single seed-bearing
  signing job; verified draft creation, round-trip verification, attestation,
  and immutable publication then proceed without a second approval.
- A repo admin separately enables immutable Releases. The default workflow
  token must not receive an administration PAT merely to query that setting;
  the environment reviewer owns that precondition, and the published Release
  response must report `immutable: true`.

### 4. Validation & Error Matrix

| Condition | Required result |
| --- | --- |
| Manifest empty/over 64 KiB, unknown field/schema, duplicate/missing target | `release_manifest_invalid`; no candidate fetch/daemon contact |
| Signature not 64 bytes, bad exact bytes, or production key unavailable | `release_signature_invalid`; no candidate execution |
| Key-derivation stdin is empty, non-lowercase-hex, not 64 bytes plus optional LF, or over 65 bytes | Fail with no public-key stdout and no seed text in the error |
| Archive over 128 MiB, wrong length/hash/inventory or candidate mismatch | `release_artifact_invalid`; current executable/daemon/Sessions unchanged |
| Fetch timeout/status/redirect/size failure from fixed origin | `release_unavailable`; errors contain no URL query/body |
| Noncanonical tag, same version, or downgrade | `update_rejected` |
| Running daemon version/wire/schema differs | `update_rejected`; do not stop it |
| a v1 or mixed-version peer reaches a v2 ALPN/wire boundary | explicit incompatibility; do not downgrade, translate terminal representation, or partially activate the attachment |
| Active Sessions and no `--force` | `update_rejected`; no stop/activation |
| Stop or lifecycle ownership release fails | Preserve installed binary; return typed lifecycle error |
| Activation/post-check/metadata fails | Restore retained executable; report ended Sessions as ended |
| Installer destination exists, or install directory is relative/symlink/non-directory/unwritable | Refuse before download or activation; do not overwrite |
| Current binary is development/ordinary CI or key is `UNCONFIGURED` | `path_unsafe`; no update fetch, uninstall preflight, state deletion, or executable replacement |
| Uninstall state validation/deletion fails | Keep executable so retry remains possible |
| Tagged commit lacks a successful `ci.yml` push run on `main` for the same SHA | Stop before Environment approval, signing secret, or Release creation |
| ShellCheck is unavailable on its Ubuntu assembly owner, or rejects the generated installer | Stop before Environment approval/signing; do not upload the unsigned inventory |
| Local HTTPS fixture inherits `HTTPServer.server_bind` or calls `getfqdn` | Static policy failure; bypass the unnecessary lookup rather than extending startup timeouts |
| Container checkout has mismatched ownership | Trust only the exact `$GITHUB_WORKSPACE` before source-policy; never use wildcard `safe.directory` |
| Container tool installs under runtime `$HOME` | Export `$HOME/.cargo/bin`; a fixed `/root/.cargo/bin` is invalid even when the job runs as euid 0 |

### 5. Good/Base/Bad Cases

- Good: signed newer target archive verifies, zero Sessions are active, daemon
  stops, candidate activates/post-checks, metadata commits, daemon remains
  stopped, and the rollback file is removed.
- Good: every participating node is upgraded to the same wire-major-two release
  before reconnecting, then local/direct/Relay smoke evidence is collected on
  the new semantic protocol.
- Base: install through an existing writable mode-`0775` default
  `~/.local/bin`, observe no `~/.zterm`, then run setup separately; pre-setup
  uninstall removes only that executable.
- Bad: downloading or stopping a daemon before exact-byte signature, length,
  digest, target, version, and candidate self-checks finish.
- Bad: treating GitHub HTTPS/attestation as a replacement for the embedded
  Ed25519 manifest signature, or allowing an environment/config URL override.
- Bad: deriving the first reviewed public key through an ad hoc OpenSSL/DER
  parser or printing the seed so that shell tooling can reuse it.
- Bad: requiring a tool merely because one hosted runner image happens to
  provide it, or silently skipping the lint when the tool is absent.
- Bad: keep a hidden v1 listener, terminal-kind translator, or capability
  fallback so nodes can be upgraded independently after the coordinated
  cutover.
- Bad: treating a socket bound on numeric loopback as proof that a framework
  will not resolve a hostname before listening, or hiding that lookup behind a
  longer fixture timeout.

### 6. Tests Required

- `cargo test -p zterm-core release::tests` asserts exact-byte authentication,
  common schema/classification boundaries, current-target uniqueness and
  artifact checks, one-target and unknown/more-than-four-target inventories,
  selected candidate identity, monotonic versions, and bounded streaming digest.
- `cargo test -p zterm-release-tool` asserts byte-identical archives, that a
  three-target manifest mechanically renders POSIX-valid installer metadata,
  exact planned publication inventory, and that `derive-public-key` matches
  RFC 8032 while rejecting oversized input without stdout or seed-bearing diagnostics.
- `cargo test -p zterm-daemon distribution::tests` asserts single-platform
  preparation, unrelated-target independence, missing-current-target rejection,
  bad archive rejection before candidate execution, and canonical exact/latest
  selection.
- Source/dependency policy and hosted native jobs verify the exact official
  Alacritty pin, license/advisory status, core/proto graph isolation and unchanged artifact dynamic-library inventory.
  Windows compilation evidence is currently paused.
- Platform executable tests assert installed-file owner/mode/symlink rejection,
  install-directory shape, no-clobber install, retained-backup rollback, and
  exact removal.
- Operations tests assert development/ordinary-CI refusal before update network
  or uninstall state access, mode-`0600` metadata, active-Session force, stop
  failure, and incompatible daemon rejection at their authoritative owners.
  The signed hosted candidate owns positive pre-setup/configured uninstall and
  reinstall identity-rotation evidence.
- `sh tests/release/static.sh` asserts exact-tag triggering, an annotated tag
  without build metadata, exact green-main-CI gating, stable PR `CI gate`, three
  main-push native candidate builds and one unsigned assembly, pinned release
  dependencies/caches/timeouts, one Environment/secret reference, runtime-HOME
  Cargo path, exact non-wildcard container `safe.directory`, one fail-closed
  Ubuntu generated-installer
  ShellCheck owner before signing, no ShellCheck assumption in the installer
  matrix, centralized raw-candidate archiving, absence of ordinary CI commands
  and product rebuilds after the tag, direct numeric-loopback fixture binding
  with no FQDN path,
  verified draft publication, and installer no-side-effect tokens.
  Exact-main CI ShellChecks maintained shell sources. Hosted three-target jobs
  own POSIX syntax, a local HTTPS happy path through an existing default
  `0775` directory created under `umask 0002` without `chmod`,
  existing-destination preflight, digest failure, native execution,
  glibc/Mach-O floor inspection, signed round-trip, attestation, and immutable
  formal publication.
- Do not execute real Iroh/Endpoint acceptance on a developer macOS host merely
  to validate this distribution contract.

### 7. Wrong vs Correct

#### Wrong

```rust
// Stops user work before the unauthenticated candidate crosses trust checks.
client.stop(true).await?;
let archive = fetch(user_configured_url).await?;
fs::write(current_executable, archive)?;
```

```sh
# A second derivation implementation can disagree with Ring or leak the seed.
echo "$release_seed_hex"
openssl pkey -in release-private-key.pem -pubout

# Wildcard trust hides unrelated checkout-ownership mistakes.
git config --global --add safe.directory '*'

# Euid 0 does not imply that Actions configured HOME=/root.
echo "/root/.cargo/bin" >> "$GITHUB_PATH"

# A tool present on Ubuntu is not implicitly present on every matrix OS.
command -v shellcheck
shellcheck generated-installer.sh

# Numeric loopback does not prevent HTTPServer.server_bind from resolving a
# hostname before listen on every platform.
http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
```

#### Correct

```rust
// Fixed-origin, bounded preparation authenticates bytes and candidate first.
let prepared = prepare_update(ReleaseSelection::parse(exact_tag)?).await?;
let impact = client.update_preflight().await?;
require_force_if_needed(&impact, force)?;
client.stop(force).await?;
let activation = activate_verified_candidate(&prepared)?;
postcheck_or_rollback(activation, &prepared)?;
```

```sh
# The reviewed tool receives the seed only on stdin and prints only public data.
set +x
release_public_hex=$(printf '%s\n' "$release_seed_hex" \
  | zterm-release-tool derive-public-key)
printf '%s' "$release_seed_hex" \
  | gh secret set ZTERM_RELEASE_SIGNING_KEY --env release --repo OWNER/REPO
unset release_seed_hex

# Container ownership exception is scoped to this exact Actions checkout.
git config --global --add safe.directory "$GITHUB_WORKSPACE"
echo "$HOME/.cargo/bin" >> "$GITHUB_PATH"
sh tests/source-policy.sh

# One explicit Ubuntu owner lints the exact generated artifact before signing;
# every platform separately runs portable syntax and behavior gates.
command -v shellcheck
shellcheck -s sh release-output/zterm-install.sh
```

```python
class FixtureHTTPServer(http.server.ThreadingHTTPServer):
    def server_bind(self) -> None:
        socketserver.TCPServer.server_bind(self)
        self.server_name, self.server_port = self.server_address[:2]
```

The ordering is the safety property: authentication precedes impact, explicit
impact approval precedes stop, and retained-binary activation precedes commit.

## Scenario: deterministic release preparation and bounded resume

### 1. Scope / Trigger

- Trigger this scenario when changing `just release-prepare`, its Cargo/Git/GitHub
  operations, recovery behavior, fixture, or operator runbook.
- Release preparation is a reversible PR-producing phase. It may update version
  metadata and publish a review branch/PR, but it must never create a tag,
  Release, signature, asset, or relay image.
- A dirty partially prepared branch is diagnostic evidence, not resumable
  operator state. Automated resume begins only after an exact clean release
  commit exists.

### 2. Signatures

The public command and direct repository owner are:

```text
just release-prepare <VERSION>
sh tools/release/operator.sh prepare <VERSION>
```

`VERSION` is canonical SemVer without a leading `v` or build metadata, and must
be strictly newer on a fresh preparation. The deterministic Cargo sequence is:

```text
cargo +1.98.0 update --workspace
cargo +1.98.0 metadata --locked --format-version 1 --no-deps
sh tests/workspace-version.sh
```

### 3. Contracts

- Fresh prepare on clean exact `origin/main` creates `release/vVERSION`.
  On a clean feature branch containing current main, it keeps that branch and
  prepares the version in the same PR. Version/tag/Release preflight is shared;
  the version commit alone must contain exactly the two version files.
- After editing exactly one `[workspace.package].version`, `cargo update
  --workspace` is the only lockfile-generation owner. Locked metadata and the
  workspace-version script validate the result; they are not generation APIs.
- Before commit and push, the complete changed/staged/untracked inventory is
  exactly `Cargo.lock` plus `Cargo.toml`. An inventory failure prints both the
  expected and actual sorted sets.
- Prepare does not run `just check`. The required release PR `CI gate` owns the
  complete format, Clippy, test, docs, dependency, and portable policy evidence;
  merged `main` CI owns the exact-SHA assembled candidate.
- Resume is accepted only while already on the clean expected release branch.
  Its HEAD is an ordinary version commit, contains current fetched main, has the
  exact subject `chore: prepare vVERSION release`, contains only the two version
  files, resolves the requested workspace version, and passes locked metadata
  plus workspace-version validation.
- A missing remote release branch may be pushed. An existing remote branch may
  be reused when its SHA equals local HEAD, or advanced by a normal fast-forward
  push from an ancestor of local HEAD; divergence is never overwritten. One
  open PR may be reused only when its head SHA, head branch, and base `main`
  match. Missing PR state may be created; closed, merged,
  multiple, malformed, or unavailable PR state fails closed.
- Push/PR network ambiguity is recovered by rerunning the same command after
  connectivity returns. The operator does not persist a parallel state file,
  infer success from an error, force-push, delete evidence, or repair dirty
  branches.
- `just release VERSION PR` / `operator.sh finish VERSION PR` requires an
  authorized reviewed PR. It waits for exact PR CI, verifies required checks,
  merges with an exact-head guard, waits for the returned main merge SHA, and
  publishes from a clean private detached worktree. The caller's branch is
  preserved. PR/merge/run/tag state is authoritative; no parallel state file is
  maintained. A merged PR or pushed matching tag resumes without repetition.
- Before a new tag, publish requires exact current main, enforced protection,
  green CI and its retained candidate. An existing local/remote annotated tag
  is reusable only for its exact source; it is never replaced. A remote tag
  rejoins its original release run. Private worktrees are removed only when
  clean. Signing approval and immutable publication retain their boundaries.

### 4. Validation & Error Matrix

| Condition | Required result |
| --- | --- |
| Real Cargo cannot update the workspace lock | Stop on the local release branch; no branch push, PR, or tag |
| Locked metadata or workspace-version validation fails | Stop with the generated diff retained; no branch push, PR, or tag |
| Inventory is missing `Cargo.lock` or contains an extra path | Print expected and actual sorted inventories; do not commit/push |
| Fresh invocation sees an existing local/remote release branch | Reject; only an invocation already on an exact clean release commit can enter resume |
| Resume branch is dirty, has an extra commit/file, wrong subject/version, or no longer contains current `origin/main` | Reject without modifying local or remote state |
| Remote release branch is absent | Push the exact local release commit once, then reconcile PR state |
| Remote release branch SHA equals local HEAD | Reuse it; do not force-push or create a second branch |
| Remote release branch is an ancestor of local HEAD | Push normally, preserving the same feature PR |
| Remote release branch diverges | Reject and report both sides; never overwrite it |
| One open PR matches exact head SHA/branch and base `main` | Reuse its URL and complete prepare |
| No PR matches after the exact remote branch is proven | Create one PR |
| PR query is unavailable, ambiguous, closed, merged, or mismatched | Reject; do not guess or create competing review state |

### 5. Good/Base/Bad Cases

- Good: a fresh prepare changes the manifest, real Cargo refreshes all workspace
  package lock entries, focused validators pass, the exact commit is pushed,
  and one release PR is returned without running the local full suite.
- Good: PR creation times out after GitHub accepted it; the next invocation from
  the unchanged clean release branch proves local, remote, and open-PR identity
  and returns the existing PR.
- Base: generation fails and leaves a dirty local release branch. The operator
  reports it for inspection; the maintainer may fix it manually, but rerunning
  does not reinterpret it as a safe checkpoint.
- Bad: mocking `cargo metadata` to rewrite `Cargo.lock`, because production
  Cargo does not provide that effect and the fixture would hide the failure.
- Bad: adding a generic state machine that accepts arbitrary existing branches,
  commits, or PRs without proving the exact version commit and ancestry, or rerunning the full local gate already required by PR CI.

### 6. Tests Required

- `sh tests/release/operator-fixture.sh` uses a task-private Git remote and fake
  GitHub boundary but pinned real Cargo 1.98 for `pkgid`, `update`, and
  `metadata`.
- The fixture asserts a real inherited workspace version moves from the
  baseline to the requested version in both `Cargo.toml` and `Cargo.lock`, the
  committed inventory is exactly two files, metadata is invoked with `--locked`,
  and no `just check` command runs.
- The fixture covers dirty/partial generation rejection, expected/actual
  inventory diagnostics, exact clean-commit resume with same remote SHA/open PR,
  missing remote/PR continuation where applicable, and remote/commit/PR
  divergence rejection. It asserts no tag in every prepare path.
- The operator fixture covers preparation on an existing feature branch,
  failure after merge but before tagging, exact-source finish/resume, missing
  candidate rejection and private-worktree cleanup. `candidate-fixture.sh`
  executes the actual jq selectors against REST-shaped responses for successful
  retries, expired/mismatched candidates and rejected PR authority.
- `sh tests/release/static.sh`, ShellCheck, shell syntax,
  `git diff --check`, and the repository broad gate remain required before
  merge. No fixture contacts the production GitHub repository.

### 7. Wrong vs Correct

#### Wrong

```sh
# metadata --no-deps is not a lockfile update command.
sed -i.bak 's/^version = .*/version = "NEXT"/' Cargo.toml
cargo +1.98.0 metadata --format-version 1 --no-deps >/dev/null
just check
```

```sh
# A fake side effect makes the test pass while production fails.
case "$*" in
  *metadata*) rewrite_every_lock_version ;;
esac
```

#### Correct

```sh
update_the_single_workspace_version
cargo +1.98.0 update --workspace
cargo +1.98.0 metadata --locked --format-version 1 --no-deps >/dev/null
sh tests/workspace-version.sh
require_exact_release_change_inventory
# The pushed PR's required CI gate owns the full suite.
```

The architectural boundary is the exact clean release commit: before it,
failures retain local evidence for manual diagnosis; after it, identity can be
proven across local Git, the remote branch, and the open PR without inventing
another durable state model.
