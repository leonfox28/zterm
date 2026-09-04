# CI and release workflow optimization design

> Implementation status (2026-09-01): this target design is implemented and
> locally verified. Use `docs/development.md` and `docs/releasing.md` for
> current operation; the sections below retain design rationale and boundaries.

## 1. Outcome

The optimized system has three visible maintainer commands and three distinct evidence boundaries:

```text
just check
  -> branch + PR
  -> required CI gate
  -> human merge
  -> exact-SHA main CI + release readiness

just release-prepare VERSION
  -> release branch/commit
  -> release PR
  -> human merge
  -> exact-SHA main CI + release readiness

just release-publish VERSION
  -> annotated tag
  -> frozen native builds
  -> assemble + protected sign
  -> final installer proof
  -> immutable native Release
  -> explicit relay image publication
```

Local checks prevent portable mistakes, PR CI prevents a red integration branch, main CI owns the exact commit eligible for release, and the tag workflow proves only facts that require final release bytes. No boundary claims evidence owned by another OS, architecture, protected secret, GitHub Release, or OCI registry.

## 2. Design principles

1. **One command owner:** `justfile` recipes and small repository scripts own commands; YAML owns triggers, permissions, runners, matrices, caches, concurrency and job dependencies.
2. **One invariant owner:** version/format/docs are not repeated per platform. Checkout bytes, OS-specific compilation/tests, final signature and uploaded-byte checks remain at their distinct boundaries.
3. **Green before irreversible state:** release preparation stops at a PR. Publish verifies the exact remote main SHA before creating a tag. The workflow creates public native state only after final bytes are signed and exercised.
4. **No silent skips:** missing local tools fail with an installation hint; hosted-only evidence is named explicitly; a skipped required CI dependency fails the aggregate gate unless the event contract permits it.
5. **Conservative first pass:** keep the native test matrix and Cargo test runner. Optimize duplicated work, cache, timeouts, known races and false positives before reducing platform evidence.
6. **No false atomicity:** native GitHub Release and GHCR cannot commit atomically. The native Release publishes first; relay publication is an explicit, visible, retryable dependent job.

## 3. Maintainer command surface

| Recipe | Owner and behavior |
| --- | --- |
| `just doctor` | Read-only prerequisite report for exact Rust, just, ShellCheck, actionlint, cargo-deny, gh, jq and Docker-related optional evidence. It prints exact install guidance and never installs global tools. |
| `just check-fast` | Source/version/release policy, secret scan, format and native Clippy owners for the edit loop. |
| `just check` | Required pre-push local gate for ordinary substantive changes: fast checks, native workspace tests/docs, dependency policy and locally reproducible relay contracts. The deterministic two-file release commit instead uses focused version/lock validation followed by required PR CI. It reports the remaining hosted OS/architecture and Docker/QEMU evidence. |
| `just ci-policy` | Canonical version, format, workflow/static release policy, maintained ShellCheck, Python fixture source and actionlint. |
| `just ci-unix` | Full Unix Clippy and workspace tests; matrix inputs decide the two OS-family CLI smoke owners and the one docs owner. |
| `just ci-windows` | Hosted Windows shared/unsupported-boundary Clippy and shared-contract tests. |
| `just ci-dependencies` | Workspace and isolated relay-probe cargo-deny checks. |
| `just ci-relay` | Relay probe lint, shell/static/publication/upstream/image/runtime/secret contracts on its Docker-capable Ubuntu owner. |
| `just release-prepare VERSION` | Creates and opens a reviewable release PR; never creates a tag or Release. |
| `just release-publish VERSION` | Revalidates protected exact-green main, pushes one annotated tag and watches the resulting release. |

Recipes use the pinned `rust-toolchain.toml`/explicit Cargo version already required by the project. CI installs an exact pinned just version; local documentation gives macOS and cargo-based installation options. `doctor` does not make `check` silently pass when a required tool is missing.

## 4. CI trigger and job graph

### Triggers and concurrency

`.github/workflows/ci.yml` becomes:

```yaml
on:
  pull_request:
  push:
    branches: [main]
  workflow_dispatch:
```

The concurrency key uses PR number when present and otherwise the ref. A new PR head cancels the old run. A branch without a PR intentionally has no hosted run; `just check` is its local owner. No path classifier or `paths-ignore` is introduced in the first pass, avoiding missing required statuses and exact-main ambiguity.

### Target jobs

