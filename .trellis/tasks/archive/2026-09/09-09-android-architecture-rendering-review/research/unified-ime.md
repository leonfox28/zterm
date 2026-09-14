# Unified live-screen IME presentation

## Approval and smallest gap

On 2026-09-10 the user approves unifying keyboard movement, early final-size
requests and retained-frame handoff across normal and alternate screens. A TUI
can use either screen. Current screen-specific guards give a main-screen TUI
caret-only displacement and late resize even with the same visible grid as an
alternate-screen TUI. This supersedes the earlier shell-preservation exception.
The phone is unavailable: use only a new task-owned emulator, and keep the tiny
phone keyboard endpoint overshoot deferred. Work remains inline, without commits
or task archival while broader acceptance is open.

## Evidence and classification

This is a **presentation policy boundary defect**. `geometryShift`,
`prepareImeViewport` and `frameForDraw` share existing owners, but their screen
guards split equivalent live presentations into different movement/scheduling
policies. The row cache is already shared; replacing the renderer is unnecessary.

The pinned host engine's `Grid::shrink_lines` scrolls only by
`max(0, cursor + 1 - targetRows)` on either screen. Growth pulls available main
history, then adds blank rows below; alternate history is absent. Screen type
does not imply application layout intent. A visible high caret can therefore
produce a resize-only snapshot whose row positions differ from the local bottom
anchor even though the child has emitted nothing. Existing early scheduling and
retention overlap that intermediate with the IME animation, but cannot identify
child repaint completion without an application publication boundary.

## Owning contract and change boundary

- `TerminalGeometry.kt`: one `terminalPan(height, cellHeight, rows, cursorRow)`
  rule, independent of screen and history extent. Start with the signed height
  difference; for a visible caret clamp upward movement at `-cursorRow *
  cellHeight`, so the caret's top never leaves the View. Hidden cursors do not
  supply a constraint. Growth moves available rows down even with no history;
  newly exposed unknown pixels retain the frame background.
- `TerminalView.kt`: remove the screen-type eligibility split from early target
  submission. Retain a compatible drawn source on either live screen during IME,
  final layout or an obsolete-size candidate; a **change** of active screen still
  bypasses retention. Preserve width/font, input epoch/readiness, selection,
  history and local-scroll fences. The drawn and newest pending source remain
  the only retained handles, and source adoption follows actual drawing.
- Extend `TerminalRenderingTest` and `AttachGeometryTest` for both screens,
  high/low/hidden caret boundaries, opening/closing, ready/late candidates,
  reversal, actual IME endpoint delivery, hardware row reuse and pixel equality.
  Use an actual-model process-free probe to verify the resize-only sequences;
  assert correct authoritative adoption separately when row layouts truly differ.
- Update the Android spec and the existing bridge spec's explanation of the
  active-screen field. No FFI/schema, host/model, cache/painter or IME callback
  changes are planned. The field remains necessary for screen-change fences.

The full-height local movement is intentional on the main screen too. This is
not a promise that a shell's independently chosen final row positions are equal
to those local positions. For example, 24 to 12 rows with cursor row 3 locally
clamps to -3 rows, while the host initially keeps old rows 0..11. On final
handoff the authoritative layout must win; do not retain a wrong viewport,
invent rows, change host terminal semantics, recognize text/processes, or wait
on a guessed repaint timer to conceal that difference.

## Validation plan

1. Extend neutral existing pixel/scheduling cases to both screens and add the
   visible top-edge boundary. Observe failures on the preceding implementation.
2. Apply the shared policy and run the focused Android rendering/geometry suite
   on one owned API 36 arm64 emulator, including real system IME in both screen
   modes and both directions, and PixelCopy/row-recording checks.
3. Run the existing native source/resize tests, build/lint and APK packaging
   checks. Record exact results, scope limitations and the development APK hash.
   Do not install on, inspect or query the user's phone or actual host sessions.

## Results

Implemented in the two planned production files only. The row renderer, painter,
IME callback/layout, host/model and native source lifecycle did not change in
this increment. `terminalPan` drops its obsolete history parameter; the View
removes screen eligibility branches and retains an explicit screen-change fence.

- The preceding implementation fails all four selected new regressions: hidden
  footer pixels on Main, different high-caret displacement, no early Main
  request, and a smaller candidate interrupting the larger moving Main drawing.
  `/tmp/zterm-unified-ime-baseline-tests.txt` records **4 failures / 4 tests**.
- Actual-model probe `unified-ime-probe.rs` passes on both screens, with cursor
  rows 0/3/21/23 at 24-to-12 shrink, synchronized partial-clear publication and
  subsequent growth with/without history. `unified-ime-model.txt` records exact
  row placement and the local/remote differences described above. The probe's
  first comparison mistakenly included changing scroll-metric revision metadata
  on Main; the corrected assertion checks retained rows and cursor, its intended
  publication invariant. No model change was made.
- `TerminalRenderingTest` + `AttachGeometryTest` on the new API 36 arm64
  `Zterm_UnifiedIme_0910` emulator: **21 passed, 1 explicit-host fixture skipped**,
  `OK (22 tests)`, 11.983 seconds. Both screen modes run the actual system IME in
  both directions: one final-size request before the visible View reaches it,
  equal to settled layout. Both hardware sequences show zero additional row
  recordings for equal moved rows and one for a changed row, with PixelCopy
  equality at compatible handoff. High caret, late target, reversal, screen
  switches and input fences pass. Log: `/tmp/zterm-unified-ime-tests.txt`.
- `cargo test -p zterm-android --lib`: **18 passed**, including actual native
  captured-source, stale A-B-A coordinates and healthy resize/input tests.
  Log: `/tmp/zterm-unified-ime-native-tests.txt`. Android pixel fixtures use
  in-memory rows without native handles; the native suite separately owns source
  authority. No new network/native end-to-end fixture or phone FPS measurement
  was performed.
- Build wrapper `assembleDebug`, `assembleDebugAndroidTest`, `lintDebug` passes.
  Final APK signature v2, every native ELF and ZIP 16 KiB alignment, and
  `git diff --check` pass. `/tmp/zterm-unified-ime-build.txt` records the build.

Development artifact: `apps/android/app/build/outputs/apk/debug/app-debug.apk`,
`io.github.leonfox28.zterm.dev`, v0.1.31 / versionCode 103199, API floor 26.
Size: 49,999,725 bytes.
SHA-256: `f66e8eea97e3534149b9eed3bf227da1ee8593289b9831a350d2ef2fe79f387a`.
Installed only on the owned emulator. The user's phone remains on the previously
installed row-layer build, and its tiny endpoint overshoot remains deferred.
The unified main-screen movement intentionally changes the earlier caret-only
policy; actual phone shell/TUI acceptance is still outstanding.

The owned emulator process is stopped and `Zterm_UnifiedIme_0910` plus its
`/tmp/zterm-unified-ime-avd` directory are deleted. No host fixture was started.
The task stays in progress, with this research included in both context manifests.
Context validation passes but warns that the existing Android spec exceeds the
32 KiB automatic-injection limit; this inline turn read its full content directly.
Future sessions must likewise read the file, not rely on a truncated injection.
