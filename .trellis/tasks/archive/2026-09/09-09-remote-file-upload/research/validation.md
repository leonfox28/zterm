# Integration validation — 2026-09-09

## Implementation

Branch: `feat/remote-file-upload`, based on `e8cb05e`. Main session implemented
all three children inline after the user approved the final review with “开始吧”.
No release, installed host replacement, push or work commit has been performed.
All task-owned changes are ready for workflow Phase 3.4 commit review.

Desktop: Ctrl+] v acquires one explicit clipboard file or raw image, Ctrl+] c
cancels, Ctrl+V is unchanged. Shared uploader streams to private effective-user
/tmp storage and inserts the completed path once without Enter. Android's far-left
upload button selects Image/File, retaining one URI/native operation and displaying
progress, size, speed, cancellation and explicit Retry/Close. Both clients discard
paused input and check original attachment authority before insertion. Successful
files have no zterm retention/cumulative quota policy; max source size is 50,000,000.

## Passed evidence

- `cargo +1.98.0 test --workspace --all-features`: PASS, including integration
  harnesses and doctests. Existing platform/explicit-only ignored cases remain
  ignored; this is macOS arm64 evidence, not Linux CI evidence.
- CLI focused suite after status changes: 99 passed, 3 existing ignored; additional
  held-release completion regression passed separately. It verifies owned enhanced
  releases stay local after unpausing. Right alignment is tested at 4/12/24/80/120
  columns with Unicode and both screens, retaining child dimensions.
- `just check-fast`: PASS (workspace Clippy, source/dependency/format/release policy,
  shell/Python/workflow checks, repository secret scan and its fixture).
- `cargo +1.98.0 deny check`: advisories, bans, licenses and sources PASS. Arboard
  3.6.1 is pinned and restricted to the CLI. Its exact two Boost-licensed Windows
  transitive crates have narrow exceptions, not a global license expansion.
- Android `lintDebug`, `testDebugUnitTest`, `assembleDebug`,
  `assembleDebugAndroidTest`: PASS. Existing JVM unit target is NO-SOURCE;
  meaningful Android evidence below comes from instrumentation.
- `UploadUiTest` on explicit `emulator-5558` / `Zterm_Acceptance`: PASS, 1 test;
  verifies one dialog, filename, 50% / 50.00 MB / 2.00 MB/s and Cancel/Retry/Close.
- `NativeUploadTest` on the same emulator with `uploadFixture=1`: PASS, 11.32 s,
  using a fresh task-owned `upload-acceptance` host, real Iroh network, real Session
  and private one-time pairing fixture. Raw native transfer and production
  `TerminalUploads` + MediaStore document URI staging both produced exact SHA-256
  on the host. It asserts cancellation before source start, paused input rejection,
  stable IME epoch, no automatic Enter, single launcher claim and duplicate URI
  callback suppression. Only its own Session/URI/source/published files are cleaned.
- Host bounded-duplex tests cover 0 / 2,000,013 / 50,000,000 bytes, source count
  mismatch, wrong attachment, cancel/detach, wrong transfer ID/offset/correlation,
  duplicate Begin and premature Finish. Failed transfers remove their own staging;
  the controlling terminal remains live. Cap+1, safe paths, private file modes,
  symlink/collision refusal and raw PNG pixel preservation have focused tests.
- Unknown host capability sends no upload bytes. Lost final replies do not retry.
  Stale final path input cannot terminate the replacement terminal driver.
- `cargo +1.98.0 build -p zterm-cli`: PASS; executable `target/debug/zterm`.
- APK 16 KiB native/zip alignment and `apksigner verify`: PASS. Development debug
  package `io.github.leonfox28.zterm.dev`, not a signed public release/update.
  APK SHA-256: `6db82b42d9bc43ea641bdd7bcea9037a287614bebcb77143b0f63cd3b6ae29e1`.
  Debug certificate SHA-256: `4050c7a543d65aa8dfa96afbd74396d84ed83da932db104db5fef2146afe3eba`.
- `git diff --check`: PASS.

## Runtime boundaries still requiring manual/platform acceptance

No physical phone, real user desktop clipboard, Linux X11/Wayland clipboard or
remote Codex/Pi UI acceptance was performed. Clipboard adapters use injected
file/image sources in tests; no user clipboard was overwritten. The Android runtime
URI test exercises the real resolver and production state owner but does not tap
through both OS picker UIs or recreate the Activity mid-transfer. Physical IME,
full 10-control toolbar/insets, cloud/unknown-size provider interruption, live
network/takeover during finalization and AI attachment-chip behavior remain explicit
acceptance follow-ups. A successful byte transfer does not prove an AI recognizes
all formats. No product requirement was dropped to make tests pass.

## Test fixes and captured lessons

Final checks caught a stale test NativeTerminal constructor, a nullable Kotlin
withLock return type, and false positive secret-scan matches on local cancellation
variable names. Corrected the sources, preserving the scanner policy. Late native
progress observers now include already-published completion. Paused-input queue
fences are separate from Android's IME epoch; desktop progress uses a persistent
interval so continuous output cannot starve it. Post-publication uncertainty is
reported without automatic retry. These are captured in `backend/file-upload.md`.

## Reproduction

`crates/daemon/examples/upload_acceptance.rs` was a temporary real-network harness;
its source is retained in the Android child research directory. To rerun, copy it
there, run it with a fresh private directory, install the generated debug/test APKs,
stream its ticket directly into app-private `files/upload-ticket.txt` through
explicit `adb -s SERIAL shell run-as`, and run `NativeUploadTest` with
`-e uploadFixture 1`. Never print a pairing ticket. Create its `stop` file afterwards,
remove the temporary example and private fixture directory. Production CLI state
has no environment/path override and was never changed by this test.

## User-requested Android entry refinement

The choice menu was replaced by two direct controls (photo, then paperclip).
Android lint/build and the real `UploadPickerUiTest` passed: File/Image twice each,
Back restores the same Session/input epoch with no relaunch. The stale picker
left by the preview's force-stop was cleared from the development task stack.
See the Android child `research/two-buttons-and-picker-back.md`. This supersedes
the earlier untested picker-navigation note and ten-control toolbar description;
there are now eleven toolbar controls. Current APK SHA-256 is
`ff017c9292355cd2eabac18e1d8bd71e1a60ff084e9ccdf71e96a2ed6f09001f`.

## Direction icon follow-up

The user approved replacing the four smaller font arrows with 18 dp vectors
matching the upload/keyboard icons. Android lint/debug build/test compilation
and diff checks passed; the updated APK was installed and visually checked on
emulator-5558. See the Android child UI evidence for scope and screenshot.
Current debug APK SHA-256:
`ed02f37f2559ecd7a96d997085c34a601ce3e650ce46d61f1bfaf2b52847052d`.

## Authorized release preflight

The user accepted the completed feature/UI and requested publication on
2026-09-09. `just doctor` and the complete `just check` pre-push gate passed on
macOS arm64, including all workspace tests, documentation, dependency checks and
relay static checks. Log: `/tmp/zterm-remote-upload-prepublish-check.log`.
All 102 dirty paths match the reviewed feature commit inventory. Prepare v0.1.31
on this branch after the work commit and task bookkeeping; the canonical release
operator owns PR/main CI, exact candidate selection, signing and publication.
