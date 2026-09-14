# Remaining IME motion: physical-device investigation

The user reports no perceptible smoothness change from the root-layout cleanup.
That cleanup removed measured redundant composition but is not an accepted fix
for the remaining uneven motion. The remaining cause was initially undetermined;
the physical-device trace below identifies a render-cache boundary gap. Do not
ship another speculative optimization based on the empty-grid emulator timings.

Collect bounded, local Perfetto frame timelines / CPU scheduling on the connected
phone's development build while operating only its observed keyboard controls.
The foreground TUI is the user's existing Herdr/Codex view. Do not type into it,
restart the host daemon, replace its session, or clear application data. Capture
the exact installed baseline and distinguish slow Main work, RenderThread/GPU
submission, IME/app synchronization and discontinuous coordinate updates. Add
temporary content-free app trace scopes/counters only if system traces cannot
distinguish the owning stage, then remove diagnostic hooks before delivery.

The initial product boundary remained unchosen until evidence identified the owner. Candidate
files are the existing View/row renderer and IME/layout owners; shared native row
projection needs separate evidence before expanding the scope. Preserve the
working local bottom anchor, early targets in both directions, retained drawing
handoff, source authority and bounded row caches. No new parser, wire delta,
output debounce, second animation clock or renderer replacement is approved by
this investigation alone.

## Baseline evidence and next boundary

Baseline is the installed 0.1.31 / 103199 development APK, SHA-256
`07179b89c240741041dfcd8fc31733f778d3492d33eb0d02cb1ce1dcdc3b6cf5`.
The phone blocks shell INJECT_EVENTS, so no injected tap occurred. The user
explicitly performed keyboard opening/closing during the subsequent capture.
Perfetto configuration is passed on stdin because its SELinux domain cannot
read `/data/local/tmp`; this uses the supported CLI input path.

The 30-second trace's 64 MiB ring retained the last 16.52 seconds, including
seven complete 419–425 ms IME animations. Initial process metadata was overwritten;
the app is identified by its observed PID and frame layer name. Do not report
the overwritten prefix as analyzed. `/tmp/zterm-motion2-baseline.pftrace` remains
local; analysis outputs are `/tmp/zterm-motion2-frame-analysis.txt` and
`/tmp/zterm-motion2-ime-events.txt`.

Across that retained interval, the app RenderThread's Drawing scope averages
7.25 ms, renderFrame 4.91 ms, and Vulkan command flush 2.08 ms. Main's
AndroidOwner:measureAndLayout averages 0.75 ms; Record View#draw averages 1.75 ms.
RenderThread executes 197,056 AtlasTextOp and 203,296 FillRectOp slices for 1,039
renderFrame calls. FrameTimeline includes 32 App Deadline Missed and 12 Dropped
Frames plus 483 Buffer Stuffing frames among 1,053 application surface frames.
These are trace-window counts, not IME-only counts or device-wide FPS. Tracing
adds overhead; compare using the same capture settings. Stop-the-world GC slices
shown in the trace peak at only 0.064 ms, so they do not explain these costs.

Classification: **render-cache boundary gap**. A row display list caches commands,
but the measured expensive stage still replays/rasterizes its text and rectangles.
The existing row cache is the right bounded owner; no new animation or semantic
state is required. Next increment: ask Android to retain composited pixels for
each existing immutable row RenderNode using `setUseCompositingLayer(true, null)`.
Keep its current dimensions/content identity, bound, cache loss checks and explicit
eviction. Android owns the per-row backing layer; there is no full-history bitmap
or second cell renderer. Selection/cursor/preedit stay outside the cached node.
API 26–28 and software Canvas retain the existing direct painter.

Expected product file: `TerminalRowRenderer.kt` only. Add one focused hardware
pixel-equivalence check to the existing rendering suite for repeated/shared rows,
text/styles and integer movement, plus its unchanged-row reuse checks. Measure
real RenderThread cost and GPU cache memory on the phone before accepting this
tradeoff; retained layers consume more bounded GPU memory. Fractional scrolling
can resample cached text, so check clipping/seams and existing scrolling behavior.
If this does not reduce the measured replay cost, revert this increment rather
than adding another layer. Keep the working keyboard scheduling and handoff.

