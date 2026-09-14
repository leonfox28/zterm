# Review execution and validation plan

This is a review task. Checked items are planning/research evidence already
collected; unchecked items require implementation or additional runtime evidence.
The user's latest response explicitly approves the proposed first row-level
implementation; other review findings remain independent follow-ups.

## Current peer-adoption execution

- [x] Add source-relative row projection and exact semantic reuse tests.
- [x] Integrate explicit projection lifetime and display-paced collection;
      check burst/final-state/cancellation/hidden-window behavior.
- [x] Evaluate compatible ASCII runs against the preceding painter with paired
      hardware pixels and recording measurements. Archive/remove the experiment:
      long-text work improves but styled controls do not establish a reliable gain.
- [x] Run focused Rust/Android checks, build/lint and APK integrity verification.
- [x] Record evidence, source contracts and the exact emulator-only APK.

Only disposable emulator/host resources are authorized for runtime checks. Do
not inspect/control/install on the phone. The earlier no-commit/no-archive
boundary applied to the implementation turn; on 2026-09-14 the user requested
the release workflow, authorizing commits, task wrap-up and formal publication.

## Release handoff (2026-09-14)

Release the reviewed implementation as v0.1.32 through `docs/releasing.md`:
local gate, product/spec commit, Trellis bookkeeping, one code-and-version PR,
exact-head merge, successful main candidate, protected signing and immutable
publication. The current main baseline is still v0.1.31 at `a2b518e`.
See `research/release-handoff.md` for evidence and retained follow-ups.
Archival closes this review and its approved implementation increments; it does
not turn the unchecked broader measurement scenarios below into passing checks.

## Completed review work

- [x] Compare released Termux and ConnectBot/termlib implementations, distinguish
      unverified Termius/JuiceSSH internals, and map row preparation, vsync,
      text-run painting and local-model resize to Zterm. Record primary sources,
      exact versions and bounded follow-ups in `research/android-terminal-peers.md`.
      No competitor runtime or product implementation was performed in this step.

- [x] Implement and check the 2026-09-10 approved unified live-screen movement,
      early endpoint scheduling and drawn-source handoff. Follow the bounded
      files, host resize evidence and emulator-only checks in
      `research/unified-ime.md`. Both-screen real IME and hardware pixel checks
      pass: 21 Android checks passed, one explicit-host check skipped; 18 native
      tests passed. Earlier screen-specific rules are historical. The new APK
      is built and checked, with no phone install or physical acceptance claim.

- [x] Create the requested Trellis task and preserve the three user priorities.
- [x] Inspect current Android, bridge, shared client and host publication code.
- [x] Compare existing Android and presentation task findings against current code.
- [x] Map architecture, synchronization and rendering identities in `design.md`.
- [x] Classify findings by owner, severity and evidence in `research/review.md`.
- [x] Compare rendering alternatives against official platform/engine documentation.
- [x] Run 17 Android-native and 13 terminal library tests.
- [x] Run 3 synchronized-output and 6 snapshot/delta integration tests.
- [x] Run the actual-code delta allocation probe (no PTY or network involved).
- [x] Run Android native build/Kotlin compilation and lint through the build wrapper.
- [x] Record that the JVM test task has no sources and no device is currently connected.
- [x] Review artifact completeness, source anchors and the absence of product edits.
- [x] Record the user's after-keyboard-arrival timing clarification and trace the
      current late resize submission and height-change row-cache invalidation.
- [x] Specify early target discovery, retained presentation, compatible row reuse
      and a whole-transition acceptance plan in `research/tui-ime-resize.md`.
- [x] Incorporate the user's local-movement-first contract: reconcile against the
      moved display, recommend row granularity first, and make early remote resize
      complementary to the core behavior.
- [x] Implement the approved first local-response/row-reuse increment; verify two
      regressions fail on the original implementation and pass after the fix.
- [x] Build and validate the development APK; 11 emulator checks passed and the
      explicit host attach fixture was skipped. Record the exact artifact and
      outstanding real-Herdr/phone acceptance in `research/first-build.md`.
- [x] Correct the visible-cursor alternate-screen anchor after phone feedback:
      export authoritative screen metadata, verify the footer regression fails
      before the View correction and passes after it. All 18 native checks and
      12 focused emulator checks pass; one host-fixture test remains skipped.
      The corrected APK is recorded in `research/tui-bottom-anchor.md`; phone
      installation was subsequently verified by the exact installed APK hash.
