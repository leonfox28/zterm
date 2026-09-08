# Android upward-scroll handoff — 2026-09-08

User report after v0.1.27: in an attached shell, dragging down looks correct;
dragging up intermittently makes every text row look doubled. Whether the image
persists after release is an optional pending clarification, not a prerequisite
for reproducing the local transition.

Classification: local implementation defect in `TerminalView.drawShift`.
The existing committed source/geometry contract is sufficient; one rendering
path violates it. GestureDetector updates fractional `scrollPixels` immediately,
Repository conflates row requests, native navigation returns semantic rows, and
the View projects them on its next draw. A queued/cached native response still
need not precede the next Android draw.

With cell height 40, current frame offset 8 and local offset 281 px, the shift is
-39 px. An upward drag to 279 px requests offset 7. Before that frame arrives,
the equality guard resets shift to zero: unchanged text jumps down 39 px. When
offset 7 arrives it jumps up 41 px. Downward crossings reach zero before changing
the requested row, explaining the reported directional asymmetry. The Canvas
already covers its background on each draw; selective erase is not this path's
failure mechanism.

Invariant: resolve fractional motion against the offset of the actual candidate
rows. Delayed delivery at a row boundary must not reverse unchanged text. Keep
the local transform bounded to the existing single-row fractional range while
waiting for rows; do not translate a stale page across an arbitrarily large
missing-history gap. Native offset/source and the resulting drawn geometry still
commit together for selection, hit testing and IME consumers.

Expected changes: `TerminalView.kt` owns the correction;
`TerminalRenderingTest.kt` exercises the production Canvas and touch path with
controlled frame delivery; existing `TerminalUiTest` owns real-host gesture
acceptance. Update the Android spec with the demonstrated invariant. No new wire
state, native cache, render cache, debounce, per-program rule or daemon restart.
If evidence shows a different owner, revisit this classification before expanding.

The native `Navigation.rows` already includes one known bottom overscan row via
`visible_rows_with_overscan(offset, 1)`. This existing producer contract supports
the bounded transform, without fetching new rows or inventing an extra cache.

Before-fix emulator evidence: all three new production View tests failed in
0.197 s. With 37 px cells, the same marker row moved **75 -> 111 -> 73 px** across
an upward two-pixel crossing and delayed delivery. Missing-page motion and quick
reversal also failed. Log: `/tmp/zterm-scroll-before-test.log`.

After-fix: the three production View pixel tests pass in 0.222 s, including
downward crossing/reversal and bounded missing-page motion. Android debug/test
APK build and lint pass; JVM task has no sources. Logs:
`/tmp/zterm-scroll-after-test.log`, `/tmp/zterm-scroll-after-build.log`.

Final validation:

- Full `just check`: PASS (`/tmp/zterm-scroll-check.log`); hosted-only platform
  and release checks remain explicitly outside this local run.
- Final test APK + Android lint: PASS (`/tmp/zterm-scroll-final-android.log`).
- `gesturesCopyOverlayAndActivityRetention` with the new actual upward cached
  history assertion, plus `childMouseAndAlternateScrollUseTouchWithoutStealingLongPress`:
  PASS, 2 tests in 39.568 s (`/tmp/zterm-scroll-real-ui.log`). Includes source,
  selection/Copy, mouse/alternate-scroll and Activity/geometry regression checks.
- Emulator: explicit `Zterm_Presentation_0908`, emulator-5556, API 36 arm64,
  1080 x 2424 at 420 dpi. Renderer pixel fixture uses 37 px cells. All host
  activity uses `/tmp/zterm-presentation-fixture-0908` and test-owned Sessions.
- Recording: `/tmp/zterm-scroll-after.mp4`; broad contact sheet and denser
  cropped samples at `/tmp/zterm-scroll-after-contact.png`,
  `/tmp/zterm-scroll-upper-rows.png` and `/tmp/zterm-scroll-upward-rows.png`.
  Inspected samples show distinct rows without stale overpaint. These samples
  are not an all-frame physical-display or jank guarantee; the deterministic
  before/after Canvas test proves the corrected reverse displacement.
- Cleanup: fixture reports no running Sessions, then its explicit daemon stop
  reports `0 sessions ended`. Only owned emulator-5556 was shut down. The real
  Mac daemon/main Session was not accessed, stopped or upgraded.

Review: only local draw-shift arithmetic changes. Existing one-row overscan,
frame/source retirement, resize pan, full Canvas coverage, input/coordinate
epochs, cache budgets and protocol semantics remain owned by their existing
paths. No new delay, queue or program-specific behavior was introduced.

