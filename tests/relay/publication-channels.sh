#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
resolver="$repo_root/deploy/relay/resolve-publication.sh"
test_dir=$(mktemp -d)
trap 'rm -rf "$test_dir"' EXIT HUP INT TERM

fail() {
    echo "relay publication channel check failed: $*" >&2
    exit 1
}

assert_output() {
    output_file=$1
    expected=$2
    grep -Fqx "$expected" "$output_file" \
        || fail "missing resolver output: $expected"
}

assert_output_count() {
    output_file=$1
    expected_count=$2
    actual_count=$(wc -l <"$output_file" | tr -d '[:space:]')
    [ "$actual_count" = "$expected_count" ] \
        || fail "expected $expected_count resolver outputs, found $actual_count"
}

write_workspace_manifest() {
    manifest_path=$1
    manifest_version=$2
    {
        printf '%s\n' '[workspace]' 'members = []'
        printf '%s\n' '[workspace.dependencies]'
        printf '%s\n' 'decoy = { version = "9.9.9" }'
        printf '%s\n' '[workspace.package]'
        printf 'version = "%s"\n' "$manifest_version"
    } >"$manifest_path"
}

stable_output="$test_dir/stable"
EVENT_NAME=release \
GITHUB_REPOSITORY_OWNER=LeonFox28 \
RELEASE_TAG=v0.1.0 \
RELEASE_PRERELEASE=false \
GITHUB_OUTPUT="$stable_output" \
    sh "$resolver"
assert_output "$stable_output" 'image=ghcr.io/leonfox28/zterm-relay'
assert_output "$stable_output" 'version=0.1.0'
assert_output "$stable_output" 'channel=stable'
assert_output "$stable_output" 'publish_latest=true'
assert_output_count "$stable_output" 4

prerelease_manifest="$test_dir/Cargo.prerelease.toml"
write_workspace_manifest "$prerelease_manifest" '0.1.0-rc.1'
prerelease_output="$test_dir/prerelease"
EVENT_NAME=release \
GITHUB_REPOSITORY_OWNER=leonfox28 \
RELEASE_TAG=v0.1.0-rc.1 \
RELEASE_PRERELEASE=true \
WORKSPACE_MANIFEST="$prerelease_manifest" \
GITHUB_OUTPUT="$prerelease_output" \
    sh "$resolver"
assert_output "$prerelease_output" 'image=ghcr.io/leonfox28/zterm-relay-dev'
assert_output "$prerelease_output" 'version=0.1.0-rc.1'
assert_output "$prerelease_output" 'channel=prerelease'
assert_output "$prerelease_output" 'publish_latest=false'
assert_output_count "$prerelease_output" 4

manual_output="$test_dir/manual"
EVENT_NAME=workflow_dispatch \
GITHUB_REPOSITORY_OWNER=leonfox28 \
MANUAL_VERSION=phase-zero \
GITHUB_OUTPUT="$manual_output" \
    sh "$resolver"
assert_output "$manual_output" 'image=ghcr.io/leonfox28/zterm-relay-dev'
assert_output "$manual_output" 'version=phase-zero'
assert_output "$manual_output" 'channel=manual'
assert_output "$manual_output" 'publish_latest=false'
assert_output_count "$manual_output" 4

stable_like_manual_output="$test_dir/stable-like-manual"
EVENT_NAME=workflow_dispatch \
GITHUB_REPOSITORY_OWNER=leonfox28 \
MANUAL_VERSION=v1.2.3 \
GITHUB_OUTPUT="$stable_like_manual_output" \
    sh "$resolver"
assert_output "$stable_like_manual_output" \
    'image=ghcr.io/leonfox28/zterm-relay-dev'
assert_output "$stable_like_manual_output" 'version=v1.2.3'
assert_output "$stable_like_manual_output" 'publish_latest=false'
assert_output_count "$stable_like_manual_output" 4

max_length_tag=$(awk 'BEGIN { for (i = 0; i < 128; i++) printf "a" }')
max_length_output="$test_dir/max-length"
EVENT_NAME=workflow_dispatch \
GITHUB_REPOSITORY_OWNER=leonfox28 \
MANUAL_VERSION="$max_length_tag" \
GITHUB_OUTPUT="$max_length_output" \
    sh "$resolver"
assert_output "$max_length_output" "version=$max_length_tag"
assert_output "$max_length_output" 'image=ghcr.io/leonfox28/zterm-relay-dev'
assert_output "$max_length_output" 'channel=manual'
assert_output "$max_length_output" 'publish_latest=false'
assert_output_count "$max_length_output" 4

