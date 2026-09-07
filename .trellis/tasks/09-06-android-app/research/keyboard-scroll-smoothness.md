# Keyboard animation and bottom scroll regression

The user found two movement defects on Xiaomi after installing build 1006.
Emulator acceptance previously checked final geometry and cache bounds, but did
not check intermediate resizes or the synchronization state at the bottom edge.

## Diagnosis and scope

- Keyboard: local implementation defect against the existing coalesced resize
  contract. Compose `imePadding` relayouts the View during the platform animation;
  `onSizeChanged -> measureGrid -> Repository.measure -> NativeTerminal.resize`
  treats each intermediate row count as a new PTY geometry. A conflated channel
  only replaces queued messages; it is not an animation boundary. The native
  actor resets the input epoch and synchronizes these transient sizes.
- Bottom: mobile navigation boundary defect. The actor applies live revisions
  and ACKs them while displaying pinned history, but the scroll-to-zero branch
  unnecessarily starts another full sync before presenting this current surface.
  Visual navigation must use the already synchronized local surface. Selection
  pins and input-triggered return/recovery fences remain independently valid.

Changes are bounded to the Android View/Compose integration, Android native
  navigation/scroll dispatch, their regression tests and task/spec contracts.
No host, wire, desktop, pairing, or persistence changes are needed. There is no
generic debounce timer and no network wait in the healthy visual return path.

## Acceptance

- A complete animated IME show/hide causes exactly one final native resize per
  transition, not one per intermediate row count. Shortcut chrome follows the
  keyboard locally; glyphs/cursor and touch coordinates share one drawing offset.
- A cached scroll to the bottom stays Active, preserves the input epoch, shows
  the latest live surface, and does not wait for a new snapshot/ACK. Late history
  replies cannot replace this surface. Active selection keeps its captured text.
- Recheck real IME input, stale input rejection, rotation/font changes, native
  selection/Copy, history return input and resource bounds on the emulator.
- Physical phone movement acceptance remains pending while it is unplugged.

## User-approved extension: child touch input

After discussing the two motion fixes, the user reported that Herdr ignores
Android taps and swipes. The previous mobile-only SCROLL-2 policy deliberately
consumed all touch locally, including programs declaring mouse reporting. This
is a gesture ownership boundary defect, not a Herdr-specific rendering failure.
Update that policy for the requested interactive terminal use case: a tap on
the active live grid sends one mode-aware click; vertical drags send wheel steps
when the child requests mouse or alternate-scroll. Local history and active
selection still own their gestures, and long press starts native selection
without an earlier remote press. Taps continue to request the system keyboard.

The added boundary is typed Android pointer intent -> native actor -> shared
mode-aware mouse encoding -> existing terminal input. Extract encoding from the
desktop adapter without changing its raw-SGR passthrough or routing. Do not add
process-name recognition, a new wire message, or mouse bytes generated in Kotlin.
Gesture source lifetime, geometry, input epoch and child input mode must remain
valid at native admission; canceled, retired or disconnected gestures cannot
replay. Validate byte-exact modes with a small raw terminal fixture, then use
an isolated Herdr instance as an external smoke test.

## Baseline evidence

Build 1006 real IME show/hide produced 22 observed input epochs (initial plus
21 resize admissions); the new one-resize-per-transition assertion fails.
The native zero-offset regression also fails before the fix. Logs are under
`target/android-toolchain/{ime-baseline-test,bottom-baseline}.log`.

## Full-grid acceptance caught an incomplete first fix

The first candidate used Compose's experimental source/target inset equality to
fence measurement. Empty-screen show/hide passed, but a full 400-line terminal
plus immediate hide produced extra epochs and left a 30-row host behind a
53-row View. Build 1007 failed this regression and is not a delivery build.
The failure was captured in `release-1007-ui.log` and
`keyboard-1007-failure.mp4` under `target/android-toolchain`.

The final owner is the Activity decor View's platform animation callback, above
Compose's consuming callback. Track IME animation objects from prepare to end,
including overlap/cancellation; other inset types cannot clear the fence.
The View measures after layout when the final IME callback retires. Full-grid
normal show/hide now passes with three epochs (initial plus two resizes); add
an interrupted-opening test to require final host/View geometry convergence.


Record-and-test load exposed another missing edge in candidate 1008: the final
animated layout reached 1964 px while the IME fence was still active; after the
animation ended, `requestLayout()` did not yield another native layout callback
through AndroidView. The host stayed at 30 rows while the View had room for 53.
A content-free layout/frame trace (`ime-trace-ui.log`, logcat ZtermImeTrace) proved
this was a missed final commit, rather than a slow network synchronization.
Use a one-shot pre-draw callback after the animation end, fencing intervening
layouts, to commit the final measurement even when pixel bounds do not change.
Candidate 1008 is also superseded; the next delivery must pass with recording.


The pre-draw commit passed full-grid normal show/hide and interrupted opening
under concurrent screen recording: the trace shows 53 -> 30 -> 53 with exactly
two resize admissions and no intermediate geometry submissions. Also corrected
two acceptance-fixture assumptions exposed by recording load: native blank
cells must be rendered as spaces when inspecting fixture text; selection must
hold until the required distance is reached rather than assuming five seconds
always gives three screens. The raw input fixture now acknowledges all preceding
wheel events before asserting that a subsequent local long press emits none.


Gesture fixtures now wait for a native draw before injecting DOWN. Awaiting a
Repository state alone could inject a gesture against the previously drawn
mouse mode; native admission correctly rejected that stale source. The test
must synchronize with actual presentation, not weaken source/mode validation.
The child pointer regression passes after this correction (18.072 s in
`touch-frame-ui.log`). Herdr pane scrolling and its responsive tab chooser/taps
passed in the same final-source recording run; the remaining failed assertion
in that run was this test-only pre-paint gesture race.


Rapid taps exposed a separate actual input loss: a 100 ms interval reproduced
three completed physical taps yielding only two press/release pairs on 1009
(`rapid-tap-baseline.log`). GestureDetector automatically uses the
SimpleOnGestureListener as its double-tap listener; the unused default handler
swallowed the second tap. Explicitly disable local double-tap recognition so
each completed tap reaches the existing child gesture owner. Long press and
scroll ownership remain intact. The fixture now checks rapid taps with a
realistic interval and a child acknowledgment before comparing counters.

Herdr's responsive chooser acceptance also waits for the host viewport to match
the measured View after IME completion. An Active frame immediately after the
animation end can still be the old geometry before the final pre-draw commit;
a DOWN injected then is intentionally retired by the upcoming resize. Waiting
for actual final geometry corrects the test without weakening stale-source
admission.
