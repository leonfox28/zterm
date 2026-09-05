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
finish_root=
finish_worktree=

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
    if [ -n "$finish_worktree" ]; then
        if git -C "$repo_root" worktree remove "$finish_worktree"; then
            rmdir "$finish_root"
        else
            echo "Retained release checkout for diagnosis: $finish_worktree" >&2
        fi
    fi
    [ "$status" -ne 0 ] || return 0
    case "$phase" in
        prepare-branch)
            echo 'Recovery: the local release branch state was retained for diagnosis. Inspect and finish dirty or invalid state manually; release-prepare resumes only an exact clean release commit.' >&2
            ;;
        prepare-commit)
            echo "Recovery: the exact clean release commit was retained. If push or PR creation had an ambiguous network result, rerun: just release-prepare $version" >&2
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
        finish)
            echo "Resume the same reviewed PR with: just release $version $pr_number" >&2
            ;;
    esac
}
trap on_exit EXIT

usage() {
    fail "usage: operator.sh <prepare|publish> <SEMVER> | finish <SEMVER> <PR-number>"
}

command_name=${1:-}
version=${2:-}
case "$command_name" in
    prepare|publish) [ "$#" -eq 2 ] || usage ;;
    finish) [ "$#" -eq 3 ] || usage ;;
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

for required in cargo gh git; do
    command -v "$required" >/dev/null 2>&1 || fail "$required is required; run just doctor"
done

cd "$repo_root"

require_clean_worktree() {
    [ -z "$(git status --porcelain --untracked-files=all)" ] \
        || fail "the worktree must be clean"
}

fetch_main() {
    git fetch --quiet --prune --no-tags origin \
        refs/heads/main:refs/remotes/origin/main
}

require_clean_main() {
    require_clean_worktree
    fetch_main
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
    require_release_vacancy
}

