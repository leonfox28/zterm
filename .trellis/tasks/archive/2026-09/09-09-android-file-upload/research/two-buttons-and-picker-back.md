# Android upload entry / picker Back follow-up

User explicitly approved replacing the menu with two controls: first a photo
icon opens the image picker, second a paperclip opens the document picker.
The existing menu appears detached from the bottom toolbar in the supplied
screenshot. This is a local presentation change; use the existing icon/button
system and shared retained source/transfer owner. Keep one selection per upload.

Change boundary: TerminalUploadUi (two direct controls and launcher lifecycle),
TerminalScreen (toolbar integration), AppUi (paperclip vector), focused Android
picker/Back regression, task/design/docs/spec synchronization. Do not change the
wire, host store, transfer limits, native terminal or desktop UI for this change.

Picker Back classification: undetermined before reproduction. Android owns the
external picker Activity; determine whether Back is not dispatched, the picker
relaunches after RESULT_CANCELED, or a stale parent Activity was killed by the
preview setup. Do not intercept system Back in unrelated terminal code without
an observed causal failure.

## Observed Back evidence before installing the revised APK

A newly opened document picker returns to MainActivity with one KEYCODE_BACK;
no relaunch occurs. `dumpsys activity activities` also exposes an older
DocumentsUI PickActivity at Hist #0 whose resultTo is a destroyed MainActivity
(`t-1 f`), beneath the current MainActivity. ActivityTaskManager logs tie that
parent destruction to the previous preview's app force-stop. This is stale preview
task state, not evidence of a broken cancellation callback. Clear the development
task for the updated preview, preserve app storage and live host Session, and add
a regression exercising both real picker cancellation paths repeatedly.

## Verification and final classification

The revised APK removes the popup and uses existing photo / new paperclip vectors
in the first two equal-width slots. The terminal's existing Back handler and
Android system picker contract were not changed: a fresh picker cancellation
was already correct. The development task was explicitly relaunched with
NEW_TASK | CLEAR_TASK while preserving app storage and the host Session.

`UploadPickerUiTest.separateButtonsCancelBothSystemPickersWithoutRelaunchOrDetach`
passed on `emulator-5558` against the task-owned `upload-acceptance` host in
12.953 s. It opened File, Image, File, Image through the real production buttons
and real OS pickers, cancelled each with Android KEYCODE_BACK, and verified
returned app focus, cleared pause, same Session/input epoch, enabled controls
and no relaunch. It checks photo-before-paperclip-before-Esc, visible Ctrl and
keyboard controls. The first test attempt timed out because coroutine delays
did not advance Compose's test clock; `ui.waitUntil` fixes the test driver and
the production launcher needed no cancellation workaround.

Android lint/JVM targets, debug/test APK compilation and `git diff --check` pass.
Debug APK SHA-256:
`ff017c9292355cd2eabac18e1d8bd71e1a60ff084e9ccdf71e96a2ed6f09001f`.
No Rust, shared transport, host storage, upload limits or desktop behavior changed.
Keep the emulator and disposable preview host running for the user's inspection.

Preview procedure: return from external picker before restarting/reinstalling
the app. If its parent was forcibly destroyed during preview, reset only the
development task stack; do not claim that stale external Activity demonstrates
a production picker callback failure.

## Reopened report: emulator sidebar Back

The user reproduced the problem after the task reset and clarified that Back
means the emulator's side toolbar button. The earlier stale-task finding was real
but did not establish the cause of this report. Instrumentation KEYCODE_BACK
success does not validate the emulator hardware input route.

The preserved picker is Hist #1 above a live MainActivity in task 140, with a
valid resultTo. `adb emu event send EV_KEY:KEY_BACK:1 EV_KEY:KEY_BACK:0` returns OK
but leaves that picker active; `adb shell input keyevent KEYCODE_BACK` immediately
returns to the same MainActivity. No app restart, reinstall or task clear was
needed. The AVD has `hw.keyboard=no`; getevent/dumpsys input list only gpio power
and virtual touch devices, with no virtio keyboard. The emulator source maps its
Back toolbar action to LINUX_KEY_BACK and creates virtio-keyboard-pci only when
hw.keyboard is enabled.

