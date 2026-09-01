# Phase Zero verification record

> **Historical record — not a current runbook.** This page preserves the
> commands and evidence used at the time. Use [Development and CI](development.md)
> and [Release operations](releasing.md) for the current workflow.

Date: 2026-08-21  
Host: macOS 26.6.2 arm64, Colima 0.10.3 (`vz`, Docker runtime)  
Scope: historical local bootstrap plus the first approved public reverse-proxy
relay deployment.

## Historical contract recorded after Phase Zero

This document preserves what was actually tested and deployed during the
initial `v0.1.0` bootstrap. References below to the old digest-only Compose,
port 9090 metrics, custom health probe, direct TLS template, reconnect test, and
rollback drills are historical evidence, not current requirements.

Starting with `v0.1.1`, the supported deployment is only
`deploy/relay/compose.yaml` plus `relay.toml`. It uses
`ghcr.io/leonfox28/zterm-relay:latest`, publishes only
`127.0.0.1:38451`, disables metrics and QAD, and updates manually with
`docker compose pull` followed by `docker compose up -d`. A release is accepted
once through host/public health and one authenticated Iroh handshake; successful
deployments are not restarted or rolled back again for rehearsal.

## v0.1.1 simplification verification

The simplified contract was released and deployed on 2026-08-21:

