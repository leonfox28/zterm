# CI and release workflow implementation plan

> Implementation status (2026-09-01): Steps 0–9 have been implemented and the
> full local gates passed. This is the retained execution plan, not a list of
> work to repeat. Real PR/`main` timing and the first formal publication remain
> post-merge operational evidence; the current runbooks are
> `docs/development.md` and `docs/releasing.md`.

## Execution model

- Use one Trellis implementation worker for the coherent `justfile`/workflow/release-script/static-policy slice, then one independent Trellis checker.
- The main session owns task/spec convergence, review of external-state boundaries, final fixes, commit proposal and branch-protection handoff.
- Do not create a real tag, PR, GitHub Release, GHCR production image or repository-setting mutation while implementing. Operator behavior uses a task-private Git remote and fake `gh` fixture.
- Preserve unrelated worktree changes. Stop if any target file contains user edits that cannot be safely composed.

## Step 0 — Baseline and executable ownership

- [ ] Reload curated task/spec context with `trellis-before-dev`; record `git status`, current main SHA, relevant tool availability and current workflow/static-test baseline.
- [ ] Run focused current owners before editing: source/workspace/release static, relay publication/secret tests and one `detached_lifecycle` run.
- [ ] Record current `ci.yml`, `release.yml` and `relay-image.yml` action pins, job names and exact-main query so later diffs cannot silently weaken a contract.
- [ ] Confirm that no real `release` Environment secret is exposed to any local command or fixture.

## Step 1 — Canonical local commands and version preparation

- [ ] Add `justfile` recipes from the design: `doctor`, `check-fast`, `check`, CI profiles, `release-prepare` and `release-publish`.
- [ ] Keep command bodies in existing tests or small checked-in scripts; do not turn the justfile into a second large shell program.
- [ ] Make `doctor` read-only and fail/explain exact missing tools. Separate required local tools from hosted-only Docker/QEMU/native evidence.
- [ ] Add `zterm-release-tool validate-next-version` using the existing Rust `semver` dependency; require canonical text and a version strictly newer than the currently compiled workspace version.
- [ ] Refactor mutable current-version prose/tests: README/development docs stop embedding the current version, and relay publication tests derive a matching tag plus a deterministic mismatching tag from `Cargo.toml`.
- [ ] Add focused release-tool tests for stable/prerelease, same, downgrade, malformed and noncanonical versions.
- [ ] Verify `just --list`, `just doctor`, `just check-fast` and the current-version relay tests before moving workflow commands.

## Step 2 — Fix known false failures at their real owners

- [ ] Change `wait_until_stopped` to observe both absent readiness/socket and missing/unlocked daemon ownership under the existing bounded deadline; reuse `ensure_daemon_ownership_released` instead of adding sleeps or broad spawn retries.
- [ ] Update `detached_lifecycle` to construct `LocalRuntime::for_test` with the explicit harness launcher and exercise the public `restart` method.
- [ ] Run one focused lifecycle test, then a bounded macOS repeat after it passes to confirm the race no longer reproduces; do not expand timeouts merely to hide it.
- [ ] Align `tests/secret-scan.sh` with its documented input scope by excluding `.trellis`, while retaining high-confidence credential patterns on shipped/tracked inputs.
- [ ] Add the smallest fixture proving a normal Trellis `token` variable passes and representative private-key/GitHub/AWS material in a scanned input fails.
- [ ] Update the applicable local-daemon and relay/quality spec owner text with the causal fix and scan boundary.

## Step 3 — Share native readiness/formal build mechanics

- [ ] Extract `tools/release/setup-glibc-builder.sh` from the duplicated Debian 10 setup, exact workspace trust and runtime-HOME path logic.
- [ ] Extract `tools/release/build-native.sh` for host/target equality, `zterm` release build, architecture/floor inspection, formal self-check and optional candidate output.
- [ ] Keep macOS 13.0 and glibc 2.28 values in one explicit repository owner consumed by both readiness and formal builds; avoid parsing human documentation.
- [ ] In readiness mode, build only shipped `zterm` and inspect it. In formal mode, stage a uniquely named raw binary plus its identity; do not build/archive with `zterm-release-tool` in every target job.
- [ ] Update ShellCheck/static coverage for both scripts, exact non-wildcard safe.directory and runtime `$HOME` rules.
- [ ] Run the shared scripts in locally supported dry/focused modes and retain hosted native execution as CI evidence.

