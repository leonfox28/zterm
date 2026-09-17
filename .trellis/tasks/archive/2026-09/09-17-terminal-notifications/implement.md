# Terminal notification forwarding implementation plan

Status: implementation and acceptance complete; commit, PR and merge authorized. The user approved the final summary on 2026-09-17 and the
task was started with `--allow-empty-context` for the required inline workflow.
Codex mode: inline implementation and check; no sub-agent JSONL curation needed.

## Planning gate

- [x] User selected Ghostty as the primary outer-terminal acceptance target.
- [x] User selected ordinary OSC 9 and OSC 777 notifications only.
- [x] User selected no detached buffering and no reconnect catch-up.
- [x] Read the current ingress, effect broker, wire, shared-client and presenter paths.
- [x] Verify Herdr 0.9.1's producer protocol and distinguish backend detection.
- [x] Persist PRD, design, implementation order and validation mapping.
- [x] User selected Android system notifications while the connection is valid,
  retaining the existing background lifetime without a foreground service/push.
- [x] Update PRD, design and implementation plan for both platform consumers.
- [x] User reviews the latest final planning summary and authorizes implementation.
- [x] Run `python3 .trellis/scripts/task.py start .trellis/tasks/09-17-terminal-notifications`.

## 1. Load execution context and establish the boundary

- [x] Load Phase 2.1 and `trellis-before-dev` before writing product code.
- [x] Read the relevant backend specs: `terminal-model.md`, `terminal-driver.md`,
  `core-wire-domain.md`, `local-daemon-ipc.md`, `shared-client.md`, and the shared
  cross-layer/code-reuse thinking guides.
- [x] Read `frontend/android-app.md` before editing the NativeTerminal/Kotlin
  bridge, application repository, permission UI and Android instrumentation.
- [x] Reconfirm kind 325 is unused and the expected files have no intervening edits.

## 2. Domain and ingress

- [x] Add the validated/redacted notification value and bounded host-effect
  aggregate in `crates/core/src/terminal.rs` (or a focused terminal submodule).
- [x] Update `crates/terminal/src/ingress.rs` and `model.rs` to collect ordinary
  OSC 9/777 requests separately from the latest clipboard value.
- [x] Preserve the existing string cap and reject excluded OSC 9 subcommands,
  other OSC 777 extensions, invalid UTF-8/control text and cancelled input.
- [x] Correct UTF-8 continuation versus standalone C1 ST handling in the existing
  string framer; preserve all other framing behavior.
- [x] Cover PRD A2/A3/A7 in focused terminal tests, including `结果` and chunk
  splits, multiple requests, adjacent normal text, and clipboard coexistence.

## 3. Wire, broker and Session

- [x] Add the typed message and kind 325 in `proto/zterm/v2/{terminal,wire}.proto`;
  update `crates/proto/{build.rs,src/lib.rs}` conversions, redaction and registry.
- [x] Extend `crates/daemon/src/terminal_driver.rs` with independent bounded
  notification storage while preserving clipboard replacement behavior.
- [x] Update `crates/daemon/src/session.rs` and `session_wire.rs` to deliver only
  to the eligible controller and consume queued entries without lost wakeups.
- [x] Cover PRD A4/A5/A6 at the owner: no-controller drop, reconnect/reattach,
  takeover, end, healthy same-epoch sync, FIFO order, overflow and PTY progress.

## 4. Shared client and desktop

- [x] Update `crates/client/src/{session,view,iroh_controller}.rs` to validate,
  allow and queue the new event independently of lifecycle/control and clipboard.
- [x] Cover malformed form/text/IDs, unsolicited-message identity, lifecycle
  clearing, coalesced wakeups, and multiple ordinary notifications.
- [x] Update `crates/cli/src/terminal_ui/{session,ansi_presenter}.rs` to emit
  canonical OSC 9/777 through the sole presenter without a frame change.
- [x] Extend presenter tests in `crates/cli/src/terminal_ui.rs` for complete
  sequences, flush behavior, unchanged visual baseline and error propagation.

## 5. Android bridge and system notification consumer

- [x] Add an independent bounded transient event API in
  `crates/android/src/terminal.rs`, factoring a focused notification submodule
  if needed. NativeFrame remains screen state. Clear pending events and invalidate
  the delivery generation on actual connection retirement/replacement.
- [x] Test queue order/overflow, coalesced wakeups, waiter cancellation, closure,
  and generation fencing across reconnect within the same NativeTerminal.
