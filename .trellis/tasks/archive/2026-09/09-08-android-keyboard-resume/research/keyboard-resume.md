# Keyboard dismissal and Android foreground focus

The investigation below led to the approved, implemented window-policy correction.
See [final verification](../verification.md) for corrected-build results and limits.

## Symptom and neutral reproduction

Use an ordinary shell in a disposable Session. Show the IME with the bottom
button, hide it using that button or Android Back, background the app task,
and bring the same task to the foreground. The user reports the IME opening
again. No terminal program or remote output is required.

## Evidence and root-cause classification

**Classification: confirmed local Android window-policy defect.** The existing
explicit-keyboard contract owns this behavior. Runtime evidence identifies
Android automatic editor showing on task re-entry as the violating path, with
no competing application show request or need for a cross-layer redesign.

Code inspected on `main` at `195a9dc`:

1. `TerminalView.kt:151` requests View focus on completed content taps without
   requesting keyboard visibility. `:165` permits focus in touch mode.
2. `TerminalView.kt:545` requests focus and `showSoftInput(SHOW_IMPLICIT)` when
   opening. Hiding only calls `hideSoftInputFromWindow(windowToken, 0)`.
3. `TerminalView.kt:552` always reports a text editor and provides the existing
   epoch-fenced InputConnection. Hidden keyboard and editor focus can coexist.
4. `TerminalScreen.kt:43` observes actual IME visibility. Its Back handler yields
   to Android while the keyboard is visible; the bottom button delegates to the
   View at `:122`. Fixing only the button's hide branch would miss system Back.
5. `AndroidManifest.xml:18` sets `adjustResize` with no explicit state policy.
   `MainActivity.kt:11` installs animation observation, with no resume/show call.
   `TerminalView.kt:524` handles window visibility for gestures/render resources
   without a keyboard restoration rule. No other application `showSoftInput`
   or window soft-input state writer was found.
6. `TerminalUiTest.kt:199` covers keyboard opening/Back, while `:300` backgrounds,
   resumes, and immediately recreates the Activity. The latter asserts Session
   continuity, not keyboard state before recreation; it cannot catch this bug.

Android's official [visibility guide](https://developer.android.com/develop/ui/views/touch-and-input/keyboard-input/visibility)
distinguishes focused editors, show/hide requests, and window state flags.
Focus alone is not a user request to show the terminal IME.

AOSP [ImeVisibilityStateComputer](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/main/services/core/java/com/android/server/inputmethod/ImeVisibilityStateComputer.java)
contains the matching branch: with unspecified visibility, a focused editor,
resize adjustment and forward navigation, `computeState` requests automatic
showing. The device trace below identifies this branch directly, rather than
inferring it from AOSP main. The separate restoration branch was not the
trigger in either observed failure.

## Confirmed runtime reproduction, 2026-09-08

Environment: dedicated `Zterm_Presentation_0908`, serial `emulator-5556`, Android
16 / API 36, build `BE2A.250530.026.F3`, docked Gboard and hardware keyboard
configured off. Rebuilt unchanged product code at `195a9dc` using
`sh tools/android/build.sh :app:assembleDebug`: PASS, 27 seconds. Installed the
resulting development APK v0.1.29 / versionCode 102999 over the existing v0.1.28
acceptance app, preserving its paired test identity. APK SHA-256:
`16f48b7a81b9e6743fbeace7d8471e0bd109d7448ceb22a2e44a83f0b64e47d1`.

The saved host belongs to `/tmp/zterm-reconnect-fixture-0908`. The initially
started older presentation fixture was stopped with zero Sessions after checking
that its host ID differed from the emulator's saved host. The correct isolated
fixture began with zero Sessions; app entry created its disposable `main`.
Cold connection attempts initially timed out; attachment eventually reached
Active through Relay. No user daemon or Session was used.

| Step | Actual IME state | Editor focus | Evidence |
| --- | --- | --- | --- |
| Explicit bottom-button open | Shown | Terminal focused | `button-open` |
| Bottom-button dismissal | Hidden | Terminal remains focused | `button-hidden` |
| Home/background | Hidden | Launcher window focused | `background` |
| Recents card tap returns to same task | Shown again | Terminal focused | `button-recents-return` |
| Android Back dismissal | Hidden | Terminal remains focused | `back-hidden` |
| Home/background | Hidden | Launcher window focused | `back-background` |
| Recents card tap returns to same task | Shown again | Terminal focused | `back-recents-return` |

Returning used the real launcher Recent Apps card, not an `am start` substitute
or Activity recreation. Both returns retain task `t90` and Activity window
`779613e`. The keyboard-hidden View spans y=268..2229, and keyboard-shown View
spans y=268..1378. Actual `mInputShown` transitions false → true and the control
changes Show keyboard → Hide keyboard.

The completed display requests in `dumpsys input_method` are:

- 19:30:08.526 (button dismissal case): `ORIGIN_SERVER`,
  `SHOW_AUTO_EDITOR_FORWARD_NAV`, `PHASE_CLIENT_ANIMATION_FINISHED_SHOW`.
