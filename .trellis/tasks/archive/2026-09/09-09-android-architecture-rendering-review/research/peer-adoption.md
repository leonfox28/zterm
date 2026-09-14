# Adopting peer rendering ideas

## Boundary and diagnosis

The user approves the three recommendations from `android-terminal-peers.md`.
Current content-generation reuse skips metadata-only conversion, but a new
window projects every semantic cell to an owned Rust/UniFFI/Kotlin DTO. The row
RenderNode cache is downstream of that allocation. This is a preparation-boundary
optimization gap, not a missing wire delta or another terminal-model defect.

The native actor installs and ACKs independently of Android observations. Its
watch already conflates unobserved complete frames. Repository currently converts
as soon as its waiter completes, without using display cadence. Cold recording
still emits independent text calls for all non-space cells. No new phone FPS
claim follows from these source observations.

## Source-relative projection

Add `NativeFrameSource.presentation_rows_from(previous)` returning one typed
`Reuse(index)` or `Replace(row)` per row of the new bounded window. Compare exact
semantic rows (including wrap, width and all styles), after verifying the same
attachment origin and color/appearance configuration. A borrowed hash map narrows
matching; full equality confirms it, including across ordinal changes. Missing
or incompatible baselines yield replacements. Hash derives on existing core
values add no fields or wire behavior. The existing complete projection remains
for independent consumers and as the correctness oracle.

Kotlin keeps one projection baseline: paired retained source and immutable row
list. Reuse records refer only to that exact list. Replace records alone cross
FFI with cell payloads. The new frame always carries its own source/epochs; row
sharing does not copy input authority. Release baseline on replacement, eager
fallback, cancellation and observation termination. Native page accounting and
three-viewports-plus-one export bounds remain unchanged.

## Display cadence

For an ordinary active content update with unchanged input/geometry/screen/
pointer/selection authority, wait for one Choreographer frame then re-read the
latest complete native frame before projection. This is a display gate, not a
network debounce. Initial, metadata-only and observed authority transitions
bypass it. Native ordered updates, ACKs and input rejection never wait on it.
A transition arriving during the gate is taken by that next re-read (at most one
display tick while visible); hiding releases the gate without waiting for vsync.
Keep one collector, no queue of content frames or second native observer. A
cancellable frame-clock callback is removed on cancellation; re-read also sees
the final already-published state after native actor closure.

## Painter experiment

Try one run for consecutive regular single-width ASCII cells with matching
render style and exact monospace advances. Preserve complex clusters, wide
heads/continuations, bold/italic and decorations on the cell path. Keep
background/clipping behavior exact; reject or narrow the run path when hardware
pixel comparison differs. Measure cold recording against the pre-change painter
with identical warmup, sparse/dense/decorated controls. Do not retain diagnostic
switches or claim emulator recording time is physical-phone smoothness.

## Expected files and exclusions

- `crates/core/src/terminal.rs` (and underline definition if separate): derive
  semantic Hash consistent with existing Eq, no model/schema changes.
- `crates/android/src/terminal.rs` and focused native tests: typed relative
  projection and borrowed equality lookup.
- `AppRepository.kt` and `TerminalFrames.kt`: one projection owner/collector and
  cancellable display clock; replace the current inline full-window resolver.
- `TerminalView.kt`: measured compatible run painting only; row renderer and
  IME geometry remain as currently implemented.
- Focused Android tests, task evidence and shared-client/Android specs: observable
  ownership, burst convergence, pixel equivalence and measured work reduction.

No dependency migration, renderer replacement, character diff wire encoding,
blank-content/child-name heuristics, keyboard-policy changes, phone operation,
unrelated defect fixes, commit or task archive.

## Acceptance owners

Native tests compare reconstructed updates with full projection for sparse/dense/
equal/moved/resize/palette/cluster cases and preserve source fences. Android
checks exercise the actual collector/clock and row reference reuse. Real FFI
checks use a disposable host if necessary, never the user's daemon. Paired pixel
and recording probes determine painter adoption. Existing unified-IME and
hardware-row regressions protect the accepted visual behavior. Build/lint, APK
signature and alignment checks identify the deliverable, not phone acceptance.

## Adoption decision

Retain source-relative projection and display pacing. Do not retain the text-run
experiment. Its exact code and paired probe are archived in
`text-run-experiment.patch`; `TerminalView.kt` and `TerminalRenderingTest.kt` are
restored byte for byte to the pre-adoption baseline, preserving all earlier
unified IME, row layers and exact-space changes.

The first run admitted glyph bounds touching the cell edge. Hardware comparison
found 34 differing pixels at 12 sp: antialias ink from K leaked into the following
L cell. Requiring strictly interior bounds restored exact pixels in all nine
regular/sparse/decorated hardware screenshots (8/12/16 sp). An independent cell
oracle also passed all nine font sizes. A four-glyph minimum, non-space runs and
short-run scan reuse reduced overhead, but did not establish a stable benefit
for styled control rows.

Recorded comparisons used 200 warmups and 100 timed cold recordings per scene,
on the same owned API 36 SwiftShader emulator. One stable four-glyph comparison
observed ASCII median 1.253041 -> 0.498333 ms, sparse 0.781750 -> 0.821583 ms,
and decorated 1.137375 -> 1.250542 ms. Subsequent sequential comparisons varied
substantially even for the baseline. The final CPU-instrumented run is preserved
in `text-run-before.json` / `text-run-after.json`:

