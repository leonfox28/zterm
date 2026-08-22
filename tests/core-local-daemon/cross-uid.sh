#!/bin/sh

set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)

if [ "$(uname -s)" != Linux ]; then
    echo "cross-UID peer credential gate skipped: Linux CI owns this evidence"
    exit 0
fi

if ! command -v sudo >/dev/null 2>&1 || ! sudo -n -u nobody true >/dev/null 2>&1; then
    if [ -n "${CI:-}" ]; then
        echo "cross-UID peer credential gate failed: CI requires noninteractive nobody privilege" >&2
        exit 1
    fi
    echo "cross-UID peer credential gate skipped: noninteractive nobody privilege unavailable"
    exit 0
fi

cd "$repo_root"
cargo test -p zterm-daemon --test cross_uid