### Animation-only baseline

`/tmp/zterm-motion2-ime-analysis.sql` filters the app's frame layer and complete
IME intervals. For the seven retained animations, RenderThread Drawing is
4.723 ms mean / 6.365 ms p95, renderFrame 2.895 / 4.051 ms, flush commands
1.596 / 2.233 ms, Record View#draw 1.060 / 2.255 ms, and measure/layout
0.751 / 1.371 ms. There are 357 renderFrame calls, 62,124 AtlasTextOp and
64,266 FillRectOp slices. These are the appropriate paired-animation baseline;
the larger values above include more expensive output between animations.
IME intervals contain 3 App Deadline Missed frames, 11 dropped frames and 151
Buffer Stuffing frames. Baseline memory dumps were taken after the phone stopped
its windows and do not establish live cache memory; capture live memory during
the next user-operated measurement instead of comparing those stopped values.

### Validation findings before delivery

The new same-frame direct-hardware / cached-layer comparison passes at integer
positions for bold, italic, dim, wide/continuation and combining cells plus colored
blanks. Its initial fractional-join oracle was wrong: direct antialiased rectangles
darken the shared join to #333333, whereas the cached row correctly keeps the
declared #444444. The fractional assertion now checks the declared solid background,
not that old seam artifact; text itself can be resampled during fractional motion.

The first suite also failed the real-IME test before the request reached its
target owner. ImeTracker reports PHASE_CLIENT_VIEW_SERVED and InputMethodManager
says the View is not served. A readiness wait made the failure clearer; querying
the actual focused window then identified an unrelated System UI ANR dialog on
the fresh emulator. Layout readiness did not prove window/editor readiness.
After tapping the observed Wait button on that owned AVD, the unchanged IME test
passed (both directions). Do not attribute the original failure to the renderer
or assert that a normal startup race was proven. Keep the test's explicit served
editor precondition. No production input/IME policy changed. Evidence:
`/tmp/zterm-row-layer-tests.txt`, `/tmp/zterm-row-layer-ime-log.txt`,
`/tmp/zterm-row-layer-focused-tests.txt` (row-layer comparison passes), and
`/tmp/zterm-row-layer-ime-final-test.txt` (1 passed after environment recovery).

### Checked comparison build

After recovering the owned emulator, the final full `TerminalRenderingTest` +
`AttachGeometryTest` run is **17 passed, 1 skipped** (`OK (18 tests)`). The skip
still requires an explicit disposable host attach fixture. Log:
`/tmp/zterm-row-layer-final-tests.txt`. Debug app/test APK build, lint, signature,
16 KiB ELF / zip alignment, and diff checks pass. Product change in this increment
is only the three lines in `TerminalRowRenderer.kt`; there are no trace hooks,
new native changes or protocol changes.

Installed comparison APK: development app 0.1.31 / 103199, 49,999,725 bytes,
SHA-256 `b4fcabc2cb0c84da177c47db6fc20ade26818a77b93ded25ed7e359fd183c5cc`.
It replaces the existing phone development APK with data preserved. Paired phone
measurement is the next acceptance step; the pixel/cache tests alone do not prove
the performance tradeoff worthwhile.

### First comparison sample and live memory

`/tmp/zterm-row-layer.pftrace` retains the full 30 seconds but only one complete
421 ms IME animation, near the beginning. The first background capture expired
before its start notice was delivered; it is excluded. The restarted capture
contains the one complete animation below. A subsequent 30-second supplemental
window (`/tmp/zterm-row-layer-repeat.pftrace`) contains no IME animations and is
also excluded. Further user-operated samples were pending at this point; the
confirmed comparison below completes that capture. Do not present this first
sample alone as a stable paired multi-transition benchmark.

Using the identical animation-scoped SQL and trace settings:

