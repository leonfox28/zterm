#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
artifact_file="$repo_root/deploy/relay/artifact.sh"
dockerfile="$repo_root/deploy/relay/Dockerfile"
dockerignore="$repo_root/deploy/relay/.dockerignore"
compose_file="$repo_root/deploy/relay/compose.yaml"
relay_config="$repo_root/deploy/relay/relay.toml"
publish_workflow="$repo_root/.github/workflows/relay-image.yml"
acceptance_workflow="$repo_root/.github/workflows/public-relay-acceptance.yml"
publication_resolver="$repo_root/deploy/relay/resolve-publication.sh"
relay_doc="$repo_root/docs/relay.md"
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

grep -Fq 'alpine:3.23.3@sha256:' "$dockerfile" || fail "base image is not pinned"
grep -Fq 'docker/dockerfile:1.7@sha256:' "$dockerfile" \
    || fail "Dockerfile frontend is not pinned"
grep -Fq 'FROM scratch AS runtime' "$dockerfile" || fail "runtime is not scratch-based"
grep -Fq 'USER 65532:65532' "$dockerfile" || fail "runtime is not non-root"
grep -Fq 'iroh_relay_verify' "$dockerfile" || fail "artifact checksum is bypassed"
grep -Fqx 'EXPOSE 38451/tcp' "$dockerfile" || fail "runtime exposure is not minimal"
grep -Fqx 'CMD ["--config-path", "/etc/iroh-relay/relay.toml"]' "$dockerfile" \
    || fail "runtime default config command is missing"
grep -Fq 'STOPSIGNAL SIGINT' "$dockerfile" || fail "upstream stop signal is missing"
if grep -Fq 'zterm-relay-healthcheck' "$dockerfile"; then
    fail "obsolete custom health probe remains in the image"
