#!/bin/sh

set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
container_name=zterm-foundation-network-gate
image_name=zterm-foundation-network-gate:local
container_started=false
relay_override_env=ZTERM_GATE_RELAY_IPV4_OVERRIDES

cleanup() {
    if [ "$container_started" = true ]; then
        docker container rm --force "$container_name" >/dev/null 2>&1 || true
    fi
}

trap cleanup EXIT INT TERM

docker info >/dev/null

if docker container inspect "$container_name" >/dev/null 2>&1; then
    echo "refusing to replace existing container: $container_name" >&2
    exit 1
fi

docker build --pull --tag "$image_name" "$script_dir/network"

# Patchbay uses 198.18.0.0/15 for its simulated network. A host TUN proxy in
# fake-IP mode can return addresses from the same range for the n0 Relay names,
# so resolve the real A records over HTTPS before entering the lab.
relay_ipv4_overrides=$(python3 - <<'PY'
import ipaddress
import json
import urllib.parse
import urllib.request

hosts = (
    "use1-1.relay.n0.iroh.link.",
    "usw1-1.relay.n0.iroh.link.",
    "euc1-1.relay.n0.iroh.link.",
    "aps1-1.relay.n0.iroh.link.",
)
resolved = []
for host in hosts:
    query = urllib.parse.urlencode({"name": host, "type": "A"})
    request = urllib.request.Request(
        f"https://dns.google/resolve?{query}",
        headers={"Accept": "application/dns-json"},
    )
    with urllib.request.urlopen(request, timeout=10) as response:
        payload = json.load(response)
    ipv4 = next(
        answer["data"]
        for answer in payload.get("Answer", ())
        if answer.get("type") == 1
        and ipaddress.ip_address(answer["data"]).version == 4
    )
    resolved.append(f"{host}={ipv4}")
print(";".join(resolved))
PY
)

container_started=true
gate_status=0
docker run \
    --name "$container_name" \
    --rm \
    --privileged \
    --env "$relay_override_env=$relay_ipv4_overrides" \
    --mount "type=bind,source=$repo_root,target=/workspace,readonly" \
    "$image_name" || gate_status=$?

cleanup
container_started=false

if docker container inspect "$container_name" >/dev/null 2>&1; then
    echo "test container remains after cleanup: $container_name" >&2
    exit 1
fi

exit "$gate_status"
