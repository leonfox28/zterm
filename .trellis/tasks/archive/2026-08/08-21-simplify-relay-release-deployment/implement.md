# Implementation plan

## 1. Simplify version publication

- Bump the lockstep workspace product version and lockfile to `0.1.1`.
- Reduce `resolve-publication.sh` to Cargo-resolved version equality, direct
  `v...` tag reuse, channel selection, and minimal manual OCI validation.
- Simplify `relay-image.yml`: publish the direct version tag and stable
  `latest`, remove immutable-reference/digest validation and unused attestation
  contracts while preserving full-SHA Actions, minimal permissions, and one
  amd64/arm64 build.
- Replace the combinatorial publication test matrix with a small behavioral
  matrix covering stable, prerelease, manual, mismatch, and invalid manual tag.

## 2. Collapse the Relay bundle to one runtime/deployment

- Remove the Rust health probe stage/binary and add the image default config
  command; keep official checksum verification, scratch, non-root, CA bundle,
  and SIGINT stop signal.
- Consolidate configuration into one metrics-off/QAD-off/Everyone
  `relay.toml` on port 38451.
- Replace all deployment variants with the minimal `compose.yaml` from the
  design and delete direct TLS/ACME/QAD, env, digest validator, and duplicate
  reverse-proxy artifacts.
- Set both the Compose project and its single explicit container name to
  `zterm-relay`; tests must reject drift in either name.
- Remove deleted artifacts from `.dockerignore` and source-policy/static
  assertions.

## 3. Reduce tests to distinct contracts

- Keep upstream checksum/tamper, dual-architecture execution, runtime health,
  Compose exposure, public handshake, workspace lockstep, dependency, and
  secret gates.
- Rewrite/fold Relay tests so each contract has one owner; delete metrics,
  Docker health-state, direct-production config, digest validator, and rollback
  expectations.
- Assert the minimal Compose shape without adding a second deployment model or
  image override mechanism solely for tests.

## 4. Update requirements, docs, and project guidelines

- Update current parent PRD/design/implementation language for `latest`, port
  38451 only, one post-deploy acceptance, and reverse-proxy-only self-hosting.
- Preserve the archived/history evidence as historical, adding a current-state
  correction where necessary instead of deleting facts.
- Rewrite the Relay deployment spec and create one general evidence-driven
  simplicity guide; link it from the guide index.
- Update README/Relay/development/verification documentation and remove stale
  commands/files/9090/digest/rollback gates.

## 5. Local implementation and independent review gates

Implementation runs before any GitHub Release or server mutation:

```bash
sh tests/source-policy.sh
sh tests/workspace-version.sh
cargo +1.98.0 fmt --all -- --check
cargo +1.98.0 clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.98.0 test --workspace --all-features
cargo +1.98.0 doc --workspace --no-deps
cargo +1.98.0 deny check
cargo +1.98.0 deny --manifest-path tests/relay/handshake-probe/Cargo.toml \
  --config tests/relay/handshake-probe/deny.toml check
sh -n deploy/relay/*.sh tests/relay/*.sh
sh tests/relay/publication-channels.sh
sh tests/relay/static.sh
sh tests/relay/verify-upstream.sh
sh tests/relay/build-platforms.sh
sh tests/relay/smoke.sh
sh tests/relay/secret-scan.sh
docker compose -f deploy/relay/compose.yaml config --quiet
git diff --check
```

- Dispatch the Trellis implementer for repository changes, then an independent
  Trellis checker for specification drift, unnecessary remaining complexity,
  exact file deletion, cross-platform shell behavior, and the full gate.
- The main session updates specs, commits/pushes reviewed code, and waits for
  Linux/macOS/Windows plus Relay/dependency CI to pass.

## 6. Publish and migrate only after green code

- Create stable GitHub Release `v0.1.1` at the reviewed green commit.
- Verify the release workflow publishes public multi-platform `:v0.1.1` and
  `:latest` with the same image; do not create or validate a deployment digest.
- Read-only preflight the selected server, stage only the two reviewed runtime
  files without overwriting the old Compose before teardown, and retain
  existing historical backups without creating a rollback exercise.
- Run the old Compose `down` once to remove its stateless container/network,
  then run the new `zterm-relay` project's explicit `pull` and `up -d`. This is
  the only migration-specific `down`; routine updates do not repeat it.
- Verify project/container name `zterm-relay`, `:latest`, Docker `local`
  logging, loopback-only 38451, host health, public health/204, and one
  authenticated Iroh handshake. Stop after success.
- Record real release/server evidence, commit/push documentation, wait for final
  CI, archive this child task, and record the session journal.

## Risk and failure handling

- Never mutate or delete `v0.1.0`, old images, server backups, or unrelated
  1Panel projects.
- Do not change OpenResty, Cloudflare, firewall, or Docker daemon defaults.
- A failed local/CI/release gate stops before server mutation.
- A failed server acceptance is reported with the observed state; do not run an
  automatic fallback or rehearse recovery after success.
