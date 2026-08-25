# Research: release workflow comparison

- Query: 用户提出“push/merge 到主干后运行一次全方位 CI；测试和构建全部通过后再打 tag 并发布”，并希望对照 `herdr` 及 2–3 个成熟 Rust CLI 项目的真实发布流程。需要比较 PR/main CI 触发、main 是否构建 release-mode、tag 由谁创建、tag/release 触发、正式产物是重建还是复用、签名/来源证明、人工审批、draft/直接发布以及失败重试边界。
- Scope: mixed（本仓库 + 仅限上游仓库/官方文档的外部只读研究）
- Date: 2026-08-25

## Findings

### 1. 结论先行

可以，而且这正是 zterm 当前已经批准并写入代码的主路径：

```text
PR merge / push exact commit to main
  -> wait for that commit's main push CI to finish successfully
     (tests, lint, docs, dependency policy, four native release-mode builds)
  -> human creates and pushes exact v<Cargo version> tag
  -> tag workflow independently revalidates the green main CI run
  -> rebuild formal artifacts from the frozen tagged commit
  -> protected-environment approval unlocks manifest signing
  -> signed installer matrix passes
  -> create late draft, round-trip verify, attest
  -> publish and require immutable=true
```

The important distinction is that the release-mode builds in `main` CI are a
**readiness gate**, not the distributable bytes. The formal assets should still
be rebuilt from the exact tag. Herdr, ripgrep, uv, and just all rebuild their
formal release artifacts in the release workflow rather than promoting ordinary
main-CI binaries. For zterm this duplication is intentional: only the tag build
has the frozen source identity, release classification, signed manifest, final
inventory, installer evidence, and provenance context.

No workflow/design change is needed to adopt the user's proposed ordering. The
operational rule should be stated precisely as: **wait for the successful
`push` run of `ci.yml` on `main` for the exact SHA**, not merely a green PR run
or a green run for a nearby commit.

### 2. Which project does `herdr` mean?

