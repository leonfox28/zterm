#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)

# shellcheck source=../../deploy/relay/artifact.sh
. "$repo_root/deploy/relay/artifact.sh"

download() {
    url=$1
    destination=$2
    if command -v curl >/dev/null 2>&1; then
        curl --fail --location --silent --show-error --output "$destination" "$url"
    elif command -v wget >/dev/null 2>&1; then
        wget -q -O "$destination" "$url"
    else
        echo "curl or wget is required" >&2
        return 69
    fi
}

test_dir=$(mktemp -d)
trap 'rm -rf "$test_dir"' EXIT HUP INT TERM

for relay_arch in amd64 arm64; do
    iroh_relay_resolve "$relay_arch"
    archive_path="$test_dir/$IROH_RELAY_ARCHIVE"
    download "$IROH_RELAY_URL" "$archive_path"
    iroh_relay_verify "$archive_path"
    tar -tzf "$archive_path" | grep -Eq '(^|\./)iroh-relay$'
    echo "verified $IROH_RELAY_ARCHIVE"
done

