# Relay bundle

This directory packages the unmodified official `iroh-relay` 1.0.3 binary.

- `artifact.sh` is the single source for version, target mapping, URLs, and
  SHA-256 values.
- `Dockerfile` verifies the release archive and runs the binary as UID/GID
  65532. Its small one-shot health probe performs real HTTP checks without
  adding a shell, downloader, or long-running monitor to the runtime image.
- `relay.toml` plus `compose.yaml` are localhost-only smoke-test inputs.
- `relay.reverse-proxy.toml` plus `compose.reverse-proxy.yaml` are the
  production contract for a same-host TLS-terminating reverse proxy. They bind
  the relay to host loopback port 38451 and the Prometheus metrics endpoint on
  port 9090 to host loopback only.
- `compose.production.yaml` remains the direct TLS/ACME deployment contract for
  self-hosters whose container owns public ports 80/443/7842.
- `.env.example` and `.env.reverse-proxy.example` contain non-secret
  placeholders; the real `.env` is ignored.
- `validate-image-reference.sh` is the production preflight that rejects
  mutable tags, local image IDs, wrong registry paths, and malformed digests.
- `.github/workflows/relay-image.yml` is the only production image publisher.
  Stable releases build `ghcr.io/leonfox28/zterm-relay`; prereleases and manual
  runs build the separate `ghcr.io/leonfox28/zterm-relay-dev` package. Manual
  tags are used verbatim except that `latest` is reserved.
  Each publication is one `linux/amd64` + `linux/arm64` manifest and reports its
  immutable digest. Production Compose accepts only the production package by
  digest and deliberately contains no `build` section; local builds remain
  available through `compose.yaml` and the relay tests only.

Read `docs/relay.md` before using either production Compose file. The
reverse-proxy mode intentionally has no QUIC address-discovery listener; a
plain HTTP proxy cannot forward UDP QAD. QAD is an optional aid for discovering
a direct path, not part of relay forwarding, so encrypted relay fallback is
fully available in this mode. Phase 1 will measure real NAT traversal before
any separate QAD-only service is considered; this mode must not be described as
proof of NAT-hole-punch success.

The default server-side Compose root is
`/opt/1panel/docker/compose/zterm-relay`. Port 9090 carries only Iroh's private
Prometheus operational metrics; it is not relay traffic and must not be routed
through OpenResty or exposed publicly.
