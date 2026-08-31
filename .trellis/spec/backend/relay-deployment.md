# Relay Infrastructure and Deployment Contract

## Scenario: official N0 by default, optional self-hosted Relay

### 1. Scope / Trigger

Apply this contract when changing:

- the Iroh endpoint preset, Relay map, QAD, address publication, or transport
  acceptance tests;
- physical-network Direct acceptance or diagnostics for transparent DNS/UDP
  proxies, fake-IP ranges, NAT, or host/router firewalls;
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
- The Linux-only network Gate may enable Iroh's `unstable-net-report` API and
  the `iroh-relay` server feature as dev dependencies. Those features exist
  only to observe redacted QAD facts and run a disposable controlled
  Relay/QAD; they must not enter the normal production dependency graph.
- The endpoint accepts exactly the normal `zterm/1` and short-lived pairing
  `zterm-pair/1` ALPNs. Adding pairing must not create a second endpoint or a
  second infrastructure profile.
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
- A successful QAD round trip and a selected Direct path are separate facts.
  The Gate may report QAD IPv4 success only when NetReport has `udp_v4 = true`
  and `global_v4 = Some(_)`. It may report a candidate as
  `NetReportGlobalMatch` only when its value equals that redacted global
  address in memory; this does not recover Iroh's erased `DirectAddrType`.
  Addresses, endpoint IDs, and bearer material are never printed.
- `mapping_varies_by_dest = None` means unknown, including the controlled
  single-QAD-destination fixture. It must never be rewritten as `false` or
  described as a stable mapping.
- Do not infer region pinning from the Relay selected by one run. Home Relay
  selection is dynamic.

#### Physical official-n0 Direct acceptance and fake-IP diagnosis

- Physical acceptance must use two genuinely routed networks. Record both
  peers with one authenticated primary connection, an active stream,
  `direct_path_count > 0`, and `relay_path_count = 0`, then verify interactive
  terminal traffic. A VPN-provided private address or two interfaces whose
  default route still uses one LAN is not public-NAT evidence.
- Before blaming official QAD, carrier UDP, or NAT type, resolve the official
  Relay names on the affected endpoint. An A record in `198.18.0.0/15` is
  benchmark/fake-IP space, not an official QAD destination. A transparent
  proxy can therefore make every UDP 7842 probe look sent while no packet ever
  reaches the WAN NAT.
- Confirm suspected interception with a bounded packet capture or router
  connection-tracking record keyed by the endpoint's UDP source port. The
  decisive fake-IP shape is: destination `198.18.0.0/15:7842`, outbound bytes,
  zero reply bytes, and no WAN source-NAT address. NAT rule counters are
  pre-filter evidence and do not by themselves prove delivery to the host.
- The deployment-local repair must provide real DNS for `*.iroh.link` and let
  both QAD UDP and arbitrary peer UDP bypass the transparent proxy. Excluding
  only destination port 7842 is insufficient because the eventual peer port
  is dynamic. For an OpenClash/Mihomo deployment, a fake-IP exclusion for the
  Iroh suffix plus a Direct UDP rule scoped to the endpoint host is an example,
  not product configuration.
