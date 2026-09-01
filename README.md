# zterm

zterm is a cross-platform remote-terminal project built around daemon-lifetime
terminal Sessions and end-to-end encrypted Iroh connections. The current tree
contains the shared terminal/domain model, versioned protobuf contract, secure
per-user state, one same-UID daemon, pairing and directional authorization, a
daemon-owned connection broker, local and remote Session adapters, and the
public raw-terminal CLI.

This is still a development build. The Linux real-Iroh remote Session target
exists but its hosted runtime result and public multi-process CLI evidence are
pending; macOS development runs compile that target without executing it.
Windows is not a terminal runtime in this milestone.

## Install

Install the latest stable native Release with the fixed bootstrap URL:

```bash
curl -fsSL https://raw.githubusercontent.com/leonfox28/zterm/main/install/install.sh | sh
```

The installer does not run `setup` or start the daemon. See
[Install, update, and uninstall](docs/install.md) for exact-version,
review-first, update, uninstall, and recovery paths.

Current public commands are:

```text
zterm setup [--name <name>] [--profile <official-n0|self-hosted>]
            [--relay-url <https-url>]
zterm status [--json]
zterm doctor [--json]
zterm pair create [--ttl <duration-with-s|m|h-suffix>]
zterm pair accept [--stdin] [--name <alias>]
zterm device list [--json]
zterm device rename <device> <alias>
zterm device revoke <device> [--yes]
zterm connect <device|local> [--session <name-or-id>] [--takeover]
              [--escape <ctrl-@..ctrl-_|ctrl-?|none>]
zterm session list <device|local> [--json]
zterm session new <device|local> <name> [--cwd <host-path>]
                   [--escape <ctrl-@..ctrl-_|ctrl-?|none>]
zterm session attach <device|local> <session> [--takeover]
                      [--escape <ctrl-@..ctrl-_|ctrl-?|none>]
zterm session rename <device|local> <session> <new-name>
zterm session close <device|local> <session> [--yes]
zterm daemon status [--json]
zterm daemon stop [--force]
zterm daemon restart [--force]
zterm logs [--lines <n>]
zterm reset --identity [--yes] [--force]
zterm update [--version <vSEMVER>] [--force]
zterm uninstall [--yes] [--force]
```

`setup` and `daemon restart` explicitly start the daemon. Pair, device,
connect, and Session commands start it on demand only after validating an
existing setup. Inspection, logs, stop, help/version, and parse failures never
start it. With setup complete, bare `zterm` is equivalent to
`zterm connect local --session main`; before setup it only prints setup
guidance.

See [Remote sessions and the public CLI](docs/remote-cli.md) for the exact
command, target, pairing, reconnect, takeover, raw-terminal, ambiguity, and
identity-reset contracts. [Persistent session engine](docs/persistent-sessions.md)
documents the daemon-lifetime PTY and resource boundaries, while
[Core and local daemon](docs/core-local-daemon.md) documents local state and
lifecycle ownership. [Install, update, and uninstall](docs/install.md) covers
the signed native Release path and its trust/recovery boundaries.

## Version policy

zterm uses one lockstep SemVer for the product rather than independent
component versions. The root `[workspace.package].version` is the source for
all product crates and is currently `0.1.9`; future CLI, daemon, desktop/mobile
apps, protocol artifacts, and the zterm Relay wrapper advance together.

A GitHub Release tag must equal `v` plus Cargo's resolved workspace version.
The same tag is used unchanged for the versioned GHCR image, so release
`v0.1.9` publishes `zterm-relay:v0.1.9` and the stable `latest` alias. GitHub
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
- `install/` and `tools/release/` — the reviewed bootstrap/template and private
  signed native-asset assembler.
- `.github/workflows/release.yml` — protected exact-tag native Release workflow
  for the four supported Unix targets.
- `docs/development.md` — exact local toolchain baseline and repeatable commands.
- `docs/relay.md` — relay trust boundary, publication, and deployment.
- `docs/phase-zero-verification.md` — evidence from the completed local gate.
- `docs/core-local-daemon.md` — current M2–M3 behavior, state, CLI, and exclusions.
- `docs/persistent-sessions.md` — M4 daemon-lifetime session and local attachment contracts.
- `docs/remote-cli.md` — M7–M8 public commands, directional trust, remote attachment, and evidence boundaries.
- `docs/install.md` — signed installer, explicit update/uninstall, recovery,
  and release-operator checkpoints.

## Local checks

```bash
sh tests/source-policy.sh
sh tests/workspace-version.sh
sh tests/release/static.sh
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

Local readiness and status do not initiate or wait on Iroh, DNS, Relay, or
Internet work; a running daemon owns its Endpoint separately. Never place SSH
private keys, identity keys, or real deployment credentials in this repository.
