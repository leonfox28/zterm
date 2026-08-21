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

stable_output="$test_dir/stable"
EVENT_NAME=release \
GITHUB_REPOSITORY_OWNER=LeonFox28 \
RELEASE_TAG=v1.2.3 \
RELEASE_PRERELEASE=false \
GITHUB_OUTPUT="$stable_output" \
    sh "$resolver"
assert_output "$stable_output" 'image=ghcr.io/leonfox28/zterm-relay'
assert_output "$stable_output" 'version=v1.2.3'
assert_output "$stable_output" 'channel=stable'
assert_output "$stable_output" 'publish_latest=true'
assert_output_count "$stable_output" 4

prerelease_output="$test_dir/prerelease"
EVENT_NAME=release \
GITHUB_REPOSITORY_OWNER=leonfox28 \
RELEASE_TAG=v1.3.0-rc.1 \
RELEASE_PRERELEASE=true \
GITHUB_OUTPUT="$prerelease_output" \
    sh "$resolver"
assert_output "$prerelease_output" 'image=ghcr.io/leonfox28/zterm-relay-dev'
assert_output "$prerelease_output" 'version=v1.3.0-rc.1'
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

assert_rejected manual-latest \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER=leonfox28 \
    MANUAL_VERSION=latest
assert_rejected release-latest \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG=latest RELEASE_PRERELEASE=false
assert_rejected malformed-tag \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER=leonfox28 \
    MANUAL_VERSION='dev-invalid/tag'
assert_rejected empty-manual-tag \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER=leonfox28 \
    MANUAL_VERSION=
assert_rejected leading-period-tag \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER=leonfox28 \
    MANUAL_VERSION=.dev
assert_rejected leading-hyphen-tag \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER=leonfox28 \
    MANUAL_VERSION=-dev
too_long_tag="${max_length_tag}a"
assert_rejected too-long-tag \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER=leonfox28 \
    MANUAL_VERSION="$too_long_tag"
newline_tag=$(printf 'safe\nchannel=stable\npublish_latest=true')
assert_rejected newline-output-injection \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER=leonfox28 \
    MANUAL_VERSION="$newline_tag"
trailing_newline_tag=$(printf 'safe\nx')
trailing_newline_tag=${trailing_newline_tag%x}
assert_rejected trailing-newline-output-injection \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER=leonfox28 \
    MANUAL_VERSION="$trailing_newline_tag"
carriage_return_tag=$(printf 'safe\rchannel=stable')
assert_rejected carriage-return-output-injection \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER=leonfox28 \
    MANUAL_VERSION="$carriage_return_tag"
newline_owner=$(printf 'leonfox28\nversion=latest')
assert_rejected owner-output-injection \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER="$newline_owner" \
    MANUAL_VERSION=manual
trailing_newline_owner=$(printf 'leonfox28\nx')
trailing_newline_owner=${trailing_newline_owner%x}
assert_rejected trailing-newline-owner \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER="$trailing_newline_owner" \
    MANUAL_VERSION=manual
assert_rejected malformed-owner \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER='leon--fox28' \
    MANUAL_VERSION=manual
assert_rejected owner-path-injection \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER='leonfox28/other' \
    MANUAL_VERSION=manual
assert_rejected missing-release-prerelease \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG=v1.2.3 RELEASE_PRERELEASE=
assert_rejected invalid-release-tag \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG='v1.2.3/invalid' RELEASE_PRERELEASE=false
assert_rejected release-output-injection \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG="$newline_tag" RELEASE_PRERELEASE=false
assert_rejected unsupported-event \
    EVENT_NAME=push GITHUB_REPOSITORY_OWNER=leonfox28 \
    MANUAL_VERSION=manual

echo "relay publication channel checks passed"
