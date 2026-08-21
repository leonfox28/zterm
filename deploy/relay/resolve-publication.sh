#!/bin/sh
set -eu

LC_ALL=C
export LC_ALL

fail() {
    echo "invalid relay image publication: $*" >&2
    exit 64
}

is_canonical_semver() {
    semver_value=$1

    [ -n "$semver_value" ] || return 1
    case "$semver_value" in
        *[!A-Za-z0-9.-]*) return 1 ;;
    esac

    case "$semver_value" in
        *-*)
            semver_has_prerelease=true
            semver_core=${semver_value%%-*}
            semver_prerelease=${semver_value#*-}
            ;;
        *)
            semver_has_prerelease=false
            semver_core=$semver_value
            semver_prerelease=
            ;;
    esac

    printf '%s\n' "$semver_core" \
        | grep -Eq '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$' \
        || return 1

    if [ "$semver_has_prerelease" = true ]; then
        [ -n "$semver_prerelease" ] || return 1
        printf '%s\n' "$semver_prerelease" \
            | grep -Eq '^[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*$' \
            || return 1

        semver_old_ifs=$IFS
        IFS=.
        # The character check above makes this split immune to shell globbing
        # and guarantees that no identifier is empty.
        set -- $semver_prerelease
        IFS=$semver_old_ifs
        for semver_identifier do
            case "$semver_identifier" in
                0 | *[!0-9]*) ;;
                0*) return 1 ;;
            esac
        done
    fi

    return 0
}

read_workspace_version() {
    workspace_manifest=$1
    [ -f "$workspace_manifest" ] || return 1

    # Parse only [workspace.package].version. In particular, do not accept a
    # dependency's version or an unrelated [package].version as the release
    # version merely because it appears first in Cargo.toml.
    awk '
        BEGIN {
            in_workspace_package = 0
            version_count = 0
        }
        /^[[:space:]]*\[/ {
            in_workspace_package = ($0 ~ /^[[:space:]]*\[workspace[.]package\][[:space:]]*(#.*)?$/)
            next
        }
        in_workspace_package && /^[[:space:]]*version[[:space:]]*=/ {
            if ($0 !~ /^[[:space:]]*version[[:space:]]*=[[:space:]]*"[^"]+"[[:space:]]*(#.*)?$/) {
                malformed = 1
                next
            }
            value = $0
            sub(/^[^=]*=[[:space:]]*"/, "", value)
            sub(/"[[:space:]]*(#.*)?$/, "", value)
            versions[++version_count] = value
        }
        END {
            if (malformed || version_count != 1) {
                exit 1
            }
            print versions[1]
        }
    ' "$workspace_manifest"
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
        case "$release_tag" in
            v*) version=${release_tag#v} ;;
            *)
                fail "GitHub release tags must use canonical vMAJOR.MINOR.PATCH SemVer"
                ;;
        esac
        is_canonical_semver "$version" \
            || fail "GitHub release tags must use canonical SemVer without build metadata"

        if ! workspace_version=$(read_workspace_version "$workspace_manifest"); then
            fail "could not read exactly one [workspace.package].version from Cargo.toml"
        fi
        is_canonical_semver "$workspace_version" \
            || fail "[workspace.package].version must use canonical SemVer without build metadata"
        [ "$version" = "$workspace_version" ] \
            || fail "release version $version does not match workspace version $workspace_version"

        case "$release_prerelease" in
            false)
                case "$version" in
                    *-*) fail "a stable GitHub release tag must not contain a prerelease suffix" ;;
                esac
                image_suffix=
                channel=stable
                publish_latest=true
                ;;
            true)
                case "$version" in
                    *-*) ;;
                    *) fail "a GitHub prerelease tag must contain a prerelease suffix" ;;
                esac
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
