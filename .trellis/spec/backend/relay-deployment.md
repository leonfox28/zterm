# Relay Deployment Contract

## Scenario: one manually updated reverse-proxy Relay

### 1. Scope / Trigger

Apply this contract when changing:

- the official Relay image or GitHub publication workflow;
- `deploy/relay/Dockerfile`, `relay.toml`, or `compose.yaml`;
- the selected 1Panel Relay deployment;
- Iroh client Relay/QAD construction or Relay acceptance tests.

The supported deployment is a single stateless Iroh Relay behind an existing
same-host TLS reverse proxy. Direct TLS/ACME/QAD deployment, automatic update,
metrics infrastructure, and rollback automation are not current capabilities.
Read the [Evidence-Driven Simplicity Guide](../guides/evidence-driven-simplicity.md)
before adding another deployment or validation layer.

### 2. Signatures

- Product version source: root `Cargo.toml` `[workspace.package].version`.
- Stable Release/image example:

  ```text
  workspace:      0.1.1
  GitHub Release: v0.1.1
  version image:  ghcr.io/leonfox28/zterm-relay:v0.1.1
  server image:   ghcr.io/leonfox28/zterm-relay:latest
  ```

- Development package: `ghcr.io/leonfox28/zterm-relay-dev`.
- Public URL: `https://relay.zenithconsulting.cn`.
- Reverse-proxy upstream: `http://127.0.0.1:38451`.
- Server Compose root: `/opt/1panel/docker/compose/zterm-relay`.
- Compose project and container: `zterm-relay`.
- Manual update:

  ```bash
  docker compose pull
  docker compose up -d
  ```

- Phase 1 client construction:

  ```rust
  let relay = RelayConfig::new(relay_url, None);
  ```

  `None` is required because this server exposes Relay forwarding but no QAD
  endpoint.

### 3. Contracts

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
  or attestations without a current verifier/consumer.

#### Image and Iroh configuration

- The runtime is scratch-based, shell-free, and runs as UID/GID 65532.
- The image default command is
  `--config-path /etc/iroh-relay/relay.toml`; Compose does not repeat it.
- The only supported `relay.toml` binds Relay HTTP to container TCP 38451,
  explicitly disables QAD and metrics, uses `access = "everyone"`, and omits
  limits/TLS.
- Relay forwarding is a valid encrypted fallback without QAD. Successful NAT
  traversal uses a direct end-to-end path and bypasses the Relay; Phase 1 tests
  direct and fallback paths separately.

#### Compose and operation

- The only supported Compose file is `deploy/relay/compose.yaml`.
- Its project and explicit single container are both named `zterm-relay`.
- It uses the literal production `:latest` image, a read-only bind mount for
  `relay.toml`, host-loopback TCP 38451, `restart: unless-stopped`, and Docker
  `logging.driver: local`.
- It has no `build`, `.env` image indirection, automatic pull policy, metrics
  port, command/environment/configs abstraction, container healthcheck,
  custom health binary, stop timeout, or additional runtime hardening.
- `logging.driver: local` remains because the selected server has no global
  Docker log rotation. Docker documents the driver as bounded and rotating by
  default: https://docs.docker.com/engine/logging/drivers/local/.
- Updates are manual. A host or Docker restart uses the already pulled local
  image; only an explicit `docker compose pull` changes `latest` locally.
- After one successful post-update health/handshake acceptance, stop. Do not
  switch to an old image, restart again, or run a recovery drill.

### 4. Validation & Error Matrix

| Condition | Required result |
| --- | --- |
| Release tag differs from `v${workspace_version}` | Stop publication before image build |
| Stable release resolves outside `zterm-relay` or does not own `latest` | Stop publication |
| Prerelease/manual run resolves outside `zterm-relay-dev` or updates `latest` | Stop publication |
| Manual tag is empty, `latest`, or illegal for OCI | Reject before writing workflow outputs |
| Official artifact checksum differs | Stop image build |
| Compose publishes anything except host-loopback TCP 38451 | Reject the deployment model |
| Compose project/container is not `zterm-relay` | Reject the deployment model |
| Compose adds automatic pull, metrics, direct TLS/QAD, health, digest, or rollback machinery without a new approved requirement | Treat as specification drift |
| Host `/healthz`, public HTTP, or authenticated Iroh handshake fails after update | Deployment is not accepted; report the observed failure |
| All post-update checks pass | End validation without rollback/restart/reconnect exercises |
| Direct path fails while Relay path succeeds | Valid fallback behavior, not a Relay failure |

### 5. Good / Base / Bad Cases

- **Good release:** workspace `0.1.1` plus Release `v0.1.1` publishes
  `zterm-relay:v0.1.1` and `zterm-relay:latest`.
- **Base development build:** manual `phase-one` publishes only
  `zterm-relay-dev:phase-one`; it never changes production `latest`.
- **Bad release:** workspace `0.1.1` plus Release `v0.1.2`, or converting
  `v0.1.1` into image tag `0.1.1`.
- **Good deployment:** an explicit pull/up recreates the stateless single
  container; loopback health and one public authenticated handshake pass.
- **Bad deployment:** a successful new container is deliberately replaced with
  an old image to prove rollback, or an unused metrics/QAD/direct template is
  kept “just in case.”

### 6. Tests Required

- `tests/workspace-version.sh`: all product crates inherit one Cargo version.
- Publication test: one stable, one prerelease, one manual, one version
  mismatch, and invalid manual input; assert direct `v...` tag reuse and package
  separation.
- Static Compose test: assert exact project/container name, literal `:latest`,
  one read-only config mount, loopback 38451, restart policy, and local logging;
  assert obsolete deployment files are absent.
- Upstream/image tests: checksum/tamper, amd64/arm64 execution, scratch/non-root,
  and Iroh 1.0.3 version.
- Runtime smoke: directly start the built image and assert `/healthz` and
  `/generate_204`; do not duplicate this with Docker health state or metrics.
- Public acceptance after a real manual update: public health/204 plus one
  authenticated Iroh Relay handshake with QAD disabled.
- Repository secret scan and the ordinary Rust/dependency/cross-platform CI
  remain required.

### 7. Wrong vs Correct

#### Wrong

```text
Release v0.1.1 -> strip v -> image :0.1.1 -> validate returned digest ->
require @sha256 in .env -> run health + metrics + handshake -> switch to an old
image -> restore the new image
```

This validates the same identity repeatedly and interrupts a stateless service
after it already passed acceptance.

#### Correct

```text
Release v0.1.1 -> image :v0.1.1 + :latest -> manual compose pull/up ->
health + one authenticated handshake -> stop
```

The artifact checksum, product version, registry image, and live service are
each validated once by the boundary that owns them.