| Scope | Baseline mean / p95 ms (7 animations) | First comparison mean / p95 ms (1 animation) |
| --- | --- | --- |
| RenderThread Drawing | 4.723 / 6.365 | 0.868 / 1.205 |
| renderFrame | 2.895 / 4.051 | 0.242 / 0.378 |
| flush commands | 1.596 / 2.233 | 0.392 / 0.561 |
| Record View#draw | 1.060 / 2.255 | 1.371 / 2.540 |
| measure/layout | 0.751 / 1.371 | 0.979 / 1.677 |

The new interval has 51 renderFrame calls, 108 AtlasTextOp and 314 FillRectOp
slices; unchanged row text/background replay is substantially reduced. Its 53
app surface frames include zero App Deadline Missed, two dropped and zero Buffer
Stuffing frames. The unequal sample counts and changing live TUI content do not
support a dropped-frame percentage comparison. Main work is not improved in this
sample. This identifies a useful renderer reduction, not universal smoothness.

Foreground GPU dumps at 8 and 18 seconds both show one active, zero stopped
contexts and the development app's focused main-display window. Total app GPU
cache memory is 64.41 / 62.80 MiB. The 1216×64 row-sized render targets account
for 44 / 39 entries at 311,296 bytes each, or 13.06 / 11.58 MiB; four full-window
render targets account for another 47.75 MiB. These are allocations attributed
by Android, not a measured before/after memory increase. The baseline's stopped
contexts are unsuitable for that comparison. The existing row-count bound and
eviction remain in force; do not sum the gralloc/imported buffer report again
into the GPU cache total.

Evidence: `/tmp/zterm-row-layer-ime-metrics.txt`,
`/tmp/zterm-row-layer-memory-8.txt`, `/tmp/zterm-row-layer-memory-18.txt` and their
focused-window companions. The owned `Zterm_RowLayer_0909` emulator was stopped
and its AVD deleted after the successful focused suite. The phone's installed
comparison APK remains unchanged for further manual evaluation.

### Confirmed opening/closing comparison

After the user explicitly confirmed readiness again, Perfetto started successfully
before the operation notice was sent. `/tmp/zterm-row-layer-confirmed.pftrace`
retains the final 22.759 seconds of the 30-second, 64 MiB recording, with **11
complete IME animations: 5 opening and 6 closing**. The overwritten prefix is
excluded. Each opening has a preceding `IMS.showSoftInput`; each closing has an
`IMS.hideSoftInput` immediately after its animation. Ten durations are 424–427 ms;
one opening is 441.5 ms and contains an early App Resynced Jitter event.

The APK, foreground TUI and capture settings match the comparison plan. Baseline
has 3 opening and 4 closing animations. This is the user's live changing TUI,
not a deterministic content replay; these measurements validate the replay-cost
reduction but cannot establish identical whole-transition workload or untraced FPS.

| Scope | Baseline mean / p95 ms (7 animations) | Row-layer mean / p95 ms (11 animations) |
| --- | --- | --- |
| RenderThread Drawing | 4.723 / 6.365 | 1.164 / 1.726 |
| renderFrame | 2.895 / 4.051 | 0.331 / 0.493 |
| flush commands | 1.596 / 2.233 | 0.501 / 0.748 |
| Record View#draw | 1.060 / 2.255 | 1.432 / 2.942 |
| measure/layout | 0.751 / 1.371 | 1.067 / 1.888 |

Mean RenderThread Drawing decreases about **75%**. Both directions improve:
opening 4.989 → 1.173 ms, closing 4.524 → 1.157 ms. The new intervals contain
564 renderFrame calls, 1,146 AtlasTextOp and 3,408 FillRectOp slices. The existing
row layer is worth retaining for its measured unchanged-content replay reduction.
Main recording/layout did not improve; do not claim that this resolves all jank.
The foreground row-memory observations above still apply to this same APK; this
capture introduces no additional cache or product change.

#### Remaining end-of-animation cost

