# Terminal presentation continuity implementation plan

Status: implementation and local quality checks complete on 2026-09-08.
See `research/runtime-acceptance.md` for exact evidence. Desktop GUI visual acceptance
is unavailable because the tool rejects access to iTerm. The user subsequently authorized release on 2026-09-08; commit and publish the
current implementation as v0.1.27 while retaining the visual follow-up. Unchecked evidence/finish items are
intentional; Android native/real-IME tests do not substitute for that desktop recording.
Read [prd.md](prd.md) for requirements and [design.md](design.md) for the chosen contracts.

Test isolation: use the Android emulator and independent test hosts/state directories. The user's
existing macOS `main` Session must not be touched, and the running macOS daemon must not be restarted.
Inspect fixture startup/cleanup before running commands that can start, attach to or stop a daemon.

## Before activation

- [x] Resolve user-owned scope and UX choices, including no extra wait for unmarked output and
  Android rounding remainder below the toolbar.
- [x] Inspect existing model, driver, Session/ACK, desktop presentation and Android geometry/input
  ownership; retain evidence and the non-integral-height counterexample in `research/`.
- [x] Converge the PRD and write the design and ordered execution plan.
- [x] Present the complete planning summary and receive its subsequent implementation approval,
  as required by the project's Brainstorm workflow.
- [x] Recheck working state and the current task before activation. Switch this task only after
  approval; do not accidentally implement under the unrelated `e2e-hardening` task.
- [x] Load applicable backend/frontend indexes and pre-development guidance for each affected
  package. This session uses inline execution; empty dispatch manifests are not implementation
  context. Refresh code anchors if the baseline has moved.

## Ordered work

### 1. Establish focused evidence

- [ ] Capture synthetic desktop and Android before recordings for high/low cursor resize, real
  final clear and a marked TUI redraw. Record device/emulator, outer terminal, font/cell metrics,
  transport conditions and observable symptoms separately.
- [x] Reproduce the 17 px / 1000-to-600 px geometry fixture in the geometry test owner. Add only
  instrumentation needed to correlate resize, eligible/applied revision and actual draw geometry.
  Keep terminal content out of diagnostics.

### 2. Implement host publication and DEC 2026 as one coherent change

Owners: `crates/terminal/src/{ingress,model,projection}.rs`, necessary private engine glue,
`crates/daemon/src/{terminal_driver,session,session_wire}.rs`, and their owning tests.

- [x] Intercept supported 2026 markers/query at streaming dispatch without upstream raw buffering;
  continue resource enforcement, PTY replies and control effects.
- [x] Add model-owned eligible state and boundary-aware revision/metadata commits. Freeze once on
  first begin, release on end/recovery, and preserve complete A when B is partial in the same read.
- [x] Route capture/sync/snapshot/attach reads and published watches through that boundary. Keep
  processed-byte accounting independent; prevent unchanged published revisions from busy looping.
- [x] Add deadline-aware driver wakeup and checks between bounded processing units. Serialize
  timeout, reset, native/model resize and EOF with the existing commit owner.
- [x] Add the bounded cancellable pending-history path. Project at the actual eligible boundary,
  release the Session actor/stream reader while pending, and deliver via the existing writer with
  request/attachment lifetime checks. Preserve real history gap/epoch semantics and request limits.
- [x] Verify no wire schema or protocol version change is necessary; retain exact snapshot and
  resume-delta ACK handling. Any discovered need for a compatibility change requires design review.
- [x] Add focused model/driver/Session tests for normal closure, parser fragmentation, A/B boundary,
  frozen colors/history metadata, direct reads/attach/history during a hold, no-further-byte timeout,
  repeated begin under continuous output, resize/reset/EOF, and input/PTY-reply progress while held.

This stage is incomplete if it merely removes the ingress rejection or delays revision watches.

### 3. Separate healthy geometry transitions from input lifetime

Owners: `crates/client` only where shared admission/resize state needs adjustment,
`crates/android/src/terminal.rs` and source/navigation state, desktop session/input ownership,
and Android `AppRepository.kt` / terminal input connection consumers.