- Never add OpenClash-, RouterOS-, or one-home-network policy to zterm's
  production profile. Product behavior remains automatic Direct with Relay
  fallback; environment-specific bypasses belong to deployment diagnostics.

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
| Case A never selects Direct and finishes on Relay in the nested Colima/Patchbay/TUN lab | Report deferred address-discovery evidence; Foundation may continue only if B, C, and D pass. Call official QAD successful for that run only when the redacted NetReport contract above passes; never call A Direct or representative of physical networks |
| Case A finishes on an unknown path or has contradictory Direct evidence | Stop with `NO_GO_ADDRESS_DISCOVERY`; do not classify an ambiguous path as deferred evidence |
| Controlled Case B cannot pass raw UDP and select direct | Stop: the direct-path diagnostic failed |
| UDP-blocked Case C cannot complete over WSS/TCP Relay | Stop: fallback failed |
| Controlled Case D does not obtain QAD IPv4/global-v4 evidence on both endpoints, begins anywhere except its controlled Relay, receives an injected direct address, or fails to retain Direct | Stop with `NO_GO_CONTROLLED_QAD`; automatic QAD candidate discovery/exchange has not been proved |
| Case D reports `mapping_varies_by_dest = None` | Record variation as unknown; one controlled QAD destination cannot measure destination dependence |
| Host fake-IP DNS returns `198.18.0.0/15` for n0 Relay names | Resolve real A records only for the disposable lab; do not change product DNS behavior |
| A physical endpoint resolves an official Relay/QAD name into `198.18.0.0/15` | Classify the run as transparent-proxy/fake-IP interception, not QAD reachability or NAT failure; restore real DNS and Direct UDP before retesting |
| QAD traffic has outbound bytes to fake-IP UDP 7842 but zero replies and no WAN NAT | Stop NAT-type diagnosis; the probe has not reached official QAD |
| Both physical peers report Direct during one active interactive stream | Accept the physical official-n0 Direct claim for that topology and preserve the evidence without endpoint IDs or public addresses |
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
- **Base network evidence:** controlled external candidates select direct,
  blocking non-DNS UDP leaves an encrypted WSS/TCP Relay path operational, and
  a controlled lab Relay/QAD promotes Relay to Direct without
  `external_addr`.
- **Inconclusive lab evidence:** automatic QAD remains relayed behind a shared
  outer Colima/TUN NAT. Redacted NetReport may prove that official QAD worked
  in that run while still leaving Direct unselected; record any observed
  destination-varying mapping and test later on two real networks rather than
  adding speculative product fallback logic.
- **Good physical evidence:** a cellular client with no VPN reaches a Debian
  endpoint behind a home RouterOS IPv4 NAT; after removing fake-IP interception
  for Iroh and bypassing the endpoint's UDP from the transparent proxy, both
  installed peers report Direct with no Relay path and an interactive terminal
  attaches successfully.
- **Bad physical diagnosis:** treats UDP sent to `198.18.0.0/15:7842` as an
  official QAD timeout, or changes Relay infrastructure before checking the
  endpoint's DNS result and actual WAN egress.
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
  QAD port 7842, both product ALPNs, N0 lookup/relay-only publication, port mapping, staging
  exclusion, self-hosted exclusion, and production lookup invariance under an
  isolated child process with `IROH_FORCE_STAGING_RELAYS` set.
- `tests/foundation/network-gate.sh`: run fresh A/B/C/D labs and verify three
  encrypted bidirectional streams per case. B requires its raw-UDP control and
  a retained Direct path; C requires WSS/TCP Relay fallback with non-DNS UDP
  blocked. D adapts Iroh 1.0.3's Patchbay pattern: one disposable test-only
  HTTPS Relay plus QAD on the simulated Internet, two independent `Nat::Home`
  endpoints, a Relay-only dial address, test-only CA bypass, and no
  `external_addr`; it must begin Relay and retain Direct with redacted QAD
  IPv4/global-v4 evidence on both endpoints. The aggregate classifier may
  return `GO_WITH_DEFERRED_ADDRESS_DISCOVERY` only when A never selects Direct
  and finishes Relay while B, C, and D satisfy those hard contracts. An
  unknown or contradictory A path, or any B/C/D failure, remains non-zero.
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
- M10 physical official-n0 acceptance records the two real network classes,
  installed versions, redacted path counters on both peers, and one successful
  interactive stream. It must also record any transparent proxy exception used
  to make QAD and peer UDP truly direct; public addresses and endpoint IDs are
  excluded from the durable evidence.
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

For network evidence, preserve tri-state NetReport semantics:

```rust
match report.mapping_varies_by_dest() {
    Some(true) => "mapping varies by QAD destination",
    Some(false) => "mapping did not vary across the measured destinations",
    None => "destination variation is unknown",
}
```

Do not turn `None` from a single controlled QAD destination into evidence that
the mapping is destination-independent.

For physical diagnostics, do not mistake a fake destination for failed
official infrastructure:

```text
Wrong: endpoint UDP -> 198.18.x.y:7842, reply bytes = 0
       => "official QAD is blocked"

Correct: resolve *.relay.n0.iroh.link, verify a real public A record,
         bypass transparent proxying for endpoint UDP, then retest QAD/NAT.
```
