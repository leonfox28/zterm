# Relay Infrastructure and Deployment Contract

## Scenario: official N0 by default, optional self-hosted Relay

### 1. Scope / Trigger

Apply this contract when changing:

- the Iroh endpoint preset, Relay map, QAD, address publication, or transport
  acceptance tests;
- the optional official Relay image or GitHub publication workflow;
- `deploy/relay/Dockerfile`, `relay.toml`, or `compose.yaml`;
- the selected 1Panel deployment for `relay.zenithconsulting.cn`.

Phase 1 product endpoints use Iroh's official N0 production infrastructure.
The existing stateless self-hosted Relay image and Compose deployment remain a
supported, explicit option, but are not silently appended to the product's
Relay map. Public n0 Relay hosting is suitable for development and testing; it
is rate-limited and has no uptime guarantee. A production managed-versus-
self-hosted decision is deferred until real usage evidence exists.

Read the [Evidence-Driven Simplicity Guide](../guides/evidence-driven-simplicity.md)
before adding another infrastructure profile or validation layer.

### 2. Signatures

- Product Iroh dependency: exactly `1.0.3` until an explicit upgrade task.
- Current Phase 1 endpoint construction:

  ```rust
  ProductionAddressLookups::from_iroh_constants()
      .apply(Endpoint::builder(presets::Minimal))
      .relay_mode(RelayMode::Default)
      .addr_filter(AddrFilter::relay_only())
  ```

- The effective Iroh 1.0.3 production Relay map is tested as:

  ```text
  https://use1-1.relay.n0.iroh.link.
  https://usw1-1.relay.n0.iroh.link.
  https://euc1-1.relay.n0.iroh.link.
  https://aps1-1.relay.n0.iroh.link.
  ```

- Each official entry retains QAD on Iroh's default UDP port 7842.
- n0 DNS/Pkarr lookup and port mapping remain enabled. Address lookup
  publication is relay-only; peers still exchange and test direct candidates
  through Iroh's connection machinery.
- Product version source: root `Cargo.toml` `[workspace.package].version`.
- Stable self-hosted Relay image example:

  ```text
  workspace:      0.1.1
  GitHub Release: v0.1.1
  version image:  ghcr.io/leonfox28/zterm-relay:v0.1.1
  server image:   ghcr.io/leonfox28/zterm-relay:latest
  ```

- Development package: `ghcr.io/leonfox28/zterm-relay-dev`.
- Optional self-hosted URL: `https://relay.zenithconsulting.cn`.
- Reverse-proxy upstream: `http://127.0.0.1:38451`.
- Server Compose root: `/opt/1panel/docker/compose/zterm-relay`.
- Compose project and container: `zterm-relay`.
- Manual update:

  ```bash
  docker compose pull
  docker compose up -d
  ```

- A future explicit self-hosted-only client profile must construct its entry as
  `RelayConfig::new(relay_url, None)`, because that deployment forwards Relay
  traffic but exposes no QAD endpoint. It must replace the default map for that
  profile, not become a hidden fallback inside the N0 profile.

### 3. Contracts

#### Product infrastructure selection

- Build from `presets::Minimal`, then install `PkarrPublisher`, `PkarrResolver`,
  and `DnsAddressLookup` with Iroh's public production constants. Do not use
  their `n0_dns()` shortcuts: those honor `IROH_FORCE_STAGING_RELAYS` and could
  create a mixed production/staging profile.
- Apply explicit `RelayMode::Default` so the production Relay map remains owned
  by the pinned Iroh version. Do not copy Relay URLs into product code.
- Do not include staging or `relay.zenithconsulting.cn` in the default profile.
- QAD assists address discovery; it does not carry fallback Relay traffic.
  Direct traffic bypasses Relay, while failed direct traversal may continue over
  the end-to-end encrypted Relay path.
- Do not infer region pinning from the Relay selected by one run. Home Relay
  selection is dynamic.

#### Publication

- A GitHub Release tag must equal the literal `v` prefix plus the Cargo-resolved
  workspace product version. The same Release tag is used unchanged as the OCI
  image tag.
- Stable releases publish only to `zterm-relay` and also update `latest`.
  GitHub prereleases and manual runs publish only to `zterm-relay-dev` and never
  update `latest`.
- Manual development input is rejected only when empty, equal to `latest`, or
  not a legal OCI tag. Do not reimplement Cargo SemVer or validate trusted
  GitHub owner syntax.
- The workflow builds one linux/amd64 + linux/arm64 image with minimal
  `contents: read` / `packages: write` permissions and full-commit Action pins.
- The image build verifies the official Iroh 1.0.3 artifact SHA-256 once. Do
  not add deployment-time digest validation, immutable-reference requirements,
  or attestations without a current verifier or consumer.

#### Optional self-hosted image and operation

- The runtime is scratch-based, shell-free, and runs as UID/GID 65532.
- The image default command is
  `--config-path /etc/iroh-relay/relay.toml`; Compose does not repeat it.
- `relay.toml` binds Relay HTTP to container TCP 38451, explicitly disables QAD
  and metrics, uses `access = "everyone"`, and omits limits and TLS.
- `deploy/relay/compose.yaml` is the only supported self-hosted Compose file.
  Its project and explicit single container are both named `zterm-relay`.
- Compose uses the literal production `:latest` image, a read-only bind mount
  for `relay.toml`, host-loopback TCP 38451, `restart: unless-stopped`, and
  Docker `logging.driver: local`.
- Compose has no `build`, `.env` image indirection, automatic pull policy,
  metrics port, command/environment/configs abstraction, container
  healthcheck, custom health binary, stop timeout, or rollback automation.
