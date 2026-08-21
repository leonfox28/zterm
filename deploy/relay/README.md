# Relay bundle

This directory packages the unmodified official `iroh-relay` 1.0.3 binary and
contains the one supported deployment shape.

- `artifact.sh` owns the upstream version, architecture mapping, download URLs,
  and SHA-256 checksums.
- `Dockerfile` verifies the selected official archive and copies the relay plus
  CA bundle into a shell-free `scratch` image running as UID/GID 65532.
- `relay.toml` enables the open Relay listener on container TCP 38451 and
  explicitly disables unused metrics and UDP QAD.
- `compose.yaml` publishes that listener only on host loopback, where a
  same-host TLS reverse proxy can reach it.
- `resolve-publication.sh` maps GitHub Releases and manual development builds to
  the production and development GHCR packages.

The default server directory is
`/opt/1panel/docker/compose/zterm-relay`. Update it manually:

```bash
docker compose pull
docker compose up -d
```

Compose uses `ghcr.io/leonfox28/zterm-relay:latest`; ordinary Docker or host
restarts reuse the already pulled image. The project and its only container are
both named `zterm-relay`. Docker's `local` log driver supplies bounded,
compressed rotation without changing daemon-wide settings.

Public TLS, WebSocket forwarding, and certificates belong to the host reverse
proxy. This bundle deliberately exposes no direct TLS/ACME template, metrics
listener, UDP QAD listener, custom health binary, monitor, database, or volume.
The Relay forwards end-to-end encrypted traffic and stores no zterm terminal
state.

See `docs/relay.md` for publication, deployment, and one-time acceptance
commands.
