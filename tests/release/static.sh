#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
workflow="$repo_root/.github/workflows/release.yml"
ci_workflow="$repo_root/.github/workflows/ci.yml"
bootstrap="$repo_root/install/install.sh"
template="$repo_root/install/versioned.sh.in"
build_script="$repo_root/crates/core/build.rs"

fail() {
    echo "release policy check failed: $*" >&2
    exit 1
}

sh -n "$bootstrap" || fail "mutable bootstrap is not POSIX shell syntax"
for invalid_tag in v1 v1.2 v01.2.3 v1.02.3 v1.2.03 v1.2.3-01 v1.2.3-; do
    if sh "$bootstrap" --version "$invalid_tag" >/dev/null 2>&1; then
        fail "mutable bootstrap accepted noncanonical SemVer tag: $invalid_tag"
    fi
done

grep -Eq '^[[:space:]]{2}push:' "$workflow" \
    || fail "release workflow must run on tag push"
grep -Fq '      - "v*"' "$workflow" \
    || fail "release workflow must accept only v-prefixed tag pushes"
if grep -Eq '^[[:space:]]{2}(workflow_dispatch|pull_request|release):' "$workflow"; then
    fail "release workflow must not have a second trigger"
fi
environment_gates=$(grep -Fc 'environment: release' "$workflow" || true)
[ "$environment_gates" -eq 1 ] \
    || fail "only the signing job may use the protected release Environment"
if grep -Eq 'immutable_release_checkpoint|enabled-and-reviewed' "$workflow"; then
    fail "release workflow must not use a self-asserted checkpoint input"
fi
grep -Fq -- '--draft' "$workflow" \
    || fail "release workflow must create a draft"
grep -Fq "gh release edit \"\$RELEASE_TAG\" --draft=false" "$workflow" \
    || fail "release workflow must publish the verified draft"
grep -Fq '.immutable == true' "$workflow" \
    || fail "release workflow must verify the published Release is immutable"
if grep -Fq 'gh release delete' "$workflow"; then
    fail "release workflow must never replace a failed Release"
fi
if grep -Fq 'self-hosted' "$workflow"; then
    fail "release workflow must use only GitHub-hosted runners"
fi
if grep -Eq 'image: .*@sha256:[0-9a-f]{64}$' "$workflow"; then
    :
else
    fail "Linux glibc-floor image must be digest-pinned"
fi

unpinned_actions=$(grep -E '^[[:space:]]*uses:' "$workflow" \
    | grep -Ev '@[0-9a-f]{40}([[:space:]]|$)' || true)
[ -z "$unpinned_actions" ] || fail "all release actions must use commit SHAs"
unpinned_ci_actions=$(grep -E '^[[:space:]]*uses:' "$ci_workflow" \
    | grep -Ev '@[0-9a-f]{40}([[:space:]]|$)' || true)
[ -z "$unpinned_ci_actions" ] \
    || fail "all CI actions used by the release gate must use commit SHAs"

secret_references=$(grep -Ec \
    'secrets[.]ZTERM_RELEASE_SIGNING_KEY' "$workflow" || true)
[ "$secret_references" -eq 1 ] \
    || fail "the signing secret must be referenced by exactly one step"
frozen_checkouts=$(grep -Fc "ref: \${{ needs.validate.outputs.commit }}" "$workflow" || true)
[ "$frozen_checkouts" -eq 6 ] \
    || fail "every downstream release job must check out the validated commit"
grep -Fq 'target/release/zterm-release-tool sign release-output' "$workflow" \
    || fail "the signing step must execute the tool built before secret exposure"
grep -Fq "git rev-list -n 1 \"\$RELEASE_TAG\"" "$workflow" \
    || fail "publication must recheck the existing tag against the validated commit"

assemble_job=$(sed -n '/^  assemble:/,/^  sign:/p' "$workflow")
release_shellcheck_requirements=$(grep -Fc 'command -v shellcheck' "$workflow" || true)
[ "$release_shellcheck_requirements" -eq 1 ] \
    || fail "release must have exactly one fail-closed ShellCheck requirement"
printf '%s\n' "$assemble_job" | grep -Fq 'runs-on: ubuntu-24.04' \
    || fail "generated installer ShellCheck must run on the Ubuntu assembly runner"
printf '%s\n' "$assemble_job" \
    | grep -Fq -- '- name: Check the generated installer with ShellCheck' \
    || fail "the generated formal installer must have one required ShellCheck gate"
printf '%s\n' "$assemble_job" | grep -Fq 'command -v shellcheck' \
    || fail "the generated installer gate must require ShellCheck"
printf '%s\n' "$assemble_job" \
    | grep -Fq 'shellcheck -s sh release-output/zterm-install.sh' \
    || fail "the generated formal installer must be checked before signing"
