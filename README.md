# zterm

zterm is a new cross-platform remote-terminal project. The intended product
will prefer direct NAT-traversed connections and fall back to an end-to-end
encrypted Iroh relay path, while keeping remote terminal sessions alive across
client disconnects.

Phase Zero and the Phase One Foundation/Core/Local-Daemon milestones are
complete. The repository now contains the shared terminal/domain model,
versioned protobuf contract, secure per-user state, one same-UID local daemon,
and the setup/status/diagnostic lifecycle CLI. There is still no session
registry, local terminal attach, remote pairing, or bound Iroh endpoint; a
usable terminal begins with the next Phase One session milestone.

Current public commands are:

```text
zterm setup --name <name> --profile official-n0
zterm status [--json]
zterm doctor [--json]
zterm daemon status [--json]
zterm daemon stop [--force]
zterm daemon restart [--force]
zterm logs [--lines <n>]
```

Only `setup` and `daemon restart` may start the daemon. Inspection and stop
commands never start it. See [Core and local daemon](docs/core-local-daemon.md)
for exact behavior and current limits.

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

- `crates/` — five Rust crates owning core, protocol, platform, daemon, and CLI boundaries.
- `proto/zterm/v1/` — the product wire schema compiled with vendored `protoc` binaries.
- `deploy/relay/` — official relay artifact verification and Compose deployment.
- `.github/workflows/relay-image.yml` — multi-platform publisher for the
  separate `zterm-relay` production and `zterm-relay-dev` development GHCR
  packages.
- `tests/relay/` — architecture, checksum, minimal configuration, and health checks.
- `docs/development.md` — exact local toolchain baseline and repeatable commands.
- `docs/relay.md` — relay trust boundary, publication, and deployment.
- `docs/phase-zero-verification.md` — evidence from the completed local gate.
- `docs/core-local-daemon.md` — current M2–M3 behavior, state, CLI, and exclusions.

## Local checks

```bash
sh tests/source-policy.sh
sh tests/workspace-version.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps
cargo deny check
sh tests/core-local-daemon/cross-uid.sh
sh tests/relay/publication-channels.sh
sh tests/relay/static.sh
sh tests/relay/verify-upstream.sh
sh tests/relay/build-platforms.sh
sh tests/relay/smoke.sh
sh tests/secret-scan.sh
```

The local daemon/readiness path deliberately does not bind Iroh or require DNS,
Relay, or Internet access. Never place SSH private keys, identity keys, or real
deployment credentials in this repository.
