# M9 signed distribution design

## 1. Design goals

M9 owns one official macOS/Linux distribution path from an immutable Git tag to an installed user-level `zterm` binary. It must authenticate release metadata, preserve the existing per-user daemon/state boundaries, and provide explicit update, rollback, and uninstall behavior without Rust, Node/npm, administrator access, background checks, or a system service.

The design deliberately reuses existing owners:

- Cargo workspace version remains the product version authority.
- `BuildIdentity`, wire major, state schema, and target identity remain shared product metadata.
- `LocalRuntime` remains the public CLI orchestration boundary.
- existing lifecycle/status/stop and exact managed-state deletion primitives remain the daemon/state owners.
- `zterm-platform` owns same-filesystem atomic file activation and supported-platform inspection.

No second daemon lifecycle, state root, HTTP control plane, updater service, or release database is introduced.

## 2. Trust chain

```text
reviewed source tag
  -> native GitHub-hosted builds
  -> checksums + bounded exact-byte manifest
  -> Ed25519 signature inside protected release Environment
  -> draft GitHub Release + provenance attestations
  -> verification of every asset
  -> publish immutable Release
  -> official installer / installed updater
```

The long-lived Ed25519 seed exists only as the `ZTERM_RELEASE_SIGNING_KEY` secret in the GitHub `release` Environment. The signing/publish job:

- runs only on a GitHub-hosted runner;
- is reached only by a manual release workflow for an existing exact tag;
- cannot read the secret until the configured reviewer approves the environment;
- receives no pull-request-controlled code or inputs after approval;
- builds the reviewed signing tool before the secret-bearing step, so Cargo,
  build scripts, and proc macros never inherit the seed;
- uses commit-pinned third-party actions;
- zeroizes the decoded seed owner and never prints derived secret material.

The public verification key is reviewed source and is compiled into `zterm`. GitHub immutable releases and attestations bind the tag, assets, workflow, repository, and commit, but do not replace the product's detached manifest verification.

## 3. Release assets and manifest

One release contains:

```text
zterm-aarch64-apple-darwin.tar.gz
zterm-x86_64-apple-darwin.tar.gz
zterm-aarch64-unknown-linux-gnu.tar.gz
zterm-x86_64-unknown-linux-gnu.tar.gz
zterm-release.json
zterm-release.json.sig
SHA256SUMS
zterm-install.sh
zterm-sbom.spdx.json
```

GitHub provenance/release attestations accompany these assets. The exact JSON bytes of `zterm-release.json` are signed; verifiers authenticate the bounded raw bytes before parsing, so no custom JSON canonicalization scheme exists. Schema v1 contains:

- schema version, product version, release tag, stable/prerelease classification, source commit, release timestamp;
- wire major and state schema version;
- for each of the four targets: exact filename, target triple, byte length, SHA-256, minimum OS/glibc boundary, and binary build identity;
- bootstrap schema and public-key identifier. The generated installer digest is intentionally excluded to avoid a manifest↔installer hash cycle; GitHub's immutable release/attestation binds that bootstrap asset.

Unknown schema major, duplicate target, inconsistent tag/version/classification, excessive size, unsupported target, or any mismatched field is rejected. One shared Rust decoder/validator owns this contract; workflow tooling, updater, self-check, and tests consume it rather than re-parsing fields independently.

## 4. Two-stage bootstrap installer

The mutable repository script `install/install.sh` is intentionally small. It accepts only the documented version and install-directory options, resolves latest stable or one exact release tag, then downloads the immutable release asset `zterm-install.sh` from GitHub HTTPS. This first hop is the disclosed bootstrap trust root; it cannot protect against compromise of the reviewed installer source plus GitHub HTTPS.

The generated versioned `zterm-install.sh` embeds its exact tag, manifest digest, and the size/SHA-256 table generated from the same validated release manifest. Its order is:

1. Detect exact OS, architecture, glibc/environment, required local tools, and writable destination before downloading a binary.
2. Download the bounded manifest, detached signature, and selected archive into a private temporary directory.
3. Verify the manifest and archive length/SHA-256 against values embedded in this immutable versioned script.
4. Extract the digest-authenticated candidate and run only side-effect-free release self-check/manifest-signature verification modes.
5. Cross-check candidate version, target, build identity, wire/schema values, manifest signature, and archive metadata.
6. Create a same-directory temporary executable, set mode, sync, and atomically rename it to the destination; sync the directory.
7. Report PATH guidance only. Do not run setup, create `~/.zterm`, start a daemon, modify shell files, or register a service.

This avoids a second verifier binary: the candidate is executed only after its archive has been authenticated by the versioned bootstrap's embedded digest, then its embedded public key verifies the signed manifest. The generated digest table is mechanically derived from the single manifest owner and cross-checked in release tests.

An existing recognizable `zterm` at the destination is not overwritten by the installer. The script directs the user to the binary's explicit update command. An unrelated file is never replaced.

## 5. Product release module

Release metadata and signature verification live in one shared Rust module below the daemon-facing runtime. It owns:

