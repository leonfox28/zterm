# Retain the reviewed design; reduce empty glyph work

The user accepts the necessity review and asks to proceed with the recommended
optimization. Retain bottom anchoring with authoritative screen metadata,
content-keyed row reuse, displayed-source handoff, symmetric early targets and
the measured bounded GPU row layers. Keep the small panel-only constraint scope;
do not claim it fixed perceived motion. The tiny endpoint overshoot stays deferred.
All device work remains emulator-only.

## Change boundary before implementation

Prior real-source emulator traces locate 2.6–4.1 ms in row preparation when 18
rows need recording. The first-build hardware tests already established that
Android can discard no-longer-displayed lists: preserving cache entries cannot
guarantee every newly exposed row keeps a display list. Do not add another cache
or retain offscreen GPU storage to evade that platform lifecycle.

Source inspection of `TerminalView.drawRow` shows every non-empty cell string,
including a single ASCII space, issues Canvas save/clip/drawText/restore and
foreground paint setup. Blank cells are common in the existing terminal fixtures.
Their background and underline remain visible, but the ASCII space itself has
no glyph ink with the existing monospace painter. Empty strings are already
handled separately. This is a **local implementation cost**, not a missing
cross-layer state owner, wire-diff defect or proven cause of the phone's end jank.

Measure the current production painter with a bounded temporary opt-in test:
same fixed row scenes, warmup and forced-cold hardware display-list recordings
before/after; include a dense-text control. Count actual text submissions and
capture pixels for equivalence. Do not equate this local recording microbenchmark
with remote timing, GPU cost or physical-phone FPS.

If reduced work and pixel equivalence are established, change only the existing
glyph branch in `TerminalView.kt` to omit exact single-space glyph submissions
and their foreground setup. Keep every background/underline, cell clipping,
wide/combining text and the painter/cache/source ownership unchanged. Do not
use generic whitespace or blank-row detection. Add focused pixel coverage in
`TerminalRenderingTest.kt` for styled blanks and adjacent actual glyphs. Archive
and remove the temporary benchmark, then run the focused rendering/geometry suite,
build/lint and packaging checks. No unrelated implementation findings, new
animation policy, native allocation sharing, or task archival belongs here.

## Paired result and retained change

An initial 50-warmup run also sped up the unchanged dense-text control, exposing
startup/JIT noise. Exclude those timing percentages. The retained comparison uses
the same isolated method invocation on both APKs, 200 warmup recordings followed
by 100 measured recordings per scene, on the same owned API 36 arm64 Pixel 9 AVD
with SwiftShader. There is no network/host fixture. Width alternates by one pixel
before each timed draw to discard row lists; layout itself is outside the measured
interval. A hardware RecordingCanvas drives the actual `TerminalView` painter.

| Fixed scene, 44 × 40 cells | Text calls before → after | Median before → after | p95 before → after |
| --- | --- | --- | --- |
| Sparse labels/spaces | 1,760 → 484 | 2.142 → 0.862 ms | 2.225 → 0.947 ms |
| Dense text control | 1,760 → 1,760 | 1.286 → 1.297 ms | 1.329 → 1.366 ms |
| Styled blanks/underlines/combining mark | 1,760 → 528 | 2.463 → 1.198 ms | 2.545 → 1.265 ms |

The measured samples record the same number of row lists before/after: 4,400
for sparse/decorated scenes, 2,600 for dense because its 26 unique row contents
share entries. Space text calls drop from 1,276/1,232 to zero; ordinary dense
text work is unchanged. The paired result supports a roughly 60%/51% reduction
in the two space-heavy **local cold-recording microbenchmarks**, not phone FPS,
total animation cost, memory savings or resolution of the final phone stall.

All three 760 × 1,628 hardware PixelCopy PNGs are byte-for-byte identical across
builds. `row-painter-pixels.json` records their shared hashes. The focused new
pixel check passes at 8/12/16 sp, covering foreground attributes, all underline
modes, colored backgrounds, wide/combining neighbors and a space with an accent.

The final product change is confined to the existing glyph conditional and paint
setup/reset in `TerminalView.kt`; no additional cache, source, parser, animation
or thread is added. Ordinary spaces keep their semantic content for Copy/input
and their visible background/decorations. The existing row renderer is byte-for-byte
unchanged from the accepted row-layer build. Keep the optional layout cleanup
without upgrading its perceived-performance claim.

## Artifacts and checks

- The temporary benchmark is archived as `row-painter-probe.patch` and removed
  from the regular test source. `row-painter-before.json` and
  `row-painter-after.json` retain the actual timings/work counts. PNGs remain at
  `/tmp/zterm-row-paint-paired-{before,after}-{sparse,dense,decorated}.png`.
- Paired logs: `/tmp/zterm-row-paint-paired-before-test.txt` and
  `/tmp/zterm-row-paint-paired-after-test.txt`, each `OK (1 test)`.
- Clean wrapper build (`assembleDebug`, `assembleDebugAndroidTest`, `lintDebug`)
  passes: `/tmp/zterm-row-paint-final-build.txt`.
- `TerminalRenderingTest` + `AttachGeometryTest`: **18 passed, 1 skipped**,
  `OK (19 tests)` in `/tmp/zterm-row-paint-final-tests.txt`. The skip requires the
  explicit remote attach fixture; the local painter change does not require it.
- APK signature, all native ELF/ZIP 16 KiB alignments and `git diff --check` pass.
  Native sources did not change in this increment, so prior native checks remain
  applicable without repeating an unrelated test matrix.
- Development APK: `apps/android/app/build/outputs/apk/debug/app-debug.apk`,
  v0.1.31 / 103199, 49,999,725 bytes, SHA-256
  `e68f91b4391d1cbff8dce0b1fd0ae0e8ed588bb25511ec4e755d16a844a421b3`.
  Installed only on the owned emulator for the final suite. The unavailable phone
  retains the previous comparison APK; no phone install or operation was attempted.
- The owned `Zterm_RowPaint_0909` emulator was stopped and its AVD deleted after
  verification. The task remains in progress for phone evaluation and the
  independently recorded follow-ups.