Among 606 app surface timeline slices, the new capture has 9 App Deadline Missed,
42 Dropped Frame and 48 Buffer Stuffing flags; baseline has 3, 11 and 151 among
368 slices. These counts do not all mean a missed physical refresh. Join each
non-dropped app `display_frame_token` to SurfaceFlinger's display slice (whose
`surface_frame_token` is **NULL** in this trace) and compare distinct presentation
times. The per-animation p95 interval is about 8.305 ms in both builds. The new
capture has eight 16.61–24.92 ms end gaps, versus three 16.61–24.92 ms gaps in the
baseline. It does not demonstrate improved whole-animation presentation cadence.
[FrameTimeline definitions](https://perfetto.dev/docs/data-sources/frametimeline).

Eight of the nine new deadline misses start only 1.9–3.3 ms before the IME ends
(all six closings and two openings). These app frames take 14.58–26.92 ms. During
those windows the longest Main `Record View#draw()` scope is 6.41–12.43 ms;
RenderThread's longest Drawing scope is 2.73–6.20 ms. Baseline's three misses are
also at the end, with 25.01–28.02 ms app duration and 10.18–14.38 ms Main recording.
The ninth new miss is the early jitter event in the longer opening and does not
produce a longer inter-presentation gap in that animation.

Source inspection locates final candidate adoption, missing row recording and
`viewportSource`/source commit inside `TerminalView.onDraw`. Their timing matches
the remaining end-of-animation burst, but the current system trace does **not**
separate their individual costs. Do not blame row recording alone, declare a
layer-caused regression, or add prewarming/background drawing based only on this
correlation. A further fix should first distinguish the existing painter/cache
work from source/geometry commit using temporary content-free scopes, preserving
the current presentation and source-ownership boundaries. The user's perceived
motion feedback remains separate from measured replay savings.

Evidence: `/tmp/zterm-row-layer-confirmed-ime-metrics.txt`,
`/tmp/zterm-row-layer-confirmed-directions.txt`,
`/tmp/zterm-row-layer-confirmed-frame-types.txt`,
`/tmp/zterm-row-layer-confirmed-presentation.txt` and corresponding
`/tmp/zterm-motion2-baseline-*` outputs. The app-layer selection was verified:
the selected app process has one frame layer, its package-qualified main-window
TX layer. No additional APK install, production edit or test rerun was needed
for this read-only comparison.

### Reproduce the measurement

The content-free capture configuration and animation-scoped queries are saved as
`ime-trace-config.pbtxt` and `ime-trace-analysis.sql` beside this note. Set
`ZTERM_TRACE_DEVICE` to the selected device's ADB serial. With the development
app's TUI already foreground and the operator ready, start Perfetto using
`adb -s "$ZTERM_TRACE_DEVICE" shell perfetto --background-wait --txt -c - -o
/data/misc/perfetto-traces/zterm-ime.pftrace` and redirect the configuration file
to stdin. Deliver the start notice immediately after that command returns; a
background recording's window continues to expire while the agent is paused.

Wait for the configured 30 seconds, pull the exact output to a local temporary
file, then use `trace_processor query -f ime-trace-analysis.sql TRACE`. Inspect
`trace_bounds` and complete `InsetsAnimation: ime` slices before comparing; no
keyboard operation or an overwritten interval is missing evidence, not a zero
cost result. The Drawing scope in this phone query includes its observed
1200×2608 dimensions; select the actual app scope on other devices. Take
`dumpsys gfxinfo io.github.leonfox28.zterm.dev` during the active capture and
confirm a live context and the app's focused main-display window. Keep trace,
screenshots and device identifiers out of version control and external services.
`ime-trace-directions.sql` verifies the observed show/hide boundaries and reports
per-direction costs. `ime-trace-presentation.sql` reports deadline-miss positions,
actual display intervals and thread scopes inside those long-frame windows. Its
scope sums can contain multiple overlapping app frames and are not an additive
single-frame CPU breakdown.

### Subsequent user feedback / device constraint

The user reports a small subjective improvement, with a tiny vertical movement
of the keyboard at the end of opening. They have taken the phone away and direct
all further testing to the emulator. The next bounded investigation is documented
in `ime-end-motion.md`; no further phone operations are authorized by the prior
capture/install instructions while this constraint is in effect.