- 19:31:04.440 (Back dismissal case): the same origin, reason and successful
  completion, targeting the same zterm Activity window.

The matching input-start entry is `WINDOW_FOCUS_GAIN` with
`STATE_UNSPECIFIED|ADJUST_RESIZE|IS_FORWARD_NAVIGATION`. The explicit original
button opening has reason `SHOW_SOFT_INPUT`, providing a distinct comparison.
Both dismissals produced `mInputShown=false` before and during backgrounding.
This rules out a failed hide, a terminal reconnection show callback, and the
`SHOW_RESTORE_IME_VISIBILITY` branch for the observed sequence. The defect is
that the window policy still permits automatic editor showing on task re-entry.
Retaining editor focus is intentional for hardware input and is not itself an
error.

Raw timestamped system dumps, UI hierarchies, the temporary observation script,
and baseline build log are local artifacts under
`/tmp/zterm-keyboard-resume-evidence/`. Snapshot labels above prefix `-input_method.txt`,
`-window.txt`, and `.xml`. Physical-phone reproduction and a corrected-build
comparison have not been performed; this evidence establishes the cause and
old-build failure, not completion of the fix.

## Owner, change boundary, and alternatives

The invariant is: dismissal remains authoritative across a foreground transition;
terminal focus is independent of a request to open the IME.

Approved and implemented correction: apply
`SOFT_INPUT_STATE_UNCHANGED` for the Terminal screen's Activity-window lifetime,
preserving `ADJUST_RESIZE` and all unrelated soft-input bits. Capture and restore
the previous policy when leaving Terminal. `TerminalScreen` can use the existing
`LocalActivity` pattern already used in `ScannerScreen`; its disposal owns
restoration. The keyboard button keeps explicit show/hide calls, and the native
View retains editor/hardware-input focus and its InputConnection.

The Android [state documentation](https://developer.android.com/reference/android/view/WindowManager.LayoutParams#SOFT_INPUT_STATE_UNCHANGED)
asks the system to leave visibility unchanged. AOSP's matching state branch
does not execute the confirmed automatic-editor-show branch. This is a
source-backed correction; final runtime checks are recorded in verification.md. `UNCHANGED` is not
per-Activity persisted keyboard state: also test returning from another app
whose keyboard is visible, as well as the recorded Home/Recents path and a
terminal left with its keyboard visible. Do not claim the flag alone preserves
all cases before running these checks.

If those checks expose a real remaining visibility-policy gap, use the existing
actual IME visibility owner for the bounded correction. Avoid a second persisted
keyboard-visible state. Do not replace the proposal with unconditional
`ALWAYS_HIDDEN` across all foreground transitions, which changes visible-state
resume and can affect editable dialogs. Any larger lifecycle-state design must
be written down and reviewed before widening the implementation.

Expected initial product edit: `TerminalScreen.kt` for terminal-scoped window
policy. No blanket manifest/Activity policy or native input rewrite is planned.
Every additional file must serve that same policy. Regression ownership stays
in `TerminalUiTest.kt`; durable contract ownership is
`.trellis/spec/frontend/android-app.md`.

Do not clear editor focus merely to hide the keyboard: hardware input uses it.
Do not retire InputConnections on healthy visibility/geometry changes, synthesize
Back, hide again after arbitrary delays, modify Rust epochs, or special-case
terminal applications. No refactor is planned.

## Implementation and validation plan

1. After final planning approval, activate this task and load Android/shared
   client specs. Use a dedicated acceptance emulator and disposable host/Session.
2. Add the smallest runtime regression for both dismissal paths. Exercise real
   task/window focus loss and return, not only Activity recreation or callbacks.
   Wait for returned focus and settled geometry, and observe stable visibility
   so a late IME show cannot pass prematurely.
3. Automate the confirmed old-build failure and system reason above before
   applying the correction. The root-cause classification is now established.
4. Apply the bounded policy. Verify repeated hidden-state resume, fresh/content-
   focused terminal, explicit reopening/input, Session identity, visible-state
   resume, hardware input, and an editable Session dialog.
5. Run existing focused IME/composition/geometry/content-tap instrumentation.
   Build/check with the command below. The current JVM source set may contain
   no tests; do not report that as runtime acceptance. No Rust tests are needed
   unless evidence changes the scope.
6. Run inline Trellis quality checks, update the Android contract and task
   evidence, and follow phase-3 completion instructions. If neighboring behavior
   fails, roll back only this task's policy edits and retain the regression and
   diagnosis for the next correction.

```sh
sh tools/android/build.sh :app:assembleDebug :app:assembleDebugAndroidTest :app:lintDebug :app:lintRelease :app:testDebugUnitTest
```

The dedicated emulator was started for this investigation after the initial
no-device inspection. No product or instrumentation source was modified. The
initial investigation was kept in planning. Implementation and corrected-build
checks followed the user's later approval.

Investigation-phase cleanup completed: stopped the acceptance app, stopped only the isolated fixture
daemon (one test-owned Session ended), and shut down only emulator-5556. Final
`adb devices -l` is empty. Git contains only the new task artifacts; no product
source change was made.
