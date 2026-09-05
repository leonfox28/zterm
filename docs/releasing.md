# Release operations

The default publication scope is **macOS arm64, Linux arm64, and Linux x64**.
macOS Intel, Windows CI/distribution, and relay image publication are paused.
Restore any of them only when a future task explicitly requests it. Existing
immutable Releases and already published relay images remain historical assets.
This platform matrix belongs to the release tool. Installed updaters require
and validate only their own target entry in the authenticated manifest; they
do not enforce the presence, number, or support policy of other platforms.

```text
reviewed feature branch + version commit in one PR
  -> PR CI -> merge the exact reviewed head
  -> exact-SHA main CI + three native builds + verified unsigned candidate
  -> annotated vVERSION tag -> select that main candidate by artifact ID
  -> protected signing -> three final installer proofs -> immutable Release
```

`VERSION` is canonical SemVer without the `v` prefix or build metadata.
Prereleases such as `0.2.0-rc.1` are supported. Run `just doctor` first. The
operator never installs global tools or changes repository protection settings.

## Prepare one reviewable PR

For an immediately released fix, commit the reviewed product changes on a clean
feature branch that contains current `origin/main`, then run (preferably before
the first PR push, or on an already open feature PR):

```bash
just release-prepare 0.2.0
```

This keeps the current feature branch, updates only the workspace version and
Cargo-generated lockfile, adds one version commit, pushes the same branch, and
creates or reuses its PR. New feature PRs derive their title/body from the
product commits. An existing PR keeps its description. Code and version are
reviewed together, so no second version PR/CI cycle is needed.

When accumulating changes for a later release, the same command from clean,
exact `origin/main` creates a standalone `release/v0.2.0` branch and version PR.

Cargo's `semver` owner validates the next version. `cargo +1.98.0 update
--workspace` owns lockfile generation; locked metadata, workspace-version checks,
and the exact two-file version-commit inventory validate it. Preparation does
not repeat `just check`; the required PR `CI gate` owns the full remote gate.

If generation fails, inspect the retained dirty diff. Resume after a push/PR
network ambiguity is accepted only from the exact clean version commit on the
same branch, still containing current main. Remote branch and open-PR identities
must agree; divergence, a moved main, or closed/ambiguous PR state is rejected.

## Finish an authorized release

Once the PR is reviewed and publication is authorized:

```bash
just release 0.2.0 42
```

This command is an explicit request to merge and publish PR 42. It requires the
local clean HEAD to equal the open PR head and contain the requested version,
waits for required checks, merges with `--match-head-commit`, waits for CI on the
returned main merge SHA, and publishes from a private detached worktree. It
preserves the caller's branch and removes only its own clean temporary worktree.
The normal repository PR/protection rules still apply; no admin bypass is used.

The PR, merge SHA, Actions run and annotated tag are the recovery record. Rerun
the same command after an interrupted wait; an already merged PR is not merged
again, and an already pushed matching tag rejoins its original workflow. There
is no parallel local release database. Watchers use GitHub CLI's compact view.

Publication remains available independently from a clean checkout of exact
`origin/main`:

```bash
just release-publish 0.2.0
```

Before a new tag is pushed, this checks version, enforced main protection,
Release vacancy, completed successful exact-SHA `ci.yml` main-push evidence,
and an unexpired candidate with a server digest. If main has advanced beyond the
reviewed merge, it stops before tagging instead of including unreviewed changes.
A retained local annotated tag may be pushed only if it already names that exact
source; remote tags and Release assets are never replaced.

## Build once, publish the verified bytes

Main CI runs the normal test/policy graph and builds only the three shipped
native binaries. Linux uses the pinned glibc 2.28 builder; macOS declares the
13.0 deployment floor. The candidate jobs explicitly embed the exact source SHA.
Ordinary PR and development builds retain their development identity.

One Ubuntu job creates deterministic archives, the manifest, installer and SBOM,
verifies the unsigned inventory, and ShellChecks the generated installer. The
candidate uses the source commit timestamp and is uploaded as
`release-candidate-SHA-ATTEMPT`, retained for seven days. The main `CI gate`
requires this job to succeed.

A failed-job rerun may reuse successful native jobs from an earlier attempt.
Intermediate per-target uploads may therefore be replaced within that same CI
run; the assembled candidate always receives a new immutable artifact ID. The
lookup chooses the newest retained assembly from the **exact successful main
run**, never a PR run or another commit. Tag validation fixes that artifact ID,
and the downloader treats digest mismatches as errors.

The tag workflow does not rebuild `zterm` or rerun ordinary CI. It rebuilds the
small reviewed signing tool before exposing the key, re-verifies the candidate
and source SHA, signs the exact manifest, and tests the final signed installers
on all three targets. It then creates one late draft, downloads and verifies its
assets, attests those exact bytes, publishes, and requires `immutable: true`.
There are eight native assets and no companion relay image publication.

Only **Approve and sign exact manifest bytes** uses the protected `release`
Environment. Confirm immutable Releases remain enabled, inspect the exact green
main/candidate status, and approve access to the signing key. No second signing
approval or extra full test suite is introduced.

## Failure and recovery

- Dirty preparation retains the local diff for diagnosis; it is not an automatic
  recovery checkpoint.
- Failed PR/main checks stop the operator. Fix the owning failure; no tag is
  pushed. Failed-job reruns can retain the same successful candidate.
- An expired/deleted candidate fails before a new tag. Rerun main CI for the
  exact source, then retry; publication never silently compiles a replacement.
- An interrupted tag push is reconciled against the exact annotated source.
  An existing remote tag rejoins the same `release.yml` run.
- Before draft creation, a failed release job can be rerun for the same tag.
  Signed inventories and fixture uploads use attempt-specific names; downstream
  jobs use the signing job's artifact IDs, including when only failed jobs rerun.
  After draft creation, inspect the nonpublic evidence before deciding recovery;
  automation never deletes drafts, clobbers assets, or force-moves tags.
- A defect in an immutable published Release needs a new version and tag.
- Removing Intel changes the manifest target inventory. Versions through
  0.1.17 require all four old targets and cannot update directly from a
  three-target manifest. Follow the one-time binary migration in
  [Install, update, and uninstall](install.md#one-time-migration-from-the-four-target-updater).

Repository administrators own immutable Releases, the signing Environment/key,
and [main protection](development.md#branch-and-pull-request-flow). The operator
checks visible prerequisites and never changes those settings.
