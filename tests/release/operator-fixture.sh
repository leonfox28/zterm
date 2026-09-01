#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
operator="$repo_root/tools/release/operator.sh"
test_root=$(mktemp -d)
trap 'rm -rf "$test_root"' EXIT HUP INT TERM
fake_bin="$test_root/bin"
mkdir -p "$fake_bin"

fail() {
    echo "release operator fixture failed: $*" >&2
    exit 1
}

cat >"$fake_bin/cargo" <<'FAKE_CARGO'
#!/bin/sh
set -eu
printf 'cargo %s\n' "$*" >>"$FAKE_TRACE"
case " $* " in
    *' run '*' validate-next-version '*)
        candidate=
        for argument in "$@"; do candidate=$argument; done
        [ "$candidate" = 0.1.10 ] || exit 1
        printf '%s\n' "next release version $candidate is valid"
        ;;
    *' pkgid '*)
        version=$(sed -n '/^\[workspace.package\]$/,/^\[/s/^version = "\([^"]*\)"/\1/p' Cargo.toml)
        printf 'path+file:///fixture#zterm-cli@%s\n' "$version"
        ;;
    *' metadata '*)
        version=$(sed -n '/^\[workspace.package\]$/,/^\[/s/^version = "\([^"]*\)"/\1/p' Cargo.toml)
        awk -v version="$version" '
            /^version = "[^"]+"$/ { print "version = \"" version "\""; next }
            { print }
        ' Cargo.lock >Cargo.lock.fixture
        mv Cargo.lock.fixture Cargo.lock
        printf '%s\n' '{}'
        ;;
    *) exit 1 ;;
esac
FAKE_CARGO

cat >"$fake_bin/just" <<'FAKE_JUST'
#!/bin/sh
set -eu
printf 'just %s\n' "$*" >>"$FAKE_TRACE"
[ "${FAKE_JUST_FAIL:-0}" != 1 ]
if [ "${FAKE_JUST_DIRTY:-0}" = 1 ]; then
    printf '%s\n' 'unexpected check output' >unexpected-check-output
fi
FAKE_JUST

cat >"$fake_bin/gh" <<'FAKE_GH'
#!/bin/sh
set -eu
printf 'gh %s\n' "$*" >>"$FAKE_TRACE"
command_name=${1:-}
shift || true
case "$command_name" in
    auth) exit 0 ;;
    repo)
        printf '%s\n' "$FAKE_REPOSITORY"
        ;;
    release) exit 1 ;;
    pr)
        [ "${1:-}" = create ] || exit 1
        [ "${FAKE_PR_FAIL:-0}" != 1 ] || exit 1
        printf '%s\n' 'https://example.invalid/pull/1'
        ;;
    api)
        case " $* " in
            *'/branches/main/protection '*)
                printf '%s\n' "${FAKE_PROTECTION:-true}"
                ;;
            *'/releases/tags/'*)
                if [ "${FAKE_RELEASE_API_ERROR:-0}" = 1 ]; then
                    echo 'gh: service unavailable (HTTP 500)' >&2
                    exit 1
                elif [ "${FAKE_RELEASE_EXISTS:-0}" = 1 ]; then
                    printf '%s\n' '{"tag_name":"fixture"}'
                else
                    echo 'gh: Not Found (HTTP 404)' >&2
                    exit 1
                fi
                ;;
            *'/actions/workflows/ci.yml/runs '*)
                if [ "${FAKE_GREEN_CI:-1}" = 1 ]; then
                    printf '%s\n' '123 https://example.invalid/actions/runs/123'
                fi
                ;;
            *) exit 1 ;;
        esac
        ;;
    run)
        case "${1:-}" in
            list) printf '%s\n' '456 https://example.invalid/actions/runs/456' ;;
            watch) exit 0 ;;
            *) exit 1 ;;
        esac
        ;;
    *) exit 1 ;;
esac
FAKE_GH
chmod 700 "$fake_bin/cargo" "$fake_bin/just" "$fake_bin/gh"

seed="$test_root/seed"
git init --quiet --initial-branch=main "$seed"
git -C "$seed" config user.name 'Operator Fixture'
git -C "$seed" config user.email fixture@example.invalid
cat >"$seed/Cargo.toml" <<'MANIFEST'
[workspace]
members = []

[workspace.package]
version = "0.1.9"
MANIFEST
cat >"$seed/Cargo.lock" <<'LOCKFILE'
version = 4

