#!/bin/sh
set -eu

LC_ALL=C
export LC_ALL

fail() {
    echo "invalid relay image publication: $*" >&2
    exit 64
}

event_name=${EVENT_NAME:-}
repository_owner=${GITHUB_REPOSITORY_OWNER:-}
manual_version=${MANUAL_VERSION:-}
release_tag=${RELEASE_TAG:-}
release_prerelease=${RELEASE_PRERELEASE:-}
output_file=${GITHUB_OUTPUT:-/dev/stdout}

[ -n "$event_name" ] || fail "EVENT_NAME is required"
[ -n "$repository_owner" ] || fail "GITHUB_REPOSITORY_OWNER is required"

# GitHub logins are one ASCII path segment. Validate the original value before
# lowercasing it so embedded or trailing newlines cannot become workflow-output
# records (command substitution would otherwise strip trailing newlines).
case "$repository_owner" in
    -* | *- | *--* | *[!A-Za-z0-9-]*)
        fail "GITHUB_REPOSITORY_OWNER is not a valid GitHub owner"
        ;;
esac
[ "${#repository_owner}" -le 39 ] \
    || fail "GITHUB_REPOSITORY_OWNER must not exceed 39 characters"

publish_latest=false
case "$event_name" in
    release)
        [ -n "$release_tag" ] || fail "a GitHub release tag is required"
        version=$release_tag
        case "$version" in
            latest)
                fail "latest is managed only as an alias of a stable GitHub release"
                ;;
        esac

        case "$release_prerelease" in
            false)
                image_suffix=
                channel=stable
                publish_latest=true
                ;;
            true)
                image_suffix=-dev
                channel=prerelease
                ;;
            *)
                fail "RELEASE_PRERELEASE must be true or false for a release"
                ;;
        esac
        ;;
    workflow_dispatch)
        version=$manual_version
        [ -n "$version" ] || fail "manual development version must not be empty"
        [ "$version" != "latest" ] \
            || fail "latest is managed only as an alias of a stable GitHub release"
        image_suffix=-dev
        channel=manual
        ;;
    *)
        fail "unsupported publication event: $event_name"
        ;;
esac

case "$version" in
    [A-Za-z0-9_]*) ;;
    *) fail "version is not a valid OCI image tag" ;;
esac
case "$version" in
    *[!A-Za-z0-9_.-]*) fail "version is not a valid OCI image tag" ;;
esac
[ "${#version}" -le 128 ] || fail "version is not a valid OCI image tag"

owner=$(printf '%s' "$repository_owner" | tr '[:upper:]' '[:lower:]')
image="ghcr.io/${owner}/zterm-relay${image_suffix}"
{
    printf '%s\n' "image=$image"
    printf '%s\n' "version=$version"
    printf '%s\n' "channel=$channel"
    printf '%s\n' "publish_latest=$publish_latest"
} >>"$output_file"
