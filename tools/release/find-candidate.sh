#!/bin/sh
set -eu

fail() {
    echo "release candidate lookup failed: $*" >&2
    exit 1
}

run_id=${1:-}
commit=${2:-}
repository=${ZTERM_RELEASE_REPOSITORY:-${GITHUB_REPOSITORY:-leonfox28/zterm}}
case "$run_id" in ''|*[!0-9]*) fail "RUN_ID must be numeric" ;; esac
case "$commit" in ''|*[!0-9a-f]*) fail "COMMIT must be lowercase hexadecimal" ;; esac
[ "${#commit}" -eq 40 ] || fail "COMMIT must contain 40 hexadecimal characters"

# A failed-job rerun can reuse a successful candidate job from an earlier
# attempt. Select the newest retained candidate within this exact green run;
# immutable artifact IDs keep publication bound to the selected bytes.
verified_run=$(gh api \
    -H 'Accept: application/vnd.github+json' \
    -H 'X-GitHub-Api-Version: 2022-11-28' \
    "repos/$repository/actions/runs/$run_id" \
    --jq ". | select(.event == \"push\" and .head_branch == \"main\"
        and .head_sha == \"$commit\" and .status == \"completed\"
        and .conclusion == \"success\" and .path == \".github/workflows/ci.yml\")
        | .id")
[ "$verified_run" = "$run_id" ] || fail "run is not exact green main CI"
artifact_id=$(gh api --paginate --slurp \
    -H 'Accept: application/vnd.github+json' \
    -H 'X-GitHub-Api-Version: 2022-11-28' \
    "repos/$repository/actions/runs/$run_id/artifacts?per_page=100" \
    --jq "[.[].artifacts[]
        | select(.name | test(\"^release-candidate-$commit-[0-9]+$\"))
        | select(.expired == false and .workflow_run.head_sha == \"$commit\"
            and .workflow_run.head_branch == \"main\"
            and ((.digest // \"\") | test(\"^sha256:[0-9a-f]{64}$\")))]
        | max_by(.id) | .id // empty")
case "$artifact_id" in
    ''|*[!0-9]*) fail "exact candidate is missing, expired, or ambiguous; rerun main CI for $commit before tagging" ;;
esac
printf '%s\n' "$artifact_id"