- [x] Reproduce the later one-row handoff jump using the actual host model.
      Implement known-endpoint early resize and retained alternate-screen IME
      presentation. The new regression fails before the change and passes after;
      15 focused Android checks pass, including a real system-IME endpoint check,
      with one explicit host fixture skipped. See `research/resize-handoff.md`.
- [x] Verify settled reversal against an obsolete resize candidate, then install
      the same checked application APK on the phone and confirm its exact hash.
      The user subsequently reports this handoff is substantially better.
- [x] Measure the remaining local IME layout path and remove redundant outer
      subcomposition by limiting BoxWithConstraints to the visible session panel.
      Confirm early target submission in both real system-IME directions, once
      per transition and equal to settled geometry. Build/lint and the final
      focused suite pass (16 passed, one explicit host fixture skipped).
      See `research/ime-motion.md` for measurement limits and phone delivery.
- [x] After the user reports unchanged perceived smoothness, capture the phone's
      real TUI and identify expensive RenderThread glyph/background replay.
      Add Android-managed compositing layers to the existing bounded row nodes;
      hardware pixel/reuse checks, build/lint and packaging pass. The final
      focused suite reports 17 passed and one explicit host fixture skipped.
      Install and verify the exact comparison APK with data preserved.
      `research/ime-phone-trace.md` records the first improved animation sample
      and 11.58–13.06 MiB of live row-sized GPU targets.
- [x] Supplement the comparison with five actual opening and six closing
      animations. Mean RenderThread Drawing falls about 75% against the same
      trace settings; both directions improve. Check real presentation timestamps
      too: the remaining end-of-animation long frames are not resolved by this
      replay saving. Record all frame flags and the unequal live-scene samples
      in `research/ime-phone-trace.md`; keep the same installed comparison APK.
- [ ] Attribute the remaining final Main drawing burst before another product
      change. The system trace combines text-row recording and source/geometry
      commit; it does not identify which substage dominates or establish physical
      phone visual acceptance.
- [x] Investigate the user's tiny keyboard overshoot on owned API 36 and API 37
      emulators, with real IME/inset/toolbar observations and a target-size local
      TUI candidate. No reversal is reproduced. Archive/remove the temporary
      diagnostic and defer this phone-specific report as explicitly requested;
      see `research/ime-end-motion.md`. Continue only emulator work.
- [x] Attribute real native-source drawing on the owned API 37 emulator: four
      normal-speed IME pairs and two deliberately slowed ordering pairs pass.
      Three ready candidates switch 1.801–2.889 ms after the IME endpoint;
      measured handoff work is mostly in rows, without reproducing the phone's
      longer Main scopes. Archive/remove the temporary stage diagnostic, retain
      wall/CPU and timing limitations, and restore/build the exact comparison
      APK. See `research/ime-draw-cost.md`; no speculative product fix is added.
- [x] After the user accepts the necessity review, retain the reviewed mechanisms
      and optimize the existing cell painter's empty-glyph branch. Paired warmed
      measurements reduce sparse/decorated recording cost with identical hardware
      screenshots and a stable dense-text control. Archive/remove the benchmark;
      the focused suite passes 18 checks with one explicit-host skip, build/lint
      and APK checks pass. See `research/row-painter.md`. Physical-phone validation
      and the tiny endpoint overshoot remain open.

The latest increment is ready for user evaluation. The broader sequence below retains
unperformed measurement/follow-up work and must not be read as completed merely
because this increment has a green focused check.

## Proposed resize implementation sequence

Earlier phone feedback confirms bottom anchoring and exposes the next missing
boundary: resize-only model snapshots interrupt the moved image before child
repaint. Follow the actual-model reproduction and early-target / retained-drawing
scope in `research/resize-handoff.md`; do not treat row-cache reuse as proof of
whole-transition continuity.

The first phone feedback requires one correction to the first-build anchor rule:
visible-cursor alternate screens also need whole-grid bottom anchoring. Follow
the root-cause, bridge/View boundary and focused checks in
`research/tui-bottom-anchor.md`. The first-build rule below is historical and is
superseded by that correction; row content reuse remains applicable.

The user's 2026-09-09 response approves the latest proposal for a first build.
These unchecked steps are pending work, not completed changes.

### First-build change boundary

