# Runtime acceptance — 2026-09-08

## Isolation and environment

Baseline `be66a16`; implementation branch `feat/terminal-presentation-continuity`.
All host operations use `presentation_fixture` with an explicit new root
`/tmp/zterm-presentation-fixture-0908`. The helper refuses ambient paths and daemon
startup from CLI operations. The real macOS daemon and `main` Session are untouched.
New AVD `Zterm_Presentation_0908`, serial `emulator-5556`, API 36 Google APIs arm64,
Pixel 9, 1080 x 2424 px at 420 dpi, hardware keyboard disabled, docked Gboard.
The AVD has only the disposable paired host `presentation-fixture`.

## Completed checks

- `just check`: PASS, full workspace format, Clippy, tests, docs, dependency/source
  policy and relay static gate. Hosted-only jobs remain identified by that command.
- Android assembleDebug + assembleDebugAndroidTest: PASS.
- Android lintDebug + testDebugUnitTest: PASS (JVM test task has no sources;
  geometry arithmetic executes in Android instrumentation).
- Host focused terminal model tests, driver 11 tests, Session 73 (1 ignored),
  pending marked history + input/snapshot ACK progress: PASS.
- Native Rust Android tests 13; desktop terminal UI 72 (3 subprocess helpers ignored
  directly and invoked by their parent tests): PASS.
- `AttachGeometryTest`: integral 17/23/29/41 px endpoints and smooth remainder,
  tiny clipped window, direct animation events/overlap/disposal: PASS.
- `NativeTerminalTest`: Unicode, retained history and multi-screen selection,
  healthy resize input, A-B-A stale coordinate rejection, hidden cursor positioning,
  rename/detach/owned-Session cleanup: PASS. A strengthened repeat sends the text
  immediately after resize instead of waiting for Active: PASS (18.23 s).
- `keyboardPresentationAndCompositionStayContinuous`: PASS (21.634 s).
  Stable font 14 produces 44 px rows: closed 1936 px = 44 rows, open 1144 px = 26 rows.
  Original InputConnection preserves Chinese preedit across IME open/close;
  finish twice still commits once, every observed healthy frame admits keyboard input.
  Actual screenshot pixel patches match after host resize for high cursor (crop),
  bottom cursor (18-row pan), and marked alternate-screen TUI (restored header plus
  unchanged body). Real final clear becomes an empty display.

## Defect found by runtime testing

The old Compose-driven animation state handoff could conflate prepare/end before
recomposition. Native layouts were correctly fenced while the Activity owner was
running, but no final native measurement followed, leaving host dimensions stale.
Direct `ImeAnimationState.observe` events now drive final pre-draw measurement;
layout reads the live owner at measurement time. A deterministic overlap/coalescing
regression plus repeated real keyboard open/close verify the repair.

## Artifacts (local only)

`/tmp/zterm-before-high.mp4` is a valid old-build high-cursor recording. The attempted
old low-cursor fixture lost characters during keyboard-triggered resize and is not
accepted as a valid baseline scene. `/tmp/zterm-presentation-after.mp4` records the
successful new high/low/TUI/Chinese/final-clear run. Screenshot caches are named
`continuity-{high,low,tui}-{before,open}.png` and `continuity-final-clear.png` in the
app cache; they contain only the owned synthetic scene. No APK/video is committed.
The 2 fps contact sheet checks scene order and final clear, not every physical frame
or frame-budget timing. No quantitative CPU/GPU/jank or universal zero-flicker claim.

## Extended acceptance

- `gesturesCopyOverlayAndActivityRetention` and
  `childMouseAndAlternateScrollUseTouchWithoutStealingLongPress`: PASS, 64.035 s.
  Includes reversed IME animation; history and three-screen Unicode selection/copy;
  copy cancellation; Activity retention/recreation; portrait/landscape; fonts
  8, 9, 13, 16 and 12; mouse, wheel, alternate scroll and stale pointer sources.
  Observed cache peak was 8,636,121 bytes (below the existing 16 MiB budget), with
  1,537 retained rows. This is fixture evidence, not a full memory/performance benchmark.
