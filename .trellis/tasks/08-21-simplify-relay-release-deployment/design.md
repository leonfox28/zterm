# Design: simplify Relay release and deployment

## 1. Boundaries

This task changes the Relay wrapper, its GitHub publication workflow, the one
supported reverse-proxy deployment, related tests/docs/specs, and the selected
1Panel deployment. It does not change Iroh Relay protocol/data-plane behavior,
OpenResty/Cloudflare, zterm client transport, NAT traversal, or any terminal
feature.

The repository will expose one supported Relay deployment shape instead of
three overlapping shapes:

```text
GitHub Release v0.1.1
        |
        v
GHCR zterm-relay:v0.1.1 + :latest
        |
        | manual docker compose pull + up -d
        v
OpenResty -> 127.0.0.1:38451 -> container:38451
```

## 2. Release contract

The root workspace Cargo version stays authoritative. A small publication
resolver remains because one workflow supports stable Release, prerelease, and
manual development channels, but its responsibilities are reduced to:

1. Resolve the workspace product version through Cargo rather than parsing
   SemVer itself.
2. On a GitHub Release, require `release_tag == "v${workspace_version}"`.
3. Use the Release tag unchanged as the OCI tag.
4. Route stable releases to `zterm-relay` and add `latest`; route prereleases
   and manual runs to `zterm-relay-dev` without `latest`.
5. For a manual development tag, reject only empty/`latest`/illegal OCI tag
   values before writing workflow output.

The resolver will not validate trusted GitHub owner syntax, reimplement
canonical SemVer, infer prerelease shape from punctuation, validate the digest
returned by the build action, or construct an immutable deployment reference.
The workflow retains full-SHA Action pins, minimal package permissions, upstream
artifact checksum verification, and one amd64/arm64 image build. Tests cover
one stable case, one prerelease case, one manual case, version mismatch, and
invalid manual input instead of a combinatorial parser matrix.

Release `v0.1.1` is new and does not mutate `v0.1.0`. Stable image tags are
`ghcr.io/leonfox28/zterm-relay:v0.1.1` and `:latest`.

## 3. Image and configuration

The Dockerfile keeps the official-artifact fetch/checksum stage and scratch
non-root runtime. It deletes the Rust health-probe build stage and binary,
narrows `EXPOSE` to TCP 38451, and adds the default command:

```dockerfile
CMD ["--config-path", "/etc/iroh-relay/relay.toml"]
```

One `deploy/relay/relay.toml` replaces the local and reverse-proxy variants:

```toml
http_bind_addr = "0.0.0.0:38451"
enable_quic_addr_discovery = false
enable_metrics = false
access = "everyone"
```

`enable_metrics = false` must remain explicit because Iroh 1.0.3 defaults it
to true. Explicit QAD and access values preserve approved product boundaries;
`enable_relay = true` and an absent limits section rely on the pinned upstream
defaults.

## 4. Compose contract

`deploy/relay/compose.yaml` becomes the only supported Compose file:

```yaml
name: zterm-relay

services:
  relay:
    container_name: zterm-relay
    image: ghcr.io/leonfox28/zterm-relay:latest
    volumes:
      - ./relay.toml:/etc/iroh-relay/relay.toml:ro
    ports:
      - "127.0.0.1:38451:38451"
    restart: unless-stopped
    logging:
      driver: local
```

There is no image/config environment indirection, `build`, automatic pull,
healthcheck, metrics port, custom runtime hardening, stop timeout, or Compose
config abstraction. The project and its single container deliberately share
the stable human-readable name `zterm-relay`; this service is never scaled to
multiple containers. Loopback binding, scratch/non-root runtime, manual update,
restart behavior, and bounded logs are the remaining observable contracts.

The following obsolete deployment artifacts are removed:

- `compose.reverse-proxy.yaml` and `relay.reverse-proxy.toml`;
- `compose.production.yaml` direct TLS/ACME/QAD template;
- `.env.example` and `.env.reverse-proxy.example`;
- `validate-image-reference.sh`;
- `healthcheck.rs` and its build/runtime plumbing.

Self-hosting documentation now describes only the reverse-proxy contract.
Direct TLS/QAD returns only when a concrete requirement justifies it.

## 5. Validation shape

Validation is divided by owner and is not duplicated:

| Boundary | One required proof |
| --- | --- |
| Official Iroh download | Pinned version/architecture SHA-256 succeeds; tampering fails |
| Product version | Existing workspace lockstep gate plus direct Release tag equality |
| Container image | Both target architectures start and report Iroh 1.0.3 |
| Compose | Rendered service has the exact minimal image, mount, loopback port, restart, and local logging contract |
| Runtime | Host `/healthz` and `/generate_204` respond from a directly run test container |
| Public deployment | One authenticated Iroh handshake through the public URL after manual update |

There is no rollback smoke, reconnect loop, metrics check, Docker health-state
check, digest validator matrix, or separate direct-production config smoke.

## 6. Selected-server migration

After local review, commit, and green CI:

1. Publish GitHub Release `v0.1.1` and wait for the multi-platform workflow.
2. Confirm `:v0.1.1` and `:latest` resolve to the same published image.
3. Stage the reviewed `compose.yaml` and `relay.toml` without replacing the old
   Compose file before it has identified and removed its own stateless service.
4. Run the old Compose `down` once to remove
   `zterm-relay-reverse-proxy-relay-1` and its old project network. This creates
   a brief accepted maintenance interruption and prevents a port/name collision.
5. Run the new project's `docker compose pull`, then `docker compose up -d`
   once; later updates omit `down`.
6. Verify project/container name `zterm-relay`, active image reference
   `:latest`, log driver `local`, only host loopback 38451, `/healthz`, and one
   public authenticated Iroh handshake.
7. End validation. Do not restart again or switch to an old image.

If a real deployment failure occurs, stop and report the observed failure. A
manual previous-version tag is an operator escape hatch, not an automated path
or acceptance exercise. Historical backups are left untouched.

## 7. Compatibility and documentation

- Historical `v0.1.0`, `:0.1.0`, digest, deployment, and rollback evidence stay
  factually recorded as history but are no longer the current contract.
- Parent Phase Zero/MVP requirements are revised prospectively to point at the
  simple current deployment rather than rewriting what previously occurred.
- `.trellis/spec/backend/relay-deployment.md` is rewritten to the new concrete
  contract.
- A single project-wide guide owns evidence-driven simplicity and is linked
  from `.trellis/spec/guides/index.md`; Relay docs reference rather than copy
  that general rule.