- [x] Add a focused Kotlin notification helper under
  `apps/android/app/src/main/java/io/github/leonfox28/zterm/`, using application
  context, a stable channel, typed text mapping and ordinary system notifications.
- [x] Add the repository-owned notification collector in `AppRepository.kt`.
  Fence native and repository generations immediately before posting; cancel/join
  it on retirement. Activity/Compose lifecycle observes rather than owns delivery.
- [x] Declare notification permission in `AndroidManifest.xml`; add the Settings
  permission/system-settings action through `AppUi.kt` and an Activity-result
  launcher. Refresh OS state on foreground return without persisting a parallel
  permission flag. Add localized English/Chinese strings and a small icon.
- [x] Keep skipped events consumed if permission is denied or app/channel
  notifications are disabled. Neither posting failure nor permission flow may
  terminate the connection or create a later delivery backlog.
- [x] Regenerate Kotlin only through the existing pinned UniFFI build pipeline;
  do not edit generated sources or add a foreground service/keepalive owner.
- [x] Add focused instrumentation under `app/src/androidTest` for A9-A11:
  both forms/Unicode, notification field mapping, permission/channel suppression,
  foreground/background with live connection, recreation without duplicates,
  and no replay after detach/reconnect; native/broker tests cover takeover/end
  retirement. Reuse private test-owned
  host/session fixtures and avoid unrelated real accounts or notification state.

## 6. Integration evidence and documentation

- [x] Cover A1/A3/A4/A8 with existing local/remote Session fixtures plus an
  isolated outer-PTY CLI regression. Reuse the disposable daemon/test account
  pattern in `crates/cli/tests/daemon_autospawn.rs` and its support module;
  factor a focused notification fixture if that keeps the test readable.
- [x] Verify notifications do not wait for synchronized screen output and that
  screen snapshots/history/resync cannot replay them.
- [x] Record a Ghostty manual smoke with both forms and Unicode. Use synthetic
  Herdr-format requests for deterministic evidence; any real Herdr instance must
  use private state/socket directories and report its backend-detection inputs.
- [x] Document supported forms, limits, attached-only behavior, matching-build
  requirement, and Ghostty/Herdr notes in `docs/persistent-sessions.md` (or a
  linked focused document).
- [x] Document Android notification enabling, foreground/background lifetime
  and no-catch-up behavior in `docs/android.md`.
- [x] Update affected Trellis contracts to describe notification FIFO semantics
  separately from the unchanged latest-only clipboard contract; update the
  Android app contract with the independent event bridge and system API owner.

## Validation commands

Use focused checks during implementation, then the repository quality gate once
the complete change is ready. Run tests only after their implementation exists.

```sh
cargo +1.98.0 test -p zterm-core -p zterm-terminal -p zterm-proto
cargo +1.98.0 test -p zterm-daemon -p zterm-client -p zterm-cli -p zterm-android
sh tools/android/build.sh :app:assembleDebug :app:assembleDebugAndroidTest :app:lintDebug
just check
```

Do not duplicate the full gate without new edits or an unresolved failure. The
gate includes native formatting/policy, Clippy, tests, docs and dependencies;
other-host and actual OS popup evidence retain their own scope.

Run focused notification instrumentation through
`sh tools/android/build.sh :app:connectedDebugAndroidTest` with the test-class
filter and private-fixture arguments required by the implemented tests. Cover an
Android 13+ emulator/device for runtime permission behavior and an older supported
API for the no-runtime-prompt channel path. Record actual devices/API levels and
which cases ran; build/lint success is not system notification runtime evidence.
Do not run unrelated device-mutating instrumentation as a substitute for these
focused checks.

## Completion and rollback checks

- [x] Load `trellis-check`; map passing evidence to PRD A1-A11 and report any
  remaining manual Ghostty/Herdr/Android validation honestly.
- [x] Verify no change to hosted TERM identity, standalone bell forwarding,
  or application-specific branching; Android posts only for a live eligible
  attachment and retains the existing background connection lifetime.
- [x] Verify the final diff contains only necessary cross-layer changes and
  tests/docs; review payload redaction and matching-version compatibility.
- [x] Present the completed implementation and validation before the normal
  commit/wrap-up flow. This plan does not authorize release or deployment.

Rollback is an atomic revert of the notification feature across its message
producers and consumers. There is no database/config migration or durable
notification backlog to reverse.

## Outcome

See [validation.md](validation.md) for passing checks, platform evidence, fixture
corrections and the user's successful Ghostty popup verification. Product code,
specs and acceptance are complete. The user authorized the commit, PR and merge
with “提交然后pr、合并吧”; task archival follows the feature commit.
