# Fix verification — 2026-09-20

## Result and implementation

Implemented on `fix/android-ime-composition`, created from `main` at `796e13c`
before changing product code. All PRD acceptance criteria are satisfied. The user
then tried the repaired app in the visible emulator, confirmed the result, and
explicitly approved commit, PR creation and merge on 2026-09-20.

The existing `TerminalView.TerminalInputConnection` now uses the Editable's real
selection/composing spans, platform replacement/cursor/deletion semantics, and
outermost-batch selection/anchor notifications. Confirmed text is admitted once;
rejection restores the complete editor state. Epoch changes clear/reset the
buffer and stale editing callbacks remain fenced. No IME-specific branch, new
composition owner, transport change or suppression of legitimate finish was added.

Review caught an additional rollback hazard: the SpannableStringBuilder copy
constructor drops Android NoCopySpan selection/composing markers. The rejected
commit test initially failed with selection `-1`; using an empty builder's
`append` for the snapshot preserved all markers, and the final test passed.

## Runtime acceptance

Same supplied Doubao 1.4.5 / 100405008 APK, basic/offline mode, 26-key Pinyin,
API 36 arm64 `Zterm_Doubao_0920` / `emulator-5560`, and isolated host
`/tmp/zterm-doubao-0920` as the [baseline](./research/reproduction.md).
Installed both debug/test APKs with `adb install -r`, retaining identity/pairing.
All Pinyin and candidate input below used actual software-key taps, without JDI.
ASCII `adb input text` was used only to prepare the disposable raw `tee` receiver.

| Display setting | After each of `n`, `i`, `h`, `a`, `o` | After selecting `你好` |
| --- | --- | --- |
| 显示在输入框 | 0 host bytes | Exactly `e4bda0e5a5bd` / `你好` |
| 显示在候选栏 | 0 host bytes | Exactly `e4bda0e5a5bd` / `你好` |

Both results have no Pinyin prefix, duplicate Chinese or trailing space. Exact
observations: [inline](./research/fixed-inline.json),
[candidate bar](./research/fixed-candidate.json). The inline screenshot shows
underlined local `ni hao` while the raw receiver is still empty.

An additional inline case typed `nihax`, tapped Backspace, typed `o`, then used
Space to confirm. Every composing/editing step sent zero bytes; confirmation
sent exactly `你好`. [Raw observations](./research/fixed-edit-retry.json); screenshots
`fixed-edit-retry-{composing,final}.png`. An earlier attempt overlapped a route
change, left an unsettled shell command and timed out before any Chinese arrived;
it was discarded and repeated with a new receiver after verifying readiness.

## Automated checks

| Check | Evidence/result |
| --- | --- |
| New editor contract on old app | Fails: expected before-cursor `n`, actual empty; `editor-regression-before.log` |
| `TerminalInputConnectionTest` | 3 pass; context queries, partial composing replacement, selection, code-point deletion, nested batch, rejection rollback, suspended/stale callbacks and retired Editable isolation |
| `TerminalUiTest#keyboardDismissalSurvivesTaskResume` | 1 pass; together with editor tests: `editor-and-lifecycle-after.log`, 20.577 s |
| `TerminalUiTest#keyboardPresentationAndCompositionStayContinuous` | 1 pass, 28.388 s; explicit finish twice, exact Unicode admission, healthy resize preserves preedit/epoch; `ime-continuity-after.log` |
| `NativeBridgeTest`, `IdentityStoreTest` | 7 pass, `bridge-identity-after.log` |
| Debug and test APK compilation | Pass; `fix-build.log` |
| Final `:app:lintDebug :app:lintRelease` | Pass; `lint-final.log` |
| `:app:testDebugUnitTest` | Gradle succeeds, NO-SOURCE; substantive regressions run as instrumentation |
| `git diff --check` | Pass |

The continuity check preceded only the final rejection-snapshot copy correction;
the editor/lifecycle, bridge/identity, real-keyboard and final lint checks ran on
the final implementation. All 12 executed instrumentation checks passed in their
respective runs. This is focused validation, not a claim about the entire suite.

## Artifacts

Local build: `apps/android/app/build/outputs/apk/debug/app-debug.apk`.
Package `io.github.leonfox28.zterm.dev`, 0.1.33 / versionCode 103399.

- App SHA-256: `1ec429b0b624cd79db3e36f152a6d11a45bc20ce096040b16ac69a1eff4d9948`.
- Test APK SHA-256: `a666263bda1e32db46127e2906b3a3faaae78b08a4429f1d7d66fa1c617fb6b9`.
- Verified debug signer SHA-256: `4050c7a543d65aa8dfa96afbd74396d84ed83da932db104db5fef2146afe3eba`.

Logs and screenshots are in `target/doubao-ime-investigation/` (ignored build
artifacts): `fixed-mode-{inline,candidate}.png`,
`fixed-{inline,candidate}-{composing,final}.png`, and the log names above.
`fixed-inline.mp4` contains 4.368 s of composition capture; use the final screenshot
and raw JSON for candidate-commit evidence, not that short video alone.

The Android spec now records the executable cursor/composition contract, rollback
span-copy pitfall and real-IME acceptance method. Inline trellis-check reviewed
the task/design, code, tests and spec: no scope drift, brand detection, production
diagnostic logging, warning suppression or cross-layer contract change.

Physical-phone acceptance and Doubao smart/cloud mode remain untested. No release
was published. Only the disposable fixture and AVD were used; the fixture daemon
(its one test Session) and emulator were explicitly stopped after acceptance.
The AVD, isolated host state and local artifacts remain available.

The visible emulator and isolated fixture were reopened at the user's request
for their own acceptance and remain running for inspection.