Development APK: `/tmp/zterm-scroll-fix-debug.apk` (separate `.dev` package),
SHA-256 `b78bb2827ce9e66b24fdad3a9c52117f107c29f3eba962d61b73ca92b8923342`.
No new formal release was published. Physical-phone confirmation and the separate
keyboard-close history-refill issue remain open. Work is ready for Phase 3.4's
single follow-up commit confirmation; do not archive the overall task.

Physical development-package update: at the user's explicit request, installed
the above APK with `adb install -r` on the sole connected physical phone (model
2509FPN0BC) on 2026-09-08. Installation succeeded; `.dev` advanced from 0.1.24 /
1014 to 0.1.27 / 102799. First-install time and credential/device-encrypted data
inodes were preserved. The installed base.apk SHA-256 equals the artifact above.
No app launch, remote Session operation, daemon restart, new publication or
physical-phone visual acceptance was performed by this installation step.

## User-confirmed correctness and smoothness review

The user subsequently confirms on the phone that ghosting is gone and the new
build is better, but bidirectional scrolling still does not feel very smooth.
This confirms the reported ghosting correction, not full scroll performance or
the separate keyboard-close acceptance. The user requests reconsideration, not
implementation of a new rendering architecture in this turn.

Confirmed source facts:

- `TerminalView.moveScroll` sends a conflated row request for each movement;
  `OverScroller` plus `postInvalidateOnAnimation` already owns frame scheduling.
- `Navigation.rows` exposes the current viewport plus at most one bottom row.
  Kotlin's bounded transform therefore cannot advance freely through multiple
  cached rows while the next native presentation is pending.
- The Rust cache already has 4,096-row / 16 MiB accounting, initial four-screen
  warmup and 2–8-screen directional prefetch. A cache hit still traverses the
  native actor/frame/FFI/presentation path; more remote prefetch alone does not
  remove that dependency.
- Each actor iteration invokes `project_navigation`; `project_rows` rebuilds
  row/cell records and clones their text. `wait_for_frame` clones the complete
  frame, including when checking an already observed generation. UniFFI lifts
  complete Kotlin rows for the Repository's Main-dispatcher observation coroutine.
- `TerminalView.onDraw` reissues per-cell background/text/clip commands for each
  visible row. `TerminalScreen` also observes every NativeFrame via Compose,
  although pixel motion need not change toolbar or connection state.
- Android Dev is debuggable, but `tools/android/native.py` already compiles the
  packaged Android Rust library with `--release`. Debug ART/Compose overhead is
  a measurement confounder, not evidence that native Rust is unoptimized.
- A read-only attempt to obtain phone gfxinfo found no connected device. No
  new frame-time or bottleneck-percentage measurement is available this turn.

Classification: the ghosting patch remains an appropriate local correctness fix.
Sustained smoothness exposes a separate presentation-boundary limitation: local
motion can depend on a full asynchronous row reprojection even on a warm cache.
The cost split among handoff waiting, FFI/conversion, Compose, UI recording and
GPU execution is still undetermined; measure before choosing extra render caches.

Recommended direction (proposal, not implemented):

1. Give presentation a bounded immutable window of nearby known rows, above and
   below the viewport. Maintain one local pixel position over stable logical row
   identities. Inside that window, finger/fling motion submits at display cadence
   without awaiting another native frame. Replenish before a window edge using
   the existing native cache/query owner; late replies provide content and cannot
   reset the current scroll position. A larger one-row clamp alone is insufficient.
2. Publish changed/new row blocks rather than recreating/transferring a complete
   screen for every local pixel/row intent. Keep lifecycle/input and chrome state
   notifications independent of pure scroll position, with existing immediate
   source retain/release and exact displayed-source selection contracts.
3. If profiling confirms recording cost, reuse row/small-block display lists;
   cursor and selection can be separate overlays. RenderNode supports this on
   API 29+ hardware Canvas; preserve the API 26 floor and software test fallback.
   Do not assume a retained display list means zero GPU raster work. Avoid an
   unbounded full-history bitmap or per-cell View hierarchy.

Constraints/trade-offs: budget Kotlin/display/GPU storage separately from existing
Rust cache accounting; key cached rendering by row identity plus content/style,
font, geometry and epoch. Historical reading stays anchored while output appends;
live-bottom following remains explicit. Width/font/IME/lifecycle changes retire
incompatible windows and coordinate sources. At genuinely missing history, keep
truthful bounded behavior rather than fabricate text. Child-owned TUI mouse/
alternate-scroll continues sending input to the application; its unseen new
screen cannot be simulated by translating an old image.