| Job | Events | Required evidence |
| --- | --- | --- |
| `Policy and portable checks` | PR/main/manual | one workspace version, one format, actionlint, release static policy, maintained ShellCheck, HTTPS fixture source |
| `Rust (macOS arm64)` | PR/main/manual | source-policy, full native Clippy/tests, macOS CLI smoke |
| `Rust (macOS Intel)` | PR/main/manual | source-policy, full native Clippy/tests |
| `Rust (Linux x64)` | PR/main/manual | source-policy, full native Clippy/tests, single docs owner, Linux CLI smoke |
| `Rust (Linux arm64)` | PR/main/manual | source-policy, full native Clippy/tests |
| `Rust (Windows)` | PR/main/manual | source-policy, Windows compile/Clippy and shared-contract tests |
| four `Release readiness` entries | main push only | exact native target, architecture and macOS/glibc floor from shared build scripts |
| `Dependency policy` | PR/main/manual | both cargo-deny owners |
| `Official relay bundle` | PR/main/manual | existing complete relay acceptance bundle with corrected secret scope |
| `CI gate` | always | stable aggregate result for branch protection and human-readable summary |

`CI gate` accepts the four readiness jobs as skipped on PR/manual only when their event condition requires that result. On `main`, any skipped/cancelled/failed required owner fails the gate and the overall workflow. The release validator continues to require a successful completed `ci.yml` push run on `main` for the exact SHA, which therefore includes all four readiness results.

### Deduplication

- Workspace version and format move out of the five Rust entries into policy.
- Docs remain only on Linux x64.
- CLI no-argument smoke remains on Linux x64 and macOS arm64, one per shipped OS family.
- Source checkout policy remains first in every Rust matrix entry. It is intentionally repeated because each working tree is host evidence and the project spec requires it.
- Full Unix Clippy/tests and Windows tests remain unchanged in the first pass.
- Release-readiness and formal release call the same checked-in native build/floor scripts, but each still builds at its own evidence boundary.

### Cache and timeout policy

- Use a commit-pinned Rust cache action keyed by Rust 1.98, OS/architecture, Cargo.lock and job profile. Do not cache installed arbitrary binaries through an unbounded shared key.
- Cache the exact cargo-deny 0.20.2 binary separately or install it on cache miss, with version in the key.
- CI may cache Cargo target artifacts. Formal release jobs may cache Cargo registry/git downloads but do not promote CI outputs or treat a cached ordinary binary as the release candidate.
- Give each job a measured timeout with headroom: short policy jobs, medium Rust/dependency jobs, and longer Docker/glibc builders. Queue time is documented separately from job execution.
- All actions and container images remain full-SHA/digest pinned and are checked by release static policy/actionlint.

## 5. Shared workflow scripts

The implementation adds small scripts rather than another general-purpose CI framework:

- `tools/release/build-native.sh` owns exact host/target checks, `zterm` release build, architecture/floor inspection, optional formal self-check and candidate staging. CI readiness and tag release pass explicit mode/output inputs.
- `tools/release/setup-glibc-builder.sh` owns the frozen Debian 10 package snapshot, required packages, runtime-HOME Cargo path and exact `safe.directory` preparation shared by CI and release Linux jobs.
- `tools/release/find-green-main-ci.sh COMMIT` is the one GitHub API query for a successful completed `ci.yml` main push run. Both local publish preflight and tag validation use it.
- `tools/release/operator.sh prepare|publish VERSION` owns local Git/gh state transitions. `justfile` is the small public wrapper.

Each shell script uses POSIX shell unless a Bash-only feature is required and named. Maintained script lists in ShellCheck/static tests are updated explicitly.

## 6. Known reliability fixes

### Restart ownership race

Current stop/restart waits return when readiness disappears, while the old daemon can still hold `daemon.lock` after removing its socket. A new child can then exit with `DaemonAlreadyRunning`, projected as `DaemonStartTimeout`.

The fix changes the bounded restart wait to require both:

1. readiness/socket is absent; and
2. `inspect_existing_lock(daemon.lock)` is missing or unlocked.

The existing `ensure_daemon_ownership_released` helper becomes part of that wait rather than adding a sleep or retrying broad startup. `detached_lifecycle` constructs `LocalRuntime::for_test` with the explicit harness launcher and exercises `LocalRuntime::restart`, so the hosted regression proves the public owner. A focused bounded repeat on macOS confirms the causal fix after one normal pass.

### Secret scan scope

The scan continues to catch high-confidence private key, SSH, AWS and GitHub credential patterns. It excludes `.trellis` because Trellis planning/runtime data is not a shipped source/deployment input, matching the existing comment and observed false positive. A fixture proves ordinary `token` variable source under `.trellis` passes while representative credential material in a shipped input fails.

## 7. Release preparation and publication

### `release-prepare`

