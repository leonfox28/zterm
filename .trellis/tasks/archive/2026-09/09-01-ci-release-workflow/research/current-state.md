# Pre-refactor push, CI, and release evidence

> Point-in-time baseline captured before the 2026-09-01 workflow refactor.
> It is research evidence, not a current runbook. See `docs/development.md` and
> `docs/releasing.md` for the implemented flow.

- Date: 2026-09-01
- Repository: `leonfox28/zterm`
- Evidence: checked-in workflows/tests/docs, Git history, GitHub Actions REST data and failed-job logs

## 1. Repository integration model

- GitHub reports `main` is not protected and the repository has no rulesets.
- The repository has no pull requests and only one remote branch, `main`.
- There is no configured Git hooks path and no `justfile`, Makefile, task runner,
  `xtask`, or other single local quality/release entry point.
- Current developer documentation presents a list of individual commands, but
  that list is not identical to `.github/workflows/ci.yml`. In particular, the
  workflow also owns maintained-shell ShellCheck, HTTPS-fixture compilation,
  five hosted Rust variants, four main-only release-mode builds, two cargo-deny
  invocations, and the full relay bundle.
- On the inspected developer machine, `cargo-deny`, `gh`, and `jq` exist;
  `just` and `shellcheck` do not. A new local entry point therefore needs an
  explicit bootstrap/preflight contract rather than assuming tools happen to
  exist.

## 2. Pre-refactor CI trigger and job graph

`.github/workflows/ci.yml` runs on every branch push, every pull request, and
manual dispatch. Its concurrency key cancels an older run for the same ref.
Because it listens to both arbitrary branch pushes and pull requests, a future
PR branch would run the same workflow twice unless the push trigger is narrowed.

For a push to `main`, one workflow expands to 12 hosted jobs:

1. `Native release policy` — static release/installer policy, maintained-shell
   ShellCheck, Python syntax.
2. Five `Rust` jobs — macOS arm64, macOS Intel, Linux x64, Linux arm64, Windows;
   checkout policy, version, format, Clippy/compile, tests, docs and CLI smoke.
3. Four main-only release-readiness jobs — native release-mode builds for the
   two macOS and two glibc targets, including floor/architecture inspection.
4. `Dependency policy` — installs cargo-deny and checks the workspace plus the
   isolated relay probe.
5. `Official relay bundle` — relay source, image, architecture, runtime and
   secret-scan contracts.

