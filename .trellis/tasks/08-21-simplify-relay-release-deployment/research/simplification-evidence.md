# Relay simplification evidence

## Observed project state

- `.github/workflows/relay-image.yml` delegates release/manual routing to
  `deploy/relay/resolve-publication.sh`, strips the release tag's leading `v`,
  validates the resulting manifest digest, constructs an immutable reference,
  and publishes provenance/SBOM attestations that no current consumer verifies.
- `tests/relay/publication-channels.sh` contains a large negative matrix for a
  repository-authorized GitHub Release or manual workflow input. The only
  product invariants needed now are direct workspace/tag equality, stable/dev
  package separation, `latest` ownership, and a legal manual OCI tag.
- `deploy/relay/compose.reverse-proxy.yaml` repeats runtime defaults and
  hardening on top of a scratch, shell-free, non-root image. It also includes a
  custom-built health binary solely to support Docker health state.
- `deploy/relay/compose.production.yaml` is an unused direct TLS/ACME/QAD
  template. The selected server and documented product default use an existing
  OpenResty/Cloudflare reverse proxy instead.
- The selected server has no Prometheus consumer. Iroh 1.0.3 enables metrics by
  default, so the minimal config must explicitly set `enable_metrics = false`.
- A read-only server check found no Docker daemon `log-driver` or `log-opts`.
  The current container's bounded logs come only from its Compose `json-file`
  options.

## Trust boundaries that remain

1. The image build downloads an official Iroh 1.0.3 artifact. Verify its
   upstream SHA-256 once before copying it into the runtime image.
2. A repository-authorized GitHub Release names a product build. Require its
   tag to equal `v` plus the Cargo-resolved workspace product version and route
   it to the stable or development package according to the Release flag.
3. Docker pulls a published GHCR image and verifies registry content. Do not
   duplicate registry content-addressing with a deployment-specific validator.
4. The running service boundary is the public endpoint. After a manual update,
   verify host health and one authenticated Iroh handshake once.

## Logging evidence

Docker documents that its default `json-file` driver does not rotate by
default and can exhaust disk space. Docker recommends the `local` driver for
ordinary deployments because it rotates and compresses by default:

- https://docs.docker.com/engine/logging/configure/
- https://docs.docker.com/engine/logging/drivers/local/

The selected server therefore needs only `logging: { driver: local }` in this
Compose. Changing the Docker daemon default would affect unrelated 1Panel
projects and is outside this task.

## Deliberately removed evidence loops

- No deployment-time digest parser or fixed-digest requirement.
- No post-success rollback/recovery exercise.
- No Docker health state when a one-time host health check already owns
  acceptance and Docker would not restart an unhealthy container automatically.
- No private metrics listener without a metrics consumer.
- No direct TLS/QAD deployment before a real self-hosting need or Phase 1 NAT
  evidence exists.
- No repeated tests that prove the same release version relation at several
  layers.
