# Relay Deployment Contract

## Scenario: Reverse-proxied relay fallback and optional QAD

### 1. Scope / Trigger

Apply this contract whenever code or infrastructure changes any of the following:

- the public zterm relay URL or its reverse-proxy deployment;
- Iroh `RelayConfig` / `RelayMode` construction;
- NAT traversal, path selection, or relay-only tests;
- any proposal to expose UDP QUIC address discovery (QAD).

Direct NAT-traversed traffic and relay fallback are independent data paths. QAD is an optional address-discovery aid that can improve direct-path establishment; it is not part of relay forwarding and is not required for encrypted fallback traffic.

### 2. Signatures

- Public relay URL: `https://relay.zenithconsulting.cn`
- Same-host upstream: `http://127.0.0.1:38451`
- Private metrics: `http://127.0.0.1:9090/metrics` (Prometheus
  observability only, never Relay client traffic)
- Default Compose root: `/opt/1panel/docker/compose/zterm-relay`
- Official production package: `ghcr.io/leonfox28/zterm-relay`
- Official development package: `ghcr.io/leonfox28/zterm-relay-dev`
- Product version source: root `Cargo.toml` `[workspace.package].version`
- Stable release mapping: Git tag `vMAJOR.MINOR.PATCH` -> production image
  tag `MAJOR.MINOR.PATCH` plus `latest`
- Prerelease mapping: Git tag `vMAJOR.MINOR.PATCH-PRERELEASE` -> development
  image tag `MAJOR.MINOR.PATCH-PRERELEASE`, without `latest`
- Production deployment image:
  `ghcr.io/leonfox28/zterm-relay@sha256:<published-digest>` (fork workflows
  resolve the same package names under their own repository owner)
- Image publisher: `.github/workflows/relay-image.yml`
- Preflight validator: `deploy/relay/validate-image-reference.sh "$RELAY_IMAGE"`
- Deployment command:

  ```bash
  docker compose --env-file .env -f compose.reverse-proxy.yaml up -d --no-build --wait
  ```

- Phase 1 client construction for this relay-only deployment:

  ```rust
  let relay = RelayConfig::new(relay_url, None);
  // Insert `relay` into the endpoint's custom RelayMode.
  ```

  The second argument is deliberately `None`: this server does not expose a QAD endpoint. Do not construct the selected relay from a bare `RelayUrl`, because Iroh 1.0.3 then assumes default UDP QAD on port 7842.

### 3. Contracts

Reverse-proxy environment keys:

| Key | Required | Contract |
| --- | --- | --- |
| `RELAY_IMAGE` | Yes | Must be the GHCR wrapper image at the exact multi-platform digest emitted by the publication workflow. Production tags and local image IDs are invalid. |
| `RELAY_PROXY_PORT` | Optional | Defaults to `38451`; Docker must publish it only on `127.0.0.1`. |
| `RELAY_METRICS_PORT` | Optional | Defaults to `9090`; Docker must publish it only on `127.0.0.1`. |
| `RELAY_LOG_LEVEL` | Optional | Defaults to `info`; upstream metadata logs remain rotated. |

Runtime contracts:

- OpenResty/Cloudflare owns public TLS and forwards HTTP/1.1 WebSocket traffic to loopback port 38451.
- Production Compose files contain no `build` section. They pull the published
  multi-platform GHCR image and start with `--no-build`; local builds are only
  for development and CI verification.
- The root workspace version is the lockstep zterm product version. Product
  crates inherit it with `version.workspace = true`; the isolated handshake
  probe is an internal acceptance tool and is not a product release component.
- Stable GitHub releases require canonical `vMAJOR.MINOR.PATCH`. Removing the
  leading `v` must exactly match `[workspace.package].version`; the v-less tag
  and `latest` are published only in `zterm-relay`. For example, workspace
  `0.1.0` plus Git tag `v0.1.0` publishes `zterm-relay:0.1.0` and `:latest`.
- GitHub prereleases require canonical
  `vMAJOR.MINOR.PATCH-PRERELEASE`. The complete v-less SemVer must exactly
  match the workspace version and is published only in `zterm-relay-dev`,
  never in the production package or `latest`.
