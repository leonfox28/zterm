#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
compose_file="$repo_root/deploy/relay/compose.reverse-proxy.yaml"
relay_config="$repo_root/deploy/relay/relay.reverse-proxy.toml"
project_name="zterm-relay-reverse-proxy-smoke-$$"
image="zterm/iroh-relay:1.0.3-local"
qad_probe_container="zterm-relay-qad-boundary-$$"

command -v jq >/dev/null 2>&1 || {
    echo "jq is required to inspect Docker port bindings" >&2
    exit 69
}

mkdir -p "$repo_root/target"
# Colima shares the project directory but not macOS's /var/folders temp root.
test_dir=$(mktemp -d "$repo_root/target/relay-reverse-proxy.XXXXXX")
headers_file="$test_dir/websocket-headers.txt"

# Let Docker choose collision-free host ports. Static checks separately assert
# that the deployment defaults are 127.0.0.1:38451 and 127.0.0.1:9090.
export RELAY_PROXY_PORT=0
export RELAY_METRICS_PORT=0

cleanup() {
    docker compose --project-name "$project_name" --file "$compose_file" down \
        --volumes --remove-orphans >/dev/null 2>&1 || true
    docker rm --force "$qad_probe_container" >/dev/null 2>&1 || true
    rm -rf "$test_dir"
}
trap cleanup EXIT HUP INT TERM

if ! docker image inspect "$image" >/dev/null 2>&1; then
    docker buildx build --load --provenance=false --tag "$image" \
        "$repo_root/deploy/relay"
fi
image_id=$(docker image inspect "$image" --format '{{.Id}}')
# Start production Compose from an immutable local image ID and forbid a
# fallback build. Local IDs are deliberately allowed only inside this runtime
# test (and the recorded initial bootstrap exception); real deployments must
# pass the validated GHCR repository digest.
export RELAY_IMAGE="$image_id"

# The exact upstream binary must reject QAD without TLS. This negative probe
# protects the documented boundary: the reverse-proxy mode may omit QAD, but it
# cannot silently turn UDP discovery on behind an HTTP-only proxy.
sed 's/enable_quic_addr_discovery = false/enable_quic_addr_discovery = true/' \
    "$relay_config" >"$test_dir/qad-without-tls.toml"
if docker run --name "$qad_probe_container" --network none \
    --mount "type=bind,src=$test_dir/qad-without-tls.toml,dst=/etc/iroh-relay/relay.toml,readonly" \
    "$image" --config-path /etc/iroh-relay/relay.toml >/dev/null 2>&1; then
    echo "Iroh unexpectedly accepted QAD without TLS" >&2
    exit 1
fi
docker logs "$qad_probe_container" 2>&1 \
    | grep -Fq 'TLS must be configured in order to spawn a QUIC endpoint'
docker rm "$qad_probe_container" >/dev/null

docker compose --project-name "$project_name" --file "$compose_file" up \
    --detach --no-build --wait --wait-timeout 180

container_id=$(docker compose --project-name "$project_name" \
    --file "$compose_file" ps -q relay)
relay_binding=$(docker compose --project-name "$project_name" \
    --file "$compose_file" port relay 38451)
metrics_binding=$(docker compose --project-name "$project_name" \
    --file "$compose_file" port relay 9090)
