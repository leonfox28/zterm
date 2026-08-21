#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
relay_context="$repo_root/deploy/relay"

for relay_arch in amd64 arm64; do
    image="zterm/iroh-relay:test-$relay_arch"
    docker buildx build --load --provenance=false --platform "linux/$relay_arch" \
        --build-arg "TARGETARCH=$relay_arch" --tag "$image" "$relay_context"

    actual_arch=$(docker image inspect "$image" --format '{{.Architecture}}')
    [ "$actual_arch" = "$relay_arch" ] || {
        echo "expected $relay_arch image, got $actual_arch" >&2
        exit 1
    }

    docker run --rm --platform "linux/$relay_arch" "$image" --version \
        | grep -F 'iroh-relay 1.0.3'
done
