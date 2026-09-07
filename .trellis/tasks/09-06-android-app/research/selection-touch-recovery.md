# Selection exit and local presentation recovery (2026-09-07)

## Boundary before implementation

Keep desktop CLI → IPC → local daemon → Iroh → host and Android → Iroh →
host. Optimize the proven local presentation path; do not introduce a new
network owner, lifecycle abstraction or protocol change merely to unify platforms.

The reported Herdr failure is in Android `Navigation`: `select_source` changes
the position to History, while `clear_selection` clears only the selection.
`project_navigation` and pointer admission require Live, so Copy/cancel leaves
the TUI frozen and pointer mode None. Keyboard input incidentally starts a full
return synchronization, masking this in the previous pointer regression.

Classification: **local implementation defect**. The existing actor already
keeps the current surface synchronized and owns pointer admission. Its healthy
scroll-to-zero path correctly restores Live locally; selection exit omits that
same transition. No second network/state owner is needed. The new native test
fails on the old implementation at `selection exit must restore child pointer
admission` (`target/android-toolchain/selection-native-baseline.log`).

Expected changes:

- `crates/android/src/terminal/navigation.rs`: share the existing local bottom
  transition between scrolling and selection exit. Restore the current live
  surface when selection ends at the bottom, preserving pinned older history,
  outstanding page correlation and real synchronization/recovery barriers.
- `TerminalUiTest.kt`: cancellation and system Copy must each be followed by
  real taps/wheels without intervening keyboard input. Cover raw mouse mode,
  alternate-scroll mode and an isolated real Herdr instance.
- Native navigation tests: establish failure first, verify current live revision,
  history retention, late query replies and recovery invariants.
- `crates/android/src/terminal.rs`: while reviewing this same presentation path,
  found `project_navigation` first projects every live cell, then discards that
  vector and projects pinned rows again. Choose the actual displayed rows before
  projection to remove one full grid allocation/conversion during selection or
  history reading. Preserve current colors, cursor rules, row ordinals and source
  provenance; verify frozen/current pixels and existing cross-page UI acceptance.
- Update applicable Android/shared-client specs and task evidence.

This also removes unnecessary frozen-page retention and the accidental need for
network synchronization to resume TUI interaction. No desktop process/topology,
pairing, wire framing, protocol deadline or authorization changes are in scope.
Test only task-owned Sessions on emulators; do not operate the user's phone or
running Herdr/Pi Session.

## Validation

- Native old implementation: new regression fails at pointer admission restoration.
- Finalized development APK 1014 + new instrumentation on emulator-5556: fails
  after cancelling selection with a tap, with active transport, no error, epoch 3
  and unchanged 56×53 grid. Exact assertion:
  `cancel restores child mouse without keyboard input` (20-second timeout).
  Log: `target/android-toolchain/selection-ui-baseline.log`.
- Corrected transition: raw mouse/alternate-scroll and isolated real Herdr both
  pass, 2 tests / 25.550 seconds (`selection-ui-fixed.log`). No keyboard command
  between Copy/cancel and child taps/wheels; dismissing tap stays local. Exit
  preserves input epoch, viewport and history query count, with IME hidden.
- Final projection optimization: all 3 `TerminalUiTest` cases pass together in
  63.799 seconds (`selection-ui-final.log`). Includes three-screen selection,
  native Copy, appearance changes, local scrolling, full IME resize, raw pointer
  and isolated Herdr tab switching plus pane scrolling after selection exit.
- All 12 Android native unit tests pass (`selection-native-tests.log`). Covers
  frozen/current rendered glyphs, old reading positions after new output,
  pending older scroll intent, late history replies, idempotent ActionMode exit,
  actual return barriers and disconnect recovery.
- Rust formatting, Android all-targets Clippy (`-D warnings`), terminal dependency
  policy, Debug/Release lint and Debug JVM checks pass. This increment changes
  only Android presentation code/tests and task/spec documents, not desktop
  transport, daemon, shared protocol or host release binaries.

## Delivery

Both versionCode 1015 APKs are finalized under `target/android-apk/1015/`:

- Development: `zterm-dev-arm64-1015.apk`, SHA-256
  `70862658357374af36114f30b5878deca84a759a014a6b92fd69779b0fb7c9a4`.
- Release acceptance: `zterm-android-arm64-1015.apk`, SHA-256
  `152c719d7364c1185acf0604ae5848bf330e0edb2182e25c81f007cb78638938`.

Both signature and 16 KiB native/ZIP alignment checks pass. Development retains
the standard debug certificate; release uses the existing external durable key.
Emulator-5554 updated from 1014 to 1015 with Success; identity and saved-state
hashes remain identical (`dev-install.json`). The dedicated headless test
emulator-5556 was stopped after acceptance. No physical device was operated,
no native release was published and the rejected network helper was not started.
Broader Android task remains in progress; no commit or task archive.