Validation recommendation: compare the same synthetic shell history on the same
physical device and actual refresh mode with a release-like non-debuggable build.
Use frame timelines plus content-free motion/row-window timing. A timely rendered
frame can still repeat a stopped reading position, so measure motion stalls and
missed frame deadlines separately. Artificially delay native/window delivery
inside already available content; test slow/fast/reversed flings, cache misses,
append/trim, source-aware Copy and geometry changes. Do not use settled screenshots
or a single average FPS as the smoothness acceptance.

Primary references:
- [Android RenderNode](https://developer.android.com/reference/android/graphics/RenderNode)
- [Slow rendering and non-debuggable profiling](https://developer.android.com/topic/performance/vitals/render)
- [Optimize a custom View](https://developer.android.com/develop/ui/views/layout/custom-views/optimizing-view)

### Emulator profiling boundary (2026-09-08)

The user authorized emulator testing after the smoothness review. The remaining
symptom is uneven motion, with the ghosting correction retained. The dominant
cost is still undetermined: measure UI rendering deadlines separately from
committed View positions that wait for native row delivery. The existing smoke
gesture sends only 24 moves over 1,200 ms and is not a smoothness benchmark.

The final opt-in `TerminalScrollProfileTest` lives under Android instrumentation
only. It owns a new Session on the explicit disposable `presentation-fixture`
host and warms 800 static, numbered, densely filled history rows. Real touchscreen
events go through `UiAutomation.injectInputEvent` from the instrumentation thread,
paced near the display's refresh rate, independently of Main. Two rounds cover
stationary redraws, same-row native requests, slow/fast up/down drags, continuous
reversal, and real flings. Releases are sent in the same input batch; delivery of
19–27 distinct native offsets after release confirms that the coast actually ran.

The test collects content-free system FrameMetrics and existing native
generation/offset/cache counters. It adds no production telemetry, reflection,
coordinate observer or API. The existing real Canvas pixel regressions separately
own delayed-row displacement correctness. Screen recording and build work stay
off during measurements. No scrolling architecture, package release or real-user
Session changes are part of this measurement step.

### Accepted measurements

AVD `Zterm_Presentation_0908`, explicit `emulator-5556`, API 36 arm64, 1080×2424,
420 dpi, 60 Hz; 12 sp, 37 px cells, 53×56 grid. Emulator HWUI is `skiagl`, with an
emulated Apple M5 OpenGL/Metal path. This is a controlled emulator comparison,
not a measurement of the physical phone's frame rate or GPU cost.

Both builds contain the existing ghosting fix. The ordinary `.dev` build is
debuggable; the comparison uses the same `.dev` ID, signer and code with Gradle
debuggability disabled. Packaged Rust is release-compiled in both. No production
build configuration is changed or new APK published. The temporary non-debuggable
APK is `/tmp/zterm-scroll-profile-nondebug.apk`, SHA-256
`2df27b83deb73d54df4c2b526affa5715ab42eab448a0cda294a9f8e078bf35c`.

Aggregated over the two rounds, **Window draw duration** in milliseconds:

| Scenario | Dev p50 / p95 | Non-debuggable p50 / p95 |
| --- | --- | --- |
| Stationary terminal, redraw every frame | 17.03 / 23.18 | 8.15 / 13.44 |
| Same row requested every frame | 16.09 / 21.22 | 9.45 / 14.81 |
| Slow drags, both directions | 16.53 / 23.04 | 8.81 / 14.24 |
| Fast drags, both directions | 15.87 / 23.21 | 8.31 / 15.99 |
| Continuous direction reversal | 15.93 / 22.50 | 8.74 / 16.85 |
| Fling gestures and coasting | 17.07 / 28.45 | 8.54 / 14.93 |

Interpretation and limits:

- Re-recording the unchanged terminal alone has material cost. The controlled
  redraw scene has zero native deliveries/history requests, while layout p95 is
  at most 0.20 ms. These are whole-Window draw measurements, not a CPU sampling
  attribution to every `TerminalView` instruction. They support reusing recorded
  row/block drawing and also show that debug overhead amplifies this cost.
- All measured gesture targets hit native cached pages: zero cache misses in
  either build. Some flings and one slow drag also prefetch in the background;
  cache misses and background request counts are distinct. Even cached slow
  scrolling misses system-reported deadlines: Dev 117/167 frames and non-debuggable
  81/210. This does not establish an equivalent phone jank percentage.
- Two 1.2 s same-row controls produce 111 native deliveries in the non-debuggable
  build with no history requests and unchanged row position. Along with the
  actor/projection/FFI source trace above, this confirms work that unchanged
  pixel scrolling should avoid. No precise FFI-vs-Compose cost split is claimed.
- Separately, the existing Canvas tests hold native delivery at a row boundary
  and prove that the corrected View waits at its one-row edge. Native cached
  pages are not yet a freely movable Kotlin row window. The experiment does
  **not** establish the natural frequency or duration of these content stalls.
- FrameMetrics uses each frame's reported `DEADLINE`, excluding first draws;
  do not label `TOTAL_DURATION > 16.7 ms` as the emulator's missed-deadline rule.
  Both accepted runs report zero dropped FrameMetrics reports. Recordings,
  average screenrecord FPS and endpoint images are not timing evidence.

This supports a coherent next implementation: a bounded adjacent-row presentation
window with a stable local reading anchor, reused row/block drawing, and fewer
identical row requests/full frame projections. Keep native semantic/source
ownership, selection, append/trim, geometry and child-owned TUI input contracts.
Tuning fling friction alone cannot remove repeated rendering or row availability
dependencies. Natural motion-stall percentiles and actual-phone acceptance remain
future measurements, rather than invented targets for this emulator run.

### Harness corrections and rejected evidence

Early `/tmp/zterm-scroll-profile-debug.json` and `...-nondebug.json` files dispatched
touches directly from animation callbacks and sampled draw metadata via a posted
OnDraw observer. These are **not accepted motion-stall/lag measurements**:
`postInvalidateOnAnimation` may be deferred another frame when called from that
callback, and posted inspection can mix two phases. An attempted `drawingTime`
fence rejected every sample because ViewRoot updates it during traversal;
`...-nondebug-final.json` has no usable motion samples, not zero stalls. The first
debug harness also delayed UP behind rendering and did not produce real flings.

Simplified the harness instead of adding production sampling hooks: use Android's
input dispatcher for real touch timing, FrameMetrics for system rendering, and
the existing isolated Canvas tests for exact delayed-frame displacement. All
reflection/posted coordinate sampling was removed. Do not quote the preliminary
stalled-frame fractions, pixel lags or reverse-event counts as product results.

### Commands and evidence

Build with `sh tools/android/build.sh :app:assembleDebug
:app:assembleDebugAndroidTest :app:lintDebug`. The comparison adds the temporary
`--init-script /tmp/zterm-scroll-nondebug.gradle`, containing:

```groovy
gradle.beforeProject { project ->
    project.pluginManager.withPlugin('com.android.application') {
        project.android.buildTypes.debug.debuggable = false
    }
}
```

After installing the APK and its matching test APK on the explicit emulator:

```sh
adb -s emulator-5556 shell am instrument -w -r \
  -e class io.github.leonfox28.zterm.TerminalScrollProfileTest \
  -e scrollProfile 1 -e hostName presentation-fixture \
  -e profileLabel real-debug \
  io.github.leonfox28.zterm.dev.test/androidx.test.runner.AndroidJUnitRunner
adb -s emulator-5556 pull \
  /sdcard/Android/data/io.github.leonfox28.zterm.dev/files/scroll-profile-real-debug.json \
  /tmp/zterm-scroll-profile-real-debug.json
```

Use `real-nondebug` for the comparison label. Accepted artifacts:

- `/tmp/zterm-scroll-profile-real-debug-test.log`: PASS, 36.053 s.
- `/tmp/zterm-scroll-profile-real-nondebug-test.log`: PASS, 37.357 s.
- `/tmp/zterm-scroll-profile-real-{debug,nondebug}.json`: raw accepted metrics.
- `/tmp/zterm-scroll-profile-timings.py`: per-phase report; nearest-rank percentiles.
- `/tmp/zterm-scroll-profile-real-input-build.log`: build/test APK/lint PASS.
- `/tmp/zterm-scroll-profile-nondebug-final-test.log`: profile plus all three
  existing Canvas regressions PASS; only its Canvas results are displacement
  acceptance, as explained above.

API references: [FrameMetrics](https://developer.android.com/reference/android/view/FrameMetrics)
and [rendering performance](https://developer.android.com/topic/performance/vitals/render).

Final verification and cleanup: default debug build/test APK/lint pass in
`/tmp/zterm-scroll-profile-restored-build.log`; the emulator is restored to that
ordinary Dev build. `/tmp/zterm-scroll-profile-final-regressions.log` confirms
three Canvas regressions pass and profiling is skipped without explicit opt-in.
`git diff --check` passes. The isolated host reports no remaining Sessions; its
explicit-root daemon is stopped (`0 sessions ended`) and only the owned emulator
is shut down. The user's actual daemon and `main` Session were never operated.