assert_rejected() {
    case_name=$1
    shift
    if env "$@" GITHUB_OUTPUT="$test_dir/rejected-$case_name" sh "$resolver" \
        >"$test_dir/$case_name.stdout" 2>"$test_dir/$case_name.stderr"; then
        fail "resolver accepted invalid case: $case_name"
    fi
    [ ! -s "$test_dir/rejected-$case_name" ] \
        || fail "resolver wrote outputs before rejecting invalid case: $case_name"
}

assert_rejected_with_error() {
    case_name=$1
    expected_error=$2
    shift 2
    assert_rejected "$case_name" "$@"
    expected_line="invalid relay image publication: $expected_error"
    grep -Fqx "$expected_line" "$test_dir/$case_name.stderr" \
        || fail "invalid case $case_name failed for the wrong reason; expected: $expected_error"
    assert_output_count "$test_dir/$case_name.stderr" 1
}

assert_rejected_with_error manual-latest \
    'latest is managed only as an alias of a stable GitHub release' \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER=leonfox28 \
    MANUAL_VERSION=latest
assert_rejected_with_error release-latest \
    'GitHub release tags must use canonical vMAJOR.MINOR.PATCH SemVer' \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG=latest RELEASE_PRERELEASE=false
assert_rejected_with_error stable-workspace-version-mismatch \
    'release version 0.1.1 does not match workspace version 0.1.0' \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG=v0.1.1 RELEASE_PRERELEASE=false
assert_rejected_with_error prerelease-workspace-version-mismatch \
    'release version 0.1.0-rc.1 does not match workspace version 0.1.0' \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG=v0.1.0-rc.1 RELEASE_PRERELEASE=true
assert_rejected_with_error stable-flag-with-prerelease-tag \
    'a stable GitHub release tag must not contain a prerelease suffix' \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG=v0.1.0-rc.1 RELEASE_PRERELEASE=false \
    WORKSPACE_MANIFEST="$prerelease_manifest"
assert_rejected_with_error prerelease-flag-with-stable-tag \
    'a GitHub prerelease tag must contain a prerelease suffix' \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG=v0.1.0 RELEASE_PRERELEASE=true
assert_rejected_with_error release-without-v-prefix \
    'GitHub release tags must use canonical vMAJOR.MINOR.PATCH SemVer' \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG=0.1.0 RELEASE_PRERELEASE=false
assert_rejected_with_error uppercase-v-prefix \
    'GitHub release tags must use canonical vMAJOR.MINOR.PATCH SemVer' \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG=V0.1.0 RELEASE_PRERELEASE=false
assert_rejected_with_error leading-zero-major \
    'GitHub release tags must use canonical SemVer without build metadata' \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG=v00.1.0 RELEASE_PRERELEASE=false
assert_rejected_with_error leading-zero-minor \
    'GitHub release tags must use canonical SemVer without build metadata' \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG=v0.01.0 RELEASE_PRERELEASE=false
assert_rejected_with_error leading-zero-patch \
    'GitHub release tags must use canonical SemVer without build metadata' \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG=v0.1.00 RELEASE_PRERELEASE=false
assert_rejected_with_error incomplete-semver \
    'GitHub release tags must use canonical SemVer without build metadata' \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG=v0.1 RELEASE_PRERELEASE=false
assert_rejected_with_error extra-core-component \
    'GitHub release tags must use canonical SemVer without build metadata' \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG=v0.1.0.1 RELEASE_PRERELEASE=false
assert_rejected_with_error empty-prerelease \
    'GitHub release tags must use canonical SemVer without build metadata' \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG=v0.1.0- RELEASE_PRERELEASE=true
assert_rejected_with_error empty-prerelease-identifier \
    'GitHub release tags must use canonical SemVer without build metadata' \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG=v0.1.0-rc..1 RELEASE_PRERELEASE=true
assert_rejected_with_error leading-zero-numeric-prerelease \
    'GitHub release tags must use canonical SemVer without build metadata' \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG=v0.1.0-rc.01 RELEASE_PRERELEASE=true
assert_rejected_with_error stable-build-metadata \
    'GitHub release tags must use canonical SemVer without build metadata' \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG=v0.1.0+build.1 RELEASE_PRERELEASE=false
assert_rejected_with_error prerelease-build-metadata \
    'GitHub release tags must use canonical SemVer without build metadata' \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG=v0.1.0-rc.1+build.1 RELEASE_PRERELEASE=true \
    WORKSPACE_MANIFEST="$prerelease_manifest"
assert_rejected_with_error malformed-tag \
    'version is not a valid OCI image tag' \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER=leonfox28 \
    MANUAL_VERSION='dev-invalid/tag'
assert_rejected_with_error empty-manual-tag \
    'manual development version must not be empty' \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER=leonfox28 \
    MANUAL_VERSION=
assert_rejected_with_error leading-period-tag \
    'version is not a valid OCI image tag' \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER=leonfox28 \
    MANUAL_VERSION=.dev
