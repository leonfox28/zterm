#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
workflow="$repo_root/.github/workflows/release.yml"
ci_workflow="$repo_root/.github/workflows/ci.yml"
justfile="$repo_root/justfile"
green_ci_lookup="$repo_root/tools/release/find-green-main-ci.sh"
operator="$repo_root/tools/release/operator.sh"
native_build="$repo_root/tools/release/build-native.sh"
glibc_setup="$repo_root/tools/release/setup-glibc-builder.sh"
bootstrap="$repo_root/install/install.sh"
template="$repo_root/install/versioned.sh.in"
build_script="$repo_root/crates/core/build.rs"
https_fixture_bind_test="$repo_root/tests/release/https_fixture_bind_test.py"
python_syntax="$repo_root/tools/ci/check-python-syntax.sh"

fail() {
    echo "release policy check failed: $*" >&2
    exit 1
}

sh -n "$bootstrap" || fail "mutable bootstrap is not POSIX shell syntax"
PYTHONDONTWRITEBYTECODE=1 python3 "$https_fixture_bind_test" \
    || fail "installer fixture bind override violated its socket-free contract"
syntax_fixture=$(mktemp -d)
trap 'rm -rf "$syntax_fixture"' EXIT HUP INT TERM
cp "$repo_root/tests/release/https_fixture.py" \
    "$repo_root/tests/release/https_fixture_bind_test.py" "$syntax_fixture/"
before_syntax_files=$(find "$syntax_fixture" -type f | wc -l | tr -d '[:space:]')
sh "$python_syntax" "$syntax_fixture/https_fixture.py" \
    "$syntax_fixture/https_fixture_bind_test.py" \
    || fail "in-memory Python syntax owner rejected the maintained fixtures"
after_syntax_files=$(find "$syntax_fixture" -type f | wc -l | tr -d '[:space:]')
[ "$before_syntax_files" = "$after_syntax_files" ] \
    || fail "Python syntax owner wrote bytecode or another generated file"
if find "$syntax_fixture" -type d -name __pycache__ | grep -q .; then
    fail "Python syntax owner created __pycache__"
fi
for invalid_tag in v1 v1.2 v01.2.3 v1.02.3 v1.2.03 v1.2.3-01 v1.2.3-; do
    if sh "$bootstrap" --version "$invalid_tag" >/dev/null 2>&1; then
        fail "mutable bootstrap accepted noncanonical SemVer tag: $invalid_tag"
    fi
done

grep -Eq '^[[:space:]]{2}push:' "$workflow" \
    || fail "release workflow must run on tag push"
grep -Fq '      - "v*"' "$workflow" \
    || fail "release workflow must accept only v-prefixed tag pushes"
grep -Fq 'formal release versions must not contain SemVer build metadata' "$workflow" \
    || fail "formal release must reject SemVer build metadata before native publication"
grep -Fq "git cat-file -t \"refs/tags/\$RELEASE_TAG\"" "$workflow" \
    || fail "formal release must require the operator-owned annotated tag"
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
for forbidden_operation in 'git push --force' 'git push -f' \
    'git push --delete' 'git tag -d' 'git tag --delete' \
    'gh release delete' '--clobber'; do
    if grep -Fq -- "$forbidden_operation" "$operator" "$workflow"; then
        fail "release automation contains a destructive replacement operation: $forbidden_operation"
    fi
done
if grep -Eq 'git push[^#]*(^|[[:space:]])main([[:space:]]|$)' "$operator"; then
    fail "release operator must never push directly to main"
fi
if grep -Fq 'self-hosted' "$workflow"; then
    fail "release workflow must use only GitHub-hosted runners"
fi
if grep -Eq 'image: .*@sha256:[0-9a-f]{64}$' "$ci_workflow"; then
    :
else
    fail "Linux glibc-floor image must be digest-pinned"
fi

unpinned_actions=$(grep -E '^[[:space:]]*(- )?uses:' "$workflow" \
    | grep -Ev 'uses: [.]?/[.]github/workflows/|@[0-9a-f]{40}([[:space:]]|$)' || true)
[ -z "$unpinned_actions" ] || fail "all release actions must use commit SHAs"
unpinned_ci_actions=$(grep -hE '^[[:space:]]*(- )?uses:' "$ci_workflow" \
    "$repo_root/.github/actions/setup-ci-tools/action.yml" \
    | grep -Ev 'uses: [.]?/[.]github/actions/|@[0-9a-f]{40}([[:space:]]|$)' || true)
