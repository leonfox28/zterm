# Investigation and approved correction

## Completed investigation

- [x] Create task, record supplied APK and user's 26-key layout.
- [x] Build unchanged mainline app/test APK and existing isolated host fixture.
- [x] Create an independent API 36 arm64 AVD; install/select supplied Doubao.
- [x] Test actual Pinyin taps and candidate selection in ordinary text input.
- [x] Identify the inline-display trigger and reproduce terminal corruption.
- [x] Capture host bytes, screenshots, video and read-only JDI callback order.
- [x] Classify the defect; document workaround, design and acceptance conditions.

## Approved product implementation

- [x] User approved the reviewed correction and requested a new branch; created
      `fix/android-ime-composition` before editing product code.
- [x] Reload `trellis-before-dev`, Android app and shared-client contracts.
- [x] Add the editor-contract regression described in design.md and establish
      failure on present code.
- [x] Correct buffer metadata, editing and batch selection publication in the
      existing input owner.
- [x] Cover explicit finish idempotence, deletion/Unicode, rejected admission,
      retired connections and healthy resize.
- [x] Repeat both Doubao display modes using real taps: no prefix, exactly one
      Chinese commit and no spurious trailing space.
- [x] Run focused existing IME/lifetime instrumentation and Android lint.
- [x] Update Android spec and perform `trellis-check` over the final scope.

## Validation and rollback

```sh
sh tools/android/build.sh :app:assembleDebug :app:assembleDebugAndroidTest
cargo build -p zterm-cli --example presentation_fixture
sh tools/android/build.sh :app:lintDebug :app:testDebugUnitTest
```

All build/check commands and post-fix regressions passed; details are in
[verification.md](./verification.md). Use the explicitly paired disposable `presentation-fixture` host
and exact emulator serial; never redirect tests to a user's active Session.

For callback observation, compile `research/ImeTrace.java` with JDK 17. Forward
a free local TCP port to `jdwp:<debug-app-pid>` using explicit emulator serial,
then run `ImeTrace <port>`. ART arguments can be unavailable; use recorded buffer
state and return values. Detach before final visual captures and remove the
forward. This observer makes no target-code changes or target method invocations.

Rollback affects only the Android code/tests/spec. Preserve identity and
state. Release publication, physical phone work and user-session takeover are
outside this investigation.