Classification: local preview environment configuration defect, subject to an
after-change hardware-route check. The owning invariant is delivery of emulator
hardware key events to Android, before any app callback. Correction boundary:
back up and enable the task AVD's keyboard device, cold boot without wiping app
data, and verify the hardware route for both pickers. Keep the existing
show_ime_with_hard_keyboard=1 setting. No product navigation interception or
picker lifecycle workaround is justified by this evidence.

The AVD config was backed up to
`/tmp/zterm-acceptance-config-before-hardware-back.ini` and changed only from
hw.keyboard=no to yes. Cold boot (`-no-snapshot`) retains application data and
now exposes `/dev/input/event12`, named qwerty2, advertising KEY_BACK. The
emulator console's `event send` still does not produce a keyboard event:
source inspection shows that it calls sendGenericEvent, whereas the sidebar
calls sendKeyCode through a SkinEvent. It is therefore not a valid substitute
for clicking the sidebar. Actual sidebar confirmation is pending while getevent
records that newly present keyboard device. Do not report the generic console
command as a successful end-to-end hardware check.

Final verification: the user clicked the actual emulator sidebar and confirmed
that Back works after the keyboard-device configuration change. The guest event
record contains KEY_BACK DOWN/UP pairs on qwerty2, and the app can return through
the terminal to Home. The temporary getevent recorder was then stopped. This
confirms the local preview configuration defect; the APK remains unchanged.

## Approved direction-icon sizing follow-up

The four direction controls currently render 13 sp text glyphs and look smaller
than the 18 dp image, attachment and keyboard icons. The user approved replacing
them with 18 dp vector arrows using the same LineIcon stroke. This is a local
presentation change. Change AppUi (four arrow paths), TerminalScreen (rendering
and accessible labels), English/Chinese strings, the existing TerminalUiTest
text selector, and the task/docs/spec description. Keep key mapping, modifier
handling, button enablement and equal-width 48 dp slots unchanged. Validate with
Android lint/build and test compilation, then inspect the installed emulator UI.

Completed: four 18 dp arrow vectors share LineIcon's 1.7/24 proportional stroke
with the other toolbar icons. English/Chinese direction descriptions replace the
font glyph semantics; the existing TerminalUiTest selector follows the new label.
`sh tools/android/build.sh :app:lintDebug :app:testDebugUnitTest :app:assembleDebug
:app:assembleDebugAndroidTest` passed (JVM target remains NO-SOURCE), as did
`git diff --check`. No new test suite was added for this presentation-only edit.
The installed debug APK SHA-256 is
`ed02f37f2559ecd7a96d997085c34a601ce3e650ce46d61f1bfaf2b52847052d`.
On emulator-5558 at 411 dp width, the production terminal shows all eleven
controls and the four larger vectors without clipping; the hierarchy exposes
Left/Down/Up/Right arrow descriptions. Screenshot:
`/tmp/zterm-toolbar-vector-arrows.png`. The emulator remains connected to the
task-owned upload-acceptance/main Session for review. This check does not add
physical-phone or IME runtime coverage.

Primary source references inspected:
- https://android.googlesource.com/platform/external/qemu/+/emu-master-dev/android-qemu2-glue/main.cpp
- https://android.googlesource.com/platform/external/qemu/+/emu-master-dev/android/android-ui/modules/aemu-ui-qt/src/android/skin/qt/tool-window.cpp
- https://android.googlesource.com/platform/external/qemu/+/emu-master-dev/android-qemu2-glue/qemu-user-event-agent-impl.c
- https://android.googlesource.com/platform/external/qemu/+/emu-master-dev/android/android-emu/android/console.cpp
