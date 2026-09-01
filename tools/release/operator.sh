#!/bin/sh
set -eu

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
default_repo_root=$(CDPATH='' cd -- "$script_dir/../.." && pwd)
canonical_repository=leonfox28/zterm
test_mode=${ZTERM_RELEASE_TEST_MODE:-0}
repository=${ZTERM_RELEASE_REPOSITORY:-$canonical_repository}
repo_root=${ZTERM_RELEASE_WORKTREE:-$default_repo_root}
phase=preflight
tag_created=false
tag_pushed=false

fail() {
    echo "release operator failed: $*" >&2
    exit 1
}

github_api() {
    gh api \
        -H 'Accept: application/vnd.github+json' \
        -H 'X-GitHub-Api-Version: 2022-11-28' \
        "$@"
}

on_exit() {
    status=$?
    [ "$status" -ne 0 ] || return 0
    case "$phase" in
        prepare-branch)
            echo 'Recovery: the local release branch and version diff were retained; inspect them, fix the failure, then rerun just check and push/open the PR manually.' >&2
            ;;
        prepare-pushed)
            echo 'Recovery: the remote release branch was retained; fix/push that branch and run gh pr create when ready. No tag was created.' >&2
            ;;
        publish)
            if [ "$tag_pushed" = true ]; then
                echo 'Recovery: the tag is already public and must not be replaced. Inspect/rerun the exact release workflow; an immutable asset defect requires a new version.' >&2
            elif [ "$tag_created" = true ]; then
                echo 'Recovery: the annotated local tag was retained. Prove the remote tag is absent, then inspect and push this exact tag manually; never replace a remote tag.' >&2
            else
                echo 'Recovery: publication stopped before tag push; fix the reported precondition and rerun release-publish.' >&2
            fi
            ;;
    esac
}
trap on_exit EXIT

usage() {
    fail "usage: operator.sh <prepare|publish> <SEMVER>"
}

command_name=${1:-}
version=${2:-}
[ "$#" -eq 2 ] || usage
case "$command_name" in
    prepare|publish) ;;
    *) usage ;;
esac
[ -n "$version" ] || usage
tag="v$version"
release_branch="release/$tag"

if [ "$repository" != "$canonical_repository" ] && [ "$test_mode" != 1 ]; then
    fail "repository override is allowed only in the isolated operator fixture"
fi
if [ "$repo_root" != "$default_repo_root" ] && [ "$test_mode" != 1 ]; then
    fail "worktree override is allowed only in the isolated operator fixture"
fi

for required in cargo gh git just; do
    command -v "$required" >/dev/null 2>&1 || fail "$required is required; run just doctor"
done

cd "$repo_root"

require_clean_main() {
    [ -z "$(git status --porcelain)" ] || fail "the worktree must be clean"
    [ "$(git branch --show-current)" = main ] || fail "run this command from main"
    git fetch --quiet --prune --no-tags origin \
        refs/heads/main:refs/remotes/origin/main
    [ "$(git rev-parse HEAD)" = "$(git rev-parse refs/remotes/origin/main)" ] \
        || fail "local main must exactly match origin/main"
}

require_canonical_context() {
    gh auth status --hostname github.com >/dev/null 2>&1 \
        || fail "GitHub CLI authentication is required; run gh auth login"
    resolved_repository=$(gh repo view --json nameWithOwner --jq .nameWithOwner)
    [ "$resolved_repository" = "$repository" ] \
        || fail "GitHub CLI resolved $resolved_repository, expected $repository"
    if [ "$test_mode" != 1 ]; then
        origin_url=$(git remote get-url origin)
        case "$origin_url" in
            git@github.com:leonfox28/zterm.git|https://github.com/leonfox28/zterm.git|ssh://git@github.com/leonfox28/zterm.git) ;;
            *) fail "origin is not the canonical zterm GitHub repository" ;;
        esac
    fi
}

remote_ref_state() {
    kind=$1
    ref=$2
    set +e
    git ls-remote --exit-code "--$kind" origin "$ref" >/dev/null 2>&1
    status=$?
    set -e
    case "$status" in
        0) return 0 ;;
        2) return 1 ;;
        *) fail "could not prove remote $kind vacancy for $ref" ;;
    esac
}

