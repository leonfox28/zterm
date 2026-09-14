# Android architecture, implementation and terminal rendering review

## Goal

Reassess the Android app and identify evidence-backed architecture improvements,
implementation defects, and better approaches to terminal diff rendering and TUI
presentation. Give the user a prioritized, actionable basis for follow-up work.

The current planning priority is a consistent local response to keyboard-driven
resizing: move/clip available displayed rows immediately, then reconcile incoming
remote content against what is actually displayed after that movement. Preserve
unchanged visual content regardless of whether the program is a shell or TUI,
the network is slow, or the transport update is a snapshot or delta.

## Background

- The user subsequently requests comparison with established Android terminal/
  SSH apps. Released Termux/ConnectBot source and official Termius/JuiceSSH/Mosh
  material are assessed in [research/android-terminal-peers.md](research/android-terminal-peers.md).
  On 2026-09-10 the user approves adopting the useful findings: unchanged-row
  projection reuse, display-paced preparation, and evaluation of compatible text runs.
  The implementation boundary is `research/peer-adoption.md`.

- On 2026-09-10 the user approves one IME movement, early-target and handoff
  policy for both live screens, with a shared visible-caret top-edge constraint.
  The bounded change and host resize compatibility limits are recorded in
  [research/unified-ime.md](research/unified-ime.md); it supersedes the older
  main-screen caret/scheduling exceptions below. Continue emulator-only work.

- Review baseline: `a2b518e61544642bb4537935e8fd6c192dff607b` on main (v0.1.31).
- Android consumes the host-authoritative semantic protocol through the shared
  Rust client; Kotlin owns platform UI/input (`docs/android.md:3`).
- Related tasks are `09-06-android-app` and
  `09-07-terminal-presentation-continuity`. The latter records accepted scrolling
  improvements and a separate unresolved keyboard-close continuity check.
- The first review's evidence and finding severities belong to
  [research/review.md](research/review.md); candidate architecture and tradeoffs
  belong to [design.md](design.md).
- The resize-focused source trace and concrete proposal are recorded in
  [research/tui-ime-resize.md](research/tui-ime-resize.md).
- The approved first build and its regression/packaging evidence are in
  [research/first-build.md](research/first-build.md). Real Herdr/phone acceptance
  remains open for user evaluation of this increment.
- The user's subsequent phone feedback identifies visible-cursor TUI content
  below the caret being clipped during IME opening. The screen-anchoring boundary
  correction is specified in [research/tui-bottom-anchor.md](research/tui-bottom-anchor.md).
- After that correction, the user confirms local movement but observes a one-row
  downward jump to the old caret-anchored image before the correct repaint. The
  actual model reproduction and next Android handoff increment are recorded in
  [research/resize-handoff.md](research/resize-handoff.md).
- The user reports that handoff is substantially better, with slight unevenness
  remaining during opening, and requests early resize during closing too. The
  measured redundant outer composition, scoped layout correction, and real-IME
  validation of both existing target directions are in
  [research/ime-motion.md](research/ime-motion.md).
- The user finds no further perceptible improvement from that layout cleanup.
  A physical-device trace identifies substantial RenderThread text/background
  replay despite row display-list reuse. The bounded row-pixel cache experiment,
  its focused checks, phone build and measurement limits are recorded in
  [research/ime-phone-trace.md](research/ime-phone-trace.md).
- The row-layer build feels a little better to the user. They report a tiny
  keyboard overshoot/return at opening completion, take the phone away and ask
  for emulator-only work. API 36 and 37 local IME/candidate checks do not reproduce
  that motion; the user explicitly defers it. Keep it recorded in
  [research/ime-end-motion.md](research/ime-end-motion.md), while continuing
  attribution of the separately measured final-drawing cost on the emulator.