A normal successful run usually has a 6–8 minute critical path. Run
[`33496462514`](https://github.com/leonfox28/zterm/actions/runs/33496462514)
took about 50 minutes wall-clock only because one 5.5-minute Ubuntu job waited
roughly 44 minutes for a runner; most jobs finished within eight minutes. The
workflow currently has no Rust/dependency cache and no per-job timeouts.

## 3. CI reliability data

For all 61 recorded `ci.yml` runs at the time of inspection:

| Conclusion | Runs |
| --- | ---: |
| success | 34 |
| failure | 18 |
| cancelled | 9 |

Excluding cancellation, the historical failed-run rate is 34.6%. From
2026-08-24 onward there were 13 successes, 10 failures and 6 cancellations;
the failed-run rate excluding cancellation was 43.5%.

Failed-step occurrences across the 18 failed runs (one run may fail on several
platforms) were:

| Step | Failed job occurrences |
| --- | ---: |
| Unix workspace tests | 16 |
| Windows shared-boundary compile | 7 |
| dependency policy | 2 |
| Linux release-mode boundary | 2 |
| maintained-shell ShellCheck | 2 |
| source checkout policy | 2 |
| Clippy | 2 |
| formatting | 1 |
| relay secret scan | 1 |

The failures divide into materially different classes:

- **Ordinary change defects discoverable before push:** formatting, Clippy,
  many Unix test failures, and dependency-policy failures. Current local docs
  do not provide one executable parity command, so these are easy to omit.
- **Cross-platform compile/runtime defects:** Windows private Unix state and
  OS-specific lifecycle tests. These need hosted-platform evidence, although
  a local Windows-target lint can sometimes fail earlier.
- **Nondeterministic/harness failures:** several timing/order tests passed on
  other platforms or on the same product source in a nearby run. The latest
  journal-only run
  [`33504336211`](https://github.com/leonfox28/zterm/actions/runs/33504336211)
  failed `detached_lifecycle` with `DaemonStartTimeout` on macOS even though the
  previous product commit was green.
- **Externally mutable dependency metadata:** runs on 2026-08-31 and 2026-09-01
  failed because `chacha20 0.10.1` became yanked. This was not introduced by
  the tested source diff, but it is a real lockfile maintenance signal.
- **Gate-scope false positive:** the same latest run failed the relay secret
  scan because `.trellis/scripts/add_session.py` contains ordinary variables
  named `token`. `tests/secret-scan.sh` claims Trellis runtime state is outside
  the shipped inputs but scans the whole repository except only `.git`,
  `target`, and `.runtime`.
- **Hosted environment assumptions:** historical release-readiness failures
  included Git dubious ownership in a container and `$HOME` differing from the
  passwd home, leaving `rustc` outside `PATH`.

The data does not support removing the platform matrix: it repeatedly caught
real Windows and host-specific defects. It does support moving portable checks
earlier, fixing flaky/over-broad owners, and making hosted-only evidence
explicit.

## 4. Current release state machine

`.github/workflows/release.yml` is triggered only by a pushed `v*` tag. Its
ordered stages are:

1. `validate` — exact tag/version, vacancy, same-SHA successful `ci.yml` push
   run on `main`, timestamp and prerelease classification.
2. four native artifact builds — rebuild from the frozen commit with official
   build identity and inspect target compatibility floors.
3. `assemble` — combine exactly four archives, manifest, checksums, installer
   and SBOM; verify unsigned inventory and ShellCheck the generated installer.
4. `sign` — wait for the protected `release` Environment; build the signer
   before exposing the Ed25519 seed, then sign/self-verify and emit fixture
   inputs.
5. four `installer` jobs — real local-HTTPS install plus smallest negative
   cases on every shipped target.
6. `publish` — verify again, create one late draft, upload/download the exact
   inventory, verify the round trip, attest, publish, and require
   `immutable: true`.

This tag workflow does **not** rerun ordinary CI: it has no workspace format,
general Clippy, general unit-test, documentation, cargo-deny, or relay test
stage. Its checks after the four builds are release-artifact verification at
new trust boundaries, not a second copy of the development test suite.

Repository settings currently confirm that immutable releases are enabled and
the `release` Environment has a required reviewer. `main` itself is not
protected.

The successful v0.1.9 release
[`33500769451`](https://github.com/leonfox28/zterm/actions/runs/33500769451)
lasted 17 minutes 22 seconds. The four builds and assembly completed after
about 7 minutes 17 seconds; the signing job did not start for another 8 minutes
22 seconds while waiting for Environment approval. After approval, signing,
all four installer jobs, draft verification, attestation and publication took
about 1 minute 41 seconds. The apparent “long workflow” is therefore roughly
half real build time and half an invisible human approval wait.

## 5. Release failure evidence

There are eight formal native-release runs: five succeeded and three failed.
The three failures were all before draft/publication, so no incomplete public
Release was created:

- v0.1.2 run
  [`32802895706`](https://github.com/leonfox28/zterm/actions/runs/32802895706):
  generated installer hit ShellCheck `SC2015` in all four installer jobs. This
  portable lint belonged before tag/signing and is now owned once by Ubuntu
  assembly plus Rust/static policy tests.
- v0.1.3 run
  [`32806708978`](https://github.com/leonfox28/zterm/actions/runs/32806708978):
  macOS jobs required `command -v shellcheck`, but the runner images did not
  provide ShellCheck. This was an invalid matrix tool assumption and is now
  encoded in the cross-platform spec.
- v0.1.4 run
  [`32830702052`](https://github.com/leonfox28/zterm/actions/runs/32830702052):
  both macOS HTTPS fixtures timed out before writing their port file because
  Python's inherited HTTPServer bind path performed hostname resolution. The
  fixture now bypasses that unnecessary lookup and has a static policy guard.

The five releases from v0.1.5 through v0.1.9 succeeded consecutively. The
current release workflow is therefore substantially more mature than its first
three tag attempts. Remaining operator pain is primarily unclear preparation,
approval/status visibility, and recovery guidance—not evidence that the
signature/draft/installer stages should be deleted.

## 6. Relay publication orchestration gap

`.github/workflows/relay-image.yml` expects a `release: published` event to
publish `zterm-relay:<version>` and optionally `latest`. The native release is
itself created and published by `.github/workflows/release.yml` with the
repository `GITHUB_TOKEN`. GitHub documents that, except for explicit
`workflow_dispatch` and `repository_dispatch`, events produced by that token do
not create another workflow run. This prevents recursive automation, but also
means the current implicit native-release-to-relay-release edge is unsound.

The Actions history agrees with this mechanism: relay-image has runs for the
early manually published releases and a development dispatch, but no matching
run for the later v0.1.5–v0.1.9 native releases. The optimized design must call
relay publication explicitly (preferably as a reusable workflow or a direct
job dependency) and make success of both native and container assets visible
in one release result.

## 7. Immediate planning implications

- A red CI currently lands *after* the commit is already on `main`; no workflow
  rearrangement can make an unprotected direct push a pre-merge gate.
- The maintainer has chosen branch + PR + required check for substantive
  changes. Branch protection is therefore part of the target operating model,
  while direct-to-main is no longer the normal integration path.
- Independent of that decision, CI and local development need one command
  owner, narrower triggers, clearer job names/reproduction hints, cache/timeout
  review, and fixes for the known flaky/over-broad gates.
- Release should keep the human tag, protected signing approval, fresh formal
  rebuild, signed manifest, four-platform installer proof, late draft and
  immutable publication. Simplification should happen at the maintainer entry
  and status presentation layer.
