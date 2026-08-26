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

assert_rejected() {
    case_name=$1
    expected_error=$2
    shift 2
    output_file="$test_dir/rejected-$case_name"
    if env "$@" GITHUB_OUTPUT="$output_file" sh "$resolver" \
        >"$test_dir/$case_name.stdout" 2>"$test_dir/$case_name.stderr"; then
        fail "resolver accepted invalid case: $case_name"
    fi
    [ ! -s "$output_file" ] \
        || fail "resolver wrote outputs before rejecting invalid case: $case_name"
    grep -Fqx "invalid relay image publication: $expected_error" \
        "$test_dir/$case_name.stderr" \
        || fail "invalid case $case_name failed for the wrong reason"
}

write_workspace_fixture() {
    fixture_dir=$1
    fixture_version=$2
    mkdir -p "$fixture_dir/core/src"
    {
        printf '%s\n' '[workspace]' 'members = ["core"]' 'resolver = "3"'
        printf '%s\n' '[workspace.package]'
        printf 'version = "%s"\n' "$fixture_version"
        printf '%s\n' 'edition = "2024"'
    } >"$fixture_dir/Cargo.toml"
    {
        printf '%s\n' '[package]' 'name = "zterm-core"'
        printf '%s\n' 'version.workspace = true' 'edition.workspace = true'
    } >"$fixture_dir/core/Cargo.toml"
    printf '%s\n' '# publication test fixture' >"$fixture_dir/core/src/lib.rs"
    cargo +1.98.0 generate-lockfile --quiet \
        --manifest-path "$fixture_dir/Cargo.toml"
}

stable_output="$test_dir/stable"
EVENT_NAME=release \
GITHUB_REPOSITORY_OWNER=LeonFox28 \
RELEASE_TAG=v0.1.6 \
RELEASE_PRERELEASE=false \
GITHUB_OUTPUT="$stable_output" \
    sh "$resolver"
assert_output "$stable_output" 'image=ghcr.io/leonfox28/zterm-relay'
assert_output "$stable_output" 'version=v0.1.6'
assert_output "$stable_output" 'publish_latest=true'
assert_output_count "$stable_output" 3

prerelease_dir="$test_dir/prerelease-workspace"
write_workspace_fixture "$prerelease_dir" '0.2.0-rc.1'
prerelease_output="$test_dir/prerelease"
EVENT_NAME=release \
GITHUB_REPOSITORY_OWNER=leonfox28 \
RELEASE_TAG=v0.2.0-rc.1 \
RELEASE_PRERELEASE=true \
WORKSPACE_MANIFEST="$prerelease_dir/Cargo.toml" \
GITHUB_OUTPUT="$prerelease_output" \
    sh "$resolver"
assert_output "$prerelease_output" 'image=ghcr.io/leonfox28/zterm-relay-dev'
assert_output "$prerelease_output" 'version=v0.2.0-rc.1'
assert_output "$prerelease_output" 'publish_latest=false'
assert_output_count "$prerelease_output" 3

manual_output="$test_dir/manual"
EVENT_NAME=workflow_dispatch \
GITHUB_REPOSITORY_OWNER=leonfox28 \
MANUAL_VERSION=phase-zero \
GITHUB_OUTPUT="$manual_output" \
    sh "$resolver"
assert_output "$manual_output" 'image=ghcr.io/leonfox28/zterm-relay-dev'
assert_output "$manual_output" 'version=phase-zero'
assert_output "$manual_output" 'publish_latest=false'
assert_output_count "$manual_output" 3

assert_rejected version-mismatch \
    'release tag v0.1.7 does not match workspace tag v0.1.6' \
    EVENT_NAME=release GITHUB_REPOSITORY_OWNER=leonfox28 \
    RELEASE_TAG=v0.1.7 RELEASE_PRERELEASE=false

assert_rejected invalid-manual-tag \
    'version is not a valid OCI image tag' \
    EVENT_NAME=workflow_dispatch GITHUB_REPOSITORY_OWNER=leonfox28 \
    MANUAL_VERSION='dev/invalid'

echo "relay publication channel checks passed"
