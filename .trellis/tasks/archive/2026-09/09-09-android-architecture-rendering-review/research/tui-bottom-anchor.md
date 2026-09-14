# Visible-cursor TUI bottom anchoring

## Feedback and causal boundary

After evaluating the first development build, the user reports that opening IME
places the TUI cursor just above the toolbar while hiding content below it.
The application-neutral case is a 12-row live grid with a visible cursor on
zero-based row 9 and footer content on rows 10–11, locally reduced to 9 rows
while the remote frame remains unchanged. Both footer rows must remain above
the toolbar; the old rule moves only one row instead of three.

The user's follow-up asks whether cursor occlusion should instead move the whole
keyboard-height number of rows. The correction uses the actual reduction of
usable terminal height, continuously in pixels during animation. For an alternate
screen, shrinking the viewport triggers movement even while the cursor is still
visible: a footer can already be occluded before the caret reaches the toolbar.
The regression's intermediate 10.5-row height specifically covers this distinction.

Root-cause classification: **architecture / boundary defect**. The first build
used cursor visibility as a proxy for presentation anchoring. Visibility says
whether to paint a caret, not whether the rows below it matter. The host already
publishes `ActiveScreen`, and the native attachment/source retain it, but
`NativeFrame` does not export it to the View. Thus the display owner cannot apply
the existing main/alternate screen semantics and instead relies on a lossy proxy.

## Correction boundary

Export the existing authoritative active screen as a typed native-frame field.
For a live alternate screen, anchor the complete grid's bottom edge regardless
of cursor visibility/position or mouse reporting. Keep main-screen visible-caret
placement and the hidden-caret bottom-edge fallback. Frozen reading/selection,
incompatible widths, row limits and coordinate admission keep their owners.
Remote frames still reconcile using the existing bounded content row cache.

Expected files: `crates/android/src/terminal.rs` for the semantic projection;
`terminal/navigation.rs` for projection/source regression coverage;
`TerminalView.kt` for choosing the anchor, `TerminalGeometry.kt` for its contract,
and the existing `TerminalRenderingTest.kt` for footer pixels, main-screen
compatibility and hardware row reuse. Generated Kotlin must come from the build
wrapper. Update the Android/shared-client specs and task evidence.

This adds no new wire state, parser, process recognition, delay, content scan or
competing state owner. Mouse mode is independent of screen selection. Main-screen
programs are not inferred to be alternate-screen programs from their appearance.
The change corrects the screen-anchoring invariant; it does not claim knowledge
of each application's eventual resized layout.

## Validation plan

- Verify frame screen metadata agrees with the authoritative attachment/source
  across main/alternate transitions, visible/hidden cursors and healthy resize.
- Before changing View policy, run the new visible-cursor/footer pixel regression
  against the first-build anchor rule and record its failure.
- Verify immediate footer placement at intermediate/final local heights with
  remote delivery held, followed by an identical authoritative resized frame.
- Preserve the main-screen caret rule, including independently enabled mouse
  reporting, plus the existing hidden-cursor and frozen-history coverage.
- Exercise the visible-cursor alternate screen in the hardware height-change
  test: no text recording for local movement/equal rows; one for a changed row.
- Build, run native and focused Android checks, lint, signature and APK alignment
  validation. Record physical-device installation separately from emulator proof.

## Results

Implemented the typed `NativeFrame.active_screen: NativeActiveScreen` projection
and regenerated the Kotlin bridge through `tools/android/build.sh`. The View now
supplies a caret anchor only for `MAIN` with a visible cursor. The existing
bottom-edge formula handles alternate screens and hidden-cursor main screens.

- Baseline: the new footer test failed against the first-build View rule. At the
  intermediate local height, the footer marker was completely absent from the
  Canvas. Log: `/tmp/zterm-tui-anchor-baseline-test.txt`.
- Corrected build: `TerminalRenderingTest` plus `AttachGeometryTest` on the
  isolated Android 36 arm64 emulator yielded **12 passed, 1 skipped**. The skipped
  test requires an explicit real-host attach fixture; the runner's `OK (13 tests)`
  includes that skip. The footer, main-screen mouse/caret, delayed-frame pixel
  equivalence, hardware row reuse, existing hidden-cursor movement, history and
  geometry cases passed. Log: `/tmp/zterm-tui-anchor-final-tests.txt`.
- `cargo test -p zterm-android --lib`: **18 passed**, including the new screen /
  cursor / healthy-resize projection regression. Log:
  `/tmp/zterm-tui-anchor-native-tests.txt`.
- Debug application/test APK build and `lintDebug` passed. `testDebugUnitTest`
  remains `NO-SOURCE`. Log: `/tmp/zterm-tui-anchor-final-build.txt`.
- APK signature and all native-library/zip 16 KiB alignment checks passed;
  `git diff --check` passed. Both APKs installed successfully on the test emulator.

Development artifact:

- Path: `apps/android/app/build/outputs/apk/debug/app-debug.apk`
- Application: `io.github.leonfox28.zterm.dev`, arm64
- Version: `0.1.31`, versionCode `103199`
- Size: 49,999,725 bytes
- SHA-256: `969d9deed5e1f01fea968375fc2d6cc1a2b99d8d6f06d42830e3b3e19bc462d5`

No physical phone was connected during the correction checks. The subsequent
phone installation is recorded below.
Real Herdr and the user's IME still require visual evaluation; the local fixture
is proof of anchor/reuse behavior, not a remote TUI layout or latency benchmark.
The test-owned emulator/AVD `Zterm_TuiAnchor_0909` is retired after validation.
Keep the task active for the user's prototype feedback.

## Corrected build installed on phone

At the user's request, installed this corrected artifact over the connected
phone's existing `zterm Dev` using `adb install -r`. Installation succeeded.
The version remains `0.1.31` / `103199`; installed `base.apk` SHA-256 exactly
matches `969d9deed5e1f01fea968375fc2d6cc1a2b99d8d6f06d42830e3b3e19bc462d5`.
Last-update time advanced to 2026-09-09 20:52:00 and first-install time remained
unchanged. No uninstall or data-clear operation was performed. This confirms
delivery of the correction, with real TUI/IME visual evaluation still open.
