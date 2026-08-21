#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
compose_file="$repo_root/deploy/relay/compose.yaml"
project_name="zterm-relay-phase-zero-smoke"
image="zterm/iroh-relay:1.0.3-local"

cleanup() {
    docker compose --project-name "$project_name" --file "$compose_file" down \
        --volumes --remove-orphans >/dev/null 2>&1 || true
}
trap cleanup EXIT HUP INT TERM

docker buildx build --load --provenance=false --tag "$image" \
    "$repo_root/deploy/relay"
docker compose --project-name "$project_name" --file "$compose_file" up \
    --detach --no-build --wait --wait-timeout 180

attempt=0
until health=$(curl --fail --silent --show-error http://127.0.0.1:3340/healthz 2>/dev/null); do
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 30 ]; then
        echo "relay /healthz did not become ready" >&2
        exit 1
    fi
    sleep 1
done
printf '%s\n' "$health" | grep -Fq '"status":"ok"'
printf '%s\n' "$health" | grep -Fq '"version":"1.0.3"'

metrics=$(curl --fail --silent --show-error http://127.0.0.1:9090/metrics)
[ -n "$metrics" ]
printf '%s\n' "$metrics" | grep -Eq 'relay(server)?_'

container_id=$(docker compose --project-name "$project_name" --file "$compose_file" ps -q relay)
[ "$(docker inspect --format '{{.State.Health.Status}}' "$container_id")" = "healthy" ]
[ "$(docker inspect --format '{{.Config.User}}' "$container_id")" = "65532:65532" ]
[ "$(docker inspect --format '{{.HostConfig.ReadonlyRootfs}}' "$container_id")" = "true" ]
[ "$(docker inspect --format '{{.HostConfig.LogConfig.Type}}' "$container_id")" = "json-file" ]
[ "$(docker inspect --format '{{index .HostConfig.LogConfig.Config "max-size"}}' "$container_id")" = "10m" ]
[ "$(docker inspect --format '{{index .HostConfig.LogConfig.Config "max-file"}}' "$container_id")" = "5" ]
[ "$(docker inspect --format '{{.Config.StopSignal}}' "$container_id")" = "SIGINT" ]

image_id=$(docker image inspect "$image" --format '{{.Id}}')
echo "local relay image id: $image_id"

docker compose --project-name "$project_name" --file "$compose_file" stop \
    --timeout 20 relay >/dev/null
[ "$(docker inspect --format '{{.State.Status}}' "$container_id")" = "exited" ]
[ "$(docker inspect --format '{{.State.ExitCode}}' "$container_id")" = "0" ]

echo "relay Compose smoke test passed"