[[package]]
name = "zterm-cli"
version = "0.1.9"
LOCKFILE
mkdir -p "$seed/tests"
cat >"$seed/tests/workspace-version.sh" <<'VERSION_TEST'
#!/bin/sh
set -eu
manifest_version=$(sed -n '/^\[workspace.package\]$/,/^\[/s/^version = "\([^"]*\)"/\1/p' Cargo.toml)
lock_version=$(sed -n '/^name = "zterm-cli"$/{n;s/^version = "\([^"]*\)"/\1/p;}' Cargo.lock)
[ "$manifest_version" = "$lock_version" ]
VERSION_TEST
chmod 700 "$seed/tests/workspace-version.sh"
git -C "$seed" add Cargo.toml Cargo.lock tests/workspace-version.sh
git -C "$seed" commit --quiet -m baseline

new_case() {
    case_name=$1
    case_root="$test_root/$case_name"
    case_remote="$case_root/remote.git"
    case_worktree="$case_root/worktree"
    case_trace="$case_root/trace.log"
    mkdir -p "$case_root"
    git init --quiet --bare "$case_remote"
    git -C "$seed" push --quiet "$case_remote" main
    git --git-dir="$case_remote" symbolic-ref HEAD refs/heads/main
    git clone --quiet "$case_remote" "$case_worktree"
    git -C "$case_worktree" config user.name 'Operator Fixture'
    git -C "$case_worktree" config user.email fixture@example.invalid
    : >"$case_trace"
}

run_operator() {
    env PATH="$fake_bin:$PATH" \
        FAKE_TRACE="$case_trace" \
        FAKE_REPOSITORY=fixture/zterm \
        ZTERM_RELEASE_TEST_MODE=1 \
        ZTERM_RELEASE_REPOSITORY=fixture/zterm \
        ZTERM_RELEASE_WORKTREE="$case_worktree" \
        ZTERM_RELEASE_POLL_ATTEMPTS=1 \
        ZTERM_RELEASE_POLL_SECONDS=0 \
        "$@" sh "$operator" "$operator_command" "$operator_version"
}

assert_no_remote_tag() {
    if git --git-dir="$case_remote" show-ref --tags --quiet; then
        fail "$1 created a remote tag"
    fi
}

new_case dirty
printf '%s\n' dirty >"$case_worktree/untracked"
operator_command=prepare
operator_version=0.1.10
if run_operator >"$case_root/stdout" 2>"$case_root/stderr"; then
    fail "dirty worktree was accepted"
fi
assert_no_remote_tag dirty

new_case behind
upstream="$case_root/upstream"
git clone --quiet "$case_remote" "$upstream"
git -C "$upstream" config user.name 'Operator Fixture'
git -C "$upstream" config user.email fixture@example.invalid
printf '%s\n' newer >"$upstream/remote-change"
git -C "$upstream" add remote-change
git -C "$upstream" commit --quiet -m newer
git -C "$upstream" push --quiet origin main
operator_command=prepare
operator_version=0.1.10
if run_operator >"$case_root/stdout" 2>"$case_root/stderr"; then
    fail "behind main was accepted"
fi
assert_no_remote_tag behind

new_case invalid-version
operator_command=prepare
operator_version=v0.2.0
if run_operator >"$case_root/stdout" 2>"$case_root/stderr"; then
    fail "noncanonical next version was accepted"
fi
assert_no_remote_tag invalid-version

new_case existing-release
operator_command=prepare
operator_version=0.1.10
if run_operator FAKE_RELEASE_EXISTS=1 >"$case_root/stdout" 2>"$case_root/stderr"; then
    fail "existing Release was accepted"
fi
assert_no_remote_tag existing-release

new_case release-api-failure
operator_command=prepare
operator_version=0.1.10
if run_operator FAKE_RELEASE_API_ERROR=1 >"$case_root/stdout" 2>"$case_root/stderr"; then
    fail "Release API failure was treated as vacancy"
fi
grep -Fq 'could not prove GitHub Release v0.1.10 is vacant' "$case_root/stderr" \
    || fail "Release API failure did not fail closed at vacancy"
assert_no_remote_tag release-api-failure

new_case existing-local-branch
git -C "$case_worktree" branch release/v0.1.10
operator_command=prepare
operator_version=0.1.10
if run_operator >"$case_root/stdout" 2>"$case_root/stderr"; then
    fail "existing local release branch was accepted"
fi
assert_no_remote_tag existing-local-branch