require_release_vacancy() {
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

expected_release_inventory='Cargo.lock
Cargo.toml'

fail_release_inventory() {
    inventory_owner=$1
    actual_inventory=$2
    {
        printf '%s\n' "release operator failed: $inventory_owner inventory mismatch"
        printf '%s\n' 'expected:'
        printf '%s\n' "$expected_release_inventory"
        printf '%s\n' 'actual:'
        if [ -n "$actual_inventory" ]; then
            printf '%s\n' "$actual_inventory"
        else
            printf '%s\n' '(empty)'
        fi
    } >&2
    exit 1
}

require_exact_release_inventory() {
    inventory_owner=$1
    actual_inventory=$2
    [ "$actual_inventory" = "$expected_release_inventory" ] \
        || fail_release_inventory "$inventory_owner" "$actual_inventory"
}

require_release_change_inventory() {
    changed_files=$(release_change_inventory)
    require_exact_release_inventory 'release preparation' "$changed_files"
}

parse_run_info() {
    run_info=$1
    parsed_run_id=${run_info%% *}
    parsed_run_url=${run_info#* }
    if [ "$parsed_run_id" = "$run_info" ] \
        || [ -z "$parsed_run_id" ] || [ -z "$parsed_run_url" ]; then
        fail "GitHub returned an ambiguous run result"
    fi
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
    cargo +1.98.0 update --workspace
    cargo +1.98.0 metadata --locked --format-version 1 --no-deps >/dev/null
    sh tests/workspace-version.sh >/dev/null
    require_release_change_inventory
}

require_exact_release_commit() {
    release_head=$(git rev-parse HEAD)
    main_head=$(git rev-parse refs/remotes/origin/main)
    git merge-base --is-ancestor "$main_head" "$release_head" \
        || fail "release branch must contain current origin/main"
    release_parents=$(git show -s --format=%P "$release_head")
    case "$release_parents" in ''|*' '*) fail "version preparation must be one ordinary commit" ;; esac

    release_subject=$(git show -s --format=%s "$release_head")
    expected_subject="chore: prepare $tag release"
    [ "$release_subject" = "$expected_subject" ] \
        || fail "release commit subject is '$release_subject', expected '$expected_subject'"

    committed_files=$(git diff-tree --no-commit-id --name-only -r "$release_head" \
        | LC_ALL=C sort -u)
    require_exact_release_inventory 'release commit' "$committed_files"

    cargo +1.98.0 metadata --locked --format-version 1 --no-deps >/dev/null
    current_version=$(workspace_version)
    [ "$current_version" = "$version" ] \
        || fail "release commit workspace version is $current_version, expected $version"
    sh tests/workspace-version.sh >/dev/null
    require_clean_worktree
}

remote_release_branch_state() {
    set +e
    remote_release_output=$(git ls-remote --heads origin \
        "refs/heads/$release_branch")
    remote_release_status=$?
    set -e
    [ "$remote_release_status" -eq 0 ] \
        || fail "could not inspect remote branch $release_branch"
    if [ -z "$remote_release_output" ]; then
        remote_release_sha=
        return 1
    fi

    remote_release_lines=$(printf '%s\n' "$remote_release_output" \
        | sed '/^[[:space:]]*$/d' | wc -l | tr -d '[:space:]')
    [ "$remote_release_lines" -eq 1 ] \
        || fail "remote branch lookup for $release_branch was ambiguous"
    remote_release_sha=$(printf '%s\n' "$remote_release_output" | awk '{ print $1 }')
    remote_release_ref=$(printf '%s\n' "$remote_release_output" | awk '{ print $2 }')
    [ "$remote_release_ref" = "refs/heads/$release_branch" ] \
        || fail "remote branch lookup for $release_branch returned an unexpected ref"
    return 0
}

reconcile_release_branch() {
    local_release_sha=$(git rev-parse HEAD)
    if remote_release_branch_state; then
        if [ "$remote_release_sha" != "$local_release_sha" ]; then
            git merge-base --is-ancestor "$remote_release_sha" "$local_release_sha" \
                || fail "remote branch $release_branch points to $remote_release_sha, expected $local_release_sha or its ancestor"
            git push --set-upstream origin "HEAD:refs/heads/$release_branch"
        fi
    else
        git push --set-upstream origin "HEAD:refs/heads/$release_branch"
    fi

    remote_release_branch_state \
        || fail "remote branch $release_branch is absent after push"
    [ "$remote_release_sha" = "$local_release_sha" ] \
        || fail "remote branch $release_branch points to $remote_release_sha, expected $local_release_sha"
}

query_release_pr_records() {
    set +e
    release_pr_records=$(gh pr list --repo "$repository" --state all \
        --head "$release_branch" --limit 100 \
        --json url,state,headRefName,baseRefName,headRefOid,isCrossRepository \
        --jq '.[] | [.url, .state, .headRefName, .baseRefName, .headRefOid, (.isCrossRepository | tostring)] | @tsv')
    release_pr_status=$?
    set -e
    [ "$release_pr_status" -eq 0 ] \
        || fail "could not reconcile pull requests for $release_branch"
}

reconcile_release_pr() {
    local_release_sha=$(git rev-parse HEAD)
    release_pr_created=false
    while :; do
        query_release_pr_records
        release_pr_count=$(printf '%s\n' "$release_pr_records" \
            | awk 'NF { count += 1 } END { print count + 0 }')
        case "$release_pr_count" in
        0)
            [ "$release_pr_created" = false ] \
                || fail "created PR for $release_branch was not discoverable"
            set +e
            set -- --repo "$repository" --base main --head "$release_branch"
            if [ "$release_branch" = "release/$tag" ]; then
                set -- "$@" --title "chore: prepare $tag release" \
                    --body "Prepare $tag. Merge after CI gate succeeds; publish the exact main candidate."
            else
                set -- "$@" --fill
            fi
            gh pr create "$@" >/dev/null
            release_pr_create_status=$?
            set -e
            [ "$release_pr_create_status" -eq 0 ] \
                || fail "could not create the release PR; its result may be ambiguous"
            release_pr_created=true
            ;;
        1)
            tab=$(printf '\t')
            release_pr_fields=$(printf '%s\n' "$release_pr_records" \
                | awk -F "$tab" 'NR == 1 { print NF }')
            [ "$release_pr_fields" -eq 6 ] \
                || fail "GitHub returned malformed PR state for $release_branch"
            IFS="$tab" read -r release_pr_url release_pr_state \
                release_pr_head release_pr_base release_pr_sha \
                release_pr_cross_repo <<EOF