- bounded manifest/signature decoding and Ed25519 verification using the existing audited crypto stack;
- exact stable/prerelease and target selection;
- build/wire/schema compatibility checks;
- SHA-256 streaming verification and bounded download results;
- redacted typed errors with no URL query, response body, secret, state path, or terminal content.

Production HTTPS fetching uses one native Rust client with rustls and the platform trust roots plus explicit connect/total/size bounds; it does not shell out to `curl`. Tests inject a bounded fetcher and local HTTPS fixtures. Release URLs are fixed product constants and cannot be overridden by ordinary config, environment variables, or public CLI flags.

Signing is not included in the product binary. A task-private release tool outside the five product crates creates/verifies manifests, signatures, generated installer assets, checksums, and SBOM inputs in CI.

## 6. Installed build and metadata

Release builds embed product version, exact target, source commit, wire major, schema version, release key identifier, and release classification. A hidden side-effect-free self-check returns a machine-readable fixed schema for the installer/workflow; ordinary public help does not expose internal state or test overrides.

Only an explicit `ZTERM_SOURCE_COMMIT` plus the configured reviewed public key
marks an official managed-distribution build. Ambient `GITHUB_SHA` never does.
Installer activation, update, and uninstall reject development, ordinary-CI,
and `UNCONFIGURED` builds before network access or destructive state
observation, while retaining support for any safe user-selected install path.

The installer creates no state metadata. On first `setup`, the existing validated `~/.zterm` transaction records `install.json` from the running binary and executable path. Update/uninstall also work before setup by validating the current executable directly. Managed metadata is mode `0600`, rejects symlinks, and never becomes identity authority.

## 7. Explicit update

`zterm update` is a LocalRuntime operation and the only update entry point:

1. Prove the running executable is an official managed-distribution build, then resolve latest stable or an exact published tag; an exact older version is rejected as a managed downgrade unless a future separately designed compatibility path exists.
2. Download bounded manifest/signature/archive and verify signature, target, version monotonicity, size, checksum, and candidate self-check before contacting/stopping a daemon.
3. Query exact installed binary/daemon build and active Session impact. A nonempty impact refuses without explicit `--force`; the CLI never invents an implicit confirmation bypass.
4. Stop the daemon using the existing bounded owner release path, ending PTYs only after verification and approval.
5. Retain the prior executable next to the target, atomically activate the candidate, and run a side-effect-free post-activation self-check.
6. On activation/self-check failure, atomically restore the prior executable. Ended PTYs are reported as ended, never recovered.
7. On success, update managed install metadata and remove only the exact rollback candidate. Do not restart the daemon; later commands use existing on-demand startup.

No current M9 change introduces a state-schema migration. A future release that changes schema must separately own backup/migration/rollback evidence; binary rollback alone may not claim database rollback.

## 8. Uninstall

`zterm uninstall` reuses the identity-reset inventory, validation, lifecycle lock, stop, and no-follow managed-state deletion boundaries. It adds ownership of the installed executable:

1. Preflight reports active Sessions, current public identity, state deletion, authorization loss, and required re-pairing.
2. Interactive use requires exact confirmation; noninteractive use requires `--yes`, plus `--force` when Sessions exist.
3. Stop the daemon and release all PTY/socket/lock owners.
4. Delete the complete validated `~/.zterm` tree.
5. Remove only the currently executing, validated managed zterm path. Unix may unlink the running executable after state cleanup.

Failure is retryable from the surviving binary or documented versioned installer. Uninstall sends no `RevokeSelf`; copied old private keys remain a per-host revoke problem as already documented.

## 9. Native build matrix

- macOS artifacts build natively on GitHub arm64 and Intel runners.
- Linux artifacts build natively inside digest-pinned glibc 2.28 build containers on GitHub x86_64 and arm64 runners. The release gate inspects ELF symbol versions and rejects a higher glibc floor.
- archives use a fixed file inventory, ownership, permissions, ordering, and source timestamp. A repeated-build comparison is evidence only after platform-specific nondeterminism is measured; the workflow must not claim bit-for-bit reproducibility without a matching digest.
- Windows shared compilation remains CI evidence only and produces no M9 artifact.
- The validate job freezes the tag's peeled commit; every later checkout uses
  that commit, and draft creation rechecks that the existing tag still peels to
  it before creating external state.

## 10. Failure and rollback boundaries

| Failure | Required outcome |
| --- | --- |
| platform unsupported | fail before artifact download |
| release/tag/version mismatch | fail before daemon observation |
| timeout, oversize, bad signature/hash/target/self-check | keep binary, daemon, and Sessions unchanged |
| active Sessions without force | list impact and refuse |
| daemon cannot fully stop | do not replace binary |
| atomic activation/post-check fails | restore old binary; report PTYs already ended |
| uninstall state deletion incomplete | keep binary for retry |
| release workflow asset validation fails | leave draft unpublished; secret is not reused by automatic retry loops |

## 11. Scope control

M9 does not add package managers, background update checks, platform auto-start, Windows runtime, Apple notarization, self-update mirrors, a central release service, automatic schema rollback, or multiple signature formats. Each trust boundary has one authoritative test plus its smallest meaningful negative case.
