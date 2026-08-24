#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
workflow="$repo_root/.github/workflows/release.yml"
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

grep -Eq '^[[:space:]]{2}workflow_dispatch:' "$workflow" \
    || fail "release workflow must be manual-only"
if grep -Eq '^[[:space:]]{2}(push|pull_request|release):' "$workflow"; then
    fail "release workflow must not have an automatic trigger"
fi
environment_gates=$(grep -Fc 'environment: release' "$workflow" || true)
[ "$environment_gates" -eq 2 ] \
    || fail "signing and draft creation must each use the protected release Environment"
grep -Fq 'immutable_release_checkpoint' "$workflow" \
    || fail "workflow must require the repo-admin immutable Release checkpoint"
grep -Fq 'enabled-and-reviewed' "$workflow" \
    || fail "workflow must fail closed until the immutable Release checkpoint"
grep -Fq -- '--draft' "$workflow" \
    || fail "release workflow must create a draft"
if grep -Eq -- '--draft=false|gh release edit|gh release delete' "$workflow"; then
    fail "M9 workflow must leave the verified Release as a draft"
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
    || fail "draft creation must recheck the existing tag against the validated commit"
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
grep -Fq "[ ! -L \"\$candidate\" ] && [ -f \"\$candidate\" ]" "$template" \
    || fail "versioned installer must reject a symlink/non-file candidate before chmod"

public_key=$(tr -d '[:space:]' < "$repo_root/release/public-key.hex")
case "$public_key" in
    UNCONFIGURED) ;;
    *[!0-9a-f]*|'') fail "release public key must be the explicit placeholder or lowercase hex" ;;
    *) [ "${#public_key}" -eq 64 ] || fail "release public key must contain 32 bytes" ;;
esac

echo "release workflow and installer policy verified"