relay_port=${relay_binding##*:}
metrics_port=${metrics_binding##*:}

[ "${relay_binding%:*}" = "127.0.0.1" ]
[ "${metrics_binding%:*}" = "127.0.0.1" ]
[ "$(docker inspect --format '{{.State.Health.Status}}' "$container_id")" = "healthy" ]
[ "$(docker inspect --format '{{.Config.User}}' "$container_id")" = "65532:65532" ]
[ "$(docker inspect --format '{{.HostConfig.ReadonlyRootfs}}' "$container_id")" = "true" ]
[ "$(docker inspect --format '{{.Config.Image}}' "$container_id")" = "$RELAY_IMAGE" ]
[ "$(docker inspect --format '{{.Image}}' "$container_id")" = "$image_id" ]
[ "$(docker inspect --format '{{.HostConfig.Privileged}}' "$container_id")" = "false" ]
[ "$(docker inspect --format '{{.HostConfig.PublishAllPorts}}' "$container_id")" = "false" ]
[ "$(docker inspect --format '{{.HostConfig.RestartPolicy.Name}}' "$container_id")" = "unless-stopped" ]
[ "$(docker inspect --format '{{.HostConfig.LogConfig.Type}}' "$container_id")" = "json-file" ]
[ "$(docker inspect --format '{{index .HostConfig.LogConfig.Config "max-size"}}' "$container_id")" = "10m" ]
[ "$(docker inspect --format '{{index .HostConfig.LogConfig.Config "max-file"}}' "$container_id")" = "5" ]
[ "$(docker inspect --format '{{.Config.StopSignal}}' "$container_id")" = "SIGINT" ]
[ "$(docker inspect --format '{{json .HostConfig.CapDrop}}' "$container_id")" = '["ALL"]' ]
[ "$(docker inspect --format '{{json .HostConfig.SecurityOpt}}' "$container_id")" = '["no-new-privileges:true"]' ]
docker inspect --format '{{json .Mounts}}' "$container_id" | jq -e '
    length == 1
    and .[0].Destination == "/etc/iroh-relay/relay.toml"
    and .[0].RW == false
' >/dev/null

health=$(curl --fail --silent --show-error "http://127.0.0.1:$relay_port/healthz")
printf '%s\n' "$health" | grep -Fq '"status":"ok"'
printf '%s\n' "$health" | grep -Fq '"version":"1.0.3"'
curl --fail --silent --show-error --output /dev/null \
    "http://127.0.0.1:$relay_port/generate_204"

metrics=$(curl --fail --silent --show-error "http://127.0.0.1:$metrics_port/metrics")
[ -n "$metrics" ]
printf '%s\n' "$metrics" | grep -Eq 'relay(server)?_'

# A reverse proxy must preserve this upgrade. We stop after the HTTP 101 because
# a complete authenticated relay handshake belongs to the public acceptance
# test, but this still proves the production listener (without `--dev`) exposes
# the actual Iroh relay WebSocket endpoint rather than only a health route.
curl --silent --show-error --http1.1 --max-time 2 \
    --dump-header "$headers_file" --output /dev/null \
    --header 'Connection: Upgrade' \
    --header 'Upgrade: websocket' \
    --header 'Sec-WebSocket-Version: 13' \
    --header 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' \
    --header 'Sec-WebSocket-Protocol: iroh-relay-v2' \
    "http://127.0.0.1:$relay_port/relay" >/dev/null 2>&1 || true
grep -Eq '^HTTP/1\.[01] 101 ' "$headers_file"
grep -Eiq '^upgrade:[[:space:]]*websocket' "$headers_file"
grep -Eiq '^sec-websocket-protocol:[[:space:]]*iroh-relay-v2' "$headers_file"

port_bindings=$(docker inspect --format '{{json .HostConfig.PortBindings}}' "$container_id")
printf '%s\n' "$port_bindings" | jq -e '
    (keys | sort) == ["38451/tcp", "9090/tcp"]
    and .["38451/tcp"][0].HostIp == "127.0.0.1"
    and .["9090/tcp"][0].HostIp == "127.0.0.1"
' >/dev/null

container_command=$(docker inspect --format '{{json .Config.Cmd}}' "$container_id")
printf '%s\n' "$container_command" | grep -Fq -- '--config-path'
if printf '%s\n' "$container_command" | grep -Fq -- '--dev'; then
    echo "reverse-proxy production mode unexpectedly uses --dev" >&2
    exit 1
fi

docker compose --project-name "$project_name" --file "$compose_file" stop \
    --timeout 20 relay >/dev/null
[ "$(docker inspect --format '{{.State.Status}}' "$container_id")" = "exited" ]
[ "$(docker inspect --format '{{.State.ExitCode}}' "$container_id")" = "0" ]

echo "reverse-proxy relay Compose smoke test passed"
