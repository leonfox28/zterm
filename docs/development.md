# Phase Zero development environment

## Exact project baseline

The project toolchain is fixed by `rust-toolchain.toml`; it does not follow the
floating `stable` channel. The initial Apple Silicon development machine was
verified on 2026-08-21 with:

| Component | Verified value |
| --- | --- |
| Host | macOS 26.6.2, arm64, 10 CPUs, 32 GiB RAM |
| Rustup | 1.29.0 |
| Rust / Cargo | 1.98.0 |
| Rust components | rustfmt, Clippy, rust-src |
| Rust targets | `aarch64-apple-darwin`, `aarch64-apple-ios`, `aarch64-linux-android` |
| Preserved user toolchain | `1.95.0-aarch64-apple-darwin` |
| Docker CLI | 29.7.2 |
| Docker Engine in Colima | 29.5.2 |
| Docker Compose | 5.5.0 |
| Docker Buildx | 0.36.1 (BuildKit 0.30.0 on the active Colima builder) |
| Colima | 0.10.3 |
| pkg-config / pkgconf | 3.0.5 |
| jq | 1.7.1 |
| cargo-deny | 0.20.2 |

Rust 1.95.0 is user-owned and must not be removed. A future Rust or cargo-deny
upgrade is an explicit repository change and must rerun every quality gate.

## Install missing tools on macOS

Always inspect installed versions before installing. Phase Zero used Homebrew's
Docker CLI and Compose with Colima; Docker Desktop and administrator-granted
startup services are not required.

```bash
brew install docker docker-compose docker-buildx colima pkg-config jq
cargo install cargo-deny --version 0.20.2 --locked
```

Homebrew's Compose plugin directory must be visible to the Docker CLI. Preserve
any existing Docker configuration while ensuring `~/.docker/config.json`
contains the following additional plugin path:

```json
{
  "cliPluginsExtraDirs": [
    "/opt/homebrew/lib/docker/cli-plugins"
  ]
}
```

The initial Colima profile is deliberately manual and does not start at login:

```bash
colima start --cpu 4 --memory 6 --disk 40 --vm-type vz --runtime docker
docker version
docker compose version
docker run --rm hello-world
```

Use `colima stop` when the local VM is not needed. `colima start` reuses the
same profile and data on the next development session.

## Rust and protobuf

Opening the repository causes rustup to select exact Rust 1.98.0 and ensure the
declared components are installed. Platform targets can be restored without
changing the default toolchain:

```bash
rustup target add \
  aarch64-apple-darwin \
  aarch64-apple-ios \
  aarch64-linux-android \
  --toolchain 1.98.0-aarch64-apple-darwin
```

Do not install a global `protoc` for this project. `zterm-proto` uses the exact
`protoc-bin-vendored` dependency from `Cargo.lock`, then passes that executable
directly to `prost-build`.

## Workspace dependency direction

```text
zterm-core
   ^       ^
   |       |
 proto  platform
   ^       ^
    \     /
     daemon
        ^
        |
       cli
```

These crates only prove compilation and code-generation boundaries. Phase Zero
must not add PTY, terminal state, daemon lifecycle, configuration, device
identity, pairing, networking, or persistent session behavior.

## Quality gate

```bash
sh tests/source-policy.sh
cargo +1.98.0 fmt --all -- --check
cargo +1.98.0 clippy --workspace --all-targets --all-features -- -D warnings
cargo +1.98.0 test --workspace --all-features
cargo +1.98.0 doc --workspace --no-deps
cargo deny check
cargo run --quiet --package zterm-cli
```

The placeholder CLI only prints static build metadata and must not create
`~/.zterm`, start a process, or open a network connection.