- SemVer build metadata is rejected. OCI tags do not portably accept `+`, and
  silently dropping metadata would make two Git release identities map to the
  same image tag.
- Manual publications target only `zterm-relay-dev`. Any non-empty valid OCI
  tag except reserved `latest` is used unchanged (`phase-zero` remains
  `phase-zero`). These development aliases may be overwritten; the emitted
  digest is authoritative. Operators should not reuse a prerelease release tag.
  Package separation prevents a manual tag from colliding with or updating a
  stable production release.
- Both packages are multi-platform manifests with provenance/SBOM and an exact
  image-plus-digest workflow output. Production deployments accept only the
  production package by digest and never consume `latest`, another mutable
  tag, or the development package.
- The literal release/manual tag `latest` is reserved for the workflow-managed
  stable alias and must be rejected at channel resolution.
- `deploy/relay/resolve-publication.sh` is the single owner of the release/
  prerelease/manual matrix. For release events it must parse exactly one
  `[workspace.package].version`, validate canonical SemVer and release type,
  and enforce lockstep equality before writing outputs. It must also validate
  the original repository owner and version as single-line ASCII values; a
  line-oriented regex that accepts one valid line inside a multiline value is
  unsafe because it permits workflow-output injection.
- The upstream relay uses official Iroh 1.0.3, `access = "everyone"`, no `[limits]`, no `--dev`, no custom data plane, and no monitor sidecar.
- Port 9090 is hard-bound to host loopback for Prometheus connection/traffic
  metrics. It is not proxied or exposed as a relay transport listener.
- The container publishes no TCP 80/443 and no UDP socket. UFW is unchanged for this deployment.
- Successful NAT traversal selects a direct end-to-end QUIC path and bypasses the relay.
- Failed NAT traversal remains on the relay path; the relay forwards end-to-end encrypted packets and does not need QAD to do so.
- Whether QAD materially improves direct-path success is an empirical Phase 1 decision based on real networks and Iroh path events, not a Phase Zero assumption.

### 4. Validation & Error Matrix

| Condition | Required result |
| --- | --- |
| `deploy/relay/validate-image-reference.sh "$RELAY_IMAGE"` fails | Reject production deployment; missing values, mutable tags, local image IDs, wrong paths, `zterm-relay-dev`, and malformed digests are invalid. |
| Production Compose contains `build` or deployment omits `--no-build` | Reject deployment; production images are built only by GitHub Actions. |
| Stable tag is not canonical `vMAJOR.MINOR.PATCH`, contains build metadata, or differs from the workspace version after removing `v` | Reject publication before writing any workflow output. |
| Prerelease tag lacks a prerelease suffix, is non-canonical, contains build metadata, or differs from the workspace version | Reject publication before writing any workflow output. |
| Release `prerelease` flag disagrees with the tag shape | Reject publication; stable and development channels must not be inferred from a mismatched tag. |
| Manual tag is `latest`, malformed, empty, overlength, multiline, or contains a control character | Reject publication without writing partial output; valid manual tags remain development-only and unchanged. |
| Host port 38451 or 9090 is not bound to `127.0.0.1` | Reject deployment. |
| Public `/healthz` is not HTTP 200 with Iroh 1.0.3 | Deployment is unhealthy. |
| `/relay` does not preserve WebSocket upgrade/subprotocol | Relay fallback is not accepted even if `/healthz` passes. |
| Authenticated Iroh relay handshake fails | Phase Zero public gate remains incomplete. |
| Client config enables QAD for this server | Configuration error; use `RelayConfig::new(url, None)`. |
| Direct path fails but relay path succeeds | Valid fallback behavior; do not classify as relay failure. |
| Relay path fails after direct-path failure | Connection failure; diagnose reverse proxy, relay, or network. |
| NAT tests show materially worse direct success without QAD | Propose QAD-only infrastructure separately; require explicit approval before UDP/firewall/certificate changes. |

### 5. Good / Base / Bad Cases

