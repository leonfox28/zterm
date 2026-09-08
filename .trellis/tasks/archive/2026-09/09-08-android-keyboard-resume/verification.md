# Keyboard resume verification

## Implemented change

`TerminalScreen` uses the Activity window's `SOFT_INPUT_STATE_UNCHANGED` while
composed. It preserves resize/other flags and restores the previous visibility
bits on disposal. `TerminalView` focus, InputConnection, and explicit button
show/hide calls are unchanged. The user approved this proposal on 2026-09-08.

## Runtime environment

Dedicated Zterm_Presentation_0908, emulator-5556, Android 16/API 36, docked Gboard,
1080x2424/420dpi. Only the explicitly paired isolated host at
`/tmp/zterm-reconnect-fixture-0908` was started. Tests create/close their own
Sessions. The user's device and real daemon/Sessions were not used.

## Recorded checks

| Check | Result | Evidence |
| --- | --- | --- |
| New regression, unchanged product code | Expected failure, 16.895 s: hidden-before-resume becomes shown | `/tmp/zterm-keyboard-resume-evidence/regression-before.log` |
| Initial corrected-build regression | Resume cases pass; additional virtual-key injection assertion fails because Gboard explicitly shows itself | `regression-after.log`, `after-input-method.txt` |
| Final regression | PASS, 1 test, 38.7 s | `regression-final.log` |
| Debug app/test assembly, Debug/Release lint | PASS, 18 s | `fix-checks.log` |
| JVM test task | NO-SOURCE; no JVM runtime claim | `fix-checks.log` |
| Existing IME/composition/geometry and child-pointer checks | PASS, 2 tests in a three-test run; the other test failed before attachment | `existing-ime-gestures.log` |
| Gesture/Copy/overlay/Activity retention retry | PASS, 1 test, 52.348 s | `gestures-retry.log` |
| Final test-source lint | PASS, 3 s | `final-lint.log` |

The initial corrected-build failure was a test assumption: `sendStringSync`
uses a virtual keyboard, and the system trace records Gboard's own
`ORIGIN_IME / SHOW_SOFT_INPUT_FROM_IME` request. The test still verifies that
key events reach the retained focused terminal, and observes keyboard visibility
across task return before injecting keys. It allows the IME's separate response
to typing, dismisses it if shown, and continues explicit-reopen/visible-resume
checks. No product workaround was added for virtual input.

The final test covers fresh entry, button and Android Back dismissal, two return
cycles for each path, unchanged Session and input epoch, retained focus and real
injected key events, visible-before-background restoration, Chinese IME commit,
content taps and restoration of the prior window policy on leaving Terminal.

## Cross-app and dialog checks

Manual checks used the same fixed APK and the existing isolated `main` Session.
Before switching, the bottom button opened/hid the terminal keyboard, and XML
confirmed native terminal focus remained true. Android Settings search then
owned a focused EditText with `mInputShown=true`.

- Returning through the real Recents card preserves the original zterm Activity
  record/window (`72449748` / `949cb58`, task `t103`), terminal focus, and hidden
  IME. Evidence prefixes: `crossapp2-terminal-hidden`, `crossapp2-settings-editor`,
  `crossapp2-recents-return`.
- Direct task return with an explicit single-top intent also retains that exact
  Activity and hidden IME: `crossapp3-settings-editor`, `crossapp3-direct-return`.
- New Session dialog: tapping Name shows the IME, actual injected text reaches
  the editable field, and Cancel restores hidden IME. No Session was created by
  this dialog check. Evidence: `dialog-editor-focused`, `dialog-input`,
  `dialog-canceled`.

An earlier manual return without single-top created another Activity in the
same task. It is retained as `crossapp-return-hidden` evidence but excluded from
resume acceptance. Same-task identity alone is not sufficient proof; check the
Activity/window identity too.

## Transient fixture failure and final result

The first three-test existing-regression run reported 2/3 passing in 110.094 s.
`gesturesCopyOverlayAndActivityRetention` hit `transport_unavailable` while
creating its fixture Session, before testing gestures (frame=null, sessions=0).
After stopping the acceptance app and restarting only the disposable daemon,
waiting for Network: online, the unchanged test passed in 52.348 s. No product
network logic, deadline, retry policy, or existing test was modified.

All four unique relevant instrumentation methods now have passing final runs.
This is not a claim that the initial aggregate run passed. Final full-scope
inline review covers all three changed paths: Android UI policy, the regression,
and the Android guideline. No Rust/wire/storage/identity code changed. The
Window effect has a matching disposal, preserves unrelated bits, and adds no
keyboard-state copy, focus change, input restart, or background connection work.

Physical phone and physical external-keyboard hardware are not tested; key-event
routing is covered by instrumentation on the emulator. The debug APK is v0.1.29 /
102999, SHA-256 `30349d43f096d4a7b8e0028cbbaf1ad693bfd5254fbedb3d1e3ba88b947a8cb0`,
at `apps/android/app/build/outputs/apk/debug/app-debug.apk`. No release publication
or installation on the user's phone was performed.

## Handoff

Cleanup completed: stopped the acceptance app, gracefully stopped the isolated
daemon (one test-owned Session ended), and shut down emulator-5556. The user's
daemon, Sessions and physical device were untouched.

Implementation, spec sync and quality verification are complete. On 2026-09-08,
the user authorized committing the approved batch, pushing
`fix/android-keyboard-resume` and creating a PR. See `commit-plan.md`.
Archive/journal steps follow the work commit; delivery metadata is in `task.json`.