## Step 4 — Rewrite CI around stable recipes

- [ ] Narrow triggers to PR, `push: main` and manual dispatch; use PR-number/ref concurrency and cancellation.
- [ ] Add/pin exact just, actionlint and cache installation owners without floating action tags.
- [ ] Create the canonical policy job for workspace version, format, actionlint, release static, maintained ShellCheck and Python fixture source.
- [ ] Keep five Rust entries and source-policy immediately after every checkout; remove repeated version/format/docs.
- [ ] Retain full Unix Clippy/tests and Windows shared-boundary tests; assign docs to Linux x64 and CLI smoke to Linux x64 plus macOS arm64.
- [ ] Point release-readiness jobs at shared native scripts and keep them main-push-only.
- [ ] Retain dependency and complete relay owners; point their command bodies at just recipes and apply the secret-scan correction.
- [ ] Add per-job timeouts and platform/toolchain/lock/profile cache keys with safe cache-miss behavior.
- [ ] Add an `if: always()` aggregate `CI gate` that treats each event's required success/skipped contract explicitly and prints local reproduction recipes.
- [ ] Extend release static tests to assert trigger narrowing, stable gate, source-policy matrix ownership, single version/format/docs owners, two OS smoke owners, timeouts and full action pins.
- [ ] Run actionlint, ShellCheck, static tests and local just recipes before the broad Rust gate.

## Step 5 — Implement the two-phase release operator

- [ ] Add `tools/release/find-green-main-ci.sh COMMIT` as the exact successful completed `ci.yml` main-push query; return a single run ID/URL or fail without side effects.
- [ ] Add `tools/release/operator.sh prepare VERSION` with all read-only preflight before branch creation: clean/synced main, canonical repo/auth, vacancy and canonical newer version.
- [ ] Prepare only `Cargo.toml` and Cargo-generated `Cargo.lock`, validate the changed-file allowlist, run `just check`, then commit/push `release/vVERSION` and open a PR. Never push main/tag from prepare.
- [ ] Add `publish VERSION` with clean exact origin/main, branch-protection visibility, version/vacancy/exact-green checks before annotated tag creation and push.
- [ ] Discover and watch the exact resulting release run with a bounded poll; make an Environment approval wait explicit and print a resume command if the local watcher exits.
- [ ] Make every failure preserve its branch/diff/remote state and print targeted recovery. Forbid force push, release/tag deletion and asset replacement tokens in static policy.
- [ ] Build a task-private Git bare remote plus fake `gh` fixture covering dirty/behind/invalid/existing/preflight-failure paths, prepare PR ordering, publish exact-green ordering and one simulated success. No fixture may contact GitHub.

## Step 6 — Simplify the formal native workflow without weakening trust

- [ ] Reuse `find-green-main-ci.sh` and shared native scripts from tag validation/build jobs.
- [ ] Change four formal jobs to upload raw `zterm-$target` bytes and identity after self-check/floor inspection.
- [ ] In Ubuntu assembly, build `zterm-release-tool` once, create all four deterministic archives, prepare/verify unsigned inventory and ShellCheck the generated installer.
- [ ] Remove assembly's exact-CI-owned `tests/release/static.sh` rerun.
- [ ] Keep the signing job's independent pre-secret tool build, protected Environment and exact manifest self-verification.
- [ ] Keep per-target POSIX syntax and authenticated install/negative fixtures, but remove the standalone repeated Python `py_compile` because each fixture executes that source and policy already compiles it once.
- [ ] Keep signed re-verification, late draft, upload/download round-trip, attestation, publication and immutable API assertion.
- [ ] Rename stages and write summaries so build time, approval wait and final verification are visible to the operator.
- [ ] Update release static/tool tests for raw-candidate inventory, centralized archive owner and absence of ordinary test-suite commands after the tag.

## Step 7 — Make relay publication explicit