assert_rejected_with_error leading-hyphen-tag \
    'version is not a valid OCI image tag' \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER=leonfox28 \
    MANUAL_VERSION=-dev
too_long_tag="${max_length_tag}a"
assert_rejected_with_error too-long-tag \
    'version is not a valid OCI image tag' \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER=leonfox28 \
    MANUAL_VERSION="$too_long_tag"
newline_tag=$(printf 'safe\nchannel=stable\npublish_latest=true')
assert_rejected_with_error newline-output-injection \
    'version is not a valid OCI image tag' \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER=leonfox28 \
    MANUAL_VERSION="$newline_tag"
trailing_newline_tag=$(printf 'safe\nx')
trailing_newline_tag=${trailing_newline_tag%x}
assert_rejected_with_error trailing-newline-output-injection \
    'version is not a valid OCI image tag' \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER=leonfox28 \
    MANUAL_VERSION="$trailing_newline_tag"
carriage_return_tag=$(printf 'safe\rchannel=stable')
assert_rejected_with_error carriage-return-output-injection \
    'version is not a valid OCI image tag' \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER=leonfox28 \
    MANUAL_VERSION="$carriage_return_tag"
newline_owner=$(printf 'leonfox28\nversion=latest')
assert_rejected_with_error owner-output-injection \
    'GITHUB_REPOSITORY_OWNER is not a valid GitHub owner' \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER="$newline_owner" \
    MANUAL_VERSION=manual
trailing_newline_owner=$(printf 'leonfox28\nx')
trailing_newline_owner=${trailing_newline_owner%x}
assert_rejected_with_error trailing-newline-owner \
    'GITHUB_REPOSITORY_OWNER is not a valid GitHub owner' \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER="$trailing_newline_owner" \
    MANUAL_VERSION=manual
assert_rejected_with_error malformed-owner \
    'GITHUB_REPOSITORY_OWNER is not a valid GitHub owner' \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER='leon--fox28' \
    MANUAL_VERSION=manual
assert_rejected_with_error owner-path-injection \
    'GITHUB_REPOSITORY_OWNER is not a valid GitHub owner' \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER='leonfox28/other' \
    MANUAL_VERSION=manual
assert_rejected_with_error missing-release-prerelease \
    'RELEASE_PRERELEASE must be true or false for a release' \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG=v0.1.0 RELEASE_PRERELEASE=
assert_rejected_with_error invalid-release-tag \
    'GitHub release tags must use canonical SemVer without build metadata' \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG='v0.1.0/invalid' RELEASE_PRERELEASE=false
release_newline_tag=$(printf 'v0.1.0\nchannel=manual\npublish_latest=false')
assert_rejected_with_error release-output-injection \
    'GitHub release tags must use canonical SemVer without build metadata' \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG="$release_newline_tag" RELEASE_PRERELEASE=false
missing_workspace_version_manifest="$test_dir/Cargo.missing-version.toml"
{
    printf '%s\n' '[workspace]' 'members = []'
    printf '%s\n' '[workspace.dependencies]'
    printf '%s\n' 'decoy = { version = "0.1.0" }'
} >"$missing_workspace_version_manifest"
assert_rejected_with_error missing-workspace-version \
    'could not read exactly one [workspace.package].version from Cargo.toml' \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG=v0.1.0 RELEASE_PRERELEASE=false \
    WORKSPACE_MANIFEST="$missing_workspace_version_manifest"
duplicate_workspace_version_manifest="$test_dir/Cargo.duplicate-version.toml"
{
    printf '%s\n' '[workspace]' 'members = []'
    printf '%s\n' '[workspace.package]'
    printf '%s\n' 'version = "0.1.0"' 'version = "9.9.9"'
} >"$duplicate_workspace_version_manifest"
assert_rejected_with_error duplicate-workspace-version \
    'could not read exactly one [workspace.package].version from Cargo.toml' \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG=v0.1.0 RELEASE_PRERELEASE=false \
    WORKSPACE_MANIFEST="$duplicate_workspace_version_manifest"
noncanonical_workspace_manifest="$test_dir/Cargo.noncanonical-version.toml"
write_workspace_manifest "$noncanonical_workspace_manifest" '00.1.0'
assert_rejected_with_error noncanonical-workspace-version \
    '[workspace.package].version must use canonical SemVer without build metadata' \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG=v0.1.0 RELEASE_PRERELEASE=false \
    WORKSPACE_MANIFEST="$noncanonical_workspace_manifest"
assert_rejected_with_error unsupported-event \
    'unsupported publication event: push' \
    EVENT_NAME=push GITHUB_REPOSITORY_OWNER=leonfox28 \
    MANUAL_VERSION=manual

echo "relay publication channel checks passed"
