# Signed Distribution and Executable Lifecycle Contract

## Scenario: native release, install, explicit update, and uninstall

### 1. Scope / Trigger

- Trigger this spec when changing native Release assets, manifest/build identity,
  installer hidden entries, executable activation, `zterm update`, `zterm
  uninstall`, or managed `install.json`.
- The only shipped targets are `aarch64/x86_64-apple-darwin` and
  `aarch64/x86_64-unknown-linux-gnu`; floors are macOS 13.0 and glibc 2.28.
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

### 3. Contracts

- `zterm-release.json` authenticates schema/product/version/tag/classification,
  40-lowercase-hex source commit, second-resolution UTC timestamp, wire/state/
  bootstrap schema, public-key ID, and exactly four artifacts. Each artifact has
  fixed filename/immutable GitHub URL, compressed length/SHA-256, one platform
  floor, and a complete embedded build identity.
- Authenticate the bounded exact JSON bytes with one raw 64-byte Ed25519
  signature before JSON parsing. `release/public-key.hex` is reviewed source;
  `UNCONFIGURED`, malformed, or all-zero values fail closed.
- Build identity embeds Cargo version, exact `TARGET`, source commit, wire/state
  versions, key ID, and stable/prerelease classification. Release builds set
  `ZTERM_SOURCE_COMMIT`; deterministic archive creation requires
  `SOURCE_DATE_EPOCH`. Ordinary development/CI builds remain `development`;
  ambient `GITHUB_SHA` is not managed-distribution authority.
- The non-product `zterm-release-tool` alone reads
  `ZTERM_RELEASE_SIGNING_KEY`. It accepts one 32-byte lowercase-hex seed only
  when its derived public key equals reviewed source. Never log/debug the seed,
  copy it into an Action artifact, or expose it to build/test jobs.
- Archive inventory is exactly one regular `zterm`, mode `0700`, uid/gid 0,
  with fixed tar/gzip timestamps. Formal assets are the four archives,
  manifest/signature, `SHA256SUMS`, generated installer, and SPDX JSON.
- The mutable bootstrap selects latest stable or one canonical exact tag. The
  generated immutable installer performs target/floor/destination validation
  before artifact download, verifies embedded manifest/archive hashes before
  executing the candidate, then uses only the three hidden entries above.
- Installer activation is same-directory, fsynced, and atomic no-clobber. It
  creates no zterm state and refuses every existing destination. Setup writes
  mode-`0600` `install.json`; metadata is diagnostic, never identity authority.
- Installer activation, update, and uninstall first require a configured
  reviewed public key plus canonical official build identity. This proof does
  not constrain a user-selected safe install directory, but it rejects
  repository, development, ordinary-CI, and `UNCONFIGURED` binaries before a
  network request or destructive state observation.
- Update proves the current executable locally, then fully
  prepares/authenticates the candidate before daemon contact.
  Incompatible CLI/daemon, active Sessions without `--force`, or failed stop
  prevent activation. Activation retains the old executable, post-checks the
  new path, rolls back on failure, updates metadata only when state exists, and
  never restarts the daemon.
- Uninstall first proves the exact running managed executable, then reuses identity
  reset/Session force/managed-inventory deletion and removes the executable
  last. It never sends `RevokeSelf` or performs setup.
- `.github/workflows/release.yml` is manual, GitHub-hosted, action/image
  digest-pinned, and draft-only. Protected `release` Environment approval gates
  signing and draft creation. A repo admin separately confirms immutable
  Releases and supplies the exact `enabled-and-reviewed` checkpoint; do not add
  an administration PAT just to query that setting from Actions.

### 4. Validation & Error Matrix

| Condition | Required result |
| --- | --- |
| Manifest empty/over 64 KiB, unknown field/schema, duplicate/missing target | `release_manifest_invalid`; no candidate fetch/daemon contact |
| Signature not 64 bytes, bad exact bytes, or production key unavailable | `release_signature_invalid`; no candidate execution |
| Archive over 128 MiB, wrong length/hash/inventory or candidate mismatch | `release_artifact_invalid`; current executable/daemon/Sessions unchanged |
| Fetch timeout/status/redirect/size failure from fixed origin | `release_unavailable`; errors contain no URL query/body |
| Noncanonical tag, same version, or downgrade | `update_rejected` |
| Running daemon version/wire/schema differs | `update_rejected`; do not stop it |
| Active Sessions and no `--force` | `update_rejected`; no stop/activation |
| Stop or lifecycle ownership release fails | Preserve installed binary; return typed lifecycle error |
| Activation/post-check/metadata fails | Restore retained executable; report ended Sessions as ended |
| Installer destination exists/symlink/foreign/unsafe | `path_unsafe`; do not overwrite |
| Current binary is development/ordinary CI or key is `UNCONFIGURED` | `path_unsafe`; no update fetch, uninstall preflight, state deletion, or executable replacement |
| Uninstall state validation/deletion fails | Keep executable so retry remains possible |

### 5. Good/Base/Bad Cases

- Good: signed newer target archive verifies, zero Sessions are active, daemon
  stops, candidate activates/post-checks, metadata commits, daemon remains
  stopped, and the rollback file is removed.
- Base: install to an empty owned `~/.local/bin`, observe no `~/.zterm`, then
  run setup separately; pre-setup uninstall removes only that executable.
- Bad: downloading or stopping a daemon before exact-byte signature, length,
  digest, target, version, and candidate self-checks finish.
- Bad: treating GitHub HTTPS/attestation as a replacement for the embedded
  Ed25519 manifest signature, or allowing an environment/config URL override.

### 6. Tests Required

- `cargo test -p zterm-core release::tests` asserts exact-byte authentication,
  schema/inventory/size/classification boundaries, monotonic versions, and
  bounded streaming digest.
- `cargo test -p zterm-release-tool` asserts byte-identical archives and that a
  four-target manifest mechanically renders POSIX-valid installer metadata.
- `cargo test -p zterm-daemon distribution::tests` asserts valid preparation,
  bad archive rejection before candidate execution, and canonical exact/latest
  selection.
- Platform executable tests assert owner/mode/symlink rejection, no-clobber
  install, retained-backup rollback, and exact removal.
- Operations tests assert development/ordinary-CI refusal before update network
  or uninstall state access, mode-`0600` metadata, active-Session force, stop
  failure, and incompatible daemon rejection at their authoritative owners.
  The signed hosted candidate owns positive pre-setup/configured uninstall and
  reinstall identity-rotation evidence.
- `sh tests/release/static.sh` asserts manual/draft-only workflow, pinned
  dependencies, one secret reference, protected checkpoints, and installer
  no-side-effect tokens. Hosted four-target jobs own `shellcheck`, local HTTPS
  happy path, existing-destination preflight, digest failure, native execution,
  glibc/Mach-O floor inspection, signed draft round-trip, and attestation.
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

The ordering is the safety property: authentication precedes impact, explicit
impact approval precedes stop, and retained-binary activation precedes commit.
