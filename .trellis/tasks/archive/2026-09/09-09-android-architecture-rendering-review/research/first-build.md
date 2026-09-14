# First Android TUI row-reuse build

2026-09-09; branch `codex/android-tui-row-reuse`, based on
`a2b518e61544642bb4537935e8fd6c192dff607b`. The user approved a first implementation
of the reviewed local-movement/row-reuse behavior. This is a development build
for feedback, not physical-phone or real-Herdr latency acceptance.

## Implemented boundary

- Hidden-cursor live grids now pan with their bottom edge during local height
  changes. Visible-cursor shell movement keeps its existing history/caret rule.
  Frozen reading/selection, incompatible width and the row cap keep their owners.
- Height-only layout and native geometry changes retire coordinates without
  discarding all text rows. Input-epoch, font/width, background visibility and
  detach invalidation remain.
- The hardware renderer caches resolved cell content plus recording dimensions,
  independently of old row ordinals and soft-wrap metadata. Exact equality follows
  hash lookup. A bounded ordinal identity shortcut avoids repeated row hashes and
  binding allocations on warmed frames. Both maps retain the existing row bound.
- Full frames still compose background, text and current overlays. Existing
  native application/ACK and drawn-source adoption paths are unchanged, including
  the zero-text-difference case. API 26–28/software retain the direct painter.
- No early resize scheduling, native/core/FFI migration, wire change, character
  patching, or unrelated review-defect fixes were included in this first build.

## Regression evidence

New tests were first built against the original product implementation and run
on a task-owned Android 36 arm64 emulator. Both failed at the intended behavior:

| Test | Original implementation | Current implementation |
| --- | --- | --- |
| `hiddenCursorGridMovesLocallyAndMatchesDelayedResizeInBothDirections` | Expected marker y=74 after local shrink; actual y=185, unchanged | Pass; actual software Canvas pixels move with no new native frame and match delayed/reversed resize frames |
| `hardwareRowContentReusesMovedOrdinalsAndIgnoresWrapMetadata` | Expected 3 cumulative recordings after moving two equal rows; actual 5 | Pass; copied equal cells/new ordinals/wrap metadata reuse, style and width changes update, clear restores recording |

`hardwareHeightChangesAndResizeFramesReuseTheMovedRows` additionally exercises
the actual View and hardware PixelCopy. With warmed visible lists:

- local shrink and matching new-size frame add zero text recordings;
- changing one row adds exactly one recording and the expected pixels;
- local growth adds no text recordings for the still-displayed rows;
- restoration produces the exact original image. Newly exposed rows and the row
  whose content really changed may record again; equal rows still displayed are
  reused.

The initial restoration assertion incorrectly required every offscreen old node
to retain its list. Actual hardware invalidated four no-longer-displayed/changed
rows. The final assertion allows these four differences while preserving exact
pixel equality and the unchanged visible rows. This is consistent with the
existing `hasDisplayList()` recovery requirement; no production delay or repaint
workaround was added.

## Final validation

- Android debug application/test APKs built; Kotlin compilation and `lintDebug`
  passed. `testDebugUnitTest` remains `NO-SOURCE`.
- `TerminalRenderingTest` and `AttachGeometryTest` on the isolated Android 36
  emulator: **11 passed, 1 skipped**. The skipped existing attach-race integration
  requires an explicit host fixture. The runner prints `OK (12 tests)` including
  that skip. Existing cached scrolling, delayed windows, append/reversal and
  visible-cursor geometry checks passed with the new implementation.
- APK signature verification and all native-library/zip 16 KiB alignment checks
  passed. Both APKs installed successfully on the disposable emulator.
- `git diff --check` passed. No daemon, user Session or personal device was used.

Commands (from the repository root; use a selected disposable device serial):

```sh
sh tools/android/build.sh :app:assembleDebug :app:assembleDebugAndroidTest :app:lintDebug :app:testDebugUnitTest
adb -s emulator-5580 shell am instrument -w -r -e class io.github.leonfox28.zterm.TerminalRenderingTest,io.github.leonfox28.zterm.AttachGeometryTest io.github.leonfox28.zterm.dev.test/androidx.test.runner.AndroidJUnitRunner
python3 tools/android/check-apk.py apps/android/app/build/outputs/apk/debug/app-debug.apk --sdk /Users/huyuanzhe/Library/Android/sdk
```

Final logs for this local run are `/tmp/zterm-row-reuse-delivery-build.txt` and
`/tmp/zterm-row-reuse-delivery-checks.txt`; original failing evidence is in
`/tmp/zterm-row-reuse-baseline.txt`. The new AVD `Zterm_RowReuse_0909` and emulator
port 5580 belong only to this task and are retired after validation.

## Tryable artifact and remaining acceptance

- APK: `apps/android/app/build/outputs/apk/debug/app-debug.apk`
- Application: `io.github.leonfox28.zterm.dev` (`zterm Dev`), arm64
- Version: `0.1.31`, versionCode `103199`; debug source identity `development`
- Size: 35,854,626 bytes
- SHA-256: `a239452d240511c23c037159f20dd7b1fd6ef79f677b2646970a0987b23a591d`

Real Herdr keyboard animation, the user's phone/IME, API 26–29 devices and
end-to-end remote latency remain unmeasured. Fixtures control View height and
frame arrival; they are not an actual network/TUI benchmark. User evaluation of
this first build remains open, so keep the task active and do not archive it.

## Phone development update

At the user's explicit request, updated the connected physical phone's existing
`zterm Dev` in place with `adb install -r`: version `0.1.27` / `102799` became
`0.1.31` / `103199`. Installation succeeded, first-install time remained unchanged,
and the installed `base.apk` SHA-256 matches the artifact above. No uninstall or
data-clear operation was performed. This verifies phone installation only; the
user's real-Herdr/IME visual evaluation remains pending.
