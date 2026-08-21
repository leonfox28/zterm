#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
artifact_file="$repo_root/deploy/relay/artifact.sh"
dockerfile="$repo_root/deploy/relay/Dockerfile"
dockerignore="$repo_root/deploy/relay/.dockerignore"
local_compose="$repo_root/deploy/relay/compose.yaml"
production_compose="$repo_root/deploy/relay/compose.production.yaml"
reverse_proxy_compose="$repo_root/deploy/relay/compose.reverse-proxy.yaml"
production_env="$repo_root/deploy/relay/.env.example"
reverse_proxy_env="$repo_root/deploy/relay/.env.reverse-proxy.example"
image_reference_validator="$repo_root/deploy/relay/validate-image-reference.sh"
relay_config="$repo_root/deploy/relay/relay.toml"
reverse_proxy_config="$repo_root/deploy/relay/relay.reverse-proxy.toml"
publish_workflow="$repo_root/.github/workflows/relay-image.yml"
publication_resolver="$repo_root/deploy/relay/resolve-publication.sh"
publication_channel_test="$repo_root/tests/relay/publication-channels.sh"
relay_doc="$repo_root/docs/relay.md"
relay_spec="$repo_root/.trellis/spec/backend/relay-deployment.md"
handshake_manifest="$repo_root/tests/relay/handshake-probe/Cargo.toml"
handshake_source="$repo_root/tests/relay/handshake-probe/src/main.rs"
handshake_deny="$repo_root/tests/relay/handshake-probe/deny.toml"

fail() {
    echo "relay static check failed: $*" >&2
    exit 1
}

# shellcheck source=../../deploy/relay/artifact.sh
. "$artifact_file"

[ "$IROH_VERSION" = "1.0.3" ] || fail "unexpected Iroh version"
[ "${#IROH_RELAY_SHA256_AMD64}" -eq 64 ] || fail "invalid amd64 checksum"
[ "${#IROH_RELAY_SHA256_ARM64}" -eq 64 ] || fail "invalid arm64 checksum"

iroh_relay_resolve amd64
[ "$IROH_RELAY_TARGET" = "x86_64-unknown-linux-musl" ] || fail "bad amd64 mapping"
iroh_relay_resolve arm64
[ "$IROH_RELAY_TARGET" = "aarch64-unknown-linux-musl" ] || fail "bad arm64 mapping"
if (iroh_relay_resolve unsupported >/dev/null 2>&1); then
    fail "unknown architecture was accepted"
fi

test_dir=$(mktemp -d)
trap 'rm -rf "$test_dir"' EXIT HUP INT TERM
printf 'tampered relay archive\n' >"$test_dir/tampered.tar.gz"
IROH_RELAY_SHA256="0000000000000000000000000000000000000000000000000000000000000000"
if iroh_relay_verify "$test_dir/tampered.tar.gz" >/dev/null 2>&1; then
    fail "tampered archive passed checksum verification"
fi

grep -Fq 'alpine:3.23.3@sha256:' "$dockerfile" || fail "base image is not digest-pinned"
grep -Fq 'docker/dockerfile:1.7@sha256:' "$dockerfile" \
    || fail "Dockerfile frontend is not digest-pinned"
grep -Fq 'rust:1.98.0-alpine3.23@sha256:' "$dockerfile" \
    || fail "health-check builder is not digest-pinned"
grep -Fq 'FROM scratch AS runtime' "$dockerfile" || fail "runtime is not scratch-based"
grep -Fq 'USER 65532:65532' "$dockerfile" || fail "runtime is not non-root"
grep -Fq 'iroh_relay_verify' "$dockerfile" || fail "Docker build bypasses checksum verification"
grep -Fq 'zterm-relay-healthcheck' "$dockerfile" || fail "runtime health probe is missing"
grep -Fq 'STOPSIGNAL SIGINT' "$dockerfile" || fail "stop signal bypasses upstream graceful shutdown"
grep -Fq '!healthcheck.rs' "$dockerignore" || fail "health probe is excluded from build context"
grep -Fq 'iroh = { version = "=1.0.3"' "$handshake_manifest" \
    || fail "public handshake probe does not pin Iroh 1.0.3"