[ -z "$unpinned_ci_actions" ] \
    || fail "all CI actions used by the release gate must use commit SHAs"
for artifact_boundary in \
    "signed-release-\${{ github.ref_name }}-\${{ github.run_attempt }}" \
    "installer-fixture-\${{ github.ref_name }}-\${{ github.run_attempt }}" \
    "artifact-ids: \${{ needs.sign.outputs.signed_artifact_id }}" \
    "artifact-ids: \${{ needs.sign.outputs.fixture_artifact_id }}"; do
    grep -Fq "$artifact_boundary" "$workflow" \
        || fail "release retries must preserve and identify signed artifacts: $artifact_boundary"
done

secret_references=$(grep -Ec \
    'secrets[.]ZTERM_RELEASE_SIGNING_KEY' "$workflow" || true)
[ "$secret_references" -eq 1 ] \
    || fail "the signing secret must be referenced by exactly one step"
frozen_checkouts=$(grep -Fc "ref: \${{ needs.validate.outputs.commit }}" "$workflow" || true)
[ "$frozen_checkouts" -eq 3 ] \
    || fail "every downstream release job must check out the validated commit"
grep -Fq 'target/release/zterm-release-tool sign release-output' "$workflow" \
    || fail "the signing step must execute the tool built before secret exposure"
grep -Fq "git rev-list -n 1 \"\$RELEASE_TAG\"" "$workflow" \
    || fail "publication must recheck the existing tag against the validated commit"

assemble_job=$(sed -n '/^  candidate:/,/^  gate:/p' "$ci_workflow")
release_shellcheck_requirements=$(grep -Fc 'command -v shellcheck' "$ci_workflow" || true)
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
grep -Fq 'shellcheck -s sh install/install.sh tests/release/*.sh' "$justfile" \
    || fail "exact-main CI must ShellCheck the maintained installer sources"
[ "$(grep -Fc 'sh tests/release/operator-fixture.sh' "$justfile")" -eq 1 ] \
    || fail "portable policy must execute the isolated release operator fixture exactly once"

installer_job=$(sed -n '/^  installer:/,/^  publish:/p' "$workflow")
if printf '%s\n' "$installer_job" | grep -Fiq 'shellcheck'; then
    fail "the three-platform installer matrix must not assume ShellCheck is preinstalled"
fi
printf '%s\n' "$installer_job" \
    | grep -Fq 'sh -n install/install.sh tests/release/installer-fixture.sh' \
    || fail "every installer platform must retain the POSIX syntax gate"
printf '%s\n' "$installer_job" \
    | grep -Fq 'sh tests/release/installer-fixture.sh' \
    || fail "every installer platform must exercise the authenticated fixture"
if printf '%s\n' "$installer_job" | grep -Fq 'python3 -m py_compile'; then
    fail "installer matrix must execute, not separately recompile, the Python fixture"
fi
if grep -Fq 'python3 -m py_compile' "$justfile"; then
    fail "portable policy must not use py_compile because it dirties the worktree"
fi
grep -Fq 'sh tools/ci/check-python-syntax.sh' "$justfile" \
    || fail "portable policy bypasses the no-output Python syntax owner"
for ci_gate in 'actions/workflows/ci.yml/runs' '-f branch=main' '-f event=push' \
    '-f status=success' "-f head_sha=\"\$commit\""; do
    grep -Fq -- "$ci_gate" "$green_ci_lookup" \
        || fail "release validation must require exact successful main push CI: $ci_gate"
done
grep -Fq "sh tools/release/find-green-main-ci.sh \"\$commit\"" "$workflow" \
    || fail "tag validation bypasses the shared exact-main CI lookup"

macos_build_job=$(sed -n '/^  candidates-macos:/,/^  candidates-linux:/p' "$ci_workflow")
linux_build_job=$(sed -n '/^  candidates-linux:/,/^  dependencies:/p' "$ci_workflow")
for native_job in "$macos_build_job" "$linux_build_job"; do
    printf '%s\n' "$native_job" | grep -Fq 'sh tools/release/build-native.sh release-input' \
        || fail "main candidates bypass the shared shipped-binary owner"
    printf '%s\n' "$native_job" | grep -Fq "ZTERM_SOURCE_COMMIT=\"\$GITHUB_SHA\"" \
        || fail "main candidates must embed the exact source commit"
    if printf '%s\n' "$native_job" | grep -Fq 'zterm-release-tool'; then
        fail "native jobs must not build the private release tool"
    fi
