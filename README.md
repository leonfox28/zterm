# zterm

zterm is a new cross-platform remote-terminal project. The intended product
will prefer direct NAT-traversed connections and fall back to an end-to-end
encrypted Iroh relay path, while keeping remote terminal sessions alive across
client disconnects.

The repository is currently at **Phase Zero**. It contains only a reproducible
Rust workspace, a build-only protobuf probe, and a verified Docker wrapper for
the official `iroh-relay` 1.0.3 binary. There is no usable terminal, daemon,
pairing, transport, or session behavior yet; the `zterm` binary is deliberately
a side-effect-free placeholder.

## Version policy

zterm uses one lockstep SemVer for the product rather than independent
component versions. The root `[workspace.package].version` is the source for
all product crates and is currently `0.1.0`; future CLI, daemon, desktop/mobile
apps, protocol artifacts, and the zterm Relay wrapper advance together.

A stable GitHub Release tag must be canonical `vMAJOR.MINOR.PATCH`, exactly
match the workspace version after removing `v`, and publishes Relay image tags
`MAJOR.MINOR.PATCH` plus `latest`. A prerelease uses
`vMAJOR.MINOR.PATCH-PRERELEASE`, must exactly match a prerelease workspace
version after removing `v`, and publishes only the corresponding v-less tag to
the development package. SemVer build metadata is rejected because `+` is not
a portable OCI tag character and dropping it would make release identities
ambiguous. Internal validation tools such as the isolated Relay handshake
probe are not product deliverables and keep their own non-product version.

## Repository boundaries

- `crates/` — five minimal Rust crates proving the planned dependency direction.
- `proto/` — a build-only schema compiled with vendored `protoc` binaries.
- `deploy/relay/` — official relay artifact verification and Compose deployment.
- `.github/workflows/relay-image.yml` — multi-platform publisher for the
  separate `zterm-relay` production and `zterm-relay-dev` development GHCR
  packages.
- `tests/relay/` — architecture, checksum, configuration, health, and metrics checks.
- `docs/development.md` — exact local toolchain baseline and repeatable commands.
- `docs/relay.md` — relay trust boundary, deployment checkpoint, and rollback.
- `docs/phase-zero-verification.md` — evidence from the completed local gate.

## Local checks

```bash
sh tests/source-policy.sh
sh tests/workspace-version.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps
cargo deny check
sh tests/relay/static.sh
sh tests/relay/verify-upstream.sh
sh tests/relay/build-platforms.sh
sh tests/relay/smoke.sh
sh tests/relay/production-config-smoke.sh
sh tests/relay/reverse-proxy-smoke.sh
sh tests/relay/secret-scan.sh
```

Do not connect this repository to a public server until the Phase Zero local
checks pass and the user explicitly provides the server entry point, relay
domain/DNS status, and Docker status. Never place SSH private keys or real
deployment credentials in this repository.