- commit `c2b574d` passed the complete [cross-platform and Relay CI run](https://github.com/leonfox28/zterm/actions/runs/32470094911);
- [GitHub Release v0.1.1](https://github.com/leonfox28/zterm/releases/tag/v0.1.1) completed the [production image workflow](https://github.com/leonfox28/zterm/actions/runs/32470318480);
- `ghcr.io/leonfox28/zterm-relay:v0.1.1` and `:latest` resolved to the same
  two-platform `linux/amd64` + `linux/arm64` image, with no extra attestation
  manifests;
- the selected server migrated once from the old reverse-proxy project to a
  single project/container named `zterm-relay`; the old active files were moved
  into `legacy-v0.1.0/` rather than deleted;
- the live container uses literal `:latest`, UID/GID 65532, Docker's `local`
  log driver, `restart: unless-stopped`, and only host
  `127.0.0.1:38451`; the old 9090 listener is absent;
- host health, public health, public `/generate_204`, and one authenticated
  Iroh Relay handshake passed. Validation then stopped without a restart,
  reconnect loop, image switch, or rollback drill.

Creating the release tag also exposed that the generic CI workflow responded
to tag pushes after the same commit had already passed on `main`. The duplicate
run was cancelled, and commit `92dda0e` restricted that workflow to branch
pushes while preserving pull-request and manual triggers; its [CI run](https://github.com/leonfox28/zterm/actions/runs/32471101894) passed.

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
sh tests/workspace-version.sh
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

- all five workspace crates compiled at the shared product version `0.1.0`;
  four unit tests and all doc tests passed;
- the CLI printed only static Phase Zero metadata and reported version `0.1.0`;
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
  prerelease/manual tag matrix, lockstep workspace/release SemVer matching,
  v-prefixed Git tag to v-less OCI tag mapping, build-metadata rejection,
  provenance/SBOM, the `deploy/relay` context, and one linux/amd64 + linux/arm64
  manifest per run; both production Compose
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

These local IDs are not registry digests. During the `v0.1.0` bootstrap, the
repository required the production image to be published by
`.github/workflows/relay-image.yml` and deployed with that run's exact GHCR
multi-platform digest. Those requirements and IDs are historical bootstrap and
rollback evidence only; the first real production digest is recorded below and
was not inferred from `zterm-relay-dev`.

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

## GHCR production publication evidence

[Zterm v0.1.0](https://github.com/leonfox28/zterm/releases/tag/v0.1.0) points
to reviewed commit `b9cac371ad7a91647a7fd1e6690230f4cf1d8c35`. Its complete
[main CI run](https://github.com/leonfox28/zterm/actions/runs/32461793456)
passed before the Release was created. Release-triggered workflow
[`32462016566`](https://github.com/leonfox28/zterm/actions/runs/32462016566)
published only `ghcr.io/leonfox28/zterm-relay:0.1.0` and `:latest`; both tags
resolve to immutable OCI index digest:

```text
sha256:c3ebd4398814aa7cfe21c145d277645f2e362b67965330500d52d1ce4c9e2da3
```

The index contains linux/amd64 runtime manifest
`sha256:2495b15529589ed428b73830857dbb5691eb00d66352e487478e795c7c14a901`
and linux/arm64 runtime manifest
`sha256:a6d7348b28473967a320b1c20f8cf02145ebb9723aceca03d8282f68c5fcc4df`,
plus one provenance/SBOM attestation manifest for each runtime platform. An
anonymous GHCR bearer request returned HTTP 200 for the production index.
Containers selected from both runtime platforms reported `iroh-relay 1.0.3`,
and the existing development tag remained at its independent digest.

## Public reverse-proxy deployment evidence

The selected server already terminated public TLS in same-host OpenResty and
routed `relay.zenithconsulting.cn` to `http://127.0.0.1:38451`. The repository
therefore included `compose.reverse-proxy.yaml`; its default host bindings were
exactly `127.0.0.1:38451` for Relay HTTP and `127.0.0.1:9090` for metrics.

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
deleted.

Before the first production switch, a read-only preflight found that the live
bootstrap Compose still contained its temporary local `build` fallback and did
not yet have `validate-image-reference.sh`. The healthy bootstrap container was
left running while the repository's digest-only Compose, env template, and
validator were staged and checksum-verified. The old Compose, `.env`, and env
template were preserved with suffix `pre-v0.1.0-20260821T0823Z`; the original
arm64 image and archive were also retained. No other 1Panel Compose project was
changed.

The server then anonymously pulled the exact production digest above and a
one-shot activation replaced only the zterm Relay container. That Compose had
no `build` section, its `.env` pinned the production package by digest, and
Docker recorded the container's `Config.Image` as that same immutable
reference. The recreated linux/arm64 container became healthy as UID/GID 65532.
The transient pull and activation jobs both exited successfully and did not
install a persistent system service.

A final production rollback drill then recreated the same Compose service from
the retained immutable bootstrap image
`sha256:26a76c210d87462e88026b31f388c92bcce8a586dafbe6824221b32493a352b5`
with `--no-build`. That rollback became healthy and passed loopback health,
private metrics, listener, and public authenticated Iroh-handshake checks. A
second one-shot job restored the exact production digest, which again passed
all of those checks. The active `.env` remained pinned to the production
digest throughout; neither transient job installed a persistent service.

The transferred archive was 4,505,088 bytes with SHA-256
`b4a3fd2055a2a275ae588ce0486eee66afb4e98418a790b5ff619d9583a7ff5e`.
After `docker load`, its linux/arm64 image ID exactly matched the locally
verified `sha256:26a76c210d87462e88026b31f388c92bcce8a586dafbe6824221b32493a352b5`.
The bootstrap Compose was rendered against that immutable ID and started with
`--no-build`; it is no longer the active production image.

Evidence collected across the initial bootstrap deployment, its recovery
drill, and the later production digest activation confirmed:

- Docker health `healthy`, UID/GID 65532, read-only root filesystem, no
  privileges, all Linux capabilities dropped, and `no-new-privileges`;
- the activated container used
  `ghcr.io/leonfox28/zterm-relay@sha256:c3ebd4398814aa7cfe21c145d277645f2e362b67965330500d52d1ce4c9e2da3`
  with no server-side build fallback;
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
  handshake through the public URL, including after the production digest
  switch;
- during bootstrap verification, while the same probe process remained active,
  a controlled relay restart was observed as disconnected and then
  automatically reconnected with a new authenticated relay connection;
- before the production switch, a full bootstrap Compose down/up recovery drill
  removed only this stateless container and its network, retained the pinned
  image/config/archive, recreated a healthy service with `--no-build`, and
  passed the public authenticated handshake again;
- after the production switch, an explicit immutable-image rollback recreated
  the service from the retained bootstrap image, passed health, metrics,
  loopback-listener, and public authenticated-handshake checks, then restored
  the exact production digest and passed those checks again.

The selected mode deliberately disables QAD and exposes no UDP port. This is a
complete encrypted relay fallback and is independent of direct NAT traversal.
QAD is only an optional observed-address source that may improve direct-path
success. Phase 1 will measure real NAT traversal with QAD disabled using
direct/relay path events; only that evidence may trigger a separately approved
manual-certificate/QAD deployment and client configuration.