- Updates are manual. After one successful post-update health and authenticated
  handshake acceptance, stop; do not restart or perform a recovery drill.

### 4. Validation & Error Matrix

| Condition | Required result |
| --- | --- |
| Effective default map differs from Iroh 1.0.3's four production entries | Reject the product profile |
| Default map contains staging or the optional self-hosted Relay | Reject the product profile |
| `IROH_FORCE_STAGING_RELAYS` changes any effective lookup URL, DNS origin, or Relay entry | Reject the product profile |
| Any official entry loses QAD UDP 7842, or N0 lookup/port mapping is removed without an approved decision | Reject the product profile |
| Case A never selects Direct and finishes on Relay in the nested Colima/Patchbay/TUN lab | Report deferred address-discovery evidence; Foundation may continue only if B and C pass, and must not call A or official QAD successful |
| Case A finishes on an unknown path or has contradictory Direct evidence | Stop with `NO_GO_ADDRESS_DISCOVERY`; do not classify an ambiguous path as deferred evidence |
| Controlled Case B cannot pass raw UDP and select direct | Stop: the direct-path diagnostic failed |
| UDP-blocked Case C cannot complete over WSS/TCP Relay | Stop: fallback failed |
| Host fake-IP DNS returns `198.18.0.0/15` for n0 Relay names | Resolve real A records only for the disposable lab; do not change product DNS behavior |
| Release tag differs from `v${workspace_version}` | Stop publication before image build |
| Stable release resolves outside `zterm-relay` or does not own `latest` | Stop publication |
| Prerelease/manual run resolves outside `zterm-relay-dev` or updates `latest` | Stop publication |
| Official artifact checksum differs | Stop image build |
| Self-hosted Compose publishes anything except host-loopback TCP 38451 | Reject the deployment model |
| Post-update health or authenticated Iroh handshake fails | Deployment is not accepted; report the observed failure |
| All post-update checks pass | End validation without rollback or reconnect exercises |

### 5. Good / Base / Bad Cases

- **Good product profile:** starts from `presets::Minimal`, installs lookup
  services from Iroh's explicit production constants, keeps
  `RelayMode::Default`, retains QAD, and excludes staging/self-hosted entries.
- **Base network evidence:** controlled external candidates select direct, and
  blocking non-DNS UDP leaves an encrypted WSS/TCP Relay path operational.
- **Inconclusive lab evidence:** automatic QAD remains relayed behind a shared
  outer Colima/TUN NAT; record the lab-specific no-go and test later on two real
  networks rather than adding speculative product fallback logic.
- **Bad product profile:** copies official URLs into production code, mixes the
  optional self-hosted Relay into the default map, or treats QAD as the Relay
  forwarding transport.
- **Good release:** workspace `0.1.1` plus Release `v0.1.1` publishes
  `zterm-relay:v0.1.1` and `zterm-relay:latest`.
- **Good optional deployment:** one explicit pull/up recreates the stateless
  container; loopback health and one public authenticated handshake pass.
- **Bad optional deployment:** adds digest pinning, unused metrics/QAD/direct
  templates, or a rollback drill without an observed need.

### 6. Tests Required

- `iroh_profile_gate`: assert the exact effective Iroh 1.0.3 production map,
  QAD port 7842, N0 lookup/relay-only publication, port mapping, staging
  exclusion, self-hosted exclusion, and production lookup invariance under an
  isolated child process with `IROH_FORCE_STAGING_RELAYS` set.
- `tests/foundation/network-gate.sh`: run fresh A/B/C labs; verify three
  encrypted bidirectional streams per case, raw-UDP control plus a retained
  Direct path in B, and WSS/TCP Relay fallback in C. The aggregate classifier
  may return `GO_WITH_DEFERRED_ADDRESS_DISCOVERY` only when A never selects
  Direct and finishes on Relay, B finishes Direct, and C finishes Relay with
  non-DNS UDP blocked. An unknown or contradictory A path, any B failure, or
  any C failure remains non-zero.
- The network runner may inject real n0 A records into its disposable DNS to
  avoid Bettbox/Patchbay `198.18.0.0/15` collision. That override must remain
  test-only and the container/network must be cleaned after the run.
- `tests/workspace-version.sh`: all product crates inherit one Cargo version.
- Publication tests cover stable, prerelease, manual, version mismatch, and
  invalid manual input; assert direct `v...` tag reuse and package separation.
- Static Compose tests assert exact project/container name, literal `:latest`,
  one read-only config mount, loopback 38451, restart policy, and local logging.
- Upstream/image tests cover checksum/tamper, amd64/arm64 execution,
  scratch/non-root, and Iroh 1.0.3 version.
- Runtime smoke directly starts the built image and asserts `/healthz` and
  `/generate_204`; do not duplicate this with Docker health state or metrics.
- Public acceptance after a manual self-hosted update consists of public
  health/204 plus one authenticated Iroh Relay handshake with QAD disabled.
- Repository secret scan and ordinary Rust/dependency/cross-platform CI remain
  required.

### 7. Wrong vs Correct

#### Wrong

```rust
let mut map = RelayMode::Default.relay_map();
map.insert(self_hosted_relay); // hidden infrastructure mixing
```

This makes the product contract depend on two infrastructure owners and masks
which path was actually tested.

#### Correct

```rust
ProductionAddressLookups::from_iroh_constants()
    .apply(Endpoint::builder(presets::Minimal))
    .relay_mode(RelayMode::Default)
    .addr_filter(AddrFilter::relay_only())
```

Keep the optional self-hosted deployment separately operable. Add an explicit
client-selectable profile only when a product requirement calls for it.
