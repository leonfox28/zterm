#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
compose_file="$repo_root/deploy/relay/compose.production.yaml"
image="zterm/iroh-relay:1.0.3-local"
container_name="zterm-relay-production-config-smoke-$$"

command -v jq >/dev/null 2>&1 || {
    echo "jq is required to extract the rendered Compose config" >&2
    exit 69
}

mkdir -p "$repo_root/target"
# Colima shares the project directory but not macOS's /var/folders temp root.
test_dir=$(mktemp -d "$repo_root/target/relay-production-config.XXXXXX")
cleanup() {
    docker rm --force "$container_name" >/dev/null 2>&1 || true
    rm -rf "$test_dir"
}
trap cleanup EXIT HUP INT TERM

RELAY_IMAGE="$image" \
    RELAY_HOSTNAME=relay.example.com \
    RELAY_ACME_CONTACT=ops@example.com \
    docker compose --file "$compose_file" config --format json \
    | jq --raw-output '.configs.relay_config.content' >"$test_dir/relay.toml"

docker image inspect "$image" >/dev/null
docker run --detach --name "$container_name" \
    --read-only \
    --network none \
    --tmpfs /tmp:rw,noexec,nosuid,size=16m \
    --tmpfs /var/lib/iroh-relay:rw,uid=65532,gid=65532 \
    --cap-drop ALL \
    --security-opt no-new-privileges \
    --env IROH_RELAY_ACME_URL=http://127.0.0.1:65535/directory \
    --mount "type=bind,src=$test_dir/relay.toml,dst=/etc/iroh-relay/relay.toml,readonly" \
    "$image" --config-path /etc/iroh-relay/relay.toml >/dev/null

# Invalid known field types or listener configuration exit synchronously. The
# disabled network plus loopback ACME URL make public ACME access impossible.
sleep 3
if [ "$(docker inspect --format '{{.State.Running}}' "$container_name")" != "true" ]; then
    docker logs "$container_name" >&2
    exit 1
fi

[ "$(docker inspect --format '{{.HostConfig.NetworkMode}}' "$container_name")" = "none" ]
docker exec "$container_name" /usr/local/bin/zterm-relay-healthcheck \
    127.0.0.1:80 /generate_204 204
docker cp "$container_name:/etc/ssl/certs/ca-certificates.crt" \
    "$test_dir/ca-certificates.crt" >/dev/null
[ -s "$test_dir/ca-certificates.crt" ]

if docker logs "$container_name" 2>&1 \
    | grep -Eiq 'config must be valid|TLS must be configured|permission denied|read-only file system'; then
    docker logs "$container_name" >&2
    exit 1
fi

echo "Iroh 1.0.3 production config, offline startup, and low-port health checks passed"