$release_pr_records
EOF
            [ "$release_pr_state" = OPEN ] \
                || fail "existing PR for $release_branch is not open"
            [ "$release_pr_head" = "$release_branch" ] \
                || fail "existing PR head is $release_pr_head, expected $release_branch"
            [ "$release_pr_base" = main ] \
                || fail "existing PR base is $release_pr_base, expected main"
            [ "$release_pr_sha" = "$local_release_sha" ] \
                || fail "existing PR head is $release_pr_sha, expected $local_release_sha"
            [ "$release_pr_cross_repo" = false ] \
                || fail "existing PR for $release_branch is from another repository"
            break
            ;;
        *)
            fail "multiple pull requests exist for $release_branch; refusing ambiguous state"
            ;;
        esac
    done
    [ -n "$release_pr_url" ] || fail "GitHub returned an empty release PR URL"
}

prepare_release() {
    current_branch=$(git branch --show-current)
    [ -n "$current_branch" ] || fail "prepare requires a named branch"
    require_clean_worktree
    require_canonical_context
    fetch_main
    require_tag_and_release_vacancy
    case "$current_branch" in
        main)
            require_clean_main
            require_prepare_branch_vacancy
            ;;
        *)
            release_branch=$current_branch
            git merge-base --is-ancestor refs/remotes/origin/main HEAD \
                || fail "update the feature branch from origin/main before preparing its release"
            ;;
    esac

    if [ "$(workspace_version)" = "$version" ] && [ "$current_branch" != main ]; then
        phase=prepare-branch
        require_exact_release_commit
    else
        cargo +1.98.0 run --quiet --locked --package zterm-release-tool -- \
            validate-next-version "$version"
        if [ "$current_branch" = main ]; then
            git switch -c "$release_branch"
        fi
        phase=prepare-branch
        update_workspace_version
        git add Cargo.toml Cargo.lock
        staged_files=$(git diff --cached --name-only | LC_ALL=C sort -u)
        require_exact_release_inventory 'staged release commit' "$staged_files"
        git commit -m "chore: prepare $tag release"
        require_exact_release_commit
    fi
    phase=prepare-commit
    reconcile_release_branch
    reconcile_release_pr
    phase=complete
    printf '%s\n' \
        "Release PR: $release_pr_url" \
        "To finish this reviewed PR under maintainer authorization: just release $version <PR-number>" \
        "Publication alone remains available: just release-publish $version"
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

discover_run() {
    workflow=$1
    run_branch=$2
    run_event=${3:-push}
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
        run=$(gh run list --repo "$repository" --workflow "$workflow" \
            --commit "$commit" --event "$run_event" --limit 20 \
            --json databaseId,url,headSha,headBranch \
            --jq "map(select(.headSha == \"$commit\" and .headBranch == \"$run_branch\"))
                | .[0] | select(.) | \"\(.databaseId) \(.url)\"")
        if [ -n "$run" ]; then
            parse_run_info "$run"
            printf '%s %s\n' "$parsed_run_id" "$parsed_run_url"
            return 0
        fi
        sleep "$delay"
        attempt=$((attempt + 1))
    done
    fail "the $workflow run was not discoverable; inspect gh run list --workflow $workflow --commit $commit"
}

watch_release() {
    release_run=$(discover_run release.yml "$tag")
    parse_run_info "$release_run"
    run_id=$parsed_run_id
    run_url=$parsed_run_url
    printf '%s\n' \
        "Formal release run: $run_url" \
        'The run can pause at protected signing until a release Environment reviewer approves it.'
    if ! gh run watch "$run_id" --repo "$repository" --exit-status --compact --interval 15; then
        echo "Resume: gh run watch $run_id --repo $repository --exit-status" >&2
        return 1
    fi
    phase=complete
    printf '%s\n' "Immutable three-platform native release completed: $run_url"
}

