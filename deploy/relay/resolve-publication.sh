#!/bin/sh
set -eu

fail() {
    echo "invalid relay image publication: $*" >&2
    exit 64
}

is_oci_tag() {
    tag=$1
    case "$tag" in
        [A-Za-z0-9_]*) ;;
        *) return 1 ;;
    esac
    case "$tag" in
        *[!A-Za-z0-9_.-]*) return 1 ;;
    esac
    [ "${#tag}" -le 128 ]
}

read_workspace_version() {
    manifest=$1
    package_id=$(cargo +1.98.0 pkgid --locked \
        --manifest-path "$manifest" --package zterm-core 2>/dev/null) || return 1
    case "$package_id" in
        *@*) printf '%s\n' "${package_id##*@}" ;;
        *) return 1 ;;
    esac
}

resolver_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
workspace_manifest=${WORKSPACE_MANIFEST:-"$resolver_dir/../../Cargo.toml"}

event_name=${EVENT_NAME:-}
repository_owner=${GITHUB_REPOSITORY_OWNER:-}
manual_version=${MANUAL_VERSION:-}
release_tag=${RELEASE_TAG:-}
release_prerelease=${RELEASE_PRERELEASE:-}
output_file=${GITHUB_OUTPUT:-/dev/stdout}

[ -n "$event_name" ] || fail "EVENT_NAME is required"
[ -n "$repository_owner" ] || fail "GITHUB_REPOSITORY_OWNER is required"

publish_latest=false
case "$event_name" in
    release)
        if ! workspace_version=$(read_workspace_version "$workspace_manifest"); then
            fail "could not resolve the workspace product version with Cargo"
        fi
        expected_tag="v$workspace_version"
        [ "$release_tag" = "$expected_tag" ] \
            || fail "release tag $release_tag does not match workspace tag $expected_tag"

        version=$release_tag
        case "$release_prerelease" in
            false)
                image_suffix=
                publish_latest=true
                ;;
            true)
                image_suffix=-dev
                ;;
            *) fail "RELEASE_PRERELEASE must be true or false for a release" ;;
        esac
        ;;
    workflow_dispatch)
        version=$manual_version
        [ -n "$version" ] || fail "manual development version must not be empty"
        [ "$version" != "latest" ] \
            || fail "latest is managed only as an alias of a stable GitHub release"
        image_suffix=-dev
        ;;
    *) fail "unsupported publication event: $event_name" ;;
esac

is_oci_tag "$version" || fail "version is not a valid OCI image tag"

owner=$(printf '%s' "$repository_owner" | tr '[:upper:]' '[:lower:]')
image="ghcr.io/${owner}/zterm-relay${image_suffix}"
{
    printf '%s\n' "image=$image"
    printf '%s\n' "version=$version"
    printf '%s\n' "publish_latest=$publish_latest"
} >>"$output_file"