require_tag_and_release_vacancy() {
    if git show-ref --verify --quiet "refs/tags/$tag"; then
        fail "local tag $tag already exists"
    fi
    if remote_ref_state tags "refs/tags/$tag"; then
        fail "remote tag $tag already exists"
    fi
    set +e
    release_lookup=$(github_api --include \
        "repos/$repository/releases/tags/$tag" 2>&1)
    release_status=$?
    set -e
    case "$release_status" in
        0) fail "GitHub Release $tag already exists; releases and assets are never replaced" ;;
        *)
            printf '%s\n' "$release_lookup" | grep -Eq 'HTTP[/][^ ]+ 404|HTTP 404' \
                || fail "could not prove GitHub Release $tag is vacant"
            ;;
    esac
}

require_prepare_branch_vacancy() {
    if git show-ref --verify --quiet "refs/heads/$release_branch"; then
        fail "local branch $release_branch already exists"
    fi
    if remote_ref_state heads "refs/heads/$release_branch"; then
        fail "remote branch $release_branch already exists"
    fi
}

workspace_version() {
    package_id=$(cargo +1.98.0 pkgid --locked --package zterm-cli 2>/dev/null) \
        || fail "Cargo could not resolve the workspace product version"
    case "$package_id" in
        *@*) printf '%s\n' "${package_id##*@}" ;;
        *) fail "Cargo returned an unexpected package ID" ;;
    esac
}

release_change_inventory() {
    {
        git diff --name-only
        git diff --cached --name-only
        git ls-files --others --exclude-standard
    } | LC_ALL=C sort -u
}

require_release_change_inventory() {
    changed_files=$(release_change_inventory)
    [ "$changed_files" = "Cargo.lock
Cargo.toml" ] || fail "release preparation changed files outside Cargo.toml and Cargo.lock"
}

parse_run_info() {
    run_info=$1
    parsed_run_id=${run_info%% *}
    parsed_run_url=${run_info#* }
    [ "$parsed_run_id" != "$run_info" ] \
        && [ -n "$parsed_run_id" ] && [ -n "$parsed_run_url" ] \
        || fail "GitHub returned an ambiguous run result"
    case "$parsed_run_url" in
        *' '*) fail "GitHub returned an ambiguous run result" ;;
    esac
}

update_workspace_version() {
    temporary_manifest=Cargo.toml.release-prepare
    awk -v next_version="$version" '
        BEGIN { in_workspace_package = 0; changed = 0 }
        /^\[workspace[.]package\]$/ {
            in_workspace_package = 1
            print
            next
        }
        in_workspace_package && /^\[/ { in_workspace_package = 0 }
        in_workspace_package && /^version = "[^"]+"$/ {
            print "version = \"" next_version "\""
            changed += 1
            next
        }
        { print }
        END { if (changed != 1) exit 42 }
    ' Cargo.toml >"$temporary_manifest" \
        || fail "could not update the single workspace.package version"
    mv "$temporary_manifest" Cargo.toml
    cargo +1.98.0 metadata --format-version 1 --no-deps >/dev/null
    sh tests/workspace-version.sh >/dev/null
    require_release_change_inventory
}