grep -Fq 'RelayConfig::new(relay_url.clone(), None)' "$handshake_source" \
    || fail "public handshake probe does not explicitly disable QAD"
grep -Fq 'RUSTSEC-2024-0436' "$handshake_deny" \
    || fail "public handshake probe dependency exception is undocumented"

test -f "$publish_workflow" || fail "relay image publication workflow is missing"
test -f "$publication_resolver" || fail "relay publication resolver is missing"
test -f "$publication_channel_test" || fail "relay publication channel test is missing"
test -x "$image_reference_validator" \
    || fail "production image-reference validator is not executable"
workflow_permissions=$(awk '
    /^permissions:/ { in_permissions = 1; next }
    in_permissions && /^[^[:space:]]/ { in_permissions = 0 }
    in_permissions && /^  [a-z-]+:/ { print }
' "$publish_workflow")
[ "$(grep -Ec '^[[:space:]]*permissions:' "$publish_workflow")" -eq 1 ] \
    || fail "relay publication workflow overrides permissions outside the top-level policy"
[ "$workflow_permissions" = "  contents: read
  packages: write" ] || fail "relay publication permissions are not least-privilege"
workflow_actions=$(sed -n 's/^[[:space:]]*uses: //p' "$publish_workflow")
[ -n "$workflow_actions" ] || fail "relay publication workflow has no pinned actions"
if printf '%s\n' "$workflow_actions" \
    | grep -Ev '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+@[0-9a-f]{40}([[:space:]]+#.*)?$'; then
    fail "relay publication workflow action is not pinned by full commit SHA"
fi
for action_pin in \
    'actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1' \
    'docker/setup-qemu-action@96fe6ef7f33517b61c61be40b68a1882f3264fb8' \
    'docker/setup-buildx-action@37fe631027851001ddb9b187196cc803df7f5f0e' \
    'docker/login-action@dbcb813823bdd20940b903addbd779551569679f' \
    'docker/metadata-action@dc802804100637a589fabce1cb79ff13a1411302' \
    'docker/build-push-action@53b7df96c91f9c12dcc8a07bcb9ccacbed38856a'; do
    grep -Fq "$action_pin" "$publish_workflow" \
        || fail "verified current Action pin is missing: $action_pin"
done
grep -Fq 'release:' "$publish_workflow" \
    || fail "stable GitHub release publication trigger is missing"
grep -Fq 'workflow_dispatch:' "$publish_workflow" \
    || fail "manual relay publication trigger is missing"
grep -Fq "ref: \${{ github.event_name == 'release' && format('refs/tags/{0}', github.event.release.tag_name) || github.sha }}" \
    "$publish_workflow" || fail "release publication does not check out the exact release tag"
grep -Fq 'persist-credentials: false' "$publish_workflow" \
    || fail "checkout leaves GitHub credentials in the relay publication workspace"
grep -Fq 'RELEASE_PRERELEASE:' "$publish_workflow" \
    || fail "prerelease channel is not distinguished"
grep -Fq 'run: sh deploy/relay/resolve-publication.sh' "$publish_workflow" \
    || fail "workflow bypasses the tested publication channel resolver"
grep -Fq 'GITHUB_REPOSITORY_OWNER: ${{ github.repository_owner }}' \
    "$publish_workflow" || fail "GHCR owner is not derived from the repository"
grep -Fq 'image="ghcr.io/${owner}/zterm-relay${image_suffix}"' \
    "$publication_resolver" || fail "canonical GHCR channel paths are missing"
grep -Fq 'image_suffix=-dev' "$publication_resolver" \
    || fail "development GHCR package is not selected"
grep -Fq 'publish_latest=true' "$publication_resolver" \
    || fail "stable release latest alias is missing"
grep -Fq 'type=raw,value=${{ steps.publish.outputs.version }}' "$publish_workflow" \
    || fail "resolved publication version does not drive the image tag"
grep -Fq 'type=raw,value=latest,enable=${{ steps.publish.outputs.publish_latest }}' \
    "$publish_workflow" || fail "stable latest alias is not gated by the resolver"
grep -Fq 'registry: ghcr.io' "$publish_workflow" \
    || fail "workflow does not authenticate to GHCR"
grep -Fq 'org.opencontainers.image.source=${{ github.server_url }}/${{ github.repository }}' \
    "$publish_workflow" || fail "published image is not linked to its source repository"
grep -Fq 'context: ./deploy/relay' "$publish_workflow" \
    || fail "workflow uses the wrong relay build context"
grep -Fq 'platforms: linux/amd64,linux/arm64' "$publish_workflow" \
    || fail "workflow does not publish one amd64/arm64 build"
grep -Fq 'digest: ${{ steps.build.outputs.digest }}' "$publish_workflow" \
    || fail "workflow does not expose the published manifest digest"
grep -Fq 'reference: ${{ steps.reference.outputs.reference }}' "$publish_workflow" \
    || fail "workflow does not expose the immutable image reference"
grep -Fq '[[ ! "$DIGEST" =~ ^sha256:[0-9a-f]{64}$ ]]' "$publish_workflow" \
    || fail "workflow does not validate the published manifest digest"
grep -Fq 'reference="$IMAGE@$DIGEST"' "$publish_workflow" \
    || fail "workflow does not construct the exact immutable image reference"
grep -Fq 'provenance: mode=max' "$publish_workflow" \
    || fail "published relay image omits provenance"
grep -Fq 'sbom: true' "$publish_workflow" \
    || fail "published relay image omits its SBOM"
grep -Fq 'push: true' "$publish_workflow" \
    || fail "relay publication build does not push its manifest"
[ "$(grep -Fc 'uses: docker/build-push-action@' "$publish_workflow")" -eq 1 ] \
    || fail "relay publication must build exactly one multi-platform manifest"
grep -Fq 'sh deploy/relay/validate-image-reference.sh "$reference"' "$publish_workflow" \
    || fail "stable publication reference bypasses production validation"
grep -Fq '[[ "$IMAGE" != */zterm-relay-dev ]]' "$publish_workflow" \
    || fail "development publication reference does not enforce the dev package"
sh "$publication_channel_test"

grep -Fq 'access = "everyone"' "$relay_config" || fail "local access is not Everyone"
grep -Fq 'access = "everyone"' "$production_compose" || fail "production access is not Everyone"
grep -Fq 'access = "everyone"' "$reverse_proxy_config" \
    || fail "reverse-proxy access is not Everyone"
if grep -Eiq '^[[:space:]]*(access\.(shared_token|allowlist|denylist)|\[limits)' \
    "$relay_config" "$production_compose" "$reverse_proxy_config"; then
    fail "access restriction or relay limit was added"
fi
if grep -Eiq 'monitor|sidecar' \
    "$local_compose" "$production_compose" "$reverse_proxy_compose"; then
    fail "custom monitor service was added"
fi
grep -Fq '127.0.0.1:${RELAY_METRICS_PORT' "$production_compose" \
    || fail "production metrics do not default to host loopback"
if grep -Fq 'RELAY_METRICS_BIND_IP' "$production_compose"; then
    fail "production metrics host binding can be changed from loopback"
fi
grep -Fq 'http_bind_addr = "0.0.0.0:38451"' "$reverse_proxy_config" \
    || fail "reverse-proxy upstream does not listen on container port 38451"
grep -Fq 'enable_quic_addr_discovery = false' "$reverse_proxy_config" \
    || fail "reverse-proxy QAD boundary is not explicit"
if grep -Eq '^[[:space:]]*\[tls\]' "$reverse_proxy_config"; then
    fail "reverse-proxy upstream unexpectedly owns TLS"
fi
grep -Fq '127.0.0.1:${RELAY_PROXY_PORT:-38451}:38451/tcp' \
    "$reverse_proxy_compose" \
    || fail "reverse-proxy listener is not hard-bound to host loopback port 38451"
grep -Fq '127.0.0.1:${RELAY_METRICS_PORT:-9090}:9090/tcp' \
    "$reverse_proxy_compose" \
    || fail "reverse-proxy metrics are not hard-bound to host loopback"
if grep -Eq ':[[:space:]]*(80|443):|7842/udp|RELAY_PUBLIC_BIND_IP|RELAY_ACME|\[tls\]' \
    "$reverse_proxy_compose"; then
    fail "reverse-proxy mode exposes direct TLS, ACME, or QAD ports"
fi
if grep -Fq -- '--dev' "$reverse_proxy_compose"; then
    fail "reverse-proxy production mode uses --dev"
fi
for production_file in "$production_compose" "$reverse_proxy_compose"; do
    grep -Fq 'image: "${RELAY_IMAGE:?Set RELAY_IMAGE to an immutable GHCR digest}"' \
        "$production_file" || fail "production Compose does not require RELAY_IMAGE"
    if grep -Eq '^[[:space:]]+build:' "$production_file"; then
        fail "production Compose permits a deployment-host build"
    fi
done
for example_env in "$production_env" "$reverse_proxy_env"; do
    grep -Fqx 'RELAY_IMAGE=ghcr.io/leonfox28/zterm-relay@sha256:REPLACE_WITH_PUBLISHED_DIGEST' \
        "$example_env" || fail "production env example is not a GHCR digest reference"
done
valid_image=ghcr.io/leonfox28/zterm-relay@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
[ "$("$image_reference_validator" "$valid_image")" = "$valid_image" ] \
    || fail "production image validator rejected a valid GHCR digest"
[ "$(RELAY_IMAGE="$valid_image" "$image_reference_validator")" = "$valid_image" ] \
    || fail "production image validator rejected RELAY_IMAGE from the environment"
valid_fork_image=ghcr.io/example-owner/zterm-relay@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
[ "$("$image_reference_validator" "$valid_fork_image")" = "$valid_fork_image" ] \
    || fail "production image validator rejected the deliberate fork-owner contract"
valid_single_character_owner=ghcr.io/a/zterm-relay@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
[ "$("$image_reference_validator" "$valid_single_character_owner")" = "$valid_single_character_owner" ] \
    || fail "production image validator rejected a valid one-character owner"
if RELAY_IMAGE= "$image_reference_validator" >/dev/null 2>&1; then
    fail "production image validator accepted a missing image reference"
fi
if "$image_reference_validator" "$valid_image" "$valid_image" >/dev/null 2>&1; then
    fail "production image validator accepted multiple image references"
fi
for invalid_image in \
    'ghcr.io/example-owner/zterm-relay:latest' \
    'ghcr.io/example-owner/zterm-relay@sha256:1234' \
    'ghcr.io//zterm-relay@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' \
    'ghcr.io/-example/zterm-relay@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' \
    'ghcr.io/example-/zterm-relay@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' \
    'ghcr.io/example_owner/zterm-relay@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' \
    'ghcr.io/example--owner/zterm-relay@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' \
    'ghcr.io/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/zterm-relay@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' \
    'ghcr.io/Example/zterm-relay@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' \
    'ghcr.io/example/another/zterm-relay@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' \
    'ghcr.io/example-owner/not-zterm-relay@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' \
    'ghcr.io/example-owner/zterm-relay-dev@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' \
    'ghcr.io/example-owner/zterm-relay:stable@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' \
    'ghcr.io/example-owner/zterm-relay@sha256:0123456789ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef' \
    'ghcr.io/example-owner/zterm-relay@sha512:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' \
    'docker.io/example-owner/zterm-relay@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' \
    'sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' \
    'zterm/iroh-relay:1.0.3-local'; do
    if "$image_reference_validator" "$invalid_image" >/dev/null 2>&1; then
        fail "production image validator accepted: $invalid_image"
    fi
done
grep -Fq '/opt/1panel/docker/compose/zterm-relay' "$relay_doc" \
    || fail "1Panel Compose deployment root is not documented"
grep -Fq '/opt/1panel/docker/compose/zterm-relay' "$relay_spec" \
    || fail "1Panel Compose deployment root is absent from the relay contract"
grep -Fq 'ghcr.io/leonfox28/zterm-relay' "$relay_doc" \
    || fail "official production GHCR package is not documented"
grep -Fq 'ghcr.io/leonfox28/zterm-relay-dev' "$relay_doc" \
    || fail "official development GHCR package is not documented"
grep -Fq 'ghcr.io/leonfox28/zterm-relay-dev' "$relay_spec" \
    || fail "development GHCR package is absent from the relay contract"
grep -Fq 'Prometheus metrics' "$relay_doc" \
    || fail "private port 9090 purpose is not documented"
grep -Fq '/usr/local/bin/zterm-relay-healthcheck' "$local_compose" \
    || fail "local Compose health check does not probe a live endpoint"
grep -Fq '/usr/local/bin/zterm-relay-healthcheck' "$production_compose" \
    || fail "production Compose health check does not probe a live endpoint"
grep -Fq '/usr/local/bin/zterm-relay-healthcheck' "$reverse_proxy_compose" \
    || fail "reverse-proxy Compose health check does not probe a live endpoint"
if grep -Fq -- '--version' \
    "$local_compose" "$production_compose" "$reverse_proxy_compose"; then
    fail "Compose health check only verifies binary existence"
fi
grep -Fq 'max-size: "10m"' "$production_compose" || fail "log size rotation missing"
grep -Fq 'max-file: "5"' "$production_compose" || fail "log file rotation missing"
grep -Fq 'max-size: "10m"' "$reverse_proxy_compose" \
    || fail "reverse-proxy log size rotation missing"
grep -Fq 'max-file: "5"' "$reverse_proxy_compose" \
    || fail "reverse-proxy log file rotation missing"
grep -Fq -- '--provenance=false' "$repo_root/tests/relay/build-platforms.sh" \
    || fail "multi-platform local evidence includes unstable provenance attestations"
grep -Fq -- '--provenance=false' "$repo_root/tests/relay/smoke.sh" \
    || fail "local smoke evidence includes unstable provenance attestations"

if command -v docker >/dev/null 2>&1 && docker compose version >/dev/null 2>&1; then
    docker compose -f "$local_compose" config --quiet
    services=$(docker compose -f "$local_compose" config --services)
    [ "$services" = "relay" ] || fail "local Compose must contain only the relay service"

    RELAY_IMAGE=ghcr.io/example/zterm-relay@sha256:0000000000000000000000000000000000000000000000000000000000000000 \
        RELAY_HOSTNAME=relay.example.com \
        RELAY_ACME_CONTACT=ops@example.com \
        docker compose -f "$production_compose" config --quiet
    services=$(RELAY_IMAGE=ghcr.io/example/zterm-relay@sha256:0000000000000000000000000000000000000000000000000000000000000000 \
        RELAY_HOSTNAME=relay.example.com \
        RELAY_ACME_CONTACT=ops@example.com \
        docker compose -f "$production_compose" config --services)
    [ "$services" = "relay" ] || fail "production Compose must contain only the relay service"

    RELAY_IMAGE=ghcr.io/example/zterm-relay@sha256:0000000000000000000000000000000000000000000000000000000000000000 \
        docker compose -f "$reverse_proxy_compose" config --quiet
    services=$(RELAY_IMAGE=ghcr.io/example/zterm-relay@sha256:0000000000000000000000000000000000000000000000000000000000000000 \
        docker compose -f "$reverse_proxy_compose" config --services)
    [ "$services" = "relay" ] \
        || fail "reverse-proxy Compose must contain only the relay service"
fi

echo "relay static checks passed"
