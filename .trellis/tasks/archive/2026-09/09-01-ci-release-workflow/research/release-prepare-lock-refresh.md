# Release prepare lockfile refresh incident

## Observed evidence

- Real release preparations v0.1.10 through v0.1.14 repeatedly stopped after
  changing `Cargo.toml`: the exact inventory contained only the manifest, not
  `Cargo.lock`.
- `tools/release/operator.sh` invoked
  `cargo +1.98.0 metadata --format-version 1 --no-deps` after the manifest edit
  and expected it to refresh workspace package versions in the lockfile.
- Real Cargo 1.98 does not make that update. The successful manual recovery used
  `cargo update --workspace` (or broader metadata without the no-deps path),
  after which the normal two-file release commit worked.
- `tests/release/operator-fixture.sh` replaced Cargo metadata with an `awk`
  rewrite of every lockfile package version. The test therefore asserted a
  side effect that production Cargo did not have and could not expose the bug.
- GitHub history showed one successful signed workflow per released version;
  the repeated work was local prepare recovery, not duplicate tag workflows.

## Root-cause classification

This is a local implementation defect plus an evidence mismatch, not a missing
release architecture. The existing architecture already requires Cargo-owned
lockfile generation, exact two-file inventory, release PR CI, exact-green main,
and immutable publish. The operator selected the wrong Cargo primitive and the
fixture over-mocked that primitive.

## Correct owner boundaries

1. `cargo +1.98.0 update --workspace` owns lockfile generation after the single
   workspace version edit.
2. `cargo +1.98.0 metadata --locked --format-version 1 --no-deps`,
   `tests/workspace-version.sh`, and exact inventory own validation only.
3. The release PR CI owns the complete format/Clippy/tests/docs/dependency gate;
   prepare does not rerun that full suite just before pushing the PR.
4. The fixture must delegate Cargo lock/update/metadata behavior to pinned real
   Cargo. A fake may still isolate the repository-specific SemVer command and
   GitHub API, but must not invent Cargo filesystem effects.
5. Network ambiguity is recoverable only past the exact clean-commit boundary:
   same branch, direct current-main parent, exact subject/version/two-file diff,
   same remote SHA, and same open PR head/base. Every divergence fails closed.

## Rejected expansion

- No generic release state machine or persistent state file.
- No custom `Cargo.lock` parser or hand-written lockfile rewrite.
- No temporary production worktree solely to hide a partial prepare failure.
- No compatibility path for arbitrary dirty/old release branches.
- No rewrite of publish/tag/signing/immutable release semantics.

These additions would create new state and recovery combinations without
addressing the observed cause. A focused Cargo fix, truthful integration test,
clear diagnostics, and one exact-commit resume edge are sufficient.

## Break-loop analysis (2026-09-04)

### 1. Root-cause category

- **Primary: D — Test coverage gap.** The operator fixture replaced a real
  package-manager boundary with a fake that wrote the desired lockfile. It
  therefore proved the mock, not the production Cargo command.
- **Contributing: E — Implicit assumption.** The implementation assumed
  `cargo metadata --no-deps` refreshed workspace package versions even though
  that behavior was neither specified nor checked against pinned Cargo 1.98.
- **Not classified as an architecture gap.** The two-phase PR/main/tag design,
  exact inventory, and immutable publication boundaries were already correct.

### 2. Why earlier recoveries did not fix it

1. Each affected release was recovered operationally by refreshing the lock
   and rerunning the process. That completed the immediate release but left the
   wrong command and false fixture unchanged.
2. The exact inventory correctly stopped a one-file commit, so weakening it
   would only have hidden the upstream generation defect.
3. Reframing the symptom as a general retry/state-machine problem would not
   make Cargo generate the lockfile and would add unrelated states to test.

### 3. Prevention mechanisms

| Priority | Mechanism | Specific action | Status |
| --- | --- | --- | --- |
| P0 | Real-boundary integration test | Delegate `pkgid`, `update`, and locked `metadata` to pinned Cargo 1.98 in the private fixture | Done |
| P0 | Explicit command ownership | Use `cargo update --workspace` for generation and locked metadata only for validation | Done |
| P0 | Fail-closed evidence | Retain exact two-file inventory and print expected/actual sets | Done |
| P1 | Executable specification | Record the command sequence, failure matrix, and exact-commit resume boundary in `distribution-lifecycle.md` | Done |
| P1 | Scope discipline | Keep publish/CI/signing unchanged and reject a generic durable operator state model | Done |

### 4. Systematic expansion

- Similar risk exists in fixtures that fake a third-party tool and also create
  files which production expects that tool to create. Future reviews should
  keep protocol/network isolation fakes, but exercise pinned local build/package
  tools directly when their filesystem effects are the behavior under test.
- This does not justify replacing all test doubles. GitHub remains fake because
  the contract under test is local ordering/identity and tests must not mutate
  external state.
- Confidence is high: real Cargo reproduced the missing side effect distinction,
  the revised fixture fails without explicit update, and production release
  history showed no duplicate tag-workflow architecture failure.

### 5. Knowledge capture

- [x] Updated `.trellis/spec/backend/distribution-lifecycle.md` with the
  executable prepare/resume contract and required real-Cargo assertions.
- [x] Added this incident record to both implementation and checker contexts.
- [x] Replaced the false fake behavior and added the regression fixture.
- [x] Confirmed this product repository has no `src/templates/markdown/spec/`
  mirror to synchronize; project-owned `.trellis/spec/` is authoritative.
