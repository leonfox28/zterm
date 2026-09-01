#!/bin/sh
set -eu

fail() {
    echo "green main CI lookup failed: $*" >&2
    exit 1
}

commit=${1:-}
repository=${ZTERM_RELEASE_REPOSITORY:-${GITHUB_REPOSITORY:-leonfox28/zterm}}
case "$commit" in
    ''|*[!0-9a-f]*) fail "COMMIT must be a lowercase hexadecimal Git object ID" ;;
esac
[ "${#commit}" -eq 40 ] || fail "COMMIT must contain exactly 40 hexadecimal characters"
command -v gh >/dev/null 2>&1 || fail "gh is required"

run=$(gh api --method GET \
    -H 'Accept: application/vnd.github+json' \
    -H 'X-GitHub-Api-Version: 2022-11-28' \
    "repos/$repository/actions/workflows/ci.yml/runs" \
    -f branch=main -f event=push -f status=success \
    -f head_sha="$commit" -f per_page=100 \
    --jq ".workflow_runs
        | map(select(.event == \"push\"
            and .head_branch == \"main\"
            and .head_sha == \"$commit\"
            and .status == \"completed\"
            and .conclusion == \"success\"))
        | sort_by(.created_at) | reverse | .[0]
        | select(.) | \"\(.id) \(.html_url)\"")
[ -n "$run" ] || fail "commit $commit has no successful completed ci.yml push run on main"

run_id=${run%% *}
run_url=${run#* }
[ "$run_id" != "$run" ] && [ -n "$run_id" ] && [ -n "$run_url" ] \
    || fail "GitHub returned an ambiguous CI result"
case "$run_url" in *' '*) fail "GitHub returned an ambiguous CI result" ;; esac
printf '%s %s\n' "$run_id" "$run_url"