- The user accepts the necessity review: retain the core resize fixes, bounded
  GPU row layers and small panel-only layout scope. The next local optimization
  removes empty ASCII-space glyph work from the existing painter, supported by
  paired cold-recording measurements and unchanged hardware pixels; see
  [research/row-painter.md](research/row-painter.md). The phone remains unavailable,
  and its tiny endpoint overshoot stays deferred.
- User clarification on 2026-09-09 places the visible stall **after keyboard
  motion completes**, rather than primarily inside the keyboard animation.
  This is a user observation; app/host build IDs and timing are not yet captured.
  The original View submitted remote resize after the animation-end pre-draw;
  the later handoff increment overlaps known final alternate-screen targets with
  the animation in both directions.
- The next clarification explicitly chooses local movement followed by comparison
  with the moved presentation as the desired general behavior. Early remote
  resize is a complementary latency optimization, not a prerequisite for that
  behavior. The user asks for an assessment of row versus character granularity;
  the proposal recommends rows first and defers cell-run refinement to evidence.

## Scope

- Android UI, repository, lifecycle, input and native bridge ownership.
- Host semantic publication, shared client snapshot/delta application, Android
  projection, frame handoff, row caching and Canvas submission.
- Shell and full-screen TUI correctness, wide/combined text, colors/styles,
  cursor, scrolling, selection, resize/IME and interrupted/recovered rendering.
- Existing tests and prior Android/presentation task evidence, including open
  acceptance gaps; distinguish historical observations from current findings.

## Requirements

- **ARCH-1:** Map the actual architecture and data flow; assess state ownership,
  coupling, concurrency, bounds and opportunities to simplify.
- **IMPL-1:** Identify concrete defects with triggers, code anchors, impact and
  focused validation; distinguish proven defects from unverified risks.
- **DIFF-1:** Evaluate transport deltas, projected row reuse and Android rendering
  separately. Explain correctness requirements and compare practical alternatives
  using repository evidence and primary technical sources where necessary.
- **TUI-1:** Include alternate-screen programs, synchronized output, rapid updates,
  geometry transitions, cursor/style-only changes and input/gesture consistency.
- **PLAN-1:** Record priorities, validation gaps and bounded follow-up options.
- **RESIZE-1:** On both keyboard opening and closing, move/clip available local
  content with the current viewport without waiting for remote progress. Retain
  the actually displayed baseline through delay and reversal, and reduce the
  observed wait for the TUI's useful new layout.
- **RESIZE-2:** Treat rendering reuse independently of network snapshot/delta
  encoding. Reuse compatible unchanged content across height changes while
  still displaying all real TUI layout/content changes correctly.
- **RESIZE-3:** Evaluate overlapping remote resize/redraw with keyboard motion
  using a reliably known target grid, with latest-target handling on reversal.
  Preserve actual usable PTY dimensions, input/IME continuity, coordinate fences,
  bounded retained state and forward progress.
- **RESIZE-4:** Base acceptance on full transitions and measured time from
  keyboard arrival to the intended visible TUI layout. Use a neutral TUI fixture
  plus Herdr; endpoint screenshots and cached-scroll drawing time are insufficient.
- **RESIZE-5:** Compare each eligible remote presentation with the moved local
  presentation in the same visible coordinate system. Reuse equal row content
  even if its old ordinal differs. Include text, width and resolved render styles;
  update cursor/selection/preedit independently. Adopt current source authority
  even when the visual difference is empty. Never use the locally moved display
  as the wire delta-application baseline.
- **RESIZE-6:** Use the same reconciliation policy for shell/TUI, fast/slow
  transport and snapshots/deltas. Missing content, genuine full-screen changes,
  width/font changes, unmarked intermediate output and cache loss still require
  correct presentation; this policy cannot promise a small dirty area every time
  or dictate which pixels the platform compositor touches.
- **RESIZE-7:** Distinguish cached drawing commands from retained row pixels.
  Accept additional bounded row layers only with measured replay reduction,
  foreground memory accounting and hardware pixel/cache-loss checks. Preserve
  dynamic overlays, complete-frame drawing and the API 26–28/software fallback.