The observed gap is local IME response and row reuse for TUI, with existing shell
behavior retained. Source inspection also finds `TerminalView.geometryShift`
returns zero whenever the cursor is hidden. Thus a hidden-cursor live grid has
no local pan, independently of the remote timing and cache invalidation findings.
Use the live grid's bottom edge as the presentation anchor when no cursor is
visible; this does not invent or reveal a caret. Keep the visible-cursor shell
rule and frozen reading/selection ownership. Newly exposed unknown area stays
neutral until real data arrives.

Root-cause classification: a presentation-boundary gap. Local placement currently
depends on visible-cursor policy, and render identity on source row ordinal / all
geometry invalidation. The reviewed invariant belongs to the Android View and
text-row cache, while authoritative state and coordinate fences remain native.

Expected product files are `TerminalGeometry.kt` (explicit live-grid anchor),
`TerminalView.kt` (use that anchor and split text/coordinate invalidation), and
`TerminalRowRenderer.kt` (bounded content reuse across moved row ordinals).
Extend the existing geometry/rendering tests to observe local movement with delayed
frames, row recording reuse and actual pixels. Update the Android spec after checks.
No host/shared-core/wire changes, early-resize scheduling, character patches or
unrelated local defects are necessary for this first build. Early scheduling and
PERF-1 remain follow-ups, not prerequisites for the approved local behavior.

1. [ ] Capture a comparable baseline on a disposable host and selected device:
       both IME directions, neutral TUI and Herdr, full-transition video, local
       stage timestamps and correlated host events. Distinguish target snapshot
       readiness from actual child-layout readiness.
2. [ ] Make the displayed rows and their current movement/clip correspondence an
       explicit bounded presentation baseline. Follow local viewport changes
       immediately even with no new remote data; preserve the wire baseline and
       apply/ACK all required updates independently of drawing.
3. [ ] Reconcile each eligible candidate with that moved baseline. Separate row
       drawing identity from its old ordinal and geometry/input authority; retain
       equal rows, update changed rows and overlays, and commit the current source
       even when no text changes. Keep complete-frame fallback drawing correct.
4. [ ] Verify zero/sparse/dense differences, movement across row ordinals and
       reversals before adding complementary early-target scheduling. Verify the
       target in the existing Compose/window hierarchy; use one reliable request
       per intent, final-layout correction and the final-measurement fallback.
5. [ ] Run focused correctness and runtime comparisons. Only if preparation is
       still material, reconverge scope for PERF-1 immutable row sharing across
       native/core/FFI; do not expand it automatically.

Candidate implementation owners are the existing `ImeAnimationState.kt`,
`TerminalScreen.kt`, `TerminalGeometry.kt`, `TerminalView.kt`,
`TerminalRowRenderer.kt`, and their repository/native frame handoff only where
required. This is a local scheduling/presentation change before any wire or
renderer redesign. Add focused tests to the existing geometry, native and Android
UI/rendering suites; do not create a parallel test framework.

Validation must cover early/late target readiness, repeated A-B-A intent,
cancellation before start, unknown endpoint fallback, width/font changes,
input/preedit and selection authority, reconnect, API compatibility and bounded
source/cache retention. When a matching target is already prepared by keyboard
arrival, verify prompt submission without another network wait. Measure the
actual reduction in arrival-to-layout latency rather than promise a universal
duration or FPS. Detailed evidence rules are in the focused research note.

Include a deliberately delayed response case in which local rows move with IME
while the remote state is unchanged. Use a synthetic movement such as
`A B C D -> B C D` followed by the authoritative `B C D*`: with a warm compatible
cache, only the changed text row should be re-recorded even though earlier row
ordinals moved. Repeat with an identical snapshot (zero text recordings), a
genuinely dense change (all changed rows), styled blanks, wide/combining text and
cursor-only changes. Compare against complete reference rendering, count content
recording independently of Canvas/GPU composition, and assert correct source
retirement/commit on zero-difference handoff. Test full-draw compatibility fallback
for identical pixels rather than asserting unsupported RenderNode reuse there.

## Reproducible local commands

Run from the repository root:

```sh
cargo test -p zterm-android -p zterm-terminal --lib
cargo test -p zterm-terminal --test synchronized_output --test terminal_snapshot_delta
python3 .trellis/tasks/09-09-android-architecture-rendering-review/research/run-delta-copy-probe.py
sh tools/android/build.sh :app:lintDebug :app:testDebugUnitTest
```