- [ ] Add `workflow_call` inputs for exact commit, exact tag and prerelease classification to `relay-image.yml`; retain manual dev dispatch.
- [ ] Remove formal dependence on `release: published` and update `resolve-publication.sh` to distinguish trusted formal call from manual input without duplicating Cargo SemVer ownership.
- [ ] Add a job-level reusable workflow call after native immutable publication with only `contents: read` and `packages: write`.
- [ ] Add a final release summary owner that reports native Release URL, relay image channel/tag and precise retry guidance.
- [ ] Extend relay publication/static tests for stable version+latest, prerelease dev-only, manual dev-only, exact frozen checkout and absence of the unreliable implicit trigger.
- [ ] Confirm a relay failure remains visible/red but cannot edit or delete the native immutable Release.

## Step 8 — Documentation, specs and external handoff

- [ ] Rewrite the quality-gate section of `docs/development.md` around `just doctor`, `just check-fast`, `just check`, PR CI and hosted-only evidence.
- [ ] Add `docs/releasing.md` with prepare/publish commands, state diagram, expected job meanings/timing, approval instructions, native/relay completion and each failure recovery boundary.
- [ ] Shorten README release/CI text to links plus the three main commands; update install/relay docs where workflow ownership changed.
- [ ] Update distribution-lifecycle, relay-deployment, local-daemon and cross-platform Trellis specs with centralized archive, explicit relay call, restart ownership and retained per-OS source-policy owners.
- [ ] Add a one-time `main` protection checklist: PR required, stable `CI gate` required, direct/force pushes and deletion disabled, zero mandatory outside approvals for the solo maintainer.
- [ ] State that the settings checklist is applied only after the workflow change lands; do not call the repository administration API to mutate it during implementation.

## Step 9 — Verification and independent check

- [ ] Run `git diff --check` and inspect every workflow permission, trigger, condition, action SHA, environment and external-state command.
- [ ] Run actionlint over all workflows and ShellCheck/sh syntax over all maintained/new shell files.
- [ ] Run focused commands:
  - `cargo +1.98.0 test -p zterm-release-tool`
  - `cargo +1.98.0 test -p zterm-daemon --test detached_lifecycle`
  - bounded repeat of the focused lifecycle test after the causal fix
  - `sh tests/source-policy.sh`
  - `sh tests/workspace-version.sh`
  - `sh tests/release/static.sh`
  - `sh tests/release/operator-fixture.sh`
  - `sh tests/relay/publication-channels.sh`
  - `sh tests/relay/static.sh`
  - `sh tests/secret-scan.sh`
- [ ] Run `just check`; then run the broad workspace format, Clippy, tests, docs and both cargo-deny owners once if any are not already exact recipe members.
- [ ] Run the relay Docker/QEMU build/runtime bundle when the local engine is available; otherwise record the exact hosted CI owner without adding a substitute.
- [ ] Dispatch one independent Trellis checker with `check.jsonl`, PRD/design and the diff. Fix concrete findings, rerun affected gates, then one final broad gate.
- [ ] Produce a before/after job/check ownership table and note that real PR/main timing and formal relay publication remain post-merge operational evidence.

## Acceptance mapping

| Steps | Acceptance criteria |
| --- | --- |
| 1 | AC1, AC2 |
| 2 | AC6, AC7 |
| 3–4 | AC3, AC4, AC5, AC11 |
| 5 | AC8, AC9 |
| 6 | AC10, AC11, AC13 |
| 7 | AC12, AC13 |
| 8 | AC1, AC15 |
| 9 | AC14 and final verification of all criteria |

## Rollback points and stop conditions

- Commit shared scripts/recipes conceptually before workflow rewiring so a failing YAML migration can be reverted without losing the local command owner.
- Keep the restart fix independently testable; do not quarantine/remove the test if the ownership wait fails.
- A failed cache or tool installer must be removable without changing test coverage.
- Operator fixture success is required before any script contains real push/PR/tag commands.
- Formal build restructuring must produce byte-valid archives and identities before removing old per-host archive commands.
- Relay reusable-call tests must pass before deleting the implicit event trigger.
- Stop after two materially different fixes or 90 minutes on a harness-only flake; preserve the product ownership contract and record a focused blocker rather than adding sleeps/retries.
- At four active implementation hours, report completed scope and largest risk; at eight hours, stop for explicit continuation approval.
- No production external state is an acceptance shortcut. The first real PR/main/release usage is a documented follow-up observation, not authorization to publish during this task.