- Repeated the high/low/marked-TUI/Chinese/final-clear pixel test at 360 dpi, font 9,
  three-button navigation: PASS, 22.65 s. Cell height 24 px; closed View 1944 px
  includes the existing cap of 80 content rows plus 24 px neutral excess. Open View
  1296 px = 54 rows. Captures at `/tmp/zterm-presentation-evidence-360-9`; first run
  retained at `/tmp/zterm-presentation-evidence-420-14`. Restored 420 dpi/gesture nav.
- Explicit `measurementsDuringAttachReachTheNewTerminal` isolated startup race:
  PASS, 8.782 s. Final geometry tests including the exact row-39/578-px fixture:
  PASS, 2 tests. Final Android lint/test APK build: PASS.
- Added desktop Main/Alternate high/middle-cursor shrink-to-authoritative handoff
  equality, hidden cursor guard and unknown-width guard. Full desktop UI suite:
  72 passed, 3 isolated subprocess helpers ignored directly. Final exact test and
  CLI Clippy/format checks: PASS after the final test adjustment.

## Unavailable and deferred

The synthetic TUI is application-independent and exercises a marked full redraw
with fixed top tabs; a new Herdr-specific run was not performed. Tiny windows are
covered by the arithmetic/clipped-minimum contract, not a foldable/multi-window device.

Desktop GUI screenshot access was rejected by the Computer Use tool for iTerm;
no successful desktop visual recording is claimed. The attempted fixture window
startup was canceled before any desktop fixture Session existed. Desktop correctness
currently has presenter byte, flush/failure and isolated real-PTY/Session evidence.

## Cleanup and review

Closed the remaining owned baseline Session by exact fixture root and name; all
other test Sessions were closed in test teardown. Gracefully stopped only the
independent fixture daemon (`0 sessions ended`) and shut down only emulator-5556.
The created AVD, explicit host fixture state and local evidence remain available
for reproduction. Existing emulator-5554, the real macOS daemon and `main` were not
stopped. No release, remote write, push or product-state migration occurred.

Inline quality/spec review covered publication/processing clocks, exact-origin ACKs,
pending history cancellation and authorization recheck, keyboard versus coordinate
lifetimes, frame/source retention, renderer flush commit, and IME observer disposal.
No wire schema/dependency or new replay path was introduced. Existing queue/cache
budgets remain bounded. Six owner specs capture the changed contracts.

## Follow-up: what the flicker evidence actually proves

The user's follow-up asked whether flicker improved. Review of the saved recordings
found that the endpoint pixel assertions do NOT verify every intermediate frame.
They compare selected unchanged patches after resize has settled; the closing path
primarily checks final geometry/input lifetime. There is no controlled before/after
flicker measurement. The old valid high-cursor recording uses a different font/theme
from the new run; the old low-cursor baseline is invalid as noted above.

A denser sample of the new recording around 10.5–13.5 s shows the high-cursor
keyboard-close transition: visible rows move down, the newly exposed top area is
initially empty, then history fills it. This is an observed residual discontinuity,
not evidence of a whole-screen clear. The sample uses 12 fps extraction from the
variable-frame-rate recording, so repeated images are not a physical jank metric.
Artifacts: `/tmp/zterm-after-high-close-frames.png` and
`/tmp/zterm-before-high-all-frames.png` (all decoded baseline frames, unused tile
slots are black and are not terminal frames).

Current conclusion: geometry handoff and healthy input improvements are verified;
reduction/elimination of transient flicker is NOT yet accepted. Whole-transition
acceptance, especially closing/history refill, remains open before this feature
should be described as providing a fully continuous visual experience. Actual
Herdr and desktop GUI visual evidence remain unavailable/unperformed. No additional
product changes, runtime restart, or commits were made during this evidence review.
