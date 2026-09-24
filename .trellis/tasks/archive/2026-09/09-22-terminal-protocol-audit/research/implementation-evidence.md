# First-batch implementation and acceptance — 2026-09-24

Branch: `feat/terminal-protocol-compatibility`. User approved the detailed design with “开始”. Work was performed inline and sequentially. Implementation was initially left uncommitted for review. The user subsequently authorized commit and merge on 2026-09-24; this does not request a release. The original audit remains the pre-change baseline.

## Implemented behavior

| Requirement | Implementation and meaningful evidence |
| --- | --- |
| R1: REP, CSI u, strike/conceal | Bounded repetition through the existing character budget; strict plain restore distinct from Kitty controls; style preserved through core/wire/history/rendering. `grid_compatibility` covers ASCII, Unicode, combining limits, reset, malformed UTF-8, controls and explicit Copy. Desktop SGR and Android pixel tests cover rendering. |
| R2: cursor presentation | Required block/beam/underline + blink metadata through snapshots/deltas/bridge; physical DECSCUSR or locally timed software caret. `cursor_protocol`, protobuf rejection, desktop blink/repair tests, Android pixel/lifecycle tests. View visibility resumes blinking without a fresh native frame; wide cells retain their full caret span. |
| R3: OSC 8 | Validated HTTP(S) targets, bounded shared identities, saved/main/alternate templates and history retention, self-contained wire dictionaries, accounted Android cache allocations. `hyperlink_protocol` and protobuf/navigation tests exercise retention, invalid targets, quotas/reclamation, sharing and ambiguous selection. CLI closes spans and cleans up on failures/exit. Native selection supplies the system Open link action with attachment/selection fencing. |
| R4: application title | Bounded, clean title in authoritative state and full metadata updates, independent of session names. `title_protocol` covers truncation, malformed UTF-8, empty/RIS/screen changes, title-only deltas and held publication. Wire validation/redaction, CLI title change/default/stack cleanup, Android header and rename/reconnect checks. |
| R5: replies | Ordered unpadded Base64 unknown-field OSC 21 replies and actual character dimensions from CSI 18t. `query_protocol` covers chunking, UTF-8 names, resize, synchronized-output replies, invalid extra arguments and overflow. |

## Host and Android checks

- `just check` passed on macOS arm64 / Rust 1.98.0: source/dependency/workflow policy, formatting, Clippy with warnings denied, secret scans, complete workspace tests, documentation, dependency/license checks and relay static/upstream verification. Existing platform-specific ignored tests remain ignored; hosted-only release/other-host coverage is not claimed.
- `sh tools/android/build.sh :app:assembleDebug :app:assembleDebugAndroidTest :app:lintDebug :app:testDebugUnitTest` passed. Final test changes were rebuilt and Android lint/unit checks passed again.
- `git diff --check` passed.
- Final focused verification after strengthening combined-hold coverage and correcting the wide software underline: `cargo test -p zterm-terminal --test title_protocol`, terminal all-target Clippy, all desktop presenter tests, and CLI all-target/all-feature Clippy. The complete host gate was rerun successfully before committing, including these final refinements.
- The final combined DEC 2026 regression additionally checks title, cursor, strike/conceal and links remain frozen together, queries reply immediately, and the released delta equals the full snapshot.

## Runtime acceptance

A new disposable AVD `Zterm_Protocols_0924` (Android API 36, arm64; explicit serial `emulator-5584`) and the existing `presentation_fixture` helper with isolated root `/tmp/zterm-protocols-0924-host` were used. No saved user host or production daemon was selected. After verification the fixture daemon was stopped, its task-owned state removed, and the disposable emulator/AVD shut down and deleted.

- 33 instrumentation cases passed: complete `TerminalRenderingTest`, `TerminalFramesTest`, `TerminalInputConnectionTest`, `NativeBridgeTest`, and `TerminalUiTest#childMouseAndAlternateScrollUseTouchWithoutStealingLongPress`.
- The additional opt-in `TerminalUiTest#terminalProtocolMetadataAndSystemLinkActionSurviveReconnect` passed against that real host/session: Chinese title, beam/blink, strike/conceal flags, manual rename, detach/reattach, source-pinned link lookup, long-press system Open link, canonical browser intent, existing system Copy, and empty-title restoration. Total: 34 distinct passing device cases.
- Browser navigation was intercepted with an Instrumentation ActivityMonitor, verifying the ACTION_VIEW target without loading a remote page. The absent-browser toast branch is implemented but was not exercised on this image.
- Real PTY integration/guard tests passed for desktop presentation, atomic flush, selection Copy and exit/panic cleanup, including OSC 8 close, cursor default and window-title pop.

## Review findings and limits

- Legacy desktop tests asserted “no OSC”; their original color-safety contract now explicitly forbids color mutation OSC while permitting canonical title/link output. The black-box cleanup byte contract includes all newly owned state.
- The Android fixture initially addressed a retained page as if it began at the live viewport; it now locates the actual styled row. Floating-toolbar automation requires FLAG_RETRIEVE_INTERACTIVE_WINDOWS. The observed menu contained both Copy and Open link; enabling the test flag made the real action test pass.
- Desktop custom-color beam/underline carets use narrow block glyphs because ANSI cannot overlay part of a cell. This temporarily substitutes the glyph beneath the caret; its original semantic contents are preserved and restored when the caret moves or blinks off. Native caret presentation has no such substitution.
- Physical desktop hyperlink activation and prior-title restoration depend on outer-terminal OSC 8/title-stack support. No cross-terminal visual certification was performed; canonical output and cleanup were verified by automated PTY tests.
- All newly shared schema/producer/consumer changes are intended to deploy together. Android host-driven OSC 52 remains outside this task; existing system selection and Copy are retained.

The user approved commit and merge after reviewing implementation. The verified implementation is committed as `98d28f9b8ad4`. This parent and its five children are archived; integration proceeds through the required GitHub CI gate.