- [x] Make healthy same-live-attachment resize distinguishable from initial attach, resume,
  recovery, takeover and history-return synchronization. Retain latest-desired resize coalescing.
- [x] Preserve keyboard epoch/preedit and ordered normal input during healthy resize. Add a
  separate local geometry generation for pointer, selection and copy source validity.
- [x] Retain mode-aware encoding and all real connection/control/session stale-input protection.
  Preserve existing frozen-history/resume behavior; add no new replay queue.
- [x] Audit queue/admission results so accepted text/key units cannot disappear silently or be
  duplicated. Ensure old callbacks cannot target a replacement attachment.
- [x] Test healthy resize input exactly once, rapid A-B-A geometry, stale coordinate sources,
  true reconnect/takeover/end rejection and required snapshot/resume ACK progress.

### 4. Fix Android layout and display handoff

Owners: `TerminalScreen.kt`, `TerminalView.kt`, existing IME animation integration,
native frame/source projection and generated FFI through the normal build (never edit generated
Kotlin manually). Extend existing Android geometry/input test owners.

- [x] Derive exact grid pixels, row count and external remainder from one measurement. Place
  remainder below the fixed-height toolbar and consume IME/navigation insets once.
- [x] Unify draw/clip/pan, handoff, hit test, selection and IME anchors on that geometry. Preserve
  smooth animation with integral stable endpoints and settled-size remote submission.
- [x] Commit drawn source and geometry together, retain/release bounded pending/drawn handles,
  and use actual drawn geometry for coordinates and IME anchors.
- [x] Preserve healthy InputConnection/preedit/cursor/readiness while geometry changes. Ensure
  true lifecycle changes retain their UI and epoch behavior.
- [x] Keep complete Canvas draws initially. Check whether identical rows are projected/copied on
  metadata-only events; optimize only if needed, preserving immediate state propagation.
- [x] Validate below-toolbar remainder on representative font metrics and densities, IME open/
  close and height changes, gesture/three-button navigation, width/font changes and small windows.
  Include Chinese composition and selection/history source retirement.

### 5. Make desktop baseline handling correct and continuous

Owners: `crates/cli/src/terminal_ui/{ansi_presenter,composition,session}.rs`, explicit
disposable `crates/cli/examples/presentation_fixture.rs`, related input module
and presenter/session tests. Use a shared pure row helper only if both platforms consume it.

- [x] Preserve final resolved-cell diff and existing outer synchronized-output wrapping.
- [x] Replace unknown-baseline preclear assumptions with complete owned-cell coverage, including
  blanks, wide-cell remnants and retired chrome. Validate old X becoming blank without ED2.
- [x] Use local height clip/pan only where physical mapping is valid; handle outer reflow and
  width changes with truthful full coverage. Do not rely on stale screen coordinates.
- [x] Commit presenter baseline and input-mode state after successful write/flush; preserve
  failure cleanup. Keep ACK progress and current viewport pacing.
- [x] Verify normal unchanged-prefix output, resize handoff, final clear, chrome retirement,
  write failure and supported/unsupported outer synchronized-output behavior.

### 6. Integrate, verify and update executable contracts

- [x] Run focused owner tests as each coherent change lands; run the required full-scope gate
  once after integration. Repeat only for new changes, failures or unresolved concerns.
- [ ] Complete the visual/input acceptance matrix below on explicitly selected targets. Record
  what was observed and any environment gap; do not call missing device evidence a pass.
- [x] Check bounded source ownership, pending-read cancellation and representative repeated
  resize/marked-output work. Broader caches and early resize remain deferred.
- [x] Update changed contracts in backend terminal-model, terminal-driver, session-service,
  local-daemon-ipc/shared-client and frontend android-app specs as applicable. Capture publication
  versus processing revision, pending history reads, healthy input lifetime and exact grid geometry.
- [ ] Run the project's inline quality/spec review and finish workflow. Keep unrelated changes
  out of this task's eventual commit; no release/deployment change is required.

## Verification map