grep -Fq 'shellcheck -s sh install/install.sh tests/release/*.sh' "$ci_workflow" \
    || fail "exact-main CI must ShellCheck the maintained installer sources"

installer_job=$(sed -n '/^  installer:/,/^  publish:/p' "$workflow")
if printf '%s\n' "$installer_job" | grep -Fiq 'shellcheck'; then
    fail "the four-platform installer matrix must not assume ShellCheck is preinstalled"
fi
printf '%s\n' "$installer_job" \
    | grep -Fq 'sh -n install/install.sh tests/release/installer-fixture.sh' \
    || fail "every installer platform must retain the POSIX syntax gate"
printf '%s\n' "$installer_job" \
    | grep -Fq 'sh tests/release/installer-fixture.sh' \
    || fail "every installer platform must exercise the authenticated fixture"
for ci_gate in 'actions/workflows/ci.yml/runs' '-f branch=main' '-f event=push' \
    '-f status=success' "-f head_sha=\"\$commit\""; do
    grep -Fq -- "$ci_gate" "$workflow" \
        || fail "release validation must require exact successful main push CI: $ci_gate"
done

main_release_gates=$(grep -Fc \
    "if: github.event_name == 'push' && github.ref == 'refs/heads/main'" \
    "$ci_workflow" || true)
[ "$main_release_gates" -eq 2 ] \
    || fail "CI must have exactly two main-push-only native release-mode jobs"
grep -Fq 'windows-latest' "$ci_workflow" \
    || fail "CI must retain the Windows shared-boundary runner"
for target in aarch64-apple-darwin x86_64-apple-darwin \
    aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu; do
    grep -Fq "target: $target" "$ci_workflow" \
        || fail "CI release-mode matrix is missing $target"
done
grep -Eq 'image: .*@sha256:[0-9a-f]{64}$' "$ci_workflow" \
    || fail "CI glibc-floor image must be digest-pinned"
for container_workflow in "$ci_workflow" "$workflow"; do
    cargo_path_rules=$(grep -Fc \
        "echo \"\$HOME/.cargo/bin\" >> \"\$GITHUB_PATH\"" \
        "$container_workflow" || true)
    [ "$cargo_path_rules" -eq 1 ] \
        || fail "container workflow must use the runtime HOME Cargo path: $container_workflow"
    if grep -Fq '/root/.cargo/bin' "$container_workflow"; then
        fail "container workflow must not assume root HOME: $container_workflow"
    fi
    safe_directory_rules=$(grep -Fc \
        "git config --global --add safe.directory \"\$GITHUB_WORKSPACE\"" \
        "$container_workflow" || true)
    [ "$safe_directory_rules" -eq 1 ] \
        || fail "container workflow must trust only its exact workspace: $container_workflow"
    if grep -F 'safe.directory' "$container_workflow" | grep -Fq '*'; then
        fail "container workflow must never trust a wildcard Git safe.directory"
    fi
done
if grep -Fq 'std::env::var("GITHUB_SHA")' "$build_script"; then
    fail "ambient ordinary-CI SHA must not mark a managed distribution build"
fi

for forbidden in sudo '.zterm/' '.bashrc' '.zshrc' 'launchctl' 'systemctl'; do
    if grep -Fq "$forbidden" "$bootstrap" "$template"; then
        fail "installer contains forbidden side effect token: $forbidden"
    fi
done
grep -Fq -- '--internal-release-self-check' "$template" \
    || fail "versioned installer must self-check the authenticated candidate"
grep -Fq -- '--internal-release-verify' "$template" \
    || fail "versioned installer must verify the detached signature"
grep -Fq -- '--internal-release-install' "$template" \
    || fail "versioned installer must delegate atomic no-clobber activation"
for bounded_installer in "$bootstrap" "$template"; do
    grep -Fq 'ulimit -f' "$bounded_installer" \
        || fail "installer download must have an OS-enforced file-size bound"
    grep -Fq -- '--max-filesize' "$bounded_installer" \
        || fail "curl installer download must enforce its byte bound"
done
grep -Fq "if [ -L \"\$candidate\" ] || [ ! -f \"\$candidate\" ]; then" "$template" \
    || fail "versioned installer must reject a symlink/non-file candidate before chmod"

public_key=$(tr -d '[:space:]' < "$repo_root/release/public-key.hex")
case "$public_key" in
    UNCONFIGURED) ;;
    *[!0-9a-f]*|'') fail "release public key must be the explicit placeholder or lowercase hex" ;;
    *) [ "${#public_key}" -eq 64 ] || fail "release public key must contain 32 bytes" ;;
esac

echo "release workflow and installer policy verified"
