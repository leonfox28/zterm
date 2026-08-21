# Phase Zero verification record

Date: 2026-08-21  
Host: macOS 26.6.2 arm64, Colima 0.10.3 (`vz`, Docker runtime)  
Scope: local bootstrap plus the approved public reverse-proxy relay deployment.

## Environment

- Rust/Cargo 1.98.0 selected exactly; rustfmt, Clippy, rust-src and the macOS,
  iOS, and Android arm64 targets are installed.
- The pre-existing `1.95.0-aarch64-apple-darwin` toolchain remains installed.
- Docker CLI 29.7.2, Docker Engine 29.5.2, Compose 5.5.0, Buildx 0.36.1,
  BuildKit 0.30.0, Colima 0.10.3, pkgconf 3.0.5, jq 1.7.1, and cargo-deny
  0.20.2 passed.
- Colima uses 4 CPUs, 6 GiB RAM, 40 GiB disk and advertises linux/amd64,
  linux/arm64, and linux/386. It is manually started and has no autostart.

## Commands passed

```text
sh tests/source-policy.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --no-deps
cargo deny check
cargo run --quiet --package zterm-cli
cargo fmt --manifest-path tests/relay/handshake-probe/Cargo.toml -- --check
cargo clippy --locked --manifest-path tests/relay/handshake-probe/Cargo.toml -- -D warnings
cargo deny --manifest-path tests/relay/handshake-probe/Cargo.toml --config tests/relay/handshake-probe/deny.toml check
actionlint .github/workflows/*.yml
sh -n deploy/relay/*.sh tests/relay/*.sh
dash tests/relay/publication-channels.sh
sh tests/relay/publication-channels.sh
sh tests/relay/static.sh
sh tests/relay/verify-upstream.sh
sh tests/relay/build-platforms.sh
sh tests/relay/smoke.sh
sh tests/relay/production-config-smoke.sh
sh tests/relay/reverse-proxy-smoke.sh
sh tests/relay/secret-scan.sh
sh tests/relay/public-handshake.sh https://relay.zenithconsulting.cn
```

Results:

- all five workspace crates compiled; four unit tests and all doc tests passed;
- the CLI printed only static Phase Zero metadata;
- cargo-deny reported advisories, bans, licenses, and sources as passing (with
  one allowed duplicate-version warning in the `prost-build` dependency tree);
- the isolated public-handshake probe also passed its native macOS/Linux/Windows
  dependency policy. Its dedicated policy records the upstream Iroh 1.0.3
  `CDLA-Permissive-2.0` root-certificate exception and the transitive
  unmaintained (not vulnerable) `paste` advisory; the main workspace policy was
  not relaxed;
- both official upstream archives matched their release SHA-256 values;
- unknown architecture and tampered-checksum paths failed as required;
- BuildKit built and executed Iroh 1.0.3 plus the static one-shot health probe on
  linux/amd64 and linux/arm64;
- local Compose became healthy through an in-container HTTP `/healthz` probe;
  the endpoint returned version 1.0.3, metrics were reachable only through host
  loopback, UID/GID was 65532, the root filesystem was read-only, Docker log
  rotation was 10 MiB times five files, and `SIGINT` shutdown exited cleanly;
- the production TLS/ACME TOML was accepted by Iroh 1.0.3 with container
  networking disabled; a live port-80 probe, low-port non-root binding, and the
  scratch-image CA bundle passed without any possible public ACME request;
- the same-host reverse-proxy production TOML was accepted by Iroh 1.0.3
  without `--dev`; runtime inspection found only ephemeral host-loopback
  mappings to container TCP 38451/9090 and no TCP 80/443 or UDP mapping;
  Compose used the prebuilt image by immutable local image ID with
  `--no-build`, and the running image ID matched that digest;
  `/healthz`, `/generate_204`, private metrics, and the `/relay` WebSocket
  `101` upgrade all passed; the same binary rejected a deliberately invalid
  QAD-without-TLS config with its documented TLS-required error;
- metrics are hard-bound to host `127.0.0.1`, and the deterministic repository-
  wide secret scan passed;
- the relay publication workflow passed actionlint 1.7.12 and repository static
  checks for exact current Action SHAs, least privileges, separate production
  (`zterm-relay`) and development (`zterm-relay-dev`) packages, the stable/
  prerelease/manual tag matrix, provenance/SBOM, the `deploy/relay` context,
  and one linux/amd64 + linux/arm64 manifest per run; both production Compose
  files require an image and contain no server-side `build` fallback, while
  the mandatory production preflight rejects the development package. The
  resolver also rejected empty, malformed,
  overlength, newline, carriage-return, and owner/path-injection inputs without
  writing partial GitHub outputs, under macOS `sh`, Debian `dash`, and Alpine
  `ash` on both linux/amd64 and linux/arm64;
- repeated builds with local provenance attestations disabled produced the same
  single-platform content IDs recorded below.

## Local image evidence

These locally loaded content digests are evidence for this run. The selected
arm64 image was loaded on the server only as the initial bootstrap image after
the remote `docker load` result matched the exact local image ID:

| Image | Platform | Local content digest |
| --- | --- | --- |
| `zterm/iroh-relay:phase-zero-amd64` | linux/amd64 | `sha256:89b756c5e4a29a28cd0cfd1c4f4322106807363d7f9a701545d57a0aa069c7bd` |
| `zterm/iroh-relay:phase-zero-arm64` | linux/arm64 | `sha256:26a76c210d87462e88026b31f388c92bcce8a586dafbe6824221b32493a352b5` |
| `zterm/iroh-relay:1.0.3-local` | linux/arm64 | `sha256:26a76c210d87462e88026b31f388c92bcce8a586dafbe6824221b32493a352b5` |

These local IDs are not registry digests and are not valid references for a new
production release. The repository now requires future production images to be
published by `.github/workflows/relay-image.yml` and deployed with the exact
GHCR multi-platform digest emitted by that run. No production digest has been
recorded yet because no stable-release publication has completed; it must not
be inferred from a mutable tag, copied from `zterm-relay-dev`, or invented in
advance.

## GHCR development publication evidence

The public repository `leonfox28/zterm` was initialized at commit `43b06ff`.
Manual workflow run
[`32456229109`](https://github.com/leonfox28/zterm/actions/runs/32456229109)
published `ghcr.io/leonfox28/zterm-relay-dev:phase-zero` with immutable index
digest:

```text
sha256:8f2bb338ca2d3841ebd8e4cd270b9aba919880dc12e5e5a24050e511442ecb40
```

The OCI index contains linux/amd64 manifest
`sha256:2905164843eca0993d00aa4fc310413149dccedfb324756113f8bd16cd31c7dc`
and linux/arm64 manifest
`sha256:5a468ca66d333d4536721e10a2ef285589f5c716c2bcda756b0231409f5ffed4`,
plus provenance/SBOM attestation manifests. An anonymous GHCR bearer request
returned HTTP 200 for the index, and containers from both runtime platforms
reported `iroh-relay 1.0.3`. This proves the development publication channel;
it is not a production deployment reference and is rejected by the production
preflight.

## Public reverse-proxy deployment evidence

The selected server already terminates public TLS in same-host OpenResty and
routes `relay.zenithconsulting.cn` to `http://127.0.0.1:38451`. The repository
therefore now includes `compose.reverse-proxy.yaml`; its default host bindings
are exactly `127.0.0.1:38451` for Relay HTTP and `127.0.0.1:9090` for metrics.

The user-authorized read-only preflight found Ubuntu 24.04.4 on linux/arm64,
Docker 29.3.0, Compose 5.1.0, and no initial `/opt/zterm-relay` directory or
38451/9090 listener. Existing OpenResty, Docker workloads, and firewall policy
were left unchanged during bootstrap.

The runtime Compose project has since been moved into the server's established
1Panel custom-Compose root at
`/opt/1panel/docker/compose/zterm-relay`. Container Compose labels resolve to
that new working directory/config, and the authenticated public Iroh handshake
passed again after migration. The previous source directory was recoverably
renamed to `/opt/zterm-relay.pre-1panel-20260821` for rollback rather than
deleted. The running image is still the verified local arm64 bootstrap image;
switching it to GHCR remains pending the first stable production publication.
The verified development digest is intentionally not accepted for that switch.

The transferred archive was 4,505,088 bytes with SHA-256
`b4a3fd2055a2a275ae588ce0486eee66afb4e98418a790b5ff619d9583a7ff5e`.
After `docker load`, its linux/arm64 image ID exactly matched the locally
verified `sha256:26a76c210d87462e88026b31f388c92bcce8a586dafbe6824221b32493a352b5`.
Compose was rendered against that immutable ID and started with `--no-build`.

Runtime inspection after both initial deployment and recovery confirmed:

- Docker health `healthy`, UID/GID 65532, read-only root filesystem, no
  privileges, all Linux capabilities dropped, and `no-new-privileges`;
- only host `127.0.0.1:38451/tcp` and `127.0.0.1:9090/tcp` are published;
  there is no UDP 7842 listener or firewall rule;
- `/healthz` reports Iroh 1.0.3, `/generate_204` returns 204, private metrics
  are available only on loopback, and startup logs contain no error;
- public TLS is valid for `zenithconsulting.cn`, issued by Let's Encrypt YE2
  from 2026-07-13 through 2026-10-11, with certificate SHA-256 fingerprint
  `5A:F8:25:F9:51:0A:A1:2B:AF:45:D2:D0:DC:9A:F3:77:86:C9:BF:E1:3B:7F:35:0F:9D:4F:C5:5A:52:B1:FF:D3`;
- the Cloudflare/OpenResty path returns public HTTP 200 health, public 204, and
  a `/relay` WebSocket 101 preserving `iroh-relay-v2`;
- the pinned Iroh 1.0.3 acceptance probe, using an ephemeral endpoint identity
  and `RelayConfig::new(url, None)`, completed the authenticated relay
  handshake through the public URL;
- while the same probe process remained active, a controlled relay restart was
  observed as disconnected and then automatically reconnected with a new
  authenticated relay connection;
- a full Compose down/up drill removed only this stateless container and its
  network, retained the pinned image/config/archive, recreated a healthy
  service with `--no-build`, and passed the public authenticated handshake
  again.

The selected mode deliberately disables QAD and exposes no UDP port. This is a
complete encrypted relay fallback and is independent of direct NAT traversal.
QAD is only an optional observed-address source that may improve direct-path
success. Phase 1 will measure real NAT traversal with QAD disabled using
direct/relay path events; only that evidence may trigger a separately approved
manual-certificate/QAD deployment and client configuration.