prepare_release() {
    require_clean_main
    require_canonical_context
    require_prepare_branch_vacancy
    require_tag_and_release_vacancy
    cargo +1.98.0 run --quiet --locked --package zterm-release-tool -- \
        validate-next-version "$version"

    git switch -c "$release_branch"
    phase=prepare-branch
    update_workspace_version
    just check
    require_release_change_inventory
    git add Cargo.toml Cargo.lock
    staged_files=$(git diff --cached --name-only | LC_ALL=C sort)
    [ "$staged_files" = "Cargo.lock
Cargo.toml" ] \
        || fail "release commit inventory is not exactly Cargo.toml and Cargo.lock: $staged_files"
    git commit -m "chore: prepare $tag release"
    [ -z "$(git status --porcelain --untracked-files=all)" ] \
        || fail "release commit left unexpected worktree changes; inspect them before pushing"
    git push --set-upstream origin "HEAD:refs/heads/$release_branch"
    phase=prepare-pushed
    pr_url=$(gh pr create --repo "$repository" --base main --head "$release_branch" \
        --title "chore: prepare $tag release" \
        --body "Prepare $tag. Merge only after the required CI gate is green; publication remains a separate maintainer action.")
    phase=complete
    printf '%s\n' \
        "Release PR: $pr_url" \
        "After human merge and exact main CI success, run: just release-publish $version"
}

require_protected_main() {
    protection=$(github_api "repos/$repository/branches/main/protection" \
        --jq '((.required_status_checks.contexts // []) | index("CI gate") != null)
            and (.required_pull_request_reviews != null)
            and ((.enforce_admins.enabled // false) == true)
            and ((.allow_force_pushes.enabled // false) == false)
            and ((.allow_deletions.enabled // false) == false)') \
        || fail "could not read main branch protection; an administrator must apply docs/development.md"
    [ "$protection" = true ] \
        || fail "main protection must require PRs and CI gate and disable force pushes/deletion"
}

discover_release_run() {
    attempts=${ZTERM_RELEASE_POLL_ATTEMPTS:-30}
    delay=${ZTERM_RELEASE_POLL_SECONDS:-2}
    if [ "$test_mode" != 1 ]; then
        attempts=30
        delay=2
    fi
    case "$attempts" in ''|*[!0-9]*) fail "invalid release watcher attempt bound" ;; esac
    case "$delay" in ''|*[!0-9]*) fail "invalid release watcher delay" ;; esac
    [ "$attempts" -gt 0 ] || fail "release watcher attempts must be positive"
    attempt=1
    while [ "$attempt" -le "$attempts" ]; do
        run=$(gh run list --repo "$repository" --workflow release.yml \
            --commit "$commit" --event push --limit 20 \
            --json databaseId,url,headSha,headBranch \
            --jq "map(select(.headSha == \"$commit\" and .headBranch == \"$tag\"))
                | .[0] | select(.) | \"\(.databaseId) \(.url)\"")
        if [ -n "$run" ]; then
            parse_run_info "$run"
            printf '%s %s\n' "$parsed_run_id" "$parsed_run_url"
            return 0
        fi
        sleep "$delay"
        attempt=$((attempt + 1))
    done
    fail "the release.yml run was not discoverable; resume with gh run list --workflow release.yml --commit $commit"
}

publish_release() {
    phase=publish
    require_clean_main
    require_canonical_context
    current_version=$(workspace_version)
    [ "$version" = "$current_version" ] \
        || fail "requested version $version must exactly equal canonical workspace version $current_version"
    case "$current_version" in
        *+*) fail "formal release versions must not contain SemVer build metadata" ;;
    esac
    require_protected_main
    require_tag_and_release_vacancy
    commit=$(git rev-parse HEAD)
    green_run=$(ZTERM_RELEASE_REPOSITORY="$repository" \
        sh "$script_dir/find-green-main-ci.sh" "$commit")
    parse_run_info "$green_run"
    printf '%s\n' "Exact green main CI: $parsed_run_url"

    git tag -a "$tag" -m "zterm $tag"
    tag_created=true
    git push origin "refs/tags/$tag:refs/tags/$tag"
    tag_pushed=true
    release_run=$(discover_release_run)
    parse_run_info "$release_run"
    run_id=$parsed_run_id
    run_url=$parsed_run_url
    printf '%s\n' \
        "Formal release run: $run_url" \
        'The run can pause at protected signing until a release Environment reviewer approves it.'
    if ! gh run watch "$run_id" --repo "$repository" --exit-status; then
        echo "Resume: gh run watch $run_id --repo $repository --exit-status" >&2
        return 1
    fi
    phase=complete
    printf '%s\n' "Native release and explicit relay publication completed: $run_url"
}

case "$command_name" in
    prepare) prepare_release ;;
    publish) publish_release ;;
esac