new_case existing-remote-branch
git -C "$case_worktree" push --quiet origin HEAD:refs/heads/release/v0.1.10
operator_command=prepare
operator_version=0.1.10
if run_operator >"$case_root/stdout" 2>"$case_root/stderr"; then
    fail "existing remote release branch was accepted"
fi
assert_no_remote_tag existing-remote-branch

new_case existing-local-tag
git -C "$case_worktree" tag -a v0.1.10 -m 'existing fixture tag'
operator_command=prepare
operator_version=0.1.10
if run_operator >"$case_root/stdout" 2>"$case_root/stderr"; then
    fail "existing local tag was accepted"
fi
assert_no_remote_tag existing-local-tag

new_case existing-remote-tag
git -C "$case_worktree" tag -a v0.1.10 -m 'existing fixture tag'
git -C "$case_worktree" push --quiet origin refs/tags/v0.1.10
git -C "$case_worktree" tag --delete v0.1.10 >/dev/null
operator_command=prepare
operator_version=0.1.10
if run_operator >"$case_root/stdout" 2>"$case_root/stderr"; then
    fail "existing remote tag was accepted"
fi
[ "$(git --git-dir="$case_remote" cat-file -t refs/tags/v0.1.10)" = tag ] \
    || fail "existing annotated tag fixture was not preserved"

new_case failed-check
operator_command=prepare
operator_version=0.1.10
if run_operator FAKE_JUST_FAIL=1 >"$case_root/stdout" 2>"$case_root/stderr"; then
    fail "failed pre-push gate was accepted"
fi
[ "$(git -C "$case_worktree" branch --show-current)" = release/v0.1.10 ] \
    || fail "failed check did not retain the local release branch"
[ -n "$(git -C "$case_worktree" status --porcelain)" ] \
    || fail "failed check did not retain the version diff"
if git --git-dir="$case_remote" show-ref --verify --quiet refs/heads/release/v0.1.10; then
    fail "failed check pushed the release branch"
fi
assert_no_remote_tag failed-check

new_case dirty-after-check
operator_command=prepare
operator_version=0.1.10
if run_operator FAKE_JUST_DIRTY=1 >"$case_root/stdout" 2>"$case_root/stderr"; then
    fail "unexpected check output was accepted"
fi
grep -Fq 'release preparation changed files outside Cargo.toml and Cargo.lock' \
    "$case_root/stderr" || fail "post-check inventory failed for the wrong reason"
if git --git-dir="$case_remote" show-ref --verify --quiet refs/heads/release/v0.1.10; then
    fail "unexpected check output pushed the release branch"
fi
assert_no_remote_tag dirty-after-check

new_case prepare-success
operator_command=prepare
operator_version=0.1.10
if ! run_operator >"$case_root/stdout" 2>"$case_root/stderr"; then
    cat "$case_root/stderr" >&2
    fail "valid release preparation failed"
fi
git --git-dir="$case_remote" show-ref --verify --quiet refs/heads/release/v0.1.10 \
    || fail "prepare did not push the release branch"
changed_files=$(git --git-dir="$case_remote" diff-tree --no-commit-id --name-only -r \
    refs/heads/release/v0.1.10 | LC_ALL=C sort)
[ "$changed_files" = "Cargo.lock
Cargo.toml" ] || fail "release commit changed files outside Cargo.toml and Cargo.lock"
grep -Fq 'gh pr create ' "$case_trace" || fail "prepare did not open a PR"
assert_no_remote_tag prepare-success

new_case missing-green-ci
operator_command=publish
operator_version=0.1.9
if run_operator FAKE_GREEN_CI=0 >"$case_root/stdout" 2>"$case_root/stderr"; then
    fail "publish accepted a commit without exact green main CI"
fi
assert_no_remote_tag missing-green-ci

new_case publish-success
operator_command=publish
operator_version=0.1.9
if ! run_operator FAKE_GREEN_CI=1 >"$case_root/stdout" 2>"$case_root/stderr"; then
    cat "$case_root/stderr" >&2
    fail "valid release publication failed"
fi
[ "$(git --git-dir="$case_remote" cat-file -t refs/tags/v0.1.9)" = tag ] \
    || fail "publish did not create an annotated tag"
[ "$(grep -Fc 'gh run watch 456 ' "$case_trace")" -eq 1 ] \
    || fail "publish did not watch exactly one formal release run"

echo "two-phase release operator fixture passed without external state"
