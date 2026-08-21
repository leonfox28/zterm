# Official Iroh relay deployment

## Boundary

zterm does not implement or fork a relay data plane. The image downloads the
official Iroh 1.0.3 Linux musl release artifact, selects it by BuildKit
`TARGETARCH`, verifies the upstream SHA-256, and copies the static binary plus
the pinned Alpine CA bundle and a purpose-built static one-shot HTTP probe into
a `scratch` runtime running as numeric user/group 65532. No shell, compiler,
downloader, package manager, or long-running monitor remains in the final image.
The probe is built with the exact project Rust 1.98.0 image pinned by digest.

Canonical artifact metadata lives only in `deploy/relay/artifact.sh`:

| Docker architecture | Official target | SHA-256 |
| --- | --- | --- |
| `amd64` | `x86_64-unknown-linux-musl` | `9e25e394c6d09b449d86bb222de535d2a6e68de8030ee8ef39f682ab6ff0cd2c` |
| `arm64` | `aarch64-unknown-linux-musl` | `331a2f35519a778a5b0a2a34baa7f495d3540b3cdb549b8203cbfdd209df7641` |

The values match the official
[Iroh v1.0.3 release](https://github.com/n0-computer/iroh/releases/tag/v1.0.3).
The configuration fields follow the tagged
[`iroh-relay` 1.0.3 source](https://github.com/n0-computer/iroh/blob/v1.0.3/iroh-relay/src/main.rs),
including `/healthz`, a separate metrics listener, `access = "everyone"`, and
the absence of a `[limits]` section. That source explicitly permits a normal
non-`--dev` server with no `[tls]` section: all Relay HTTP services are then
served over plain HTTP. It also makes QUIC address discovery optional and
disabled by default.

The runtime is not an application encryption endpoint. It can observe source
addresses, endpoint identifiers, timing, sizes, and encrypted traffic patterns,
but it cannot decrypt zterm's future end-to-end QUIC payload. Upstream logs may
contain connection metadata; `info` logging plus Docker size/count rotation is
therefore an operational control, not a claim that the relay sees no metadata.
Iroh 1.0.3's default ACME client uses embedded WebPKI roots; the runtime also
retains the pinned Alpine CA bundle, and its ACME cache path is the only writable
persistent volume. No terminal payload or zterm device key is stored there.

## Local validation

`compose.yaml` starts a localhost-only HTTP relay in upstream `--dev` mode. It
does not enable production TLS or QUIC address discovery.

```bash
sh tests/relay/static.sh
sh tests/relay/verify-upstream.sh
sh tests/relay/build-platforms.sh
sh tests/relay/smoke.sh
sh tests/relay/production-config-smoke.sh
sh tests/relay/reverse-proxy-smoke.sh
sh tests/relay/secret-scan.sh
```

The public acceptance probe is a separate, pinned Iroh 1.0.3 client. It creates
an ephemeral endpoint identity, configures exactly the supplied Relay URL with
`RelayConfig::new(url, None)`, and succeeds only after Iroh reports the
authenticated home-relay connection as connected. It does not store the
identity or attempt QAD:

```bash
sh tests/relay/public-handshake.sh https://relay.example.com
```

For an approved maintenance test, add `--expect-reconnect`, wait for the
initial success line, and restart the relay container from a second terminal.
The same endpoint process must observe both the disconnect and a new
authenticated connection before the 90-second deadline.

The smoke test builds the real image, starts one relay service, waits for an
in-container HTTP `/healthz` check, confirms that endpoint reports version
1.0.3, checks the private metrics endpoint, verifies user 65532, prints the
local image ID, and removes its temporary Compose resources. The
production-config smoke renders the TLS/ACME TOML, starts 1.0.3 with networking
disabled and ACME pointed at unreachable loopback, then probes the live port-80
`/generate_204` service. This proves the non-root, capability-free process can
bind the production low ports without making a public request. The
reverse-proxy smoke separately starts the normal 1.0.3 binary without `--dev`,
uses its immutable local image ID with `--no-build`, publishes only
collision-free host-loopback ports, verifies `/healthz`,
`/generate_204`, private metrics, and a `/relay` WebSocket `101` upgrade, and
asserts that no 80/443/UDP binding exists. It also checks the CA bundle exists
in the scratch image. There is no monitor sidecar, account service, database,
shared token, allowlist, denylist, or zterm-specific rate limit.

Local evidence builds disable optional BuildKit provenance attestations so an
unchanged single-platform image has a stable local content ID across repeated
runs. Those IDs are development/CI evidence only; they are not production image
references.

Docker health is a local liveness check. After ACME issuance, deployment
readiness must additionally verify the upstream HTTPS `/healthz` endpoint and
certificate chain from outside the container. The image uses `SIGINT` as its
stop signal because upstream 1.0.3 handles that signal and performs graceful
relay shutdown.

## GHCR publication

`.github/workflows/relay-image.yml` is the production image source, following
[GitHub's container publishing guidance](https://docs.github.com/en/actions/tutorials/publish-packages/publish-docker-images).
It derives and lowercases the GitHub repository owner for OCI compatibility.
In the official `leonfox28/zterm` repository it publishes two separate package
channels, each as one manifest covering `linux/amd64` and `linux/arm64`:

- production: `ghcr.io/leonfox28/zterm-relay`;
- development: `ghcr.io/leonfox28/zterm-relay-dev`.

Forks resolve the same two names under their own repository owner. Every Action
in the workflow is pinned to a full commit SHA. The workflow has only
`contents: read` and `packages: write` permissions and logs in to GHCR with the
repository-scoped `GITHUB_TOKEN`.

Publishing has two deliberate entry points:

- publishing a non-prerelease GitHub release requires a canonical
  `vMAJOR.MINOR.PATCH` Git tag whose v-less SemVer exactly matches
  `[workspace.package].version`. The workflow checks out that original Git tag,
  then writes the v-less version and the mutable `latest` convenience tag only
  to `zterm-relay`; for example, `v0.1.0` publishes `:0.1.0` and `:latest`;
- publishing a GitHub prerelease requires a canonical
  `vMAJOR.MINOR.PATCH-PRERELEASE` tag whose entire v-less SemVer exactly matches
  the workspace version. It writes only that v-less tag to `zterm-relay-dev`
  and never touches the production package or `latest`; for example,
  `v0.2.0-rc.1` requires workspace version `0.2.0-rc.1` and publishes
  `zterm-relay-dev:0.2.0-rc.1`;
- a manual workflow dispatch writes only to `zterm-relay-dev`. Its non-empty,
  valid OCI tag input is used unchanged, so input `phase-zero` produces
  `ghcr.io/leonfox28/zterm-relay-dev:phase-zero`; only the reserved `latest`
  value is rejected. Manual tags are development aliases and may be replaced by
  a later manual run, so operators should avoid reusing a prerelease tag. The
  workflow's emitted digest, rather than the mutable alias, is authoritative.
  The separate package prevents any manual tag from colliding with or updating
  a stable production release.

Stable and prerelease releases therefore share the product's lockstep version
with the CLI, daemon, protocol, platform libraries, and future apps. A mismatch
with Cargo.toml, a non-canonical SemVer, a release/prerelease flag mismatch, or
SemVer build metadata is rejected before any workflow outputs are written.
Build metadata is deliberately unsupported because `+` is not a portable OCI
tag character and silently dropping it would make release identities
ambiguous. The literal tag `latest` is workflow-managed and is rejected both as
a GitHub release tag and as a manual input.

Both channels enable provenance and SBOM attestations and expose the selected
package, channel, version, multi-platform digest, and full immutable reference
as workflow outputs and in the job summary. Production deployment must copy the
exact `ghcr.io/leonfox28/zterm-relay@sha256:<published-digest>` reference into
`.env`; `zterm-relay-dev` is deliberately rejected by the production preflight.
A deployment must never infer a digest from a tag or reuse one of the local
image IDs in the Phase Zero evidence record.

Per [GitHub's package visibility documentation](https://docs.github.com/en/packages/learn-github-packages/configuring-a-packages-access-control-and-visibility),
personal-account packages default to private. The OCI source label links these
images to the public `leonfox28/zterm` repository so repository access
permissions can be inherited, but GitHub explicitly states that this does not
inherit repository visibility. The first development package is now public,
as verified by an anonymous GHCR bearer pull. Verify the production
package independently after its first stable-release run; if it is private,
make it public for the default relay distribution or authenticate the
deployment host with a read-only package credential. Never claim a channel or
digest before that channel's workflow has actually succeeded.

## Mandatory public-server checkpoint

Stop after all local checks pass. Before the first remote connection, obtain
from the user:

1. the safe SSH entry point and login name plus the existing authentication
   mechanism (never ask the user to paste a private key);
2. the intended relay hostname and current DNS state;
3. whether Docker/Compose already exists on the server.

The first remote step must be read-only. Report any required Docker install,
firewall change, port conflict, DNS change, or other system mutation before
applying it. Do not infer a host from local SSH configuration and do not write
credentials, private keys, or a real `.env` into Git or task notes.

## Production contracts

Two production contracts share the same verified GHCR image digest. Both omit
Compose `build` configuration, require `RELAY_IMAGE`, and use `--no-build` so a
production host cannot silently become an image builder. Local building remains
available through `compose.yaml` and the test scripts. Choose exactly one on a
deployment host:

- `compose.reverse-proxy.yaml` is for a TLS-terminating HTTP reverse proxy
  already running on the same host. It is the selected contract for
  `relay.zenithconsulting.cn`.
- `compose.production.yaml` is the direct TLS/ACME mode for other self-hosters;
  the relay container owns public TCP 80/443 and UDP 7842.

### Same-host reverse proxy (selected deployment)

`compose.reverse-proxy.yaml` runs the official binary with
`relay.reverse-proxy.toml`, without `--dev`, TLS, ACME, or a certificate volume.
Docker publishes exactly these host sockets by default:

| Listener | Exposure | Purpose |
| --- | --- | --- |
| TCP 38451 | `127.0.0.1` only | Plain HTTP upstream consumed by the host reverse proxy |
| TCP 9090 | `127.0.0.1` only | Prometheus metrics; never proxy or publish publicly |

Port 9090 exposes Iroh's Prometheus counters for connections, traffic, and
operational health. It is observability data, not Relay client traffic. Only
local host monitoring should read it; OpenResty and Cloudflare must not route
it.

The public client URL remains `https://relay.zenithconsulting.cn`; only the
same-host OpenResty-to-container hop is plain HTTP. The public TLS proxy must
preserve path and query data and support the long-lived HTTP/1.1 WebSocket
upgrade on `/relay`. Its effective configuration must preserve `Upgrade`,
`Connection`, `Sec-WebSocket-Protocol`, and
`X-Iroh-Relay-Client-Auth-V1`, avoid response buffering, and use timeouts long
enough for persistent relay connections. A successful external `/healthz`
request is necessary but insufficient: public acceptance also requires an
actual Iroh 1.0.3 relay handshake through the proxy.

[Cloudflare's current WebSocket documentation](https://developers.cloudflare.com/network/websockets/)
confirms that proxied WebSockets are supported on all plans, but the zone's
WebSockets setting must remain enabled. The initial `101` request is still
subject to WAF and rate-limit rules, Argo is incompatible, and Cloudflare may
terminate WebSockets during edge deployments or after an idle timeout. Iroh
1.0.3 sends relay-protocol pings every 15 seconds, but Phase 1 must still verify
automatic reconnection without losing the host PTY. Public acceptance must
therefore exercise a complete authenticated connection and reconnect, not only
an HTTP upgrade, and must confirm that Cloudflare/OpenResty policy does not
challenge or block `/relay`.

TLS termination also means the relay process cannot reproduce the TLS exporter
material from the client-to-OpenResty connection. Iroh 1.0.3 explicitly treats
this as normal: if its fast header authentication cannot be verified, it falls
back to a signed WebSocket challenge round trip. The endpoint identity remains
cryptographically authenticated; the proxy mode costs one extra relay-auth
round trip rather than weakening that identity check. This fallback is defined
in the tagged upstream
[`handshake.rs`](https://github.com/n0-computer/iroh/blob/v1.0.3/iroh-relay/src/protos/handshake.rs).

The repository does not own or rewrite the existing OpenResty configuration.
For a new installation, the essential Nginx/OpenResty semantics are equivalent
to the following; integrate them into the host's existing conventions rather
than copying this block blindly:

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

The selected server uses 1Panel's custom Compose root. After a successful GHCR
publication, copy the reverse-proxy Compose/config files, env template, and
`validate-image-reference.sh` into that directory, replace the image
placeholder with the digest from the workflow summary, and run:

```bash
cd /opt/1panel/docker/compose/zterm-relay
cp .env.reverse-proxy.example .env
# Edit RELAY_IMAGE to ghcr.io/leonfox28/zterm-relay@sha256:<published-digest>.
relay_image=$(sed -n 's/^RELAY_IMAGE=//p' .env)
sh ./validate-image-reference.sh "$relay_image" >/dev/null
docker pull "$relay_image"
docker image inspect "$relay_image" --format '{{.Id}} {{json .RepoDigests}}'
docker compose --env-file .env -f compose.reverse-proxy.yaml config --quiet
docker compose --env-file .env -f compose.reverse-proxy.yaml up -d --no-build --wait
```

Then, from a zterm project checkout on the operator machine, run the
authenticated public acceptance probe:

```bash
sh tests/relay/public-handshake.sh https://relay.zenithconsulting.cn
```

The remote preflight must select commands appropriate to the host's shell; the
block above documents the values and invariants, not authorization to overwrite
an existing `/opt/1panel/docker/compose/zterm-relay` directory. Never silently
fall back to a server-side build during deployment.

The locally loaded, verified arm64 image used for the initial Phase Zero
bootstrap is a temporary exception until a stable-release workflow publishes a
real production GHCR digest. It may keep the running relay available, but all
subsequent production releases and upgrades must come from the production
package and use its immutable digest. A successful development-package build
does not satisfy this gate. Moving the Compose project into the 1Panel root does
not by itself turn that bootstrap image ID into a registry digest.

Do not change `RELAY_PROXY_PORT=38451` for the selected server unless the
existing OpenResty upstream is changed in the same maintenance window.

### QUIC address discovery and the NAT boundary

An HTTP reverse proxy cannot forward Iroh's UDP QUIC address-discovery (QAD)
traffic. The selected mode therefore sets
`enable_quic_addr_discovery = false`, publishes no UDP socket, and remains a
valid Iroh Relay service: Iroh 1.0.3 explicitly supports plain HTTP Relay with
QAD absent. Relay forwarding and QAD are independent. When NAT traversal finds
a direct path, endpoint traffic uses that path; when it cannot establish or
maintain one, the same end-to-end encrypted connection can use this Relay as
its fallback. QAD only reports an endpoint's observed public address and may
improve direct-path success in some networks; it neither forwards relay traffic
nor is required for fallback.

This deployment therefore proves encrypted relay fallback, but it is **not
evidence of QAD-assisted address discovery or complete NAT-hole-punch
behavior**. Phase 1 must first run representative real NAT pairs with QAD
disabled, record Iroh direct/relay path events and connection timings, and
measure direct-path success. Only those results can justify proposing a
separate QAD-only service. The current design deliberately makes no such
decision in advance.

Iroh 1.0.3 keeps the client boundary equally explicit:
`RelayConfig::new(url, None)` disables QAD attempts for that relay. Constructing
a map from bare `RelayUrl` values defaults to a QAD configuration on UDP 7842,
so zterm's first-stage client must use the explicit `None` form while this
deployment has no QAD endpoint. Otherwise clients will make misleading failed
UDP attempts. See the pinned upstream
[`RelayConfig` API](https://docs.rs/iroh/1.0.3/iroh/struct.RelayConfig.html).

If Phase 1 evidence shows this host should later provide QAD, HTTP proxying
alone is insufficient. The supported upstream design is a separately reachable
UDP QAD endpoint with a
certificate trusted for its public hostname. The same official binary can run
QAD-only with `enable_relay = false`,
`enable_quic_addr_discovery = true`, and `[tls] cert_mode = "Manual"`; the
certificate/key would be mounted read-only and the client would use
`RelayConfig::new(url, Some(RelayQuicConfig::new(7842)))`. That is a separate
security and firewall change, is not included in the selected 38451-only
deployment, and must be approved and tested independently. The current
deployment must not open UDP 7842 or modify the firewall. Using `--dev` to
combine a plain HTTP Relay with manual-certificate QAD is intentionally rejected
as the production contract because upstream documents its HTTP-only TLS bypass
as development-only.

### Direct TLS/ACME mode (preserved for self-hosters)

`compose.production.yaml` uses Compose `configs.content` interpolation so the
TLS hostname and ACME contact are supplied at deployment time without adding a
template renderer to the image. Compose 2.23.1 or newer is required for this
feature. Copy `.env.example` to the ignored `.env` only on the deployment host.
Use plain DNS hostname/email values without quotes or newlines.

| Listener | Exposure | Purpose |
| --- | --- | --- |
| TCP 80 | Public | ACME HTTP challenge and captive-portal probe |
| TCP 443 | Public | TLS relay/WebSocket transport and `/healthz` |
| UDP 7842 | Public | Iroh QUIC address discovery |
| TCP 9090 | Host loopback by default | Prometheus metrics; never publish publicly |

The production configuration explicitly selects `Everyone` and omits `limits`,
matching the approved open/unlimited policy. This is simple but exposes the
operator to third-party Iroh traffic, abuse, and unpredictable bandwidth cost.
The metrics host binding is deliberately hard-coded to `127.0.0.1`; unlike the
public listener, it cannot be changed through `.env`.

After the user authorizes deployment, the read-only preflight succeeds, and a
published GHCR digest has been selected:

```bash
cd /path/to/relay-deployment
cp .env.example .env
# Edit .env on the server without committing it, including the GHCR digest.
relay_image=$(sed -n 's/^RELAY_IMAGE=//p' .env)
sh ./validate-image-reference.sh "$relay_image" >/dev/null
docker pull "$relay_image"
docker compose --env-file .env -f compose.production.yaml config --quiet
docker image inspect "$relay_image" --format '{{.Id}} {{json .RepoDigests}}'
docker compose --env-file .env -f compose.production.yaml up -d --no-build --wait
```

Public acceptance for this direct mode must verify DNS and certificate chains,
TCP 80/443, UDP 7842, an actual Iroh relay handshake, `/healthz`, loopback-only
metrics, log rotation, and absence of terminal content or device secrets on the
host. Reverse-proxy-mode acceptance instead verifies the external HTTPS chain,
WebSocket relay handshake, host-loopback 38451/9090 bindings, and confirms that
QAD is disabled rather than claiming a UDP test passed. The lack of QAD does
not make relay fallback incomplete; direct-path performance is a separate Phase
1 measurement.

## Digest pinning and rollback

Record the verified image repository digest before changing a running relay.
For upgrades, retain the prior digest and validated non-secret Compose model.
Rollback never requires changing zterm protocol state because the relay stores
no zterm sessions or payloads:

```bash
previous_image='ghcr.io/leonfox28/zterm-relay@sha256:<previous-digest>'
sh ./validate-image-reference.sh "$previous_image" >/dev/null
docker pull "$previous_image"
RELAY_IMAGE="$previous_image" docker compose --env-file .env \
  -f compose.reverse-proxy.yaml up -d --no-build --wait
```

Use `compose.production.yaml` in that command when rolling back direct ACME
mode. The reverse-proxy mode has no persistent volume. In direct mode,
`relay_certs` only holds ACME certificate cache material. Docker's `json-file`
driver rotates at 10 MiB with five files. Removing the container does not remove
that volume; deleting it is a separate destructive operation and is not part of
normal rollback.
