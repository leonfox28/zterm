# Release operations

The maintainer interface is deliberately split at the irreversible boundary:

```text
just release-prepare VERSION
  -> release/vVERSION branch -> release PR -> human merge
  -> exact-SHA main CI and four native readiness builds

just release-publish VERSION
  -> one annotated vVERSION tag
  -> four frozen native builds -> assemble -> protected sign
  -> four final installer proofs -> immutable native Release
  -> explicit multi-architecture relay image publication
```

`VERSION` is canonical SemVer without the `v` prefix or build metadata. A
prerelease such as `0.2.0-rc.1` is supported. Run `just doctor` first; neither
operator command installs global tools or changes repository settings.

## Prepare the release pull request

Start from a clean local `main` that exactly matches `origin/main`:

```bash
just release-prepare 0.2.0
```

Before creating a branch, the command verifies GitHub authentication and the
canonical repository, branch/tag/Release vacancy, and a strictly newer version
through the Rust `semver` owner. It then creates `release/v0.2.0`, changes only
the workspace version in `Cargo.toml` plus Cargo-generated `Cargo.lock`, runs
`just check`, commits, pushes only the release branch, and opens a PR.

If a check fails, no tag or public release state exists. The local release
branch and diff are intentionally retained for diagnosis. Fix the branch, run
`just check`, and push/open the PR manually; the operator never force-pushes or
deletes the failure evidence.

Merge the PR only after `CI gate` is green. Then wait for the new `main` push
run to succeed. That exact run, including all four native readiness builds, is
the source eligibility record consumed by publication.

## Publish the frozen version

Update local `main` to the exact remote commit and run:

```bash
just release-publish 0.2.0
```

Before creating a tag, the command verifies a clean exact `origin/main`, the
Cargo version, readable/enforced `main` protection, local/remote tag and GitHub
Release vacancy, and one successful completed `ci.yml` main-push run for the
exact commit. It creates one annotated `v0.2.0` tag, pushes that tag once,
discovers the corresponding `release.yml` run and watches it to completion.

The watcher can appear idle at **Approve and sign exact manifest bytes**. This
is the protected `release` Environment waiting for its reviewer; it is not a
second test suite. Confirm immutable Releases remain enabled, inspect the
validated source/build status, and approve access to the signing key. If the
local watcher exits, use the exact `gh run watch ... --exit-status` resume
command printed by the operator.

## What the tag workflow does

The tag workflow intentionally does not rerun formatting, Clippy, workspace
tests, docs, cargo-deny, or the relay test bundle. Exact green `main` already
owns those facts. It performs only release-boundary work:

1. validate the annotated tag, Cargo version, vacancy, and exact green main CI;
2. build only the shipped `zterm` binary on macOS arm64/Intel and glibc 2.28
   Linux arm64/x64, checking architecture and deployment floor;
3. build the private release tool once on Ubuntu, create deterministic
   archives, manifest, checksums, installer and SBOM, verify the unsigned
   inventory, and ShellCheck the generated installer;
4. wait for the protected Environment, build the reviewed signer before secret
   exposure, sign the exact manifest bytes and self-verify;
5. execute POSIX syntax plus authenticated local-HTTPS install and negative
   cases against every final signed target;
6. create a late draft, download and verify its exact bytes, attest them,
   publish, and require the API response to report `immutable: true`;
7. explicitly call the reusable relay workflow for the same frozen commit,
   tag and stable/prerelease classification.

The most recent measured run before this refactor spent roughly seven minutes
building/assembling, eight minutes awaiting human approval, and under two
minutes signing, testing final installers and publishing. Cache hits can reduce
downloads, but formal binaries are always rebuilt from the frozen tag and are
never promoted from ordinary CI artifacts.

## Failure and recovery boundaries

- Before the tag push, fix the precondition and rerun. If annotated tag
  creation succeeded locally but its push failed, first prove the remote tag is
  absent, inspect the retained tag, and push that exact tag manually.
- Before draft creation, a workflow failure has no GitHub Release state. Rerun
  the failed job only for the same immutable tag/SHA.
- A failure after draft creation leaves nonpublic evidence for explicit
  inspection. Automation never deletes a Release, replaces assets, uses
  `--clobber`, or force-moves a tag.
- After immutable native publication, a native defect requires a new version
  and tag. Published assets are never repaired in place.
- GitHub Release and GHCR are separate services, so publication is not atomic.
  If native publication succeeds and relay publication fails, keep the correct
  immutable native Release and rerun only the relay job for that same frozen
  commit/tag with
  `gh run rerun <release-run-id> --failed --repo leonfox28/zterm`. In this state
  the native publish dependency is already successful, so the failed-job rerun
  targets relay publication and its summary. Do not recreate or replace the
  native Release.

Repository administrators separately own immutable Releases, the protected
`release` Environment/signing secret, and the `main` protection checklist in
[Development and CI](development.md). The operator checks visible
preconditions but never calls an administration API to change them.
