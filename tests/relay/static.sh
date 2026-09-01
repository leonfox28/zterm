#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
artifact_file="$repo_root/deploy/relay/artifact.sh"
dockerfile="$repo_root/deploy/relay/Dockerfile"
dockerignore="$repo_root/deploy/relay/.dockerignore"
compose_file="$repo_root/deploy/relay/compose.yaml"
relay_config="$repo_root/deploy/relay/relay.toml"
publish_workflow="$repo_root/.github/workflows/relay-image.yml"
native_release_workflow="$repo_root/.github/workflows/release.yml"
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
grep -Fq '  workflow_call:' "$publish_workflow" \
    || fail "formal relay publication must be a reusable workflow"
if grep -Eq '^[[:space:]]{2}release:' "$publish_workflow"; then
    fail "formal relay publication must not rely on the implicit release event"
fi
grep -Fq 'workflow_dispatch:' "$publish_workflow" \
    || fail "manual development publication trigger is missing"
grep -Fq 'ref: ${{ inputs.commit || github.sha }}' \
    "$publish_workflow" || fail "formal publication does not check out the frozen commit"
grep -Fq "if: inputs.commit != ''" "$publish_workflow" \
    || fail "formal publication does not distinguish the required frozen input"
grep -Fq "EVENT_NAME: \${{ inputs.commit != '' && 'workflow_call' || 'workflow_dispatch' }}" \
    "$publish_workflow" || fail "publication resolver mode is not derived from trusted inputs"
if grep -Fq "github.event_name == 'workflow_call'" "$publish_workflow"; then
    fail "called workflow must not expect the caller event name to become workflow_call"
fi
grep -Fq 'test "$(git rev-parse HEAD)" = "$EXPECTED_COMMIT"' "$publish_workflow" \
    || fail "formal publication does not prove the frozen checkout"
grep -Fq 'persist-credentials: false' "$publish_workflow" \
    || fail "publication checkout retains unnecessary Git credentials"
grep -Fq 'run: sh deploy/relay/resolve-publication.sh' "$publish_workflow" \
    || fail "publication workflow bypasses its resolver"
grep -Fq 'GITHUB_REPOSITORY_OWNER: ${{ github.repository_owner }}' "$publish_workflow" \
    || fail "GHCR owner is not derived from the repository"
grep -Fq 'MANUAL_VERSION: ${{ inputs.version }}' "$publish_workflow" \
    || fail "manual tag is not passed to the publication resolver"
grep -Fq 'RELEASE_TAG: ${{ inputs.tag }}' "$publish_workflow" \
    || fail "release tag is not passed to the publication resolver"
grep -Fq 'RELEASE_PRERELEASE: ${{ inputs.prerelease }}' "$publish_workflow" \
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

grep -Fq 'uses: ./.github/workflows/relay-image.yml' "$native_release_workflow" \
    || fail "native release does not explicitly call relay publication"
grep -Fq 'commit: ${{ needs.validate.outputs.commit }}' "$native_release_workflow" \
    || fail "native release does not pass the frozen commit to relay publication"
grep -Fq 'tag: ${{ github.ref_name }}' "$native_release_workflow" \
    || fail "native release does not pass the exact tag to relay publication"
relay_call=$(sed -n '/^  relay:/,/^  complete:/p' "$native_release_workflow")
printf '%s\n' "$relay_call" | grep -Fq 'contents: read' \
    || fail "relay caller lacks minimal contents permission"
printf '%s\n' "$relay_call" | grep -Fq 'packages: write' \
    || fail "relay caller lacks package publication permission"
grep -Fq 'ghcr.io/$relay_owner/zterm-relay-dev:$RELEASE_TAG' "$native_release_workflow" \
    || fail "release summary does not identify the prerelease relay target"
grep -Fq 'ghcr.io/$relay_owner/zterm-relay:$RELEASE_TAG and :latest' \
    "$native_release_workflow" || fail "release summary does not identify stable relay targets"
grep -Fq 'gh run rerun $GITHUB_RUN_ID --failed --repo $GITHUB_REPOSITORY' \
    "$native_release_workflow" || fail "release summary lacks a precise relay retry command"

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
