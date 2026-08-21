#!/bin/sh
set -eu

fail() {
    echo "invalid production RELAY_IMAGE: $*" >&2
    exit 64
}

[ "$#" -le 1 ] || fail "pass one image reference or set RELAY_IMAGE"
if [ "$#" -eq 1 ]; then
    image_reference=$1
else
    image_reference=${RELAY_IMAGE:-}
fi

[ -n "$image_reference" ] || fail "an immutable GHCR digest is required"

case "$image_reference" in
    ghcr.io/*/zterm-relay-dev@*)
        fail "development package zterm-relay-dev is never valid for production"
        ;;
    ghcr.io/*/zterm-relay@sha256:*) ;;
    *) fail "expected ghcr.io/<github-owner>/zterm-relay@sha256:<digest>" ;;
esac

repository=${image_reference%@sha256:*}
owner=${repository#ghcr.io/}
owner=${owner%/zterm-relay}
digest=${image_reference##*@sha256:}

case "$owner" in
    "" | -* | *- | *[!a-z0-9-]*)
        fail "GitHub owner must be one lowercase path segment"
        ;;
    *--*)
        fail "GitHub owner must not contain consecutive hyphens"
        ;;
esac
[ "${#owner}" -le 39 ] || fail "GitHub owner must not exceed 39 characters"

[ "${#digest}" -eq 64 ] || fail "sha256 digest must contain exactly 64 hex characters"
case "$digest" in
    *[!0-9a-f]*) fail "sha256 digest must use lowercase hexadecimal" ;;
esac

printf '%s\n' "$image_reference"