Confidence: **very high (about 99%)** that the intended project is
[`herdrdev/herdr`](https://github.com/herdrdev/herdr).

Evidence:

- Its repository description and official site identify it as the Rust terminal
  agent runtime relevant to zterm, and its own maintainer instructions explicitly
  call `herdrdev/herdr` the canonical repository
  ([AGENTS.md:12-17](https://github.com/herdrdev/herdr/blob/6e8b138d0f7d7d695530657a6d8dc475bd3fba2b/AGENTS.md#L12-L17)).
- The former official-looking path `ogulcancelik/herdr` resolves through the
  GitHub API to `herdrdev/herdr` as of the research date.
- Exact/near-name search also found
  [`thinkerisme/herdr`](https://github.com/thinkerisme/herdr) (a separate stale
  copy with no official homepage),
  [`motionharvest/herdr`](https://github.com/motionharvest/herdr) (a separate
  downstream), and
  [`re2zero/herdrzero`](https://github.com/re2zero/herdrzero) plus
  [`2lab-ai/herdr-mx`](https://github.com/2lab-ai/herdr-mx) (downstreams/forks).
  None has the canonical-repository declaration and `herdr.dev` ownership signal
  of `herdrdev/herdr`.

The Herdr comparison below therefore uses the stable release path in
`herdrdev/herdr`, not its preview channel or a downstream fork.

### 3. Comparative matrix

| Project | PR/main CI and release-mode boundary | Tag authority and release trigger | Formal artifact origin | Signing / provenance | Human approval and publish shape | Retry / failure behavior visible in code |
| --- | --- | --- | --- | --- | --- | --- |
| **zterm (current approved flow)** | `ci.yml` runs for pushes, PRs, and manual dispatch; full Rust checks span macOS arm64/Intel, Linux arm64/x64, and Windows. Four release-mode jobs run only for a `push` to `main` and inspect macOS/glibc floors (`.github/workflows/ci.yml:3-15,35-80,82-172`). | CI never creates a tag. The contract requires a human to push exact `v` + Cargo version only after the exact main run is green (`.trellis/spec/backend/distribution-lifecycle.md:116-124`). A `v*` tag triggers release, whose first job proves exact version/tag and queries for a successful `ci.yml`, `event=push`, `head_branch=main`, same `head_sha` (`.github/workflows/release.yml:3-18,41-69`). | Rebuilds all four formal binaries from the frozen tagged SHA; ordinary CI artifacts are not reused (`.github/workflows/release.yml:84-210`). | Detached Ed25519 signature authenticates exact manifest bytes; only the `sign` job uses the protected `release` Environment and secret. Final round-tripped assets receive GitHub build-provenance attestations (`.github/workflows/release.yml:255-307,423-436`). | Human tag is the release decision; a protected Environment approval separately gates access to the signing seed. Installer tests finish before a late draft is created. The draft is downloaded, verified, attested, then automatically published and checked for `immutable: true` (`.github/workflows/release.yml:309-451`). | Failures before draft create no Release. Once any Release exists, validation/publish refuses replacement. Failure after draft intentionally leaves it unpublished; there is no automatic delete/overwrite/retry loop (`.trellis/tasks/08-24-distribution-release/design.md:154-166`, `.github/workflows/release.yml:54-57,391-451`). |
| **Herdr stable** | CI runs for PR open/sync/reopen and pushes to `master`/`windows`; the Unix path runs formatting, Clippy, and debug-profile nextest, not the formal release build ([ci.yml:3-19](https://github.com/herdrdev/herdr/blob/6e8b138d0f7d7d695530657a6d8dc475bd3fba2b/.github/workflows/ci.yml#L3-L19), [ci.yml:43-143](https://github.com/herdrdev/herdr/blob/6e8b138d0f7d7d695530657a6d8dc475bd3fba2b/.github/workflows/ci.yml#L43-L143), [justfile:19-36](https://github.com/herdrdev/herdr/blob/6e8b138d0f7d7d695530657a6d8dc475bd3fba2b/justfile#L19-L36)). | A maintainer runs `just release`; local recipes prepare/check a release commit, push it to `master`, create an annotated tag, then push the tag ([AGENTS.md:253-260](https://github.com/herdrdev/herdr/blob/6e8b138d0f7d7d695530657a6d8dc475bd3fba2b/AGENTS.md#L253-L260), [justfile:147-219](https://github.com/herdrdev/herdr/blob/6e8b138d0f7d7d695530657a6d8dc475bd3fba2b/justfile#L147-L219)). `v*` tag push triggers stable release ([release.yml:3-6](https://github.com/herdrdev/herdr/blob/6e8b138d0f7d7d695530657a6d8dc475bd3fba2b/.github/workflows/release.yml#L3-L6)). The recipe does **not** wait for or query a green main CI run between pushing the release commit and tag. | Fresh `cargo build --release --locked` for five targets in the tag workflow; release artifacts do not come from main CI ([release.yml:38-180](https://github.com/herdrdev/herdr/blob/6e8b138d0f7d7d695530657a6d8dc475bd3fba2b/.github/workflows/release.yml#L38-L180)). | Stable workflow has no artifact attestation job, detached asset signature, checksum asset, or OIDC signing permission. The current stable GitHub Release is visibly immutable, but that repository setting is not asserted by stable workflow code. | Human runs the tag recipe, but there is no protected release Environment. After all builds/Nix/input validation, `softprops/action-gh-release` automatically creates/uploads/publishes in the same job; no human draft-review step is requested ([release.yml:183-232](https://github.com/herdrdev/herdr/blob/6e8b138d0f7d7d695530657a6d8dc475bd3fba2b/.github/workflows/release.yml#L183-L232); the pinned action documents `draft` default false in [action.yml:18-23](https://github.com/softprops/action-gh-release/blob/b4309332981a82ec1c5618f44dd2e27cc8bfbfda/action.yml#L18-L23)). | Core stable release creation has no explicit existing-release reconciliation/resume logic. Pre-release build/input failures occur before the release job. Noncritical issue closing is `continue-on-error`, and the later docs push has a bounded three-attempt retry ([release.yml:233-239](https://github.com/herdrdev/herdr/blob/6e8b138d0f7d7d695530657a6d8dc475bd3fba2b/.github/workflows/release.yml#L233-L239), [release.yml:420-441](https://github.com/herdrdev/herdr/blob/6e8b138d0f7d7d695530657a6d8dc475bd3fba2b/.github/workflows/release.yml#L420-L441)). Herdr's preview workflow has stronger immutable-release resume checks, but stable does not ([preview.yml:347-416](https://github.com/herdrdev/herdr/blob/6e8b138d0f7d7d695530657a6d8dc475bd3fba2b/.github/workflows/preview.yml#L347-L416)). |
| **ripgrep** | CI runs for PRs, `master` pushes, and nightly schedule. Its broad target matrix builds/tests debug profile; it does not build the final `release-lto` distribution artifacts on main ([ci.yml:1-8](https://github.com/BurntSushi/ripgrep/blob/3fce3b5bb0236da2df6d99672afb8a719642eca7/.github/workflows/ci.yml#L1-L8), [ci.yml:30-187](https://github.com/BurntSushi/ripgrep/blob/3fce3b5bb0236da2df6d99672afb8a719642eca7/.github/workflows/ci.yml#L30-L187)). | A pushed bare SemVer tag such as `15.2.0` triggers release. The repository workflow validates that tag against `Cargo.toml` but never creates it, so tag creation is external to Actions; no checked-in source identifies the exact human/tool ([release.yml:3-40](https://github.com/BurntSushi/ripgrep/blob/3fce3b5bb0236da2df6d99672afb8a719642eca7/.github/workflows/release.yml#L3-L40)). It does not query prior main CI. | Creates a draft first, then freshly builds `release-lto`, packages, hashes, and uploads each target from the tag workflow; no main artifact reuse ([release.yml:16-65](https://github.com/BurntSushi/ripgrep/blob/3fce3b5bb0236da2df6d99672afb8a719642eca7/.github/workflows/release.yml#L16-L65), [release.yml:184-299](https://github.com/BurntSushi/ripgrep/blob/3fce3b5bb0236da2df6d99672afb8a719642eca7/.github/workflows/release.yml#L184-L299)). | Each main archive gets SHA-256 and GitHub build-provenance attestation. The `.deb` path has SHA-256 but no attestation step in this workflow. No detached product signature is present ([release.yml:269-299](https://github.com/BurntSushi/ripgrep/blob/3fce3b5bb0236da2df6d99672afb8a719642eca7/.github/workflows/release.yml#L269-L299), [release.yml:300-386](https://github.com/BurntSushi/ripgrep/blob/3fce3b5bb0236da2df6d99672afb8a719642eca7/.github/workflows/release.yml#L300-L386)). | Workflow deliberately leaves a draft; no job publishes it. Publication therefore occurs outside this workflow (normally human review), but the responsible identity/process is not documented in the inspected files. There is no protected release Environment. | A failed matrix leaves a nonpublic draft. Whole-workflow retry is not fully idempotent in the checked-in code: `gh release create` expects vacancy and `gh release upload` does not use `--clobber`. Selectively rerunning failed jobs can reuse the already-successful draft, but partial same-name uploads may need manual cleanup. |
| **uv** | CI runs on `main` push, PR, and dispatch, with reusable lint/test/dev-binary/smoke/integration/system jobs ([ci.yml:3-7](https://github.com/astral-sh/uv/blob/a61e5c918cedf9ecc7e831bae2650657ae413445/.github/workflows/ci.yml#L3-L7), [ci.yml:78-157](https://github.com/astral-sh/uv/blob/a61e5c918cedf9ecc7e831bae2650657ae413445/.github/workflows/ci.yml#L78-L157)). Formal release-binary builds in CI are conditional on release-relevant files/labels, not every main push ([plan.yml:131-184](https://github.com/astral-sh/uv/blob/a61e5c918cedf9ecc7e831bae2650657ae413445/.github/workflows/plan.yml#L131-L184)). Inference: the normal release-preparation PR changes Cargo manifests, so its eventual main commit should select the release-binary job, but the release workflow does not query that run. | A manual “Prepare release” workflow bumps version/changelog and opens/updates a PR ([release-prepare.yml:1-18](https://github.com/astral-sh/uv/blob/a61e5c918cedf9ecc7e831bae2650657ae413445/.github/workflows/release-prepare.yml#L1-L18), [release-prepare.yml:53-63](https://github.com/astral-sh/uv/blob/a61e5c918cedf9ecc7e831bae2650657ae413445/.github/workflows/release-prepare.yml#L53-L63), [release-prepare.yml:107-129](https://github.com/astral-sh/uv/blob/a61e5c918cedf9ecc7e831bae2650657ae413445/.github/workflows/release-prepare.yml#L107-L129)). Actual Release is another `workflow_dispatch` with a tag input, **not** a tag-push trigger ([release.yml:20-49](https://github.com/astral-sh/uv/blob/a61e5c918cedf9ecc7e831bae2650657ae413445/.github/workflows/release.yml#L20-L49)). Final `gh release create TAG --target github.sha` creates the tag if absent; the workflow comment explicitly obtains workflow/content permission for tag creation ([release.yml:355-379](https://github.com/astral-sh/uv/blob/a61e5c918cedf9ecc7e831bae2650657ae413445/.github/workflows/release.yml#L355-L379), [official `gh release create` behavior](https://cli.github.com/manual/gh_release_create)). | Reuses the same release-builder workflow definition in CI and Release, but the Release invocation performs a fresh build and passes its own temporary workflow artifacts through checksum/global assembly; it does not download artifacts from a prior CI run ([build-release-binaries.yml:1-31](https://github.com/astral-sh/uv/blob/a61e5c918cedf9ecc7e831bae2650657ae413445/.github/workflows/build-release-binaries.yml#L1-L31), [release.yml:119-230](https://github.com/astral-sh/uv/blob/a61e5c918cedf9ecc7e831bae2650657ae413445/.github/workflows/release.yml#L119-L230)). | Generates per-archive checksums and a manifest; the final GitHub assets are attested, Docker is attested separately, and registry publishing uses OIDC. No product-specific detached manifest signature is present ([release.yml:311-379](https://github.com/astral-sh/uv/blob/a61e5c918cedf9ecc7e831bae2650657ae413445/.github/workflows/release.yml#L311-L379)). | A dedicated `release-gate` Environment requires approval by another team member; a protection app then admits the separate `release` environment. Final GitHub Release is automatically published with all assets in one `gh release create` call, not left for human draft review ([release.yml:56-71](https://github.com/astral-sh/uv/blob/a61e5c918cedf9ecc7e831bae2650657ae413445/.github/workflows/release.yml#L56-L71), [release.yml:311-379](https://github.com/astral-sh/uv/blob/a61e5c918cedf9ecc7e831bae2650657ae413445/.github/workflows/release.yml#L311-L379)). GitHub CLI internally uses a draft while uploading assets when immutable releases are enabled, but there is no human pause ([official manual](https://cli.github.com/manual/gh_release_create#immutable-releases)). | Publishing waits on build/host/gate/PyPI. The workflow explicitly excludes crates.io publication from the GitHub Release dependency because crates publication is not idempotent on retry ([release.yml:311-322](https://github.com/astral-sh/uv/blob/a61e5c918cedf9ecc7e831bae2650657ae413445/.github/workflows/release.yml#L311-L322)). It has no explicit same-SHA green-main-CI lookup and no checked-in existing-Release recovery branch. |
| **just** | CI runs on PRs and `master` pushes, performing lint/MSRV/docs/tests in normal debug/check modes. It does not build formal release archives on main ([ci.yaml:3-9](https://github.com/casey/just/blob/b20386abdbae867a49cdff6c3c0f2b547faa9b23/.github/workflows/ci.yaml#L3-L9), [ci.yaml:18-102](https://github.com/casey/just/blob/b20386abdbae867a49cdff6c3c0f2b547faa9b23/.github/workflows/ci.yaml#L18-L102)). | A maintainer's local `just publish` recipe shallow-clones current GitHub `master`, creates/pushes an annotated version tag, then publishes crates.io; any pushed tag triggers Actions ([justfile:78-90](https://github.com/casey/just/blob/b20386abdbae867a49cdff6c3c0f2b547faa9b23/justfile#L78-L90), [release.yaml:3-6](https://github.com/casey/just/blob/b20386abdbae867a49cdff6c3c0f2b547faa9b23/.github/workflows/release.yaml#L3-L6)). It does not wait for/query exact main CI. | Fresh per-target `cargo build --release` in the tag workflow; no CI artifact reuse ([bin/package:19-22](https://github.com/casey/just/blob/b20386abdbae867a49cdff6c3c0f2b547faa9b23/bin/package#L19-L22)). | A consolidated `SHA256SUMS` is generated only after all package jobs. No attestation, detached signature, OIDC signing, or immutable-release assertion appears in the workflow ([release.yaml:161-191](https://github.com/casey/just/blob/b20386abdbae867a49cdff6c3c0f2b547faa9b23/.github/workflows/release.yaml#L161-L191)). | Human tag is the only release gate. Each parallel target calls the release action with `draft: false`; there is no protected Environment or later human publish step ([release.yaml:32-148](https://github.com/casey/just/blob/b20386abdbae867a49cdff6c3c0f2b547faa9b23/.github/workflows/release.yaml#L32-L148)). | Because each matrix job publishes directly, some assets can already be public while another target fails; the checksum appears only after every package job succeeds. This is simple but is the least suitable model for zterm's authenticated updater and immutable inventory. |

### 4. Lessons applicable to zterm

#### Keep the exact proposed ordering

The user's ordering is clearer and more enforceable than the stable flows in
Herdr, ripgrep, and just, all of which rely on maintainer discipline/local
checks rather than querying the exact successful main CI run. uv has stronger
human approval, but its manual release workflow creates the tag at the final
release step and likewise does not prove an exact prior main CI success.

zterm already implements the useful combination:

1. PR review/CI before merge.
2. A fresh, comprehensive `push` CI run for the exact commit after it lands on
   `main`.
3. Human-controlled version tag only after that run is green.
4. Machine enforcement in the tag workflow so a mistaken early tag still
   cannot reach signing or Release creation.

The human operation should not be phrased merely as “CI 看起来绿了.” Record the
exact SHA and successful main-push run ID, then tag that SHA. The release
workflow already records the matching run ID as output.

#### Keep release rebuilds; do not promote main-CI binaries

Every inspected project rebuilds. Reusing ordinary CI artifacts would save
compute but would force zterm to add a new promotion trust contract: retention,
exact digest selection, artifact provenance, official build identity, and proof
that the PR/main job had the release-only environment and flags. That conflicts
with the existing simpler contract. zterm's current “build twice” pattern has
two distinct owners:

- main CI: prove all code/tests/platform release-mode boundaries are green
  before the irreversible tag decision;
- tag workflow: create the official, frozen, signed, attested inventory.

#### Keep the tag human-created

Herdr and just also use human-created tags; ripgrep requires an external tag;
uv instead creates it inside a two-person-approved manual workflow. Either can
work, but automatic tagging on every green main push would conflate “merge is
healthy” with “publish a version now.” zterm's explicit human tag is the smaller
authority surface and avoids granting an automation token permission to create
release tags.

If a future maintainer UI is desired, a manually dispatched tag-creation
workflow could imitate uv, but that is not required by the current acceptance
criteria and should not be added merely for convenience.

#### zterm's draft timing is stronger for this product

- ripgrep creates a draft before matrix builds. This is recoverable but exposes
  incomplete draft state early.
- Herdr stable and just automatically publish in their release jobs; just can
  expose a partially populated public Release.
- uv gates publication and uploads all assets in the final CLI call, with no
  human draft pause.
- zterm waits until tag validation, four builds, signing, and four installer
  fixture runs finish, then creates one draft, downloads it back, verifies the
  exact inventory, attests those downloaded bytes, and publishes. This is the
  best fit for an updater whose trust unit is one signed, immutable inventory.

#### Preserve both human gates, because they authorize different things

The tag push authorizes “release this exact reviewed commit/version.” The
protected `release` Environment approval authorizes use of the long-lived
signing seed. They are not redundant. Publication itself can remain automatic
after signed installer tests pass; requiring a third manual “publish” click
would add coordination without protecting a new secret or invariant.

#### Provenance complements, but does not replace, the product signature

ripgrep and uv demonstrate the current GitHub attestation pattern. Neither
inspected workflow has zterm's detached product manifest signature. That extra
signature is justified here because the installed binary/updater must verify
release metadata without treating GitHub's web/API identity alone as the
product trust root. The current spec correctly keeps both.

### 5. Failure/retry implications for the first formal zterm release

The desired release is intentionally fail-closed:

- Before draft creation, a failure has no Release-side state. Diagnose the
  failed job and rerun only after preserving the same tag/SHA invariant.
- After draft creation, verification or attestation failure leaves a draft and
  does not publish, delete, or replace it automatically.
- After immutable publication, assets/tag are never repaired in place; a real
  defect requires a new version and tag.

This is safer than just's partial direct publication and more explicit than
Herdr stable. It is intentionally less automatically idempotent than Herdr's
preview channel. One operational gap remains: the repository documents the
state outcome after a failed draft, but no exact **manual recovery runbook** for
an existing failed draft was found. The release workflow rejects any existing
Release, including a draft, so a full rerun after that point cannot silently
resume or replace it. Before the first real tag, the team should decide and
record whether the recovery is “inspect and explicitly remove the draft, then
rerun the same tag” or “abandon that version and issue a new tag”; automation
must not make that choice.

There is also an external precondition that must be checked before the first
tag: immutable Releases must already be enabled. The workflow verifies
`immutable: true` only after it changes the draft to public. If the repository
setting is off, the job detects the problem but a mutable public Release already
exists. The task artifacts deliberately assign this precondition to the
environment reviewer; it should be part of the release checklist.

### 6. Files found

#### Local zterm

- `.github/workflows/ci.yml` — PR/push checks and exact main-only native
  release-mode readiness builds.
- `.github/workflows/release.yml` — exact-tag gate, four fresh builds, protected
  signing, installer matrix, late draft, round-trip attestation, immutable
  publication.
- `.trellis/tasks/08-24-distribution-release/prd.md` — approved user-facing
  distribution requirements and human-tag decision.
- `.trellis/tasks/08-24-distribution-release/design.md` — trust-chain ordering,
  build/rebuild boundary, and fail-closed draft policy.
- `.trellis/tasks/08-24-distribution-release/implement.md` — outstanding formal
  main-CI/tag/release rehearsal and evidence checklist.
- `.trellis/spec/backend/distribution-lifecycle.md` — executable contract for
  green-main-CI gating, human tag, signing, draft verification, and immutable
  publication.
- `.trellis/spec/guides/evidence-driven-simplicity.md` — requires one owner per
  invariant and discourages unmotivated recovery automation.

#### Herdr (`herdrdev/herdr`, revision `6e8b138d0f7d7d695530657a6d8dc475bd3fba2b`)

- [`.github/workflows/ci.yml`](https://github.com/herdrdev/herdr/blob/6e8b138d0f7d7d695530657a6d8dc475bd3fba2b/.github/workflows/ci.yml) — PR/master checks.
- [`.github/workflows/release.yml`](https://github.com/herdrdev/herdr/blob/6e8b138d0f7d7d695530657a6d8dc475bd3fba2b/.github/workflows/release.yml) — stable tag rebuild and automatic GitHub Release.
- [`.github/workflows/preview.yml`](https://github.com/herdrdev/herdr/blob/6e8b138d0f7d7d695530657a6d8dc475bd3fba2b/.github/workflows/preview.yml) — preview-only draft/resume comparison.
- [`justfile`](https://github.com/herdrdev/herdr/blob/6e8b138d0f7d7d695530657a6d8dc475bd3fba2b/justfile) — maintainer release preparation, annotated tag, and push commands.
- [`AGENTS.md`](https://github.com/herdrdev/herdr/blob/6e8b138d0f7d7d695530657a6d8dc475bd3fba2b/AGENTS.md) — canonical repository identity and documented stable release process.

#### ripgrep (`BurntSushi/ripgrep`, revision `3fce3b5bb0236da2df6d99672afb8a719642eca7`)

- [`.github/workflows/ci.yml`](https://github.com/BurntSushi/ripgrep/blob/3fce3b5bb0236da2df6d99672afb8a719642eca7/.github/workflows/ci.yml) — PR/master/nightly target matrix.
- [`.github/workflows/release.yml`](https://github.com/BurntSushi/ripgrep/blob/3fce3b5bb0236da2df6d99672afb8a719642eca7/.github/workflows/release.yml) — SemVer-tag draft, release-LTO rebuild, checksums, attestations, asset upload.

#### uv (`astral-sh/uv`, revision `a61e5c918cedf9ecc7e831bae2650657ae413445`)

- [`.github/workflows/ci.yml`](https://github.com/astral-sh/uv/blob/a61e5c918cedf9ecc7e831bae2650657ae413445/.github/workflows/ci.yml) — main/PR orchestration and conditional release builder.
- [`.github/workflows/plan.yml`](https://github.com/astral-sh/uv/blob/a61e5c918cedf9ecc7e831bae2650657ae413445/.github/workflows/plan.yml) — path/label decisions for tests and release-mode builds.
- [`.github/workflows/release-prepare.yml`](https://github.com/astral-sh/uv/blob/a61e5c918cedf9ecc7e831bae2650657ae413445/.github/workflows/release-prepare.yml) — version/changelog release PR automation.
- [`.github/workflows/build-release-binaries.yml`](https://github.com/astral-sh/uv/blob/a61e5c918cedf9ecc7e831bae2650657ae413445/.github/workflows/build-release-binaries.yml) — shared but freshly invoked formal build matrix.
- [`.github/workflows/release.yml`](https://github.com/astral-sh/uv/blob/a61e5c918cedf9ecc7e831bae2650657ae413445/.github/workflows/release.yml) — manual input, two-person gate, fresh assembly, attestations, automatic tag/direct publication.

#### just (`casey/just`, revision `b20386abdbae867a49cdff6c3c0f2b547faa9b23`)

- [`.github/workflows/ci.yaml`](https://github.com/casey/just/blob/b20386abdbae867a49cdff6c3c0f2b547faa9b23/.github/workflows/ci.yaml) — PR/master checks.
- [`.github/workflows/release.yaml`](https://github.com/casey/just/blob/b20386abdbae867a49cdff6c3c0f2b547faa9b23/.github/workflows/release.yaml) — any-tag matrix, per-target direct publish, later checksum aggregation.
- [`justfile`](https://github.com/casey/just/blob/b20386abdbae867a49cdff6c3c0f2b547faa9b23/justfile) — local maintainer tag/publish recipe.
- [`bin/package`](https://github.com/casey/just/blob/b20386abdbae867a49cdff6c3c0f2b547faa9b23/bin/package) — formal release-mode binary/archive creation.

### 7. External references and versions

- Repository snapshots were pinned to the four commit revisions listed above,
  resolved from each default branch on 2026-08-25.
- [Official GitHub CLI `gh release create` manual](https://cli.github.com/manual/gh_release_create)
  documents that an absent tag is created automatically unless `--verify-tag`
  is used, and that immutable-release asset upload uses a draft internally
  before publication.
- [`softprops/action-gh-release` pinned input contract](https://github.com/softprops/action-gh-release/blob/b4309332981a82ec1c5618f44dd2e27cc8bfbfda/action.yml#L18-L35)
  documents default automatic publication and overwrite behavior for the exact
  revision used by Herdr.

### 8. Related specs

- `.trellis/spec/backend/distribution-lifecycle.md:70-128` — authoritative
  release-manifest, human-tag, protected-signing, draft, attestation, immutable
  contract.
- `.trellis/spec/backend/distribution-lifecycle.md:130-188` — failure matrix and
  required release workflow tests.
- `.trellis/spec/guides/evidence-driven-simplicity.md` — validation ownership,
  manual recovery defaults, and avoidance of unjustified retry machinery.
- `.trellis/spec/guides/cross-platform-thinking-guide.md` — hosted native runner
  evidence is required; cross-compilation alone is not platform acceptance.

## Caveats / Not Found

- GitHub repository settings are not fully represented in workflow YAML.
  Branch/ruleset required-check configuration, protected-Environment reviewer
  lists, and immutable-release toggles were not mutated or authenticated in
  this read-only research. A visible immutable past release is evidence of
  current state, not proof that a workflow enforces the setting.
- For ripgrep, the checked-in workflow proves that Actions consumes an existing
  tag but does not identify who or what creates/pushes it. Calling that step
  “maintainer/manual” is an inference, not an encoded contract.
- For uv, the inference that the release-preparation merge runs the conditional
  release-mode builder depends on Cargo manifest changes made by its release
  script. The Release workflow itself does not verify or consume that main CI
  result.
- Herdr preview has a schedule mentioned in its maintainer documentation, but
  the inspected snapshot's visible workflow trigger is manual dispatch; this
  discrepancy is outside the stable-release question and was not used as a
  conclusion.
- Retry conclusions describe explicit YAML/commands only. GitHub UI choices
  such as “rerun failed jobs,” action-internal retries, and administrator manual
  cleanup can alter recovery behavior but are not substitutes for a checked-in
  release contract.
- No code/workflow/spec was edited, no repository setting was changed, and no
  tag, CI run, draft, or Release was created.