| Scene | Glyph calls before -> after (cell + run) | Median wall ms before -> after | Median Main CPU ms before -> after |
| --- | --- | --- | --- |
| ASCII | 1583 -> 259 + 130 | 2.208833 -> 1.122500 | 2.182000 -> 1.116125 |
| Sparse | 400 -> 400 + 0 | 1.592875 -> 1.545834 | 1.571791 -> 1.525083 |
| Decorated | 520 -> 520 + 0 | 2.033458 -> 2.791250 | 2.021166 -> 2.757501 |

These samples support less work for compatible long text, not a net TUI/phone
improvement. CPU readings do not eliminate emulator/host variability; neither
all slowdown nor all improvement is attributed to one source branch. The
conservative outcome is retaining the already verified painter, rather than
shipping a broader path from the best scene alone. No diagnostic or batch switch
remains in production or the instrumentation APK.

## Retained implementation and validation

New production work is limited to `AppRepository.kt`, `TerminalFrames.kt`, the
relative projection / factored row resolver in `crates/android/src/terminal.rs`,
and Hash derives on five existing core cell/row/style/color/underline types.
There is no wire/schema/model/IME/renderer change in this increment. Kotlin owns
one explicitly retained projection baseline; native borrowed row lookup avoids
cell conversion/serialization for equal rows, including moved content. The
ordinary-content display gate re-reads the latest installed native frame after
one Choreographer tick; observed authority changes and metadata bypass it, and
hiding/cancellation cannot leave a native source queued behind vsync.

- `cargo clippy -p zterm-core -p zterm-android --all-targets --all-features -- -D warnings`
  passed. The test fixture's initial unwrap was replaced with a descriptive
  expect; no lint suppression was added.
- `cargo test -p zterm-core -p zterm-android --lib`: 67 core + 19 bridge tests passed.
  Source and terminal dependency policies passed; full workspace release/push
  gates were not run for this scoped increment.
- `TerminalFramesTest`: production projection/clock tests verify exact Kotlin
  reference reuse and fresh authority, off-Main conversion, burst convergence,
  hidden final-state delivery, and candidate/baseline cleanup on cancellation.
- `NativeFrameProjectionTest`, run alone with `projectionFixture=1`, passed on a
  real fresh fixture host and the owned emulator. It compared actual UniFFI
  reconstruction to complete projection, changed one of 40 rows (one Replace),
  shrank to 30 rows (zero Replace relative to the prior final content), grew to
  40 (ten new rows), and rejected the old selection source after resize while
  preserving input epoch. Its standalone NativeRuntime is a bridge fixture,
  not a test of Repository saved-host policy or physical phone presentation.
- The first native fixture attempt timed out on a test predicate: host default
  blanks legitimately project empty cell text, so a joined marker did not contain
  the expected ASCII space. Matching the fixed non-space marker resolved it;
  no product timing/protocol change was made. Initial host pairing-ticket creation
  also reported address_unavailable before its endpoint became ready; issuance
  later succeeded. The rerun used a newly issued fixture ticket after the earlier successful pairing;
  no ambiguous request was replayed. Every test-created
  Session was closed; the fixture confirmed zero Sessions before daemon shutdown.
- Final clean Android wrapper build includes assembleDebug,
  assembleDebugAndroidTest, lintDebug and testDebugUnitTest. Build/lint passed;
  there are no JVM unit-test sources. Final focused instrumentation passed 24
  checks, with one explicit AttachGeometry host fixture skipped (`OK (25 tests)`,
  75.286 s). The new real-bridge test above separately passed in 22.870 s.
  Thus 25 Android checks passed across these two runs, with one explicit skip.
- Existing both-screen IME/retention/reversal, real system-keyboard scheduling,
  pixel, row-layer and source-fence regressions remain green. No whole-phone
  smoothness, Herdr-specific latency, or tiny endpoint-overshoot claim is made.
- Development APK: `io.github.leonfox28.zterm.dev`, 0.1.31 / 103199,
  50,013,109 bytes, SHA-256
  `eb3831a050cb3993149fa38bd6435672f769a5c48de07d0e3c844d68bdd53f03`.
  APK v2 signature and every packaged ELF/ZIP 16 KiB alignment check passed.
  The installed APK on `Zterm_PeerAdoption_0910` / `emulator-5580` has the same
  exact hash. No phone was queried, controlled, installed or captured.

Evidence logs are retained locally as `/tmp/zterm-peer-adoption-{clippy,native-final,
source-policy,final-build,final-tests,ffi-tests}.txt`. Final paired screenshots
are under `/tmp/zterm-peer-adoption-painter-paired/`; all nine before/after PNGs
were byte-identical. The prototype and probes were archived and removed before
final build, and the pre-adoption View/rendering-test files were restored exactly.
The owned fixture daemon was stopped with zero Sessions and its marked root
`/tmp/zterm-peer-adoption-host` removed. Emulator cleanup is recorded below.

The task remains in progress for its earlier physical-device/whole-transition
acceptance gaps. No commit/archive is performed. The large existing Android app
spec exceeds the automatic context injection cap, so it was read manually; the
new frame-preparation contract is a separate bounded spec and is included in both
context manifests. No unrelated spec reorganization is bundled.

Owned emulator cleanup: verified its exact AVD name, stopped emulator-5580,
observed process exit, and deleted only `Zterm_PeerAdoption_0910` and its custom
`/tmp/zterm-peer-adoption-avd` directory. Both owned host and AVD paths are absent.
The checked development APK and evidence remain available in the workspace.