| Requirement | Primary evidence |
| --- | --- |
| PRES-1 | Presenter known/unknown baseline tests; styles, wide cells and final-clear fixture; desktop/Android stable-prefix recording. |
| PRES-2, PRES-2a | Host-aligned row-transform arithmetic; Android exact-grid and drawn-source tests; high/low-cursor open/close recordings, including 17 px remainder handoff. |
| PRES-3 | Client/Session exact-origin ACK tests, resize snapshot followed by later updates, coalesced candidates with required delta dependencies. |
| PRES-4 | Unmarked fixture with no new timer; legitimate empty final screen remains visible. |
| PRES-4a | Streaming marker/query and resource/reply tests; independent deadline plus continuous-input/reset/resize/EOF progress. |
| PRES-4b | Same-ingest complete A/partial B, unique revision content, frozen metadata, direct snapshot/new attach/history read consistency. |
| PRES-5 | Existing CLI/Android targets build; platform scheduling retained; Android dependency policy excludes host parsing dependencies. |
| PRES-6, PRES-6a | Rapid A-B-A, history/selection, reconnect/takeover/end; valid coordinate generation and Chinese composition committed exactly once. |
| PRES-7 | Repeated transitions/begins do not accumulate handles or queued frames; bounded pending history cancellation and projection cost inspection. |

Focused commands, executed after relevant implementation (not as a docs-only planning check):

```sh
cargo +1.98.0 test -p zterm-terminal
cargo +1.98.0 test -p zterm-daemon --test terminal_drain
cargo +1.98.0 test -p zterm-daemon --test attachment_resync
cargo +1.98.0 test -p zterm-daemon --lib session
cargo +1.98.0 test -p zterm-client
cargo +1.98.0 test -p zterm-cli --lib terminal_ui
cargo +1.98.0 test -p zterm-android
just android-build
just android-check
```

Run `cargo +1.98.0 test -p zterm-core` if a shared pure helper or source identity lives there.
Final required local gate: `just check`, plus Android checks for the changed platform. The gate
already owns workspace formatting, Clippy, tests and dependency policy; avoid duplicating it without
a reason. Hosted-only platform checks remain accurately identified as such.

For Android instrumentation, select a device/emulator by explicit serial and use the existing
test runner/fixture setup. Extend relevant `AttachGeometryTest`, `TerminalUiTest` and native bridge
tests rather than introducing a parallel harness. Use isolated startup arguments where the fixture
requires them. Build success does not replace an observed keyboard/composition test.

## Risk and rollback boundaries

- Host publication is the highest concurrency risk: timers, exact-boundary history reads and
  metadata revisions must land together with tests. Never wait for a batch while holding its producer.
- Input lifetime is the highest interaction risk: the healthy resize exception must not escape
  into reconnect, takeover or frozen history. Geometry invalidation remains independent.
- Desktop unknown-baseline coverage must land with removal of the old clear assumption; rollback
  both together to avoid stale blanks. Android exact-grid geometry must land with below-toolbar
  allocation and source consumers to avoid moving the same rounding error elsewhere.
- Unmarked programs and outer emulators can still expose real intermediate layouts. Acceptance is
  elimination of our avoidable discontinuities and correct marked batching, not universal atomicity.

Planning completion does not imply runtime acceptance. Store implementation test/recording evidence
with this task and mark any unperformed checks explicitly when implementation is reported.

## Authorized release extension — 2026-09-08

- [x] User authorizes publication despite the documented remaining visual transition.
- [x] Confirm current upstream main, latest v0.1.26, local quality evidence and doctor.
- [x] Commit the two reviewed implementation groups, then canonical v0.1.27 version commit.
- [x] Review the exact PR diff and document known visual limitations in its body.
- [x] Run `just release 0.1.27 35` through required PR/main CI, candidate, tag,
  normal protected-environment approval, signatures, final installer proofs and publication.
- [x] Verify the immutable release inventory and record exact source/run/tag evidence.
- [x] Keep the task open for visual continuity follow-up; do not claim flicker resolved.

Publication authorization covers its normal GitHub operations, not local daemon activation.