done
if grep -Fq 'build-native.sh' "$workflow"; then
    fail "tag publication must reuse the verified main candidate, not rebuild the product"
fi
for candidate_boundary in 'sh tools/release/find-candidate.sh' \
    "artifact-ids: \${{ needs.validate.outputs.candidate_id }}" \
    "run-id: \${{ needs.validate.outputs.ci_run_id }}" 'digest-mismatch: error'; do
    grep -Fq "$candidate_boundary" "$workflow" \
        || fail "tag publication bypasses the exact candidate boundary: $candidate_boundary"
done
grep -Fq "release-candidate-\${{ github.sha }}-\${{ github.run_attempt }}" "$ci_workflow" \
    || fail "main must retain an immutable candidate for each assembly attempt"
if grep -Eq 'macos-15-intel|windows-latest|x86_64-apple-darwin|ci-windows' "$ci_workflow" "$workflow"; then
    fail "Intel macOS and Windows require an explicit future task"
fi
if grep -Eq 'relay-image.yml|packages: write|docker/build-push-action' "$workflow"; then
    fail "native releases must not publish relay images"
fi
printf '%s\n' "$assemble_job" | grep -Fq 'zterm-release-tool archive' \
    || fail "Ubuntu assembly must own deterministic archives"
if printf '%s\n' "$assemble_job" | grep -Fq 'tests/release/static.sh'; then
    fail "tag assembly must not rerun exact-green source policy"
fi
for ordinary_ci in 'fmt --all' 'clippy --workspace' 'test --workspace' \
    'doc --workspace' 'deny check'; do
    if grep -Fq "$ordinary_ci" "$workflow"; then
        fail "tag workflow reruns ordinary CI command: $ordinary_ci"
    fi
done

main_release_gates=$(grep -Fxc \
    "    if: github.event_name == 'push' && github.ref == 'refs/heads/main'" \
    "$ci_workflow" || true)
[ "$main_release_gates" -eq 3 ] \
    || fail "CI must have two main native builders and one main-only candidate assembly"
rust_job=$(sed -n '/^  rust:/,/^  candidates-macos:/p' "$ci_workflow")
[ "$(printf '%s\n' "$rust_job" | grep -Ec '^[[:space:]]{10}- label:')" -eq 3 ] \
    || fail "Rust matrix must retain three hosted OS entries"
[ "$(printf '%s\n' "$rust_job" | grep -Fc 'run: sh tests/source-policy.sh')" -eq 1 ] \
    || fail "every expanded Rust matrix entry must share one source-policy step"
rust_first_steps=$(printf '%s\n' "$rust_job" \
    | grep -E '^[[:space:]]{6}- (uses:|name:)' | sed -n '1,2p')
