# Android upload implementation plan

## Prerequisites

- [x] Parent review approved; shared service child validated and API available.
- [x] Activate child, load trellis-before-dev, frontend/android-app, backend/shared-
  client, core-wire-domain, terminal-input-command and relevant source lifetime specs.

## Ordered work

1. Add retained native upload reservation/progress/cancel API and exact-origin input
   pause/insertion, reusing the shared service and native actor execution model.
2. Generate the bridge through sh tools/android/build.sh; maintain dependency policy.
3. Add repository operation/source staging owner with bounded IO, original-epoch
   picker callbacks and process/Activity lifetime separation.
4. Add first photo and second paperclip toolbar controls and single system picker contracts.
5. Add preparing/transfer/finalization/error dialog with size/speed and Cancel/Back,
   explicit retry and focus restoration; bind callbacks to one operation identity.
6. Preserve IME preedit and reject paused/old input without falsely acknowledging
   composition commits. Ensure success insertion bypasses only upload pause.
7. Verify actual runtime scenarios on explicitly selected device/emulator and host.

## Validation

- cargo +1.98.0 test -p zterm-android -p zterm-client
- cargo +1.98.0 clippy -p zterm-android --all-targets --all-features -- -D warnings
- sh tests/terminal-dependency-policy.sh
- just android-check
- just android-build

Run focused Android instrumentation using the project's explicit device/host fixture
pattern; derive the final test class/runner command from new tests, not guessed IDs.
Cover real Image/File pickers and cancel, 50 MB boundary, unknown size, source failure,
dialog speed/size/progress and safe filename display, background/recreation/nav/
reconnect/takeover, pause/no replay, preserved IME composition and insert once.
Verify 11-control toolbar with full IME, narrow width, locales/themes and system
insets. Generated bridge compilation is not Android runtime acceptance.

## Risk and rollback

Risky owners: NativeTerminal actor, AppRepository input queue/epochs, TerminalView
InputConnection, TerminalScreen toolbar/insets and generated bridge toolchain.
Reuse existing state boundaries; do not manufacture reconnect state for upload or
clear preedit merely to disable input. Feature rollback removes picker/dialog/native
upload wiring without deleting completed host files or changing saved identities.
Return to parent planning if the approved source/input/cancel behavior must change.