fi
[ "$(sed '/^$/d' "$dockerignore")" = "*
!Dockerfile
!artifact.sh" ] || fail "relay build context contains unnecessary files"

for obsolete_file in \
    compose.production.yaml \
    compose.reverse-proxy.yaml \
    relay.reverse-proxy.toml \
    .env.example \
    .env.reverse-proxy.example \
    validate-image-reference.sh \
    healthcheck.rs; do
    [ ! -e "$repo_root/deploy/relay/$obsolete_file" ] \
        || fail "obsolete deployment artifact remains: $obsolete_file"
done

grep -Fq 'iroh = { version = "=1.0.3"' "$handshake_manifest" \
    || fail "public handshake probe does not pin Iroh 1.0.3"
grep -Fq 'RelayConfig::new(relay_url.clone(), None)' "$handshake_source" \
    || fail "public handshake probe does not explicitly disable QAD"
if grep -Fq 'expect_reconnect' "$handshake_source"; then
    fail "obsolete reconnect exercise remains in the public probe"
fi
grep -Fq 'RUSTSEC-2024-0436' "$handshake_deny" \
    || fail "public handshake dependency exception is undocumented"

[ "$(grep -Ec '^[[:space:]]*permissions:' "$acceptance_workflow")" -eq 1 ] \
    || fail "public acceptance workflow overrides its top-level permissions"
acceptance_permissions=$(awk '
    /^permissions:/ { in_permissions = 1; next }
    in_permissions && /^[^[:space:]]/ { in_permissions = 0 }
    in_permissions && /^  [a-z-]+:/ { print }
' "$acceptance_workflow")
[ "$acceptance_permissions" = "  contents: read" ] \
    || fail "public acceptance permissions are not read-only"
grep -Fq 'workflow_dispatch:' "$acceptance_workflow" \
    || fail "public acceptance is not manually dispatched"
if grep -Eq '^[[:space:]]+(push|pull_request|schedule):' "$acceptance_workflow"; then
    fail "public acceptance must not run from push, pull request, or schedule"
fi
grep -Fq 'timeout-minutes: 10' "$acceptance_workflow" \
    || fail "public acceptance job is not bounded"
grep -Fq 'ZTERM_ACCEPTANCE_RELAY_URL: ${{ inputs.relay_url }}' "$acceptance_workflow" \
    || fail "public acceptance input bypasses the quoted environment boundary"
grep -Fq -- "--proto '=https'" "$acceptance_workflow" \
    || fail "public acceptance HTTP checks permit a non-HTTPS protocol"
grep -Fq -- '--max-time 15' "$acceptance_workflow" \
    || fail "public acceptance HTTP checks are not bounded"
grep -Fq -- '--retry 0' "$acceptance_workflow" \
    || fail "public acceptance HTTP checks can retry"
for contract in '/healthz 200' '/generate_204 204' '/ping 200'; do
    grep -Fq "expect_status $contract" "$acceptance_workflow" \
        || fail "public acceptance omits $contract"
done
grep -Fq 'sh tests/relay/public-handshake.sh "$ZTERM_ACCEPTANCE_RELAY_URL"' \
    "$acceptance_workflow" || fail "public acceptance bypasses the handshake probe"

workflow_permissions=$(awk '
    /^permissions:/ { in_permissions = 1; next }
    in_permissions && /^[^[:space:]]/ { in_permissions = 0 }
    in_permissions && /^  [a-z-]+:/ { print }
' "$publish_workflow")
[ "$(grep -Ec '^[[:space:]]*permissions:' "$publish_workflow")" -eq 1 ] \
    || fail "publication workflow overrides its top-level permissions"
[ "$workflow_permissions" = "  contents: read
  packages: write" ] || fail "publication permissions are not least-privilege"
workflow_actions=$(sed -n 's/^[[:space:]]*uses: //p' "$publish_workflow")
[ -n "$workflow_actions" ] || fail "publication workflow has no actions"
if printf '%s\n' "$workflow_actions" \
    | grep -Ev '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+@[0-9a-f]{40}([[:space:]]+#.*)?$'; then
    fail "publication workflow action is not pinned by full commit SHA"
fi
grep -Fq 'release:' "$publish_workflow" \
    || fail "stable and prerelease publication trigger is missing"
grep -Fq 'workflow_dispatch:' "$publish_workflow" \
    || fail "manual development publication trigger is missing"
grep -Fq "ref: \${{ github.event_name == 'release' && format('refs/tags/{0}', github.event.release.tag_name) || github.sha }}" \
    "$publish_workflow" || fail "publication does not check out the selected release tag"
grep -Fq 'persist-credentials: false' "$publish_workflow" \
    || fail "publication checkout retains unnecessary Git credentials"
grep -Fq 'run: sh deploy/relay/resolve-publication.sh' "$publish_workflow" \
    || fail "publication workflow bypasses its resolver"
grep -Fq 'GITHUB_REPOSITORY_OWNER: ${{ github.repository_owner }}' "$publish_workflow" \
    || fail "GHCR owner is not derived from the repository"
grep -Fq 'MANUAL_VERSION: ${{ inputs.version }}' "$publish_workflow" \
    || fail "manual tag is not passed to the publication resolver"
grep -Fq 'RELEASE_TAG: ${{ github.event.release.tag_name }}' "$publish_workflow" \
    || fail "release tag is not passed to the publication resolver"
grep -Fq 'RELEASE_PRERELEASE: ${{ github.event.release.prerelease }}' "$publish_workflow" \
    || fail "release channel flag is not passed to the publication resolver"
grep -Fq 'registry: ghcr.io' "$publish_workflow" \
    || fail "publication does not authenticate to GHCR"
grep -Fq 'images: ${{ steps.publish.outputs.image }}' "$publish_workflow" \
    || fail "resolved production/development package does not drive publication"
grep -Fq 'type=raw,value=${{ steps.publish.outputs.version }}' "$publish_workflow" \
    || fail "resolved release tag does not drive the image tag"
grep -Fq 'type=raw,value=latest,enable=${{ steps.publish.outputs.publish_latest }}' \
    "$publish_workflow" || fail "latest is not stable-channel-only"
grep -Fq 'context: ./deploy/relay' "$publish_workflow" \
    || fail "publication uses the wrong build context"
grep -Fq 'platforms: linux/amd64,linux/arm64' "$publish_workflow" \
    || fail "publication is not one amd64/arm64 build"
grep -Fq 'push: true' "$publish_workflow" \
    || fail "publication build does not push its image"
grep -Fq 'provenance: false' "$publish_workflow" \
    || fail "unused provenance manifests are not disabled"
if grep -Eq 'sbom:|provenance:[[:space:]]+mode=|validate-image-reference|steps[.]build[.]outputs[.]digest' \
    "$publish_workflow"; then
    fail "obsolete attestation or digest deployment logic remains"
fi
[ "$(grep -Fc 'uses: docker/build-push-action@' "$publish_workflow")" -eq 1 ] \
    || fail "publication must build exactly one multi-platform image"

if grep -Eq 'release_tag#v|is_canonical_semver|sha256' "$publication_resolver"; then
    fail "obsolete tag conversion, SemVer parser, or digest logic remains"
fi

grep -Fqx 'http_bind_addr = "0.0.0.0:38451"' "$relay_config" \
    || fail "relay does not listen on container port 38451"
grep -Fqx 'enable_quic_addr_discovery = false' "$relay_config" \
    || fail "QAD boundary is not explicit"
grep -Fqx 'enable_metrics = false' "$relay_config" \
    || fail "unused metrics are not disabled"
grep -Fqx 'access = "everyone"' "$relay_config" || fail "relay access is not Everyone"
if grep -Eiq '^[[:space:]]*(metrics_bind_addr|\[tls\]|\[limits\]|access[.])' "$relay_config"; then
    fail "unsupported metrics, TLS, limits, or access policy remains"
fi

command -v jq >/dev/null 2>&1 || fail "jq is required to inspect Compose"
docker compose -f "$compose_file" config --quiet
compose_json=$(docker compose -f "$compose_file" config --format json)
printf '%s\n' "$compose_json" | jq -e '
    .name == "zterm-relay"
    and (.services | keys) == ["relay"]
    and .services.relay.container_name == "zterm-relay"
    and .services.relay.image == "ghcr.io/leonfox28/zterm-relay:latest"
    and .services.relay.command == null
    and .services.relay.entrypoint == null
    and .services.relay.restart == "unless-stopped"
    and .services.relay.logging == {"driver":"local"}
    and (.services.relay.ports | length) == 1
    and .services.relay.ports[0].host_ip == "127.0.0.1"
    and .services.relay.ports[0].target == 38451
    and (.services.relay.ports[0].published | tostring) == "38451"
    and .services.relay.ports[0].protocol == "tcp"
    and (.services.relay.volumes | length) == 1
    and .services.relay.volumes[0].type == "bind"
    and (.services.relay.volumes[0].source | endswith("/deploy/relay/relay.toml"))
    and .services.relay.volumes[0].target == "/etc/iroh-relay/relay.toml"
    and .services.relay.volumes[0].read_only == true
    and (.services.relay | has("build") | not)
    and (.services.relay | has("environment") | not)
    and (.services.relay | has("healthcheck") | not)
    and (.services.relay | has("read_only") | not)
    and (.services.relay | has("security_opt") | not)
    and (.services.relay | has("tmpfs") | not)
' >/dev/null || fail "Compose does not match the approved minimal contract"

grep -Fq '/opt/1panel/docker/compose/zterm-relay' "$relay_doc" \
    || fail "1Panel deployment root is not documented"
grep -Fq 'ghcr.io/leonfox28/zterm-relay-dev' "$relay_doc" \
    || fail "development package is not documented"

echo "relay static checks passed"