The command performs all read-only checks before creating a branch:

1. verify canonical repository/maintainer gh context, clean worktree, current `main`, fetched tags and exact equality with `origin/main`;
2. reject an existing release branch, local/remote tag or GitHub Release;
3. use the Rust `semver` owner in `zterm-release-tool validate-next-version` to require canonical strictly newer input;
4. create `release/vVERSION`, update only `[workspace.package].version`, run `cargo +1.98.0 update --workspace`, then validate with `cargo +1.98.0 metadata --locked --format-version 1 --no-deps`, `tests/workspace-version.sh`, and the exact changed-file inventory;
5. do not repeat `just check`: the release PR's required CI is the complete format/Clippy/test/docs/dependency owner, while prepare owns only deterministic version/lock generation and focused validation;
6. commit `chore: prepare vVERSION release`, push only the release branch and open a PR targeting `main`;
7. print the PR URL and state that publication is blocked until human merge and exact main CI success.

README/development prose stops embedding a mutable current version. Relay publication tests derive the workspace version and construct their matching/mismatching tags dynamically. That makes Cargo.toml/Cargo.lock the normal release-PR diff rather than a recurring list of manual text replacements.

If generation or focused validation fails after the local branch is created, the command leaves the dirty branch and diff for inspection and prints recovery steps. It never automatically repairs/deletes partial work or pushes a tag.

If the exact release commit was created but branch push or PR creation returned an ambiguous network failure, rerunning the same command from that clean `release/vVERSION` branch enters a bounded resume path. The operator requires one exact commit directly on current `origin/main`, the expected subject, requested workspace version, locked Cargo validity, and an exact `Cargo.toml`/`Cargo.lock` diff. It may then reuse only a remote branch at the same SHA and one open PR for that same head/base, or create the missing remote branch/PR. A divergent remote, closed/merged/ambiguous PR, extra commit/file, dirty tree, or changed main fails closed. This is not a generic release state machine.

### `release-publish`

The command is intentionally separate and performs:

1. clean `main`, fetch, exact local/remote equality and version/tag canonicality;
2. branch-protection visibility check with an actionable failure if protection cannot be confirmed;
3. local/remote tag and GitHub Release vacancy;
4. exact commit lookup through `find-green-main-ci.sh` and a successful main run URL;
5. one annotated `vVERSION` tag and one tag push;
6. bounded discovery of the resulting `release.yml` run, then `gh run watch --exit-status` or an exact printed resume command and reviewer-wait explanation.

No command force-pushes main/tag, deletes a failed Release, replaces assets or silently retries with a different commit.

## 8. Tag workflow

The formal graph becomes:

```text
validate exact tag/version/main-CI/vacancy
  -> build zterm on four native targets
  -> assemble raw binaries into deterministic archives
     + manifest/checksums/SBOM
     + verify unsigned
     + generated-installer ShellCheck
  -> protected approval + build signer + sign/self-verify
  -> four final signed installer jobs
  -> late draft + upload/download verify + attest + immutable publish
  -> reusable relay image publish for the same commit/tag/channel
  -> release-complete summary
```

The four build jobs compile only the shipped `zterm` package, execute its formal self-check, inspect target/floor and upload a uniquely named raw binary plus identity. Ubuntu assembly builds `zterm-release-tool` once and creates all four archives. The signing job still builds the reviewed tool before the secret-bearing step, and publish still independently builds/verifies at the external-state boundary.

Removed tag-time duplication:

- `tests/release/static.sh` from assembly, because exact green CI already owns source/workflow policy;
- independent `python3 -m py_compile` in all four installer entries, because policy checks source once and the actual fixture executes the Python owner on every host.

Retained release-specific proof:

- one generated-installer ShellCheck after generation;
- per-host POSIX syntax, because different shipped host shells remain evidence;
- per-target happy install, existing-destination and digest negative cases;
- unsigned, signed and uploaded round-trip verification, each at a different trust boundary;
- protected signing approval, provenance and immutable response assertion.

## 9. Explicit relay integration

`.github/workflows/relay-image.yml` drops the unreliable formal `release: published` edge and supports:

- `workflow_call` with required frozen commit, exact release tag and prerelease boolean for formal releases;
- `workflow_dispatch` with the existing manual development tag semantics.

The reusable workflow checks out the supplied frozen commit and still delegates stable/prerelease/manual channel resolution to `deploy/relay/resolve-publication.sh`. The caller grants only `contents: read` and `packages: write` to the reusable job.

Native publication precedes relay publication. If GHCR fails, the native immutable Release remains correct, the overall release run is red at a named relay job, and the same relay job can be retried. Documentation states this cross-service recovery boundary without adding staging registries, rollback automation or a second signature format.

