# M9 implementation plan

## Step 0: Re-establish the release boundary

- [ ] Load the curated task context and record HEAD, current GitHub settings, latest CI, existing Releases, and clean worktree.
- [ ] Confirm M7–M8 implementation task is archived or has an explicit handoff; do not modify remote Session/transport behavior in M9.
- [ ] Fix the exact four target names, glibc 2.28 floor, manifest size bounds, release asset inventory, and current lockstep version before code changes.
- [ ] Record the one-time external checkpoint for creating the GitHub `release` Environment, required reviewer, immutable-release setting, and secret; never place key material in task files or command output.

## Step 1: Shared release contract and fixtures

- [ ] Add one release manifest/selection/verification owner with exact-byte signature verification, strict bounds, typed redacted errors, and stable/prerelease rules.
- [ ] Extend build identity with exact source/target/release fields without changing wire identity or persisting new secrets.
- [ ] Add offline fixtures for valid stable/prerelease manifests and the smallest invalid signature, size, target, version, duplicate, and schema cases.
- [ ] Add the non-product release tool for manifest/signature/checksum/generated-installer creation and self-verification; signing code must not enter the product binary.

Focused gate: release-contract units, fixture verification, secret/debug scan, package check/Clippy.

## Step 2: Native artifact and draft-release workflow

- [ ] Build four native artifacts with fixed inventory/timestamps/modes and embed exact build identity.
- [ ] Build Linux artifacts in pinned glibc 2.28 containers and inspect ELF symbol versions; build both macOS architectures on native hosted runners.
- [ ] Generate checksums, signed manifest, versioned installer, SBOM, and provenance attestations from one asset inventory.
- [ ] Add a manual workflow that verifies tag = `v` + Cargo version, builds without secrets, waits on the protected `release` Environment only for signing/publish, assembles a draft, verifies all downloaded draft assets, then publishes once.
- [ ] Pin third-party actions and keep release permissions job-scoped; no PR or self-hosted path may reach the signing environment.

Focused gate: workflow/static policy, local unsigned dry-run, four hosted artifact builds, draft asset round-trip.

## Step 3: Official installer

- [ ] Implement the small mutable bootstrap and generated immutable `zterm-install.sh` stages from the design.
- [ ] Detect target/glibc/NixOS/musl and destination ownership before artifact download.
- [ ] Verify embedded manifest/archive metadata, then candidate signature/self-check, before same-directory atomic install.
- [ ] Prove install creates no `~/.zterm`, identity, config, socket, daemon, PTY, login item, or shell-rc change; existing managed/unrelated destinations are not overwritten.
- [ ] Add local HTTPS fixtures for latest stable, exact stable, exact prerelease, draft/missing tag, truncation, wrong target, bad digest/signature, interrupted install, and PATH guidance, with one owner per contract rather than syntax enumeration.

Focused gate: `shellcheck`, POSIX syntax, installer fixture suite on macOS/Linux x86_64/arm64 hosted runners.

## Step 4: Explicit update, rollback, and uninstall

- [ ] Add CLI parsing/help and LocalRuntime methods for `update` and `uninstall`, preserving the no-autospawn/read-only boundaries until an operation genuinely requires daemon state.
- [ ] Implement bounded native HTTPS fetch, manifest verification, candidate staging, exact impact preflight, force boundary, daemon stop, atomic activation/post-check, and binary rollback.
- [ ] Reuse existing identity-reset/state inventory and deletion primitives for uninstall; add only installed-binary ownership and install metadata.
- [ ] Cover pre-setup update/uninstall, active Session refusal/force, stop failure, activation failure, retryable partial uninstall, reinstallation identity change, and incompatible CLI/daemon diagnostics.
- [ ] Confirm no update check/download occurs in setup, status, doctor, daemon startup, or ordinary commands.

Focused gate: CLI side-effect tests, local daemon lifecycle tests, activation fault tests, install/update/uninstall integration fixtures.

## Step 5: Documentation and release rehearsal

- [ ] Document reviewed-script, one-line, exact-version, manual checksum/signature, update, rollback, uninstall, PATH, support floor, bootstrap trust, key rotation, and emergency recovery paths.
- [ ] Update README/help/specs only where behavior is directly implemented; keep M10 real-network and user acceptance pending.
- [ ] Run one unsigned/local release rehearsal and one protected signed draft rehearsal. Do not publish a stable Release until all M9 checks and an independent checker pass.
- [ ] Run the full workspace/platform/release gate once at phase end, then independent `trellis-check`; fix only concrete findings.

## Step 6: M9 completion handoff

- [ ] Save exact commit, workflow run, four artifact digests, manifest/public-key ID, draft/signed rehearsal, and installer matrix evidence.
- [ ] Publish the exact formal candidate required by M10 only after the approved release checkpoint.
- [ ] Update parent M9 evidence without marking M10 or first-stage completion.

## Guardrails and stop conditions

- Use one implementation owner at a time and one independent checker; no external provider/channel without explicit user approval.
- Focused checks during iteration; broad workspace gates only at phase end and pre-commit.
- At four active hours, report completed scope, remaining scope, and largest risk. At eight hours, stop for explicit approval rather than adding hardening.
- A harness-only flake gets at most two materially different fixes or 90 minutes, then is simplified/deferred.
- Do not add a second signature format, verifier binary, release database, package-manager channel, mirror, background updater, or routine rollback drill without a failed acceptance criterion that requires it.

## Rollback points

- Contract/tooling commits precede workflow/external GitHub configuration.
- Workflow stays manual and draft-only until local/hosted asset verification is green.
- Product update/uninstall remains unreachable from public help until its fault and lifecycle gates pass.
- A failed external secret/environment setup leaves ordinary CI and current releases unchanged.
