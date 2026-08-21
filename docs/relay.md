# Optional self-hosted Iroh Relay

## Product boundary

Phase One endpoints currently use Iroh's official n0 production infrastructure
by default. This document covers the separately operable, optional self-hosted
Relay; it is not silently added to that default profile.

zterm does not implement or fork a Relay data plane. Its optional image
downloads the official Iroh 1.0.3 Linux musl artifact, selects it from
BuildKit's `TARGETARCH`, verifies the upstream SHA-256, and copies the binary
and Alpine CA bundle into a shell-free `scratch` runtime. The process runs as
numeric UID/GID 65532.

Canonical upstream metadata lives in `deploy/relay/artifact.sh`:

| Docker architecture | Official target | SHA-256 |
| --- | --- | --- |
| `amd64` | `x86_64-unknown-linux-musl` | `9e25e394c6d09b449d86bb222de535d2a6e68de8030ee8ef39f682ab6ff0cd2c` |
| `arm64` | `aarch64-unknown-linux-musl` | `331a2f35519a778a5b0a2a34baa7f495d3540b3cdb549b8203cbfdd209df7641` |

These values match the official
[Iroh v1.0.3 release](https://github.com/n0-computer/iroh/releases/tag/v1.0.3).
The checksum is verified at the artifact-download boundary. Deployment does
not add another image digest parser; Docker and GHCR already transfer images by
content address.

The Relay forwards zterm's future end-to-end encrypted packets and stores no
terminal state, device key, session, or business record. Like any network
forwarder, it can still observe connection metadata such as source addresses,
endpoint identifiers, timing, and encrypted traffic sizes.

## Supported deployment

There is one supported configuration for the optional self-hosted Relay:

- OpenResty or another same-host reverse proxy owns public TLS;
- Iroh serves plain HTTP inside the container on TCP 38451;
- Docker publishes that listener only as `127.0.0.1:38451`;
- QAD and metrics are disabled;
- Relay access is `everyone`, with no zterm-specific limits;
- Docker's rotating and compressed `local` log driver bounds logs.

The complete deployment consists of only:

- `deploy/relay/compose.yaml`;
- `deploy/relay/relay.toml`.

Compose project and container are both named `zterm-relay`. The image supplies
its own default `--config-path /etc/iroh-relay/relay.toml` command, so Compose
does not repeat it. There is no server-side build, `.env` image indirection,
container healthcheck, custom probe, metrics port, certificate volume, monitor,
or direct TLS/ACME/QAD variant.

## Local verification

Run each contract once at its owning boundary:

```bash
sh tests/relay/publication-channels.sh
sh tests/relay/static.sh
sh tests/relay/verify-upstream.sh
sh tests/relay/build-platforms.sh
sh tests/relay/smoke.sh
sh tests/relay/secret-scan.sh
```

- `verify-upstream.sh` downloads and verifies both official archives.
- `build-platforms.sh` builds and executes the amd64 and arm64 images.
- `static.sh` checks the minimal image, publication workflow, configuration,
  and rendered Compose contract.
- `smoke.sh` reuses the native-platform image built by `build-platforms.sh`,
  then verifies `/healthz` and `/generate_204` without rebuilding it or
  introducing a second Compose shape.

The public acceptance probe creates an ephemeral Iroh endpoint identity and
succeeds only when the requested Relay reports an authenticated connection:

```bash
sh tests/relay/public-handshake.sh https://relay.example.com
```

It passes `RelayConfig::new(url, None)` because this deployment has no UDP QAD
endpoint. It performs one connection attempt; restart/reconnect and rollback
exercises are not part of normal release acceptance.

## Release mapping

The root Cargo workspace version is the product version source. The publisher
asks Cargo for the inherited `zterm-core` package version and requires a GitHub
Release tag equal to `v${workspace_version}`. That tag is used unchanged as the
versioned OCI tag.

For workspace version `0.1.1`, a stable GitHub Release named `v0.1.1`
publishes one `linux/amd64` + `linux/arm64` image under:

```text
ghcr.io/leonfox28/zterm-relay:v0.1.1
ghcr.io/leonfox28/zterm-relay:latest
```

GitHub prereleases use the same direct tag spelling but publish only to
`ghcr.io/leonfox28/zterm-relay-dev`. Manual workflow runs also target only that
development package, use their valid OCI input tag unchanged, and cannot use
the reserved `latest` alias. The production and development packages are
separate so a manual build cannot replace the stable image.

The workflow checks out the exact Release tag, pins every third-party Action by
full commit SHA, has only `contents: read` and `packages: write`, verifies the
official Iroh archive during the build, and pushes one dual-architecture image.

Historical release `v0.1.0` and image tag `:0.1.0` remain unchanged. The direct
`v...` image-tag convention starts with `v0.1.1`; old tags are not renamed or
deleted.

## Manual deployment and updates

The default 1Panel Compose directory is:

```text
/opt/1panel/docker/compose/zterm-relay
```

Place the reviewed `compose.yaml` and `relay.toml` there. The normal install and
every later update use the same two commands:

```bash
cd /opt/1panel/docker/compose/zterm-relay
docker compose pull
docker compose up -d
```

`pull` is deliberate: `up -d` and ordinary Docker/host restarts reuse the local
image and do not fetch a new `latest`. Updates therefore remain manually
triggered.

After an update, accept the deployment once:

```bash
curl --fail http://127.0.0.1:38451/healthz
curl --fail https://relay.zenithconsulting.cn/healthz
curl --fail --output /dev/null https://relay.zenithconsulting.cn/generate_204
sh tests/relay/public-handshake.sh https://relay.zenithconsulting.cn
```

Also inspect the host once to confirm that only loopback TCP 38451 is published,
the container is named `zterm-relay`, its configured image is
`ghcr.io/leonfox28/zterm-relay:latest`, and its log driver is `local`. Stop after
those checks pass; do not restart or switch images merely to repeat the same
evidence.

Credentials, SSH keys, and machine-specific secrets must never be copied into
the repository. This deployment does not need a real `.env` file.

## Reverse-proxy requirements

The public URL is `https://relay.zenithconsulting.cn`; only the same-host hop to
`http://127.0.0.1:38451` is plain HTTP. The reverse proxy must preserve path,
query data, the long-lived HTTP/1.1 WebSocket upgrade on `/relay`, and Iroh's
authentication headers. Equivalent Nginx/OpenResty semantics are:

```nginx
map $http_upgrade $zterm_connection_upgrade {
    default upgrade;
    ''      close;
}

location / {
    proxy_pass http://127.0.0.1:38451;
    proxy_http_version 1.1;
    proxy_set_header Host $host;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection $zterm_connection_upgrade;
    proxy_set_header Sec-WebSocket-Protocol $http_sec_websocket_protocol;
    proxy_set_header X-Iroh-Relay-Client-Auth-V1 $http_x_iroh_relay_client_auth_v1;
    proxy_buffering off;
    proxy_read_timeout 1d;
    proxy_send_timeout 1d;
}
```

The repository does not own the selected server's existing OpenResty or
Cloudflare configuration. Integrate these semantics with the host's conventions
instead of overwriting unrelated proxy configuration.

## NAT and QAD boundary

Relay fallback and direct NAT traversal are separate paths. A successful NAT
traversal uses a direct end-to-end QUIC path and bypasses this Relay. If direct
establishment fails, encrypted traffic can still use the Relay; forwarding does
not require QAD.

The current deployment sets `enable_quic_addr_discovery = false` and publishes
no UDP port because an HTTP reverse proxy cannot forward QAD. Phase One's
[Foundation Gate](foundation-gate.md) instead exercised the product-default
official Iroh production map: Case A remained relayed in the nested
Colima/Patchbay/TUN lab and is retained as environment-specific deferred
address-discovery evidence; the controlled known-candidate Case B selected a
direct path; and Case C completed three encrypted streams through the official
WSS/TCP Relay after endpoint non-DNS UDP was blocked. B and C permit Foundation
work to continue, but Case A neither passes automatic discovery nor establishes
that official QAD generally fails on ordinary physical networks. Parent M10
must test the unchanged profile across two real networks. That evidence, or a
concrete self-hosting requirement without a reverse proxy, can justify a
separately reviewed UDP/TLS deployment later.

## Failure handling

The service is stateless. If its container has an ordinary runtime problem,
recreate it:

```bash
docker compose up -d --force-recreate
```

If a newly published image is confirmed defective, an operator may temporarily
replace `:latest` in the server Compose with the preceding version tag, then run
`docker compose pull` and `docker compose up -d`. This is a manual escape hatch,
not automatic rollback logic, a routine drill, or a release gate.