## 10. Documentation and repository settings

- `docs/development.md` becomes the CI/push authority: install tools, `just` recipes, PR flow, job map, hosted-only boundaries and reproduction commands.
- `docs/releasing.md` becomes the operator authority: prepare PR, merge/main evidence, publish/tag, stage meanings, approval wait, native/relay result and recovery.
- README links those documents and contains only the short daily/release commands.
- Distribution, relay and cross-platform Trellis specs are updated only where owners move; existing security and platform contracts remain intact.
- Branch protection is a one-time administrator action after the new workflow is merged: require PRs, require the stable `CI gate`, disallow force pushes/deletion/direct pushes, and require no second reviewer for a solo-maintainer repository. The implementation documents but does not mutate this external setting.

## 11. Validation strategy

Validation is layered and non-destructive:

1. actionlint plus ShellCheck/sh syntax for every modified workflow/script;
2. focused release-tool/version, publication-channel, static policy, secret-scan and restart tests;
3. task-private Git remote and fake GitHub CLI fixture for prepare/publish ordering and no-side-effect failures;
4. `just check` and the broad workspace format/Clippy/tests/docs/deny gates once after focused fixes;
5. Docker relay bundle where locally available, with hosted CI retaining final QEMU/native evidence;
6. independent Trellis checker against PRD/specs and one final affected/broad rerun.

No real tag, GitHub Release, production image, branch-protection mutation or release secret is used during implementation.

## 12. File ownership map

Expected new files:

- `justfile`
- `tools/release/operator.sh`
- `tools/release/build-native.sh`
- `tools/release/setup-glibc-builder.sh`
- `tools/release/find-green-main-ci.sh`
- `tests/release/operator-fixture.sh`
- `docs/releasing.md`

Expected primary modifications:

- `.github/workflows/ci.yml`
- `.github/workflows/release.yml`
- `.github/workflows/relay-image.yml`
- `tools/release/src/main.rs` and release-tool tests for canonical next-version validation
- `crates/daemon/src/operations.rs`
- `crates/daemon/tests/detached_lifecycle.rs`
- `tests/secret-scan.sh`, `tests/release/static.sh`, relay publication/static tests
- `README.md`, `docs/development.md`, `docs/install.md`, `docs/relay.md`
- applicable `.trellis/spec/backend/*` and `.trellis/spec/guides/*` owner text

One implementation worker owns the coherent workflow/script slice, followed by one independent checker. Splitting CI and release workers would create overlapping ownership in `justfile`, release static policy and shared native scripts, so separate child tasks are not used.

## 13. Rollback and recovery

- Before merge, all repository changes are an ordinary PR and can be reverted without external release state.
- Cache failures fall back to install/build; removing cache steps does not change evidence contracts.
- A failed `release-prepare` preserves a local/remote release branch or PR for inspection and creates no tag.
- A failed `release-publish` before tag push creates no release state. After tag push, the immutable correction path is a new version/tag; assets are never replaced.
- A native-success/relay-failure state keeps the correct native Release and retries only explicit relay publication for the same frozen source/channel.
- Branch protection changes are administrator-owned and independently reversible; bypass is not embedded in repository scripts.

## 14. Deferred decisions

After 20–30 real PR/main runs, compare critical path, execution time, cache hit rate, flaky failures and unique platform catches. Only then consider nextest, path classification, or changing Linux ARM/macOS Intel from full tests to compile-only. Preview channels, docs snapshot automation, issue closing and stronger OCI immutability remain separate product/operations decisions.

## 15. 2026-09-04 prepare reliability correction

The repeated first-attempt failures in v0.1.10–v0.1.14 came from a local implementation/test mismatch: production invoked `cargo metadata --no-deps` as though it generated lockfile changes, while the fixture silently rewrote `Cargo.lock` on that command. Real Cargo does not provide that side effect.

The correction keeps the existing two-phase architecture and narrows each owner:

- Cargo explicitly generates the lock change with `update --workspace`.
- Locked metadata, workspace-version policy, and exact inventory validate the generated state.
- The private fixture delegates those commands to the pinned real Cargo; it only substitutes the repository-specific SemVer tool and GitHub API.
- Release PR CI is the sole full quality gate for the generated commit; local prepare does not duplicate it immediately before push.
- A small resume transition exists only after the exact clean commit boundary. Partial generation remains deliberately manual, and `release-publish` is unchanged.

This avoids a custom lockfile parser, a persistent operator-state schema, a temporary production worktree, or broad reconciliation rules that would add more failure states than the observed problem warrants.