- Good release: workspace `0.1.0` and stable Git tag `v0.1.0` produce only
  `zterm-relay:0.1.0`, `zterm-relay:latest`, and one immutable digest.
- Base publication: manual tag `phase-zero` remains unchanged but is written
  only to `zterm-relay-dev`; its digest cannot pass production preflight.
- Bad release: workspace `0.1.0` is published from `v0.1.1`, a Git tag keeps
  its leading `v` in the OCI tag, or build metadata is silently discarded.
- Good network path: two devices discover a direct path; `path_events()` shows direct selection and application traffic bypasses the relay.
- Base network path: direct attempts fail; the same encrypted connection remains usable through `relay.zenithconsulting.cn`.
- Bad network claim: a test treats missing UDP 7842 as proof that relay fallback cannot work, or reports a healthy relay as proof that NAT traversal works.

### 6. Tests Required

- Local: `tests/relay/reverse-proxy-smoke.sh` must assert loopback bindings, `/healthz`, `/generate_204`, private metrics, WebSocket `101`, non-root/read-only runtime, and graceful stop.
- Publication: `tests/relay/static.sh` and
  `tests/relay/publication-channels.sh` must assert least-privilege workflow
  permissions, full-SHA Action pins, dynamic GHCR ownership, the
  `deploy/relay` context, one `linux/amd64,linux/arm64` manifest per run,
  production/development package isolation, stable/prerelease/manual tag
  behavior, `v0.1.0` -> `0.1.0` mapping, workspace lockstep equality,
  build-metadata rejection, provenance/SBOM, exact digest-reference output,
  production rejection of the development package, digest-only Compose, and
  the 1Panel root. The channel matrix must include malformed/empty/overlength
  tags, SemVer leading-zero and empty-identifier cases, release-flag mismatch,
  and multiline/control-character owner or tag inputs. Every rejection must
  assert its expected error and that no output record was written; successful
  cases must assert the exact output record count.
- Product version: `tests/workspace-version.sh` must use locked Cargo metadata
  to prove that every `crates/*` product manifest inherits one workspace
  version, all resolved product package versions agree, and `Cargo.lock` is
  current. CI must run it on Linux, macOS, and Windows.
- Public Phase Zero: verify DNS/TLS, public health, OpenResty WebSocket forwarding, a complete authenticated Iroh relay handshake, loopback-only host bindings, image identity, logs, and rollback/recreate.
- Phase 1 NAT gate: test devices on genuinely different networks, observe `Connection::path_events()`, distinguish direct and relay paths, force relay fallback, then repeat enough trials to decide whether QAD is needed.
- Any future QAD proposal: add a separate QAD-only test matrix for UDP reachability, trusted manual certificate reload, client `RelayQuicConfig`, and before/after direct-path success rates.

### 7. Wrong vs Correct

#### Wrong

```text
workspace version: 0.1.0
Git release tag:    v0.1.0
published image:    ghcr.io/leonfox28/zterm-relay:v0.1.0
```

Keeping the Git-only `v` prefix in the OCI tag breaks the unified public
version convention. Publishing when the workspace version differs is also
invalid.

#### Correct

```text
workspace version: 0.1.0
Git release tag:    v0.1.0
convenience tags:   ghcr.io/leonfox28/zterm-relay:0.1.0, :latest
deployment:         ghcr.io/leonfox28/zterm-relay@sha256:<published-digest>
```

The release workflow verifies the lockstep version, strips only the leading
`v` for the OCI convenience tag, and records the immutable deployment digest.

#### Wrong QAD construction

```rust
// A bare RelayUrl implicitly enables default QAD expectations in Iroh 1.0.3.
let relay_mode = RelayMode::Custom(vec![relay_url.into()]);
```

This also leads to the incorrect architectural claim that relay fallback depends on UDP 7842.

#### Correct QAD construction

```rust
// Current server provides HTTPS/WebSocket relay fallback, with QAD disabled.
let relay = RelayConfig::new(relay_url, None);
```

Test direct NAT traversal and relay fallback separately. Add QAD only if the Phase 1 evidence justifies its additional UDP, certificate, and firewall surface.
