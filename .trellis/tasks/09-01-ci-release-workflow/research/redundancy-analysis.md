# CI and release redundancy analysis

> Point-in-time analysis of the pre-refactor workflow. Repetition counts below
> explain the chosen design and are not current commands or job ownership. See
> `docs/development.md` and `docs/releasing.md` for the implemented flow.

- Date: 2026-09-01
- Scope: checked-in GitHub Actions plus observed run history

## CI: what ran before the refactor

A `main` push creates 12 jobs: one release-policy job, five Rust host jobs,
four main-only release-mode boundary jobs, one dependency-policy job, and one
relay bundle job. The five Rust jobs each repeat source policy, workspace
version, formatting and documentation, then run platform-specific compilation
and tests. Four Unix hosts also repeat the no-argument CLI smoke test.

## Recommended first-pass ownership

| Check | Current repetition | Recommendation | Reason |
| --- | --- | --- | --- |
| Workspace version | 5 hosts | Run once in portable policy | Pure repository-text invariant |
| Rust formatting | 5 hosts | Run once on canonical Linux | `rustfmt` result is not host evidence |
| Rust documentation | 5 hosts | Run once on canonical Linux | Current docs build has no host-specific flags |
| No-argument CLI smoke | 4 Unix hosts | Run once per OS family, or once on canonical Linux if no host contract remains | Same milestone behavior is currently repeated on both architectures |
| Source checkout policy | 5 Rust hosts plus release-mode jobs | Keep immediately after checkout in every OS matrix owner | The project cross-platform contract treats actual checkout bytes and Git attributes as host evidence; this probe is cheap and previously caught Windows newline drift |
| Clippy and tests | Full on 4 Unix hosts; shared subset on Windows | Keep platform evidence in the first pass | Historical failures include real Windows and host-specific lifecycle defects; command similarity is not proof of redundant evidence |
| Main release-mode builds | 4 targets on every main push | Keep as exact-SHA release readiness, but isolate/name clearly and consider change-aware scheduling later | They prove deployment floor and architecture before a tag; release rebuild has a different formal-artifact owner |
| Dependency policy | One Ubuntu job | Keep; cache pinned `cargo-deny` | License/advisory/yank state is independent of compiler tests |
| Relay bundle | One broad Ubuntu job | Keep functional owners; scope secret scan to shipped material | The current whole-repository scan produces unrelated Trellis false positives |

The trigger itself is redundant under the chosen PR-first model: listening to
both every branch push and every pull request starts two equivalent runs for a
PR branch. The target should run full admission CI on `pull_request`, and a
main integration/readiness run on `push: main`; optional branch-push feedback
must be a deliberately smaller workflow rather than another full copy.

The first optimization should also fix or quarantine deterministic owners for
the known macOS lifecycle flake, add Rust/dependency caching and explicit job
timeouts, and print the matching local reproduction command in every failure.
Deleting platform coverage while the existing gate is noisy would conflate
reliability cleanup with risk acceptance and make regressions harder to
attribute.

## Release: not a second CI suite

The current tag workflow already skips general formatting, Clippy, unit tests,
documentation, dependency policy and relay test bundles. Its intended sequence
is:

1. Prove the tag points to the exact green `main` commit and that the version is
   vacant.
2. Build the four official binaries from the frozen tag.
3. Assemble and verify the exact inventory, then sign the manifest behind the
   protected Environment approval.
4. Exercise the signed installer against each final target using local HTTPS.
5. Create a late draft, round-trip the uploaded bytes, attest them, and publish
   the immutable Release.

Steps 3–5 are release verification, not ordinary code tests: they validate
facts that do not exist until the official artifacts, signature and draft have
been created. The four installer jobs completed in only 7–17 seconds each in
v0.1.9 and run in parallel, so deleting them would save negligible wall time.
The 17-minute run was dominated by roughly seven minutes of native build work
and eight minutes waiting for human approval.

There are still small source-level repetitions inside the tag workflow that
can move before the tag: `tests/release/static.sh` reruns policy that the exact
green CI commit already proved, and each installer entry separately compiles
the Python fixture source even though policy checks it once and the fixture
then executes it on every host. Per-host POSIX syntax remains because the
project's cross-platform contract treats the selected host shell as evidence.
The generated installer must still be checked once after assembly, and the
final signed installer must still exercise each shipped artifact; those facts
do not exist before the release build.

The optimized release should preserve the same conceptual chain—validate,
build, sign, final-artifact smoke, publish—while presenting it as one clear
operator flow. It must additionally make relay-image publication an explicit
part of that chain because the current `release: published` trigger is not
reliably emitted into a new run when the native Release is published with
`GITHUB_TOKEN`.