- **RESIZE-8:** Use one height-difference movement, visible-caret top boundary,
  early endpoint scheduling and retained-frame handoff for live main/alternate
  screens. A screen switch still invalidates the old presentation authority.
  Preserve authoritative final layout when the child/model actually changes
  row placement; local anchoring does not redefine remote terminal semantics.

- **PREP-1:** Export only rows that differ from the preceding prepared window;
  preserve equal Kotlin row objects, including moved rows, without reusing old
  input/coordinate authority or changing wire semantics/cache bounds.
- **PREP-2:** Combine ordinary content bursts before expensive row preparation
  using display cadence. Mandatory native installation/ACK/input fences continue;
  authority changes, hidden-window progress and final states remain deliverable.
- **PAINT-1:** Evaluate compatible equal-width text with paired recording/pixel
  measurements. Retain only a justified implementation; reject it when control
  scenes regress or evidence cannot establish a reliable benefit.

## Acceptance Criteria

- [x] Architecture and end-to-end rendering flow are documented with source anchors (ARCH-1).
- [x] Findings state severity, evidence level, impact, root-cause owner and validation (IMPL-1).
- [x] Diff alternatives state compatibility, complexity, likely benefit and what
      must be measured before claiming a performance improvement.
      This includes an actual-code cursor-only allocation probe (DIFF-1).
- [x] TUI and interruption scenarios have an explicit coverage/gap matrix (TUI-1).
- [x] Recommendations preserve working behavior and identify independent follow-up work (PLAN-1).

These checkboxes record review-artifact completion and explicit evidence gaps. They do
not mark proposed fixes, whole-transition visual behavior or device performance
as accepted. [implement.md](implement.md) separates completed review checks from
unperformed runtime validation. On 2026-09-09 the user explicitly approved the
latest local-movement/row-reuse proposal for a first implementation: shell already
largely behaves correctly, TUI needs the improvement, and they want a build to try.

The resize-focused extension has these additional acceptance criteria:

- [ ] The resize submission, remote/model progress, TUI output, projection and
      platform submission stages are distinguished with correlated evidence.
- [x] A concrete design covers early reliable target discovery, overlapping
      remote work, retained presentation and reuse across compatible height changes.
- [x] Both IME directions and reversals have a defined plan for observable correctness and latency
      checks; no arbitrary all-device FPS or zero-network-latency guarantee is made.
- [x] The design specifies the moved-display comparison baseline, row-first
      granularity, source-authority separation and the limits of universal reuse
      (RESIZE-5, RESIZE-6).
- [ ] Runtime checks verify local motion with delayed/no new remote content,
      reuse after changed row ordinals, zero/sparse/dense visual differences and
      final semantic equivalence through rapid reversals (RESIZE-1, RESIZE-5).
- [x] The user reviews the resulting implementation scope before product edits.

Source ordering is established; correlated runtime timings and execution of the
resize checks remain pending. No additional product preference is needed to
complete this investigation. Actual target-insets reliability, timing and API
fallback behavior are engineering validation items, not facts delegated to the user.

## Constraints and out of scope

- The latest follow-up also authorizes bounded preparation improvements and a measured text-run
  evaluation documented in `research/peer-adoption.md`. Dependency migrations, release and the
  three unrelated implementation defects remain outside this increment.
- Preserve the host-authoritative terminal and existing transport/security model
  when evaluating incremental improvements; justify any proposed replacement.
- Use disposable test state where needed. Do not affect the user's running host
  sessions or daemon. Do not equate source/tests with physical-device acceptance.
- Maintain one review task; propose follow-up tasks only after concrete findings.
- Character/code-point patching, a new wire schema and a renderer replacement are
  outside the proposed first implementation. Further granularity must respect
  terminal wide/continuation cells and combining text, with measured benefit.
