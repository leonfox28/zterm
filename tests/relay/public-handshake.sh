#!/bin/sh
set -eu

if [ "$#" -ne 1 ]; then
    echo "usage: $0 <relay-url>" >&2
    exit 64
fi

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
manifest="$repo_root/tests/relay/handshake-probe/Cargo.toml"

CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$repo_root/target/relay-handshake-probe}" \
    cargo +1.98.0 run --locked --quiet --manifest-path "$manifest" -- "$@"