publish_release() {
    phase=publish
    require_clean_worktree
    require_canonical_context
    current_version=$(workspace_version)
    [ "$version" = "$current_version" ] \
        || fail "requested version $version must exactly equal canonical workspace version $current_version"
    case "$current_version" in
        *+*) fail "formal release versions must not contain SemVer build metadata" ;;
    esac
    commit=$(git rev-parse HEAD)
    if remote_ref_state tags "refs/tags/$tag"; then
        git fetch --quiet --no-tags origin "refs/tags/$tag"
        if [ "$(git cat-file -t FETCH_HEAD)" != tag ] \
            || [ "$(git rev-parse 'FETCH_HEAD^{commit}')" != "$commit" ]; then
            fail "existing remote tag does not identify this exact annotated source"
        fi
        tag_pushed=true
        watch_release
        return
    fi

    require_clean_main
    require_protected_main
    require_release_vacancy
    green_run=$(ZTERM_RELEASE_REPOSITORY="$repository" \
        sh "$script_dir/find-green-main-ci.sh" "$commit")
    parse_run_info "$green_run"
    printf '%s\n' "Exact green main CI: $parsed_run_url"
    candidate_id=$(ZTERM_RELEASE_REPOSITORY="$repository" \
        sh "$script_dir/find-candidate.sh" "$parsed_run_id" "$commit")
    printf '%s\n' "Exact release candidate artifact: $candidate_id"

    if git show-ref --verify --quiet "refs/tags/$tag"; then
        if [ "$(git cat-file -t "refs/tags/$tag")" != tag ] \
            || [ "$(git rev-list -n 1 "$tag")" != "$commit" ]; then
            fail "existing local tag does not identify this exact annotated source"
        fi
    else
        git tag -a "$tag" -m "zterm $tag"
    fi
    tag_created=true
    git push origin "refs/tags/$tag:refs/tags/$tag"
    tag_pushed=true
    watch_release
}

read_finish_pr() {
    pr_record=$(gh pr view "$pr_number" --repo "$repository" \
        --json state,headRefOid,headRefName,baseRefName,isCrossRepository,mergeCommit \
        --jq '[.state, .headRefOid, .baseRefName, (.isCrossRepository | tostring), (.mergeCommit.oid // "-"), .headRefName] | @tsv')
    tab=$(printf '\t')
    IFS="$tab" read -r pr_state pr_head pr_base pr_cross_repo pr_merge pr_branch <<EOF
$pr_record
EOF
    if [ "$pr_base" != main ] || [ "$pr_cross_repo" != false ]; then
        fail "release requires one same-repository PR targeting main"
    fi
}

finish_release() {
    pr_number=$1
    case "$pr_number" in ''|*[!0-9]*) fail "PR-number must be numeric" ;; esac
    phase=finish
    require_clean_worktree
    require_canonical_context
    read_finish_pr
    case "$pr_state" in
        OPEN)
            [ "$(git rev-parse HEAD)" = "$pr_head" ] \
                || fail "local HEAD must equal the reviewed PR head before merging"
            [ "$(workspace_version)" = "$version" ] \
                || fail "prepare the requested version in this PR before release"
            require_protected_main
            require_tag_and_release_vacancy
            expected_pr_head=$pr_head
            commit=$pr_head
            pr_run=$(discover_run ci.yml "$pr_branch" pull_request)
            parse_run_info "$pr_run"
            gh run watch "$parsed_run_id" --repo "$repository" --exit-status --compact --interval 15
            gh pr checks "$pr_number" --repo "$repository" \
                --required --watch --fail-fast --interval 15
            gh pr merge "$pr_number" --repo "$repository" --merge \
                --match-head-commit "$expected_pr_head"
            read_finish_pr
            ;;
        MERGED) ;;
        *) fail "release PR is neither open nor merged" ;;
    esac
    [ "$pr_state" = MERGED ] || fail "GitHub did not confirm the PR merge"
    commit=$pr_merge
    case "$commit" in ''|*[!0-9a-f]*) fail "GitHub returned an invalid merge commit" ;; esac
    [ "${#commit}" -eq 40 ] || fail "GitHub returned an invalid merge commit"
    fetch_main
    git merge-base --is-ancestor "$commit" refs/remotes/origin/main \
        || fail "the PR merge commit does not belong to origin/main"

    # Resume a pushed tag directly; otherwise wait for this exact main commit.
    if ! remote_ref_state tags "refs/tags/$tag"; then
        main_run=$(discover_run ci.yml main)
        parse_run_info "$main_run"
        gh run watch "$parsed_run_id" --repo "$repository" --exit-status --compact --interval 15
    fi
    finish_root=$(mktemp -d)
    finish_worktree="$finish_root/source"
    git worktree add --detach "$finish_worktree" "$commit"
    if [ "$test_mode" = 1 ]; then
        ZTERM_RELEASE_WORKTREE="$finish_worktree" \
            sh "$script_dir/operator.sh" publish "$version"
    else
        sh "$finish_worktree/tools/release/operator.sh" publish "$version"
    fi
    phase=complete
}

case "$command_name" in
    prepare) prepare_release ;;
    publish) publish_release ;;
    finish) finish_release "$3" ;;
esac
