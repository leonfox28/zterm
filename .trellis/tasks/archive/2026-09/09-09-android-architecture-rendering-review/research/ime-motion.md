# IME motion cost and closing target

The phone user confirms the resize handoff is substantially better. The remaining
report is slight unevenness while opening, and a request to overlap closing with
the remote resize too. Closing already uses the same signed endpoint calculation;
the existing real-system-IME test only checks opening.

## Investigation / change boundary

First measure layout and drawing separately. `TerminalScreen` currently wraps the
entire terminal in `BoxWithConstraints` below `imePadding`, solely to calculate
the optional session panel's maximum height. Thus every animated constraint can
enter subcomposition even when the panel is absent. This is a candidate **local
layout-phase ownership defect**, not evidence of a row-diff or transport problem.
Use a temporary, host-free diagnostic on the actual production screen to count
composition and collect Android FrameMetrics during real opening/closing. Its
empty terminal isolates chrome/layout cost; it does not measure Herdr, network,
native row projection, or phone FPS. Remove diagnostic product hooks afterward.

If confirmed, replace the root with a normal Box and limit constraint-dependent
composition to the visible panel. Expected product boundary: `TerminalScreen.kt`
only. Keep the same panel height rule and the existing `TerminalGridLayout`
measurement/inset/early-target owner. Extend the existing real-IME test in
`TerminalRenderingTest.kt` to assert early closing and final geometry too.
Keep current local pan, frame/source handoff, row caches, native parser and wire
unchanged. Do not add another animation clock, output delay, or bitmap layer.

Validation: baseline/corrected diagnostic; both real IME directions; existing
pixel/hardware handoff regressions; Android build/lint/APK checks. Deliver a new
development APK if the change is supported, preserving phone data. Keep this task
open for the user's actual TUI evaluation.

## Findings and implemented boundary

Confirmed **local implementation defect**: the grid already owns animated
measurement, but its outer subcomposition scope unnecessarily depends on the
same changing constraints for an absent panel. The correction is a normal root
Box plus a BoxWithConstraints only inside `state.panel`. The panel receives the
same consumed-inset parent bounds and retains its 65% / 420 dp maximum and 48 dp
top padding. No new state or animation owner is needed.

The temporary diagnostic used the real `TerminalScreen`, fixed detached state,
the system IME and an otherwise empty test-owned Android 36 arm64 AVD. It counted
the root content's SideEffect and AndroidView's update lambda, and collected
FrameMetrics. One warmup pair and three measured open/close pairs gave:

| Observation | Previous root | Corrected root |
| --- | --- | --- |
| Opening content compositions per transition | 3, 3, 3 | 1, 1, 1 |
| Closing content compositions per transition | 2, 3, 2 | 1, 1, 1 |
| AndroidView update calls in all measured transitions | 0 | 0 |

The remaining one composition follows the keyboard visibility change. This
removes measured redundant composition; it does **not** mean the previous code
rebuilt all controls or reprojected terminal rows on each inset. The actual View
update was skipped. FrameMetrics did not establish a reliable latency improvement:
this SwiftShader AVD delivered only 2–4 frames in most transitions, with large
and inconsistent timing (e.g. measured layout medians 9.64–67.18 ms before and
0.09–76.16 ms after). Do not report these as phone FPS or a speedup percentage.
Drawing/cached rows and native live-content cost are outside this empty-grid
diagnostic. Phone perception remains the acceptance for this incremental change.

Evidence: `/tmp/zterm-motion-baseline-test.txt`,
`/tmp/zterm-motion-corrected-test.txt`, `/tmp/zterm-motion-paired-metrics.txt`.
Both diagnostic executions passed. The archived harness is `ime-motion-probe.kt`.
To reproduce on an owned AVD, temporarily copy it to androidTest as
`TerminalMotionProbeTest.kt`, add the package-local object
`TerminalMotionProbe { var compositions = 0; var updates = 0 }`, increment
`compositions` in a SideEffect directly inside the root Box/BoxWithConstraints,
and increment `updates` in the production AndroidView update lambda. Build and
run only `TerminalMotionProbeTest`. All product counters/hooks and the diagnostic
test entry were removed before the delivery build; the archived file is not part
of the application or regular test suite.

The existing real-system-IME target test now checks both opening and closing.
It passed: each direction submits exactly one final target while the old View
height still differs, and the target equals the settled grid. This confirms the
existing symmetric scheduling in the actual Compose/IME hierarchy; no new
closing-only resize path was added. Focused log:
`/tmp/zterm-motion-both-directions-test.txt` (1 passed).

## Final build verification

- Production source contains only the small root/panel layout change for this
  increment; all temporary counters and diagnostic entry points are removed.
- Debug app/test APK build and lint passed through `tools/android/build.sh`.
  Log: `/tmp/zterm-motion-final-build.txt`.
- The final APK's `TerminalRenderingTest` + `AttachGeometryTest` suite reports
  **16 passed, 1 skipped** (`OK (17 tests)`). The skip is the existing test that
  requires an explicit real-host attach fixture. The run includes actual pixels,
  hardware row reuse, resize-only handoff, reversal, and real opening/closing
  targets. Log: `/tmp/zterm-motion-final-tests.txt`.
- APK signature, all native ELF / zip 16 KiB alignments, and diff whitespace
  checks passed. No native code changed in this increment.

Artifact: `apps/android/app/build/outputs/apk/debug/app-debug.apk`, development
application `io.github.leonfox28.zterm.dev`, version 0.1.31 / 103199,
49,999,725 bytes, SHA-256
`07179b89c240741041dfcd8fc31733f778d3492d33eb0d02cb1ce1dcdc3b6cf5`.

## Phone delivery

Installed over the connected phone's existing development app with `adb install
-r`; Android reported success. The installed base.apk hash exactly matches the
artifact above. Last update: 2026-09-09 21:46:59; first install remains
2026-09-07 11:51:23. No uninstall/data clear, host change, or user-terminal input.
The owned `Zterm_Motion_0909` emulator is stopped and its AVD retired. Keep the
task active for phone TUI motion feedback; this increment is not an assertion of
universal smoothness or remote child-redraw completion.

## Subsequent phone feedback

The user reports essentially unchanged perceived smoothness after this layout
cleanup. Keep the proven composition reduction, but do not describe it as fixing
the remaining motion problem. The subsequent physical-device replay-cost
investigation and row-layer comparison are in `ime-phone-trace.md`.
