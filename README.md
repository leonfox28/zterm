# zterm

zterm is a new cross-platform remote-terminal project. The intended product
will prefer direct NAT-traversed connections and fall back to an end-to-end
encrypted Iroh relay path, while keeping remote terminal sessions alive across
client disconnects.

Phase Zero is complete. The repository currently contains a reproducible Rust
workspace, a build-only protobuf probe, and a verified Docker wrapper for the
official `iroh-relay` 1.0.3 binary. There is no usable terminal, daemon,
pairing, transport, or session behavior yet; the `zterm` binary is deliberately
a side-effect-free placeholder while Phase One begins.

## Version policy

zterm uses one lockstep SemVer for the product rather than independent
component versions. The root `[workspace.package].version` is the source for
all product crates and is currently `0.1.1`; future CLI, daemon, desktop/mobile
apps, protocol artifacts, and the zterm Relay wrapper advance together.

A GitHub Release tag must equal `v` plus Cargo's resolved workspace version.
The same tag is used unchanged for the versioned GHCR image, so release
`v0.1.1` publishes `zterm-relay:v0.1.1` and the stable `latest` alias. GitHub
prereleases and manual builds publish only to `zterm-relay-dev`; manual tags are
used unchanged except that `latest` is reserved. Internal validation tools such
as the isolated Relay handshake probe are not product deliverables and keep
their own non-product version.

## Repository boundaries

- `crates/` — five minimal Rust crates proving the planned dependency direction.
- `proto/` — a build-only schema compiled with vendored `protoc` binaries.
- `deploy/relay/` — official relay artifact verification and Compose deployment.
- `.github/workflows/relay-image.yml` — multi-platform publisher for the
  separate `zterm-relay` production and `zterm-relay-dev` development GHCR
  packages.
- `tests/relay/` — architecture, checksum, minimal configuration, and health checks.
- `docs/development.md` — exact local toolchain baseline and repeatable commands.
- `docs/relay.md` — relay trust boundary, publication, and deployment.
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
sh tests/relay/publication-channels.sh
sh tests/relay/static.sh
sh tests/relay/verify-upstream.sh
sh tests/relay/build-platforms.sh
sh tests/relay/smoke.sh
sh tests/relay/secret-scan.sh
```

Do not connect this repository to a public server until the Phase Zero local
checks pass and the user explicitly provides the server entry point, relay
domain/DNS status, and Docker status. Never place SSH private keys or real
deployment credentials in this repository.