[ "$rust_first_steps" = "      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
      - name: Check source checkout policy" ] \
    || fail "all three expanded Rust entries must run source-policy immediately after checkout"
checkout_line=$(printf '%s\n' "$rust_job" | grep -n 'uses: actions/checkout@' | sed -n '1s/:.*//p')
source_line=$(printf '%s\n' "$rust_job" | grep -n 'run: sh tests/source-policy.sh' | sed -n '1s/:.*//p')
toolchain_line=$(printf '%s\n' "$rust_job" | grep -n 'name: Install exact Rust toolchain' | sed -n '1s/:.*//p')
if ! [ "$checkout_line" -lt "$source_line" ] \
    || ! [ "$source_line" -lt "$toolchain_line" ]; then
    fail "source-policy must run after checkout and before Rust tooling/compilation"
fi
linux_readiness_job=$(sed -n \
    '/^  candidates-linux:/,/^  dependencies:/p' "$ci_workflow")
linux_policy_job=$linux_readiness_job
{
    linux_first_steps=$(printf '%s\n' "$linux_policy_job" \
        | grep -E '^[[:space:]]{6}- (uses:|name:)' | sed -n '1,4p')
    printf '%s\n' "$linux_first_steps" | sed -n '1p' \
        | grep -Fq -- '- name: Bootstrap Git for the pinned container checkout' \
        || fail "pinned Linux container must bootstrap Git before checkout"
    printf '%s\n' "$linux_first_steps" | sed -n '2p' \
        | grep -Eq -- '- (uses: actions/checkout@|name: Check out the exact existing tag)' \
        || fail "pinned Linux container must use a real Git checkout"
    printf '%s\n' "$linux_first_steps" | sed -n '3p' \
        | grep -Fq -- '- name: Check source checkout policy' \
        || fail "pinned Linux checkout must run source-policy immediately"
    printf '%s\n' "$linux_first_steps" | sed -n '4p' \
        | grep -Fq -- '- name: Prepare the pinned glibc builder' \
        || fail "shared glibc setup must follow source-policy"
}
for target in aarch64-apple-darwin \
    aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu; do
    grep -Fq "target: $target" "$ci_workflow" \
        || fail "CI release-mode matrix is missing $target"
done
grep -Eq 'image: .*@sha256:[0-9a-f]{64}$' "$ci_workflow" \
    || fail "CI glibc-floor image must be digest-pinned"
grep -Fq '  pull_request:' "$ci_workflow" \
    || fail "CI must admit pull requests"
grep -Fq '      - main' "$ci_workflow" \
    || fail "CI push trigger must be narrowed to main"
if grep -Fq '      - "**"' "$ci_workflow"; then
    fail "CI must not duplicate PR runs with all-branch push runs"
fi
grep -Fq 'name: CI gate' "$ci_workflow" \
    || fail "CI lacks the stable branch-protection gate"
grep -Fq 'if: always()' "$ci_workflow" \
    || fail "CI gate must aggregate failed, cancelled, and skipped owners"
[ "$(grep -Fc 'timeout-minutes:' "$ci_workflow")" -eq 8 ] \
    || fail "every CI job must have an explicit timeout"
grep -Fq 'actions/cache@668228422ae6a00e4ad889ee87cd7109ec5666a7' "$ci_workflow" \
    || fail "CI must use the pinned cache owner"
for profile in 'just ci-policy' 'just ci-unix' \
    'just ci-dependencies' 'just ci-relay'; do
    grep -Fq "$profile" "$ci_workflow" \
        || fail "CI bypasses repository command owner: $profile"
done
[ "$(grep -Fc 'sh tests/workspace-version.sh' "$justfile")" -eq 1 ] \
    || fail "workspace version must have one canonical recipe owner"
[ "$(grep -Fc 'cargo +1.98.0 fmt --all -- --check' "$justfile")" -eq 1 ] \
    || fail "workspace formatting must have one canonical recipe owner"
[ "$(grep -Fc 'cargo +1.98.0 doc --workspace --no-deps' "$justfile")" -eq 2 ] \
    || fail "docs must appear only in local check and the one matrix profile"
[ "$(grep -Fc 'smoke: true' "$ci_workflow")" -eq 2 ] \
    || fail "CLI smoke must have exactly one Linux and one macOS owner"
container_workflow=$ci_workflow
{
    [ "$(grep -Fc 'sh tools/release/setup-glibc-builder.sh' "$container_workflow")" -eq 1 ] \
        || fail "container workflow bypasses the shared glibc setup: $container_workflow"
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
}
grep -Fq "echo \"\$HOME/.cargo/bin\" >>\"\$GITHUB_PATH\"" "$glibc_setup" \
    || fail "container setup must use the runtime HOME Cargo path"
if grep -Fq '/root/.cargo/bin' "$glibc_setup"; then
    fail "container setup must not assume root HOME"
fi
for floor in 'minimum_macos=13.0' 'minimum_glibc=2.28'; do
    grep -Fq "$floor" "$native_build" \
        || fail "shared native build lacks platform floor owner: $floor"
done
for api_owner in "$workflow:3" "$operator:1" "$green_ci_lookup:1"; do
    owner_path=${api_owner%:*}
    expected_headers=${api_owner##*:}
    actual_headers=$(grep -Fc 'X-GitHub-Api-Version: 2022-11-28' \
        "$owner_path" || true)
    [ "$actual_headers" -eq "$expected_headers" ] \
        || fail "GitHub API owner has a missing or duplicate stable version header: $owner_path"
done
if grep -Fq 'X-GitHub-Api-Version: 2026-' \
    "$workflow" "$operator" "$green_ci_lookup"; then
    fail "GitHub API calls must use the supported 2022-11-28 version"
fi
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
