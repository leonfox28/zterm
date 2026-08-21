#!/bin/sh
# Canonical metadata and verification helpers for the upstream relay artifact.
# This file is sourced by the Docker build and the host-side regression tests.

IROH_VERSION="1.0.3"
IROH_RELEASE_ROOT="https://github.com/n0-computer/iroh/releases/download"
IROH_RELAY_SHA256_AMD64="9e25e394c6d09b449d86bb222de535d2a6e68de8030ee8ef39f682ab6ff0cd2c"
IROH_RELAY_SHA256_ARM64="331a2f35519a778a5b0a2a34baa7f495d3540b3cdb549b8203cbfdd209df7641"

iroh_relay_resolve() {
    case "${1-}" in
        amd64)
            IROH_RELAY_TARGET="x86_64-unknown-linux-musl"
            IROH_RELAY_SHA256="$IROH_RELAY_SHA256_AMD64"
            ;;
        arm64)
            IROH_RELAY_TARGET="aarch64-unknown-linux-musl"
            IROH_RELAY_SHA256="$IROH_RELAY_SHA256_ARM64"
            ;;
        *)
            echo "unsupported relay architecture: ${1-<unset>}" >&2
            return 64
            ;;
    esac

    IROH_RELAY_ARCHIVE="iroh-relay-v${IROH_VERSION}-${IROH_RELAY_TARGET}.tar.gz"
    IROH_RELAY_URL="${IROH_RELEASE_ROOT}/v${IROH_VERSION}/${IROH_RELAY_ARCHIVE}"
    export IROH_RELAY_ARCHIVE IROH_RELAY_SHA256 IROH_RELAY_TARGET IROH_RELAY_URL
}

iroh_relay_file_sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{ print $1 }'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{ print $1 }'
    else
        echo "neither sha256sum nor shasum is available" >&2
        return 69
    fi
}

iroh_relay_verify() {
    archive_path="${1-}"
    if [ -z "$archive_path" ] || [ ! -f "$archive_path" ]; then
        echo "relay archive does not exist: ${archive_path:-<unset>}" >&2
        return 66
    fi

    actual_sha256=$(iroh_relay_file_sha256 "$archive_path") || return
    if [ "$actual_sha256" != "$IROH_RELAY_SHA256" ]; then
        echo "relay checksum mismatch for $archive_path" >&2
        echo "expected: $IROH_RELAY_SHA256" >&2
        echo "actual:   $actual_sha256" >&2
        return 65
    fi
}

