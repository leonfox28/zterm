#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
scanner="$repo_root/tests/secret-scan.sh"
test_dir=$(mktemp -d)
trap 'rm -rf "$test_dir"' EXIT HUP INT TERM

fail() {
    echo "secret scan fixture failed: $*" >&2
    exit 1
}

mkdir -p "$test_dir/.trellis/scripts" "$test_dir/deploy"
printf '%s%s\n' 'tok' 'en = part.strip()' >"$test_dir/.trellis/scripts/example.py"
SECRET_SCAN_ROOT="$test_dir" sh "$scanner" >/dev/null \
    || fail "ordinary Trellis token source was scanned"

assert_secret_rejected() {
    case_name=$1
    shift
    rm -f "$test_dir/deploy/credential.txt"
    "$@" >"$test_dir/deploy/credential.txt"
    if SECRET_SCAN_ROOT="$test_dir" sh "$scanner" >/dev/null 2>&1; then
        fail "$case_name fixture was accepted"
    fi
}

begin_marker=BEGIN
private_marker=PRIVATE
key_marker=KEY
# Positional parameters belong to the child shell.
# shellcheck disable=SC2016
assert_secret_rejected private-key \
    sh -c 'printf "%s %s %s\n" "$1" "$2" "$3"' \
    sh "$begin_marker" "$private_marker" "$key_marker"
assert_secret_rejected github-token \
    sh -c 'printf "ghp_%040d\n" 0'
assert_secret_rejected aws-access-key \
    sh -c 'printf "AKIA%016d\n" 0'

echo "secret scan scope and credential fixtures verified"