The probe links the current repository libraries and writes its executable into
a temporary directory. It measures allocation identity only. It does not alter
product sources, create a Session, connect to a host, or measure FPS.

## Additional runtime evidence, in priority order

- [ ] **RESIZE:** Execute the baseline and paired whole-transition comparisons
      above first. Count resize requests, retained/re-recorded text rows and late
      preparation separately. Phone RenderThread timing now exists in
      `research/ime-phone-trace.md`; correlated host/child-layout latency and
      the broader scene matrix remain unmeasured.
- [ ] **IMPL-1:** Suspend attach, change system appearance while no native handle
      is adopted, then release attach. Check Compose appearance, actual native
      default colors and host color observations. Repeat for initial/default
      attach and exact-Session attach. Compare with the existing viewport race test.
- [ ] **IMPL-2:** With cached main-screen history and mouse reporting enabled,
      start a child-owned drag, turn mouse reporting off, deliver and draw the
      new frame, then continue MOVE without another DOWN. Assert no local history
      movement and no child input; a new DOWN must admit the new owner. Repeat
      coordinate invalidation and pending long-press cancellation.
- [ ] **IMPL-3:** Draw dotted/dashed underlines on API 26/27 hardware, API 28 and
      current API hardware, plus software Canvas; compare actual pixels.
- [ ] **VIS-1:** Record complete IME close/open transitions, including newly
      exposed history, high/low/hidden cursor, and A-B-A sizes. Reassess the old
      keyboard-close gap on this baseline; do not inherit an old pass or failure.
- [ ] **PERF-1:** Measure cursor-only, one-cell/one-row, dense full-screen, marked
      and unmarked TUI updates separately from cached history scrolling.
- [ ] **LIFE-1:** Observe actual disconnect/reconnect, Activity recreation and
      background/foreground with pending projection and input; use disposable
      Sessions and retain exact-source/epoch assertions.

No connected devices were reported by `adb devices -l` during the initial review.
The first build subsequently used an isolated emulator for focused local tests;
the remaining items above are unperformed, not failed. Use a disposable test host
or a user-selected phone in a later validation session; preserve unrelated devices
and the user's running Mac daemon/main Session.

## Measurement contract

For PERF-1, keep the same scene, geometry, build variant and refresh rate across
comparisons. Collect native row allocations/projections, FFI bytes/conversions,
Main-thread work, GC, display-list recording, total/GPU duration where available,
deadline misses and network bytes. Record blank/incorrect pixels separately from
slow frames. Compare final semantic state with a fresh snapshot, including
wide cells, resolved styles/colors, cursor and selection.

Use existing `TerminalRenderingTest`, `TerminalUiTest`, `NativeTerminalTest` and
opt-in `TerminalScrollProfileTest` as starting points. Add only focused missing
cases; warmed scrolling alone cannot validate live-TUI conversion costs.
Inspect the full transition rather than selected endpoint screenshots.

## Suggested follow-up boundaries

| Follow-up | Candidate files | Acceptance / rollback boundary |
| --- | --- | --- |
| Adopt latest appearance | `AppRepository.kt` and focused attach test | Latest desired settings reach the exact new handle; preserve existing epochs and viewport behavior |
| Retire the whole gesture | `TerminalView.kt` or a small gesture helper and pixel/input regression | No owner switch or stale long-press after invalidation; new gestures still work |
| Compatible underline painter | Cell painter and focused API-specific rendering test | Supported underline shapes have hardware/software evidence at the declared API floor |
| Improve keyboard-driven TUI resize | Existing IME/grid, frame handoff and row renderer owners | Early reliable target, latest intent convergence, prompt matching handoff, compatible row reuse and measured arrival-to-layout improvement |
| Preserve row content identities if profiling warrants | Core/client/native projection and repository row delivery as evidence dictates | Snapshot equivalence, exact frozen Copy, bounded shared allocations, color/geometry invalidation; no wire change |
| Close transition evidence gap | Geometry/presentation owners only if reproduced evidence requires changes | Continuous newly exposed content within available history; no invented rows or completion heuristic |

Do not bundle these into a broad refactor or a renderer replacement. Reconverge
the implementation PRD/design before modifying shared row/cache ownership or
wire contracts. This plan creates no child tasks and performs no release.
