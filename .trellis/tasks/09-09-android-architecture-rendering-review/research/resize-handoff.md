# Resize snapshot and TUI repaint handoff

## Reproduction and classification

Phone feedback after the bottom-anchor correction confirms the local movement
amount, but reports a one-row downward jump (the previous cursor-anchored image),
followed by flicker and recovery when remote content updates.

`resize-handoff-probe.rs` reproduces the positional sequence using the actual
host `TerminalModel`, without a process or network. A 12-row alternate screen
has a visible cursor on row 10 and one footer row. Reducing it to 9 rows gives:

| Presentation | Row labels |
| --- | --- |
| Android's immediate bottom-anchored local movement | 03 through 11 |
| Alacritty resize before child repaint | 02 through 10, cursor on row 8 |
| Completed child repaint | 03 through 11, cursor on row 7 |

The host's `TerminalDriver::resize` commits/publishes model resize before later
PTY output can acquire the commit owner. `Grid::shrink_lines` scrolls only enough
to expose the caret. Android currently requests this only after IME completion
and immediately displays every resolved frame. A synchronized-output marker
around the child's repaint correctly holds its partial clear, but holds the
already-published resize-only image. Snapshot/Active/first output are not proof
of completed child layout. The probe log is `/tmp/zterm-resize-handoff-probe.txt`.

Classification: **presentation handoff boundary defect**. The approved design
already separated the displayed baseline from the newest prepared candidate,
but the initial increment implemented row reuse and local anchoring only. A
new authoritative geometry was still immediately treated as the drawing's
geometry. Exact row or cell diff cannot suppress actual intermediate positional
changes. The earlier synthetic tests omitted the resize-only stage.

## Next increment boundary

Implement the previously planned early target scheduling and retained IME
presentation together, in Android only:

- `TerminalGridLayout` owns usable pixels/chrome and existing source/current/target
  insets. During a live alternate-screen IME animation, pass a known final usable
  target to `TerminalView`, calculated from these measured values rather than
  from a remembered keyboard height. Unknown endpoints retain final measurement.
- `TerminalView` sends only changed final grid intent through the existing
  repository resize owner. Final layout verifies/corrects it, including reversal.
  Interpolated View heights must not become network resize requests.
- While IME is running or its final layout is pending, keep the actual drawn
  compatible alternate-screen geometry when a differently sized candidate arrives.
  Reuse the existing drawn frame/source and newest pending frame/source; do not
  add a queue. Complete the drawing handoff at settled layout to the latest
  prepared frame. Same-size live output can continue normally.
- A late candidate for a previous resize target cannot replace a compatible
  drawn grid while the latest measured target is still outstanding.
- Native install/ACK, current input readiness and coordinate retirement continue
  independently. A retained drawing must never adopt the pending candidate's
  source. Reconnect, screen/width incompatibility and selection keep their owners.

Expected product files: `TerminalScreen.kt`, `TerminalView.kt` and, if needed to
centralize endpoint math, `TerminalGeometry.kt`. Extend the existing geometry /
rendering tests and Android specs. No host parser/grid changes, protocol fields,
paint-completion guesses, process matching or output debounce are in this increment.

This overlaps remote work with the existing animation and prevents its early
resize-only candidate from interrupting local movement. It cannot guarantee
child-layout completion within IME duration. If the child/network completes later,
ordinary eligible output remains authoritative; do not hide that limitation with
a timer or claim every unmarked TUI redraw is atomic. Real phone evaluation must
distinguish this increment's result from that unresolved general limitation.

## Validation

Add the real cursor-anchored resize-only row sequence before completed TUI rows
to the existing pixel/hardware regression. Assert that held local pixels do not
change at either early delivery, and that the final prepared handoff preserves
equal rows and independent cursor/source state. Cover early target deduplication,
final correction/reversal and no-target fallback, plus real IME endpoint delivery
where the existing harness can execute it. Record results and the exact APK before
another phone update. Keep the task active.

## Implemented and verified

The Android-only increment is implemented in the files above. `submitGrid`
centralizes capped dimensions and deduplication. `frameForDraw` selects between
the existing candidate and drawn baseline; source adoption explicitly follows
the selected frame. No additional frame queue, wire change or output timer was
introduced.

- The actual-model probe passed and prints the 03–11 → 02–10 → 03–11 sequence.
- The new Android handoff regression failed against the previous phone build's
  View policy at `resize-only snapshot must not jump the moving rows down`.
  Log: `/tmp/zterm-handoff-baseline-test.txt`.
- Corrected `TerminalRenderingTest` plus `AttachGeometryTest`: **15 passed,
  1 skipped** on a task-owned Android 36 arm64 emulator. The skipped existing
  test needs an explicit real-host attach fixture. `OK (16 tests)` includes it.
  Log: `/tmp/zterm-handoff-final-tests.txt`.
- The settled-reversal/obsolete-target branch was then checked separately with
  `reversedImeKeepsTheDrawnGridUntilTheLatestTargetArrives`: **1 passed**. It also
  verifies that matching-target changed content resumes, rather than remaining
  frozen. Log: `/tmp/zterm-handoff-reversal-test.txt`. Only tests changed between
  these runs; the application APK SHA-256 remained unchanged, and lint passed.
- This run includes the actual system IME with the production Compose layout:
  early target delivery occurred while the visible grid was still larger, and
  matched its final measured height. Synthetic delayed-frame pixels and real
  hardware PixelCopy/row-recording checks also passed.
- Debug app/test APK build, Kotlin compilation and lint passed; APK signature,
  native-library/zip 16 KiB alignment and `git diff --check` passed.
  Build log: `/tmp/zterm-handoff-final-build.txt`.

Artifact: `apps/android/app/build/outputs/apk/debug/app-debug.apk`, application
`io.github.leonfox28.zterm.dev`, version `0.1.31` / `103199`, 49,999,725 bytes.
SHA-256: `c97a0239d8bfe69c8fb1a5a97fcbd8eb26c8263c549035b9ba2a54bc742c7a33`.
Phone installation and real Herdr acceptance are recorded separately below.

## Phone delivery

Installed the corrected development APK over the connected phone's existing
`zterm Dev` with `adb install -r`. Android reported success. Installed `base.apk`
SHA-256 exactly matches `c97a0239d8bfe69c8fb1a5a97fcbd8eb26c8263c549035b9ba2a54bc742c7a33`.
Last-update time is 2026-09-09 21:23:52; first-install time is unchanged. No
uninstall, data clear, host daemon replacement or user-terminal input was used.
The task-owned emulator/AVD `Zterm_Handoff_0909` is retired after validation.
The user's next real-TUI check reports that this is substantially better, with
slight unevenness remaining during opening. The follow-up layout-cost and closing
target investigation is in `ime-motion.md`. This feedback is not proof of
end-to-end redraw completion in every network/TUI timing case.
